"""Chain the cascade levels: L1 (128 m) -> L2 (32 m) -> L3 (4 m) from a layout.

python -m topography_training.diffusion.cascade_chain --l1-run <dir> --l2-run <dir> --l3-run <dir>
    (--release <context release.json> --region <id> | --layout <l0-dir>) --out <new-dir> [options]
python -m topography_training.diffusion.cascade_chain --collage <collage-dir> --l3-run <dir> --out <new-dir> [options]

The layout (envelope, relief, classes) is either derived from a real GLO-30 region, as an
author's L0 would give it, with snow/ice and vegetation from the same region; or an
authored L0 from `topography world layout`, with a class per cell, its lakes, and
snow/ice and vegetation from its snowline, treeline and forest share; or a collage
(`--collage`, collage.py) whose 32 m surface is L2 and its 128 m mean L1, so no L1 or L2
model runs, with its dithered 32 m classes for L3 and a description.json. L1 and L2 cover the
whole field; L3 the whole field too, or a square with `--l3-km` (0 stops after L2), central
or around `--l3-centre`. L3 runs in bands of rows with cross-faded halos (as cascade_l4), into
memory-mapped files, so host memory stays bounded; rerunning with the same arguments resumes
after the last finished band. Windows of deep sea only are not diffused at L3.
Terrain continues under water: after
each level the sea is the ground below 0 m connected to the field edge. After L1 the
land is made to drain (lakes kept, other depressions carved). After L2 and L3 the hydrology
v3 level (water.run.run_level: rain, breach, erosion, water, checks, review) sets the ground
and water (32 m, then 4 m on the kept raw L3 surface, guided by the 32 m level); L3 refines
the 32 m level's final ground with its water as input. A failed 32 m level stops before L3;
a failed 4 m level writes manifest-failed.json and exits 1.
Writes l1/l2/l3 hillshades, the real region for comparison, arrays.npz, the hydrology
levels (hydrology/run-32m, run-4m; hydrology/32m and 4m link their outputs) and a 4 m world
output (manifest.json + layers) for `topography export heightmap`.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import sys
import time
from pathlib import Path

import numpy as np

from . import cascade, description, hydrology, layout, legend, paint
from .cascade_eval import shade
from .data import smooth
from .evaluate import _latest

NUM_CLASSES = legend.NUM_CLASSES

# The local plane of authored worlds (as `topography world sketch`): metres east and north.
LAYOUT_CRS = ('ENGCRS["Topography sketch plane",EDATUM["Topography sketch"],CS[Cartesian,2],'
              'AXIS["easting (X)",east,ORDER[1],LENGTHUNIT["metre",1]],'
              'AXIS["northing (Y)",north,ORDER[2],LENGTHUNIT["metre",1]]]')
# Landmarks are held above this scale (cells of the level held: 3 x 128 m at L1); craters,
# calderas and real exemplars at 1 cell (128 m), so a 600-800 m crater is not blurred away.
HOLD_SIGMA_CELLS = 3.0
FINE_HOLD_SIGMA_CELLS = 1.0
CRATER_CLASSES = (4,)
# L3 learned land only (lidar stops at the shore); deeper than this its seabed is the L2 surface.
L3_SEABED_M = 5.0
# Judgment: L1 lakes get a bed sloping down from their shore at L2, so 32 m hydrology keeps
# them. Breaching cuts at most 30 m (BREACH_MAX_M), so beds that reach 40 m keep large lakes
# whatever the budget, with margin for a spill lower than the L1 level.
LAKE_BED_SLOPE = 0.1
LAKE_BED_MAX_M = 40.0
# 32 m rivers drawn at 4 m for L3's water channel: blur (4 m cells) and threshold.
RIVER_SIGMA_4M = 8.0
RIVER_LEVEL = 0.3
# L3 runs in bands of rows; its 4 m fields are computed with this margin of 32 m cells
# (wider than every smoothing, the form blend and the cover's terrain measures).
L3_MARGIN = 16
FINISH_ROWS = 1024  # 4 m rows a block when writing world layers
COVER_MARGIN_4M = 96  # about 400 m around a block for the cover's terrain measures
# L1 128 m lake picker for authored and region layouts (not collages; B22): one share for every
# class, dry basins in classes whose lake-to-catchment ratio is at most L1_DRY_R.
L1_LAKE_SHARE = 0.01
L1_DRY_SHARE = 0.01
L1_DRY_R = 0.05


def _lake_bed(lake: np.ndarray, level: np.ndarray, h: np.ndarray, cell_m: float) -> np.ndarray:
    """Ground under lake cells at least as deep below the level as the shore distance gives."""
    from scipy import ndimage

    depth = np.clip(ndimage.distance_transform_edt(lake) * cell_m * LAKE_BED_SLOPE, 1.0, LAKE_BED_MAX_M)
    return np.where(lake, np.minimum(h, level - depth), h).astype(np.float32)


def _load(run: Path, device, class_runs: Path | None = None):
    """A run's model; with `class_runs`, a mixture of its per-class experts (cN subdirs) over it."""
    from safetensors.torch import load_file

    from .model import Denoiser, load_mixture

    config = json.loads((run / "config.json").read_text())
    if class_runs is not None:
        runs = {c: class_runs / f"c{c}" for c in range(1, NUM_CLASSES) if (class_runs / f"c{c}" / "config.json").exists()}
        print(f"{run.name}: mixture of {len(runs)} per-class runs from {class_runs}", flush=True)
        return load_mixture(run, runs, device), config
    model = Denoiser(config["arch"], config["conditioning"]).to(device).eval()
    model.load_state_dict(load_file(str(_latest(run)), device=str(device)))
    return model, config


def _class_runs(args) -> dict[str, str]:
    """Per-class expert directories used, by level."""
    return {k: str(v.resolve()) for k in ("l1", "l2", "l3") if (v := getattr(args, f"{k}_class_runs")) is not None}


def _runs(args, levels) -> dict[str, str]:
    """Model runs used, by level (a collage skips L1 and L2)."""
    return {k: str(v.resolve()) for k in levels if (v := getattr(args, f"{k}_run")) is not None}


def _collage_path(args) -> str | None:
    return str(args.collage.resolve()) if args.collage else None


def _model_classes(config, classes):
    """Legend v3 classes (one id or one per cell) as the run's model classes."""
    n, lut = legend.model_lut(config)
    return n, (int(lut[classes]) if isinstance(classes, (int, np.integer)) else lut[classes])


def _forms(config, classes, shape, src) -> list[np.ndarray]:
    """Form channels for runs that take them (forms.py): the run's per-class table over the
    class map, dune direction from the layout's wind; [] for runs without."""
    from . import forms

    if config["conditioning"] not in forms.BASE:
        return []
    _, model = _model_classes(config, classes)
    if np.ndim(model) == 0:
        model = np.full(shape, model, np.uint8)
    wind = src.l0.spec.get("wind_from_deg") if src.l0 is not None else None
    f = forms.from_classes(model, config["forms"]["table"], wind, config["forms"]["sigma"] * config["crop"])
    return list(f.astype(np.float16))


def _condition(level, coarse, extra, classes, config) -> np.ndarray:
    """Like cascade.condition, but filled in place so big fields are not copied. `classes`
    is one legend v3 class id or a class per cell."""
    h, w = coarse.shape
    n, classes = _model_classes(config, classes)
    cond = np.zeros((4 + len(extra) + n, h, w), np.float32)
    cond[0] = coarse
    cond[1], cond[2] = cascade.slopes(coarse, level.cell_m)
    cond[3] = coarse / 4096.0
    for i, e in enumerate(extra):
        cond[4 + i] = cascade.decode(e)
    cascade.one_hot(cond[4 + len(extra) :], classes)
    return cond


def _sample(model, config, coarse, cond, steps, seed, batch, holds=(), guidance=1.0):
    """Sample a level; each hold (target surface, weight 0-1, sigma cells) guides the shape
    of held cells above that scale toward the target (see model.sample_field)."""
    import torch

    from .model import sample_field

    size = config["crop"]
    if isinstance(cond, np.ndarray):
        cond = torch.from_numpy(cond)[None]
    hold = [(torch.from_numpy(((target - coarse) / config["residual_scale_m"]).astype(np.float32))[None, None],
             torch.from_numpy(weight.astype(np.float32))[None, None], sigma) for target, weight, sigma in holds]
    out = sample_field(model, cond, steps=steps, window=size, stride=size // 2,
                       batch=batch, seed=seed, hold=hold or None, guidance=guidance)
    return coarse + out[0, 0].numpy() * config["residual_scale_m"]


def _spread(level: np.ndarray, mask: np.ndarray, cells: int) -> np.ndarray:
    """Lake levels carried `cells` cells beyond their mask, for rounded shores."""
    own = np.where(mask, level, -np.inf).astype(np.float32)
    out = own
    for _ in range(cells):
        out = np.maximum(out, hydrology._maxpool3(out))
    # Lake cells keep their own level; a higher neighbouring lake must not raise them.
    return np.where(mask, own, out)


def _nearest(a: np.ndarray, k: int) -> np.ndarray:
    return np.repeat(np.repeat(a, k, axis=0), k, axis=1)


def _png(path: Path, h: np.ndarray, cell_m: float, sea: np.ndarray, size: int = 1536):
    from PIL import Image

    img = Image.fromarray(shade(h, cell_m, sea))
    img.resize((size, size), Image.LANCZOS if h.shape[0] > size else Image.NEAREST).save(path)


@dataclasses.dataclass
class Source:
    """What the chain generates from: a real region's derived layout or an authored L0."""

    name: str
    coarse: np.ndarray  # L1 mid surface (128 m)
    extra: list[np.ndarray]  # L1 relief channel
    classes: int | np.ndarray  # one class, or a uint8 class per 128 m cell
    snow: np.ndarray | None  # real 32 m cover, or None to derive it from the surface
    vegetation: np.ndarray | None
    reference: tuple[str, np.ndarray, float, np.ndarray]  # picture name, surface, cell, water
    crs: str
    origin: tuple[float, float]  # north-west corner of the field
    marks_from: np.ndarray  # 128 m surface whose valleys and peaks are the L1 landmarks
    l0: layout.L0 | None = None
    cover_sampler: object = None
    # A collage (--collage): its 32 m surface stands in for L1 and L2, with its dithered
    # 32 m classes and region map.
    l2: np.ndarray | None = None
    classes32: np.ndarray | None = None
    region32: np.ndarray | None = None
    provenance32: np.ndarray | None = None  # the collage's provenance sidecar (B10)

    def classes_at(self, k: int):
        """Classes on the 128/k m grid; a collage's from its dithered 32 m classes (k of 4 and up)."""
        if self.classes32 is not None:
            if k % 4:
                raise ValueError(f"a collage has no classes finer than 32 m on the 128/{k} m grid")
            return self.classes32 if k == 4 else _nearest(self.classes32, k // 4)
        return self.classes if isinstance(self.classes, int) else _nearest(self.classes, k)

    def sampler(self):
        if self.cover_sampler is None:
            from .cover import Sampler

            self.cover_sampler = Sampler(self.l0)
        return self.cover_sampler

    def cover(self, h: np.ndarray, k: int, y0_m: float = 0.0, x0_m: float = 0.0) -> tuple[np.ndarray, np.ndarray]:
        """Snow/ice and vegetation on a surface at 128/k m whose corner lies (y0_m, x0_m) from
        the layout's; layout grids are read smoothly (see cover.Sampler)."""
        if self.l0 is None:
            return self.snow, self.vegetation
        return self.sampler().cover(h, y0_m, x0_m, self.l0.cell_m / k)

    def dry_at(self, k: int):
        """Cells of classes with dry basins (authored layouts; a real region has one class)."""
        if isinstance(self.classes, int):
            return None
        from ..water import table as water_table

        return np.array([water_table.r(c) <= L1_DRY_R for c in range(NUM_CLASSES)])[self.classes_at(k)]

    def craters_at(self, k: int):
        """Painted crater cells, whose hollows always stay dry (authored layouts)."""
        if isinstance(self.classes, int):
            return None
        return np.isin(self.classes_at(k), CRATER_CLASSES)

    @property
    def wind(self) -> float | None:
        """Wind direction (degrees from) of an authored layout, for the orographic rain."""
        wind = self.l0.spec.get("wind_from_deg") if self.l0 is not None else None
        return float(wind) if wind is not None else None

    def class_grid(self, k: int, sl, shape) -> np.ndarray:
        """uint8 class per cell of a window of the 128/k m grid (hydrology grids)."""
        c = self.classes_at(k)
        return np.full(shape, c, np.uint8) if isinstance(c, int) else c[sl].astype(np.uint8)


def _region_source(release: Path, region: str) -> Source | None:
    regions = cascade.load_release(release, {region})
    if not regions:
        return None
    r = regions[0]
    meta = next(m for m in json.loads(release.read_text())["regions"] if m["id"] == region)
    f1 = cascade.l1_fields(r)
    sea = r.water | cascade.open_sea(np.nan_to_num(r.height))
    return Source(region, f1.coarse, f1.extra, r.class_id, r.snow, r.vegetation, ("real.png", np.nan_to_num(r.height), 32.0, sea), meta["crs"],
                  tuple(meta["origin"]), f1.target)


def _layout_source(l0: layout.L0, coarse: np.ndarray | None = None) -> Source:
    """Envelope from the authored mid surface widened by its relief (or a given L1 `coarse`
    without a relief channel)."""
    if coarse is None:
        lo, hi = cascade.envelope(l0.mid)
        lo, hi = lo - l0.relief / 2, hi + l0.relief / 2
        coarse, extra = ((lo + hi) / 2).astype(np.float32), [((hi - lo) / 2048.0).astype(np.float32)]
    else:
        extra = []
    extent = l0.cell_m * l0.mid.shape[0]
    return Source(l0.spec["name"], coarse, extra, l0.cls, None, None,
                  ("layout.png", l0.mid, l0.cell_m, (l0.mid < 0) | l0.lake), LAYOUT_CRS, (0.0, extent), l0.mid, l0)


def _collage_source(d: Path) -> Source:
    """A collage (collage.py): its layout L0, 128 m `l1` and 32 m `l2` with seabed, and the
    dithered `class32.u8`, `region32.u8` and the provenance sidecar `provenance32.u8`."""
    meta = json.loads((d / "collage.json").read_text())
    arrays = np.load(d / "arrays.npz")
    l1, l2 = arrays["l1"].astype(np.float32), arrays["l2"].astype(np.float32)

    def grid(entry):
        return np.fromfile(d / meta[entry]["file"], np.uint8).reshape(l2.shape)

    src = _layout_source(layout.load(d), l1)
    prov_path = d / "provenance32.u8"
    if not prov_path.exists():
        raise SystemExit(f"error: {d} has no provenance32.u8; run "
                         f"`python -m topography_training.diffusion.collage provenance {d}`")
    prov = np.fromfile(prov_path, np.uint8).reshape(l2.shape)
    return dataclasses.replace(src, reference=("collage.png", l2, 32.0, cascade.open_sea(l2)),
                               marks_from=l1, l2=l2, classes32=grid("class32"), region32=grid("region32"),
                               provenance32=prov)


def _reach(src: Source, h: np.ndarray, cell_m: float, out: Path, name: str):
    """Tier reachability of a generated level against the authored layout."""
    if src.l0 is None:
        return
    text, tiers, warnings = layout.reach(h, cell_m, src.l0.spec)
    (out / f"reach-{name}.txt").write_text(text)
    layout.reach_png(h, cell_m, tiers, src.l0.spec, out / f"reach-{name}.png")
    print(f"{name} reachability: {warnings} warning(s), see reach-{name}.txt", flush=True)


def _lakes(src: Source, lake: np.ndarray, level: np.ndarray, out: Path):
    """Where each authored lake ended up after L1: its kept hollow's area and level."""
    if src.l0 is None:
        return
    from scipy import ndimage

    labels, _ = ndimage.label(lake)
    lines = []
    for lk in src.l0.spec.get("lakes", []):
        cells = layout.lake_cells(lk, src.l0.cell_m, lake.shape[0])
        ids = np.unique(labels[cells & (labels > 0)])
        if not len(ids):
            lines.append(f"  WARN {lk['id']:<13} missing: the generated surface has no hollow there")
            continue
        got = np.isin(labels, ids)
        km2, surface = got.sum() * src.l0.cell_m**2 / 1e6, float(np.nanmean(level[got]))
        ok = surface <= lk["surface_m"] + 50
        lines.append(f"  {'ok  ' if ok else 'WARN'} {lk['id']:<13} {km2:5.2f} km² at {surface:.0f} m "
                     f"(spec {cells.sum() * src.l0.cell_m**2 / 1e6:.2f} km² at {lk['surface_m']:.0f} m)")
    text = "Authored lakes after L1\n" + "\n".join(lines) + "\n"
    (out / "lakes-l1.txt").write_text(text)
    print(text, end="", flush=True)


def _link_dir(link: Path, target: str) -> None:
    """A relative directory symlink (replaced if it points elsewhere)."""
    if link.is_symlink() and str(link.readlink()) == target:
        return
    if link.is_symlink():
        link.unlink()
    link.symlink_to(target, target_is_directory=True)


def _region_layer(grid: Path, region32: np.ndarray | None) -> None:
    """Collage region ids beside a grid's provenance (32 m cells), for the blend-zone figures."""
    if region32 is not None:
        np.ascontiguousarray(region32, np.uint8).tofile(grid / "region.u8")


def _l1_l2(args, src: Source, device, hyd: Path, times: dict):
    """L1 and L2 (or a collage's surfaces) and the 32 m hydrology; returns the 128 m surface,
    its sea and lakes, and the 32 m surface before hydrology."""
    if src.l2 is not None:  # a collage is the L2 surface, with L1 its 128 m mean
        h128, h32 = src.coarse, src.l2.copy()
        sea128 = cascade.open_sea(h128)
        lake128 = np.zeros(h128.shape, bool)
        raw32 = src.l2
    else:
        # L1: the whole field from its layout, then drainage: the biggest depressions become
        # lakes, the rest are carved through like real valleys. Authored lakes are kept.
        t = time.time()
        model, config = _load(args.l1_run, device, args.l1_class_runs)
        level = cascade.LEVELS["l1"]
        if config["conditioning"] not in (5, 6, 7):
            raise SystemExit("error: the L1 run uses a retired layout")
        extra = src.extra
        if config["conditioning"] in (6, 7):  # landmarks of the authored (or real) surface, all kept
            extra = extra + cascade.mark_channels(cascade.landmarks(src.marks_from, 128.0))
        extra = extra + _forms(config, src.classes, src.coarse.shape, src)
        cond = _condition(level, src.coarse, extra, src.classes, config)
        holds = []
        if src.l0 is not None:  # authored landmarks, and sea toward the map border
            target, border = paint.border_hold(src.l0.mid, 128.0)
            holds.append((target, np.maximum(src.l0.hold, border), HOLD_SIGMA_CELLS))
            # Held craters keep their painted shape at 128 m; real exemplars fill painted
            # calderas and cinder-cone fields (catalog/exemplars).
            fine = np.where(np.isin(src.l0.cls, CRATER_CLASSES), src.l0.hold, 0.0).astype(np.float32)
            fine_target = target
            if not args.no_exemplars:
                from . import exemplar

                fine_target, w, used = exemplar.compose(src.l0, target, seed=args.seed)
                fine = np.maximum(fine, w)
                for line in used:
                    print(f"exemplar: {line}", flush=True)
            if fine.any():
                holds.append((fine_target, fine, FINE_HOLD_SIGMA_CELLS))
        h128 = _sample(model, config, src.coarse, cond, args.steps, args.seed, args.batch, holds, args.guidance)
        if src.l0 is not None:  # the map border stays sea
            h128 = np.minimum(h128, paint.border_limit(h128.shape[0], 128.0)).astype(np.float32)
        authored = src.l0.lake if src.l0 else None
        # Craters, dolines and interdune hollows stay dry basins here too, not lakes or carved valleys.
        h128, lake128, level128, _ = hydrology.drain(h128, cascade.open_sea(h128), L1_LAKE_SHARE, 1.0,
                                                     keep_at=authored, dry_share=L1_DRY_SHARE, dry_at=src.dry_at(1),
                                                     dry_keep_at=src.craters_at(1))
        sea128 = cascade.open_sea(h128)
        lake128 &= ~sea128
        times["l1_s"] = time.time() - t
        print(f"L1 {h128.shape} in {times['l1_s']:.0f} s; sea {sea128.mean():.1%}, lakes {lake128.mean():.1%}", flush=True)
        _reach(src, h128, 128.0, args.out, "l1")
        _lakes(src, lake128, level128, args.out)

        # L2: the whole region, seabed included; lakes are the only water it is told about.
        t = time.time()
        level = cascade.LEVELS["l2"]
        model, config = _load(args.l2_run, device, args.l2_class_runs)
        coarse = cascade.refine_surface(level, h128)
        lake32 = cascade.refine_mask(lake128, 4, 2.0)
        lake_level32 = _spread(_nearest(np.nan_to_num(level128), 4), _nearest(lake128, 4), 4)
        snow_in, veg_in = src.cover(coarse, 4)
        # Authored rivers are water for L2, which learned valley floors around rivers.
        river32 = layout.rivers(src.l0.spec, 32.0, coarse.shape[0]) & (coarse >= 0) if src.l0 else lake32 & False
        extra = [np.where(lake32 | river32, 255, 0).astype(np.uint8), cascade.soft_u8(snow_in, 1.0),
                 cascade.soft_u8(veg_in, 1.0)] + _forms(config, src.classes_at(4), coarse.shape, src)
        h32 = _sample(model, config, coarse, _condition(level, coarse, extra, src.classes_at(4), config), args.steps,
                      args.seed + 1, args.batch, guidance=args.guidance)
        if src.l0 is not None:
            h32 = np.minimum(h32, paint.border_limit(h32.shape[0], 32.0)).astype(np.float32)
        lake32 &= ~cascade.open_sea(h32)
        h32 = _lake_bed(lake32, lake_level32, h32, 32.0)
        raw32 = h32.copy()
        times["l2_s"] = time.time() - t
    # Hydrology v3 at 32 m: rain, breach, erosion (provenance K), water, checks, review. Its
    # final ground is what L3 refines.
    from ..water import grids

    t = time.time()
    run32 = hyd / "run-32m"
    if not (run32 / "level.json").exists():
        g = grids.write_grid(run32 / "grid", h32, 32.0, src.origin, src.class_grid(4, np.s_[:, :], h32.shape),
                             wind_from_deg=src.wind, provenance=src.provenance32, provenance_cell_m=32.0)
        if src.provenance32 is not None:
            _region_layer(g, src.region32)
    code = _run_level(run32, river_km2=args.stream_km2)
    _link_dir(hyd / "32m", "run-32m/out")
    times["hydrology_32m_s"] = time.time() - t
    if code != 0:
        raise SystemExit(f"error: the 32 m water failed ({run32 / 'level.json'}); fix it before L3")
    return h128, sea128, lake128, raw32


def _run_level(run: Path, **kw) -> int:
    """water.run.run_level; a refused level stops the chain."""
    from ..water import run as water_run

    try:
        return water_run.run_level(run, **kw)
    except water_run.Refused as e:
        raise SystemExit(f"error: {e}") from e


def _l3(args, src: Source, w32: hydrology.Output, x0: int, y0: int, n32: int, device, out: Path, progress: dict,
        save) -> np.memmap:
    """L3 over the n32² square of 32 m cells at (x0, y0), in bands of rows north to south
    into a memory-mapped raw 4 m surface (resumable). Each band has a halo above and below,
    cross-faded into the band above; its starting noise is drawn per tile of the map grid."""
    import torch

    from .model import plan_windows, sample_field, world_noise

    level = cascade.LEVELS["l3"]
    model, config = _load(args.l3_run, device, args.l3_class_runs)
    window, scale = config["crop"], config["residual_scale_m"]
    stride = window // 2
    n4, band, halo = n32 * 8, args.l3_band_m // 4, args.l3_halo_m // 4
    m = L3_MARGIN
    first, last = args.l3_bands
    # 32 m inputs with a margin repeating the field's edge, read per band.
    h32, sea32, wet32, river32 = np.array(w32.ground), w32.kind("sea"), w32.wet, w32.kind("river")
    lake32 = wet32 & ~sea32 & ~river32
    pad = {k: np.pad(v, m, mode="edge") for k, v in (("h", h32), ("sea", sea32), ("lake", lake32), ("river", river32))}
    if src.l0 is None:
        snow32, veg32 = src.cover(h32, 4)
        pad["snow"], pad["veg"] = np.pad(snow32 & ~sea32, m, mode="edge"), np.pad(veg32 & ~sea32, m, mode="edge")
    cls32 = src.classes_at(4)
    if not isinstance(cls32, int):
        pad["cls"] = np.pad(cls32, m, mode="edge")
    del h32, sea32, wet32, river32, lake32
    n_classes, lut = legend.model_lut(config)
    path = out / "l3-raw.f32"
    raw = np.memmap(path, dtype="<f4", mode="r+" if path.exists() else "w+", shape=(n4, n4))
    bands = cascade.bands(n4, band, halo)
    win = progress.setdefault("l3_windows", {"total": 0, "skipped": 0, "guided": 0})
    for k in range(progress["l3_bands_done"], len(bands)):
        t = time.time()
        a, b = bands[k]
        g0, g1 = max(a - halo, 0), min(b + halo, n4)
        # 32 m rows and columns of the band with the margin (padded coordinates); fields at 4 m
        # are computed with it and cropped back.
        sl = np.s_[y0 + g0 // 8 : y0 + g1 // 8 + 2 * m, x0 : x0 + n32 + 2 * m]
        crop = np.s_[m * 8 : -m * 8, m * 8 : -m * 8]
        coarse_m = cascade.refine_surface(level, pad["h"][sl])
        coarse = np.ascontiguousarray(coarse_m[crop])
        sea4 = cascade.refine_mask(pad["sea"][sl], 8, 6.0)[crop]
        lake4 = cascade.refine_mask(pad["lake"][sl], 8, 6.0)[crop] & ~sea4
        river4 = (smooth(_nearest(pad["river"][sl], 8).astype(np.float32), RIVER_SIGMA_4M)[crop] > RIVER_LEVEL) \
            & ~sea4 & ~lake4
        if src.l0 is not None:  # at 4 m from the layout, not 32 m steps
            snow4, veg4 = (c[crop] for c in src.cover(coarse_m, 32, (y0 - m) * 32.0 + g0 * 4.0, (x0 - m) * 32.0))
        else:
            snow4, veg4 = (_nearest(pad[c][sl], 8)[crop] for c in ("snow", "veg"))
        del coarse_m
        classes_m = cls32 if isinstance(cls32, int) else _nearest(pad["cls"][sl], 8)
        classes4 = classes_m if isinstance(classes_m, int) else classes_m[crop]
        extra = [np.where(sea4 | lake4 | river4, 255, 0).astype(np.uint8), cascade.soft_u8(snow4, 1.0),
                 cascade.soft_u8(veg4, 1.0)]
        extra += [f[crop] for f in _forms(config, classes_m, (g1 - g0 + 16 * m, n4 + 16 * m), src)]
        del snow4, veg4, classes_m
        h = coarse.copy()
        note = "not diffused (outside --l3-bands)"
        if first <= k < last:
            guide, g = None, args.guidance
            if g != 1.0 and args.guidance_classes:
                guide = np.isin(classes4, args.guidance_classes)
                if guide.ndim == 0:
                    guide, g = None, (g if bool(guide) else 1.0)
            # L3 learned land only: deep sea ends as the coarse ground, so its windows are skipped.
            skip = coarse < cascade.DEEP_SEA_M
            plan = plan_windows(g1 - g0, n4, window, stride, skip, guide)
            win["total"] += len(plan[1])
            win["skipped"] += len(plan[2])
            win["guided"] += 0 if g == 1.0 else len(plan[1]) if plan[3] is None else len(plan[3] - plan[2])
            note = f"{len(plan[2])}/{len(plan[1])} windows deep sea"
            _, model4 = _model_classes(config, classes4)
            field = cascade.LazyField(level, coarse, extra, model4, n_classes)
            # Noise of the map's 4 m grid, so a square previews the whole map at its place.
            wy, wx = y0 * 8 + g0, x0 * 8
            noise = world_noise(args.seed + 2, wy, wy + g1 - g0, wx, wx + n4)
            res = sample_field(model, field, steps=args.steps, window=window, stride=stride, batch=args.batch,
                               noise=noise, guidance=g, plan=plan)
            h += res[0, 0].numpy() * scale
            del field, res, skip, guide, model4, noise
        del extra
        w = np.clip(-coarse / L3_SEABED_M, 0, 1)
        h = (1 - w) * h + w * coarse
        # L3 learned land only: under lakes (and, with --l3-river-clamp, rivers) the ground eases
        # down to the 32 m ground (B32; rivers left out by default, owner rule 2026-11-08).
        clamp = (lake4 | river4) if args.l3_river_clamp else lake4
        under = smooth(clamp.astype(np.float32), 2.0)
        h = (h - under * np.maximum(h - coarse, 0)).astype(np.float32)
        del under, clamp, w, coarse, sea4, lake4, river4
        cascade.write_band(raw, h, bands, k, g0, halo, out)
        del h
        progress["l3_band_s"].append(time.time() - t)
        progress["l3_bands_done"] = k + 1
        if torch.cuda.is_available():  # the largest of the run's processes (resumed runs have several)
            progress["times"]["peak_vram_gb"] = max(progress["times"].get("peak_vram_gb", 0.0),
                                                    round(torch.cuda.max_memory_reserved() / 2**30, 2))
        save()
        cascade.drop_overlaps(out, k)
        print(f"L3 band {k + 1}/{len(bands)} rows {a}-{b} in {progress['l3_band_s'][-1]:.0f} s, {note}", flush=True)
    del model
    return raw


def _rows(n: int):
    """Row blocks for whole-field passes over the 4 m world."""
    return [(r, min(r + FINISH_ROWS, n)) for r in range(0, n, FINISH_ROWS)]


def _write_layer(path: Path, blocks) -> str:
    """A layer written block by block (each a uint8 array of whole rows); its SHA-256."""
    with path.open("wb") as f:
        for block in blocks:
            np.ascontiguousarray(block, dtype=np.uint8).tofile(f)
    return hydrology.sha256(path)


def _preview(ground: np.ndarray, wet_rows, n: int, size: int) -> tuple[np.ndarray, np.ndarray, int]:
    """The 4 m surface and its water averaged to at most `size` cells a side; the factor."""
    k = max(1, n // size)
    h = np.concatenate([cascade.block_mean(np.asarray(ground[a:b]), k) for a, b in _rows(n)])
    wet = np.concatenate([cascade.block_mean(wet_rows(a, b).astype(np.float32), k) for a, b in _rows(n)]) > 0.5
    return h, wet, k


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    for level in ("l1", "l2", "l3"):
        p.add_argument(f"--{level}-run", type=Path, required=level == "l3",
                       help=None if level == "l3" else "required unless --collage")
        p.add_argument(f"--{level}-class-runs", type=Path,
                       help=f"directory of per-class expert runs (c1, c2, ...) mixed over --{level}-run")
    p.add_argument("--release", type=Path, help="context release with the layout region")
    p.add_argument("--region", help="derive the layout from this real region")
    p.add_argument("--layout", type=Path, help="authored L0 (`topography world layout`) instead of --release/--region")
    p.add_argument("--collage", type=Path, help="world collage (collage.py) as the L2 surface; no L1 or L2 models")
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--l3-km", type=int,
                   help="side of an L3 square in units of 1,024 m (power of two); default the whole world; 0 stops after L2")
    p.add_argument("--l3-centre", help="X,Y: centre of the --l3-km square in map metres from the north-west "
                   "corner (y south); default the map centre")
    p.add_argument("--l3-band-m", type=int, default=8192, help="L3 band height (multiple of 512 m)")
    p.add_argument("--l3-halo-m", type=int, default=1024, help="L3 halo above and below a band (multiple of 512 m)")
    p.add_argument("--l3-bands", default=None, help="A:B: diffuse only L3 bands A to B-1, the rest keep the "
                   "coarse surface (test runs)")
    p.add_argument("--steps", type=int, default=12)
    p.add_argument("--batch", type=int, default=16)
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--guidance", type=float, default=1.0, help="class guidance scale (1: off)")
    p.add_argument("--guidance-classes", type=int, nargs="+",
                   help="L3: legend classes whose windows get --guidance; other windows none (default: all)")
    p.add_argument("--no-exemplars", action="store_true", help="authored layouts: do not hold real exemplars")
    p.add_argument("--stream-km2", type=float, help="drainage area where 32 m rivers begin (default 10)")
    p.add_argument("--l3-river-clamp", dest="l3_river_clamp", action="store_true", default=False,
                   help="L3: also ease the ground to the 32 m surface under rivers (default: lakes and sea "
                        "only; owner rule 2026-11-08)")
    p.add_argument("--no-l3-river-clamp", dest="l3_river_clamp", action="store_false",
                   help="the default; kept so older commands still run")
    return p


def main(argv=None) -> int:
    p = parser()
    args = p.parse_args(argv)
    if sum((args.layout is not None, args.collage is not None, args.release is not None and args.region is not None)) != 1:
        p.error("give --layout, --collage, or --release and --region")
    if args.collage is None and (args.l1_run is None or args.l2_run is None):
        p.error("--l1-run and --l2-run are required without --collage")
    centre = None
    if args.l3_centre is not None:
        try:
            centre = tuple(float(v) for v in args.l3_centre.split(","))
        except ValueError:
            centre = ()
        if len(centre) != 2 or not args.l3_km:
            p.error("--l3-centre takes X,Y and needs --l3-km")
    if args.l3_band_m % 512 or args.l3_halo_m % 512 or not 0 < args.l3_halo_m <= args.l3_band_m:
        p.error("--l3-band-m and --l3-halo-m must be multiples of 512 m, the halo at most a band")
    try:
        args.l3_bands = (0, 1 << 30) if args.l3_bands is None else tuple(int(v) for v in args.l3_bands.split(":"))
        assert len(args.l3_bands) == 2
    except (ValueError, AssertionError):
        p.error("--l3-bands takes A:B")

    import torch

    # Rerunning with the same arguments resumes an unfinished L3.
    params = {k: str(v.resolve()) if isinstance(v, Path) else v for k, v in vars(args).items()}
    progress_path = args.out / "progress.json"
    progress = None
    if args.out.exists():
        if (args.out / "manifest.json").exists():
            print("error: output is already complete", file=sys.stderr)
            return 2
        if not progress_path.exists() or json.loads(progress_path.read_text())["params"] != json.loads(
                json.dumps(params)):
            print("error: output directory exists with other arguments", file=sys.stderr)
            return 2
        progress = json.loads(progress_path.read_text())
        if (args.out / "manifest-failed.json").exists() and progress.get("hydrology_4m_done"):
            print(f"error: output is complete but its 4 m water failed; after a fix, build and replace "
                  f"the pinned binary in {args.out / 'hydrology' / 'run-4m' / 'bin'} (delete it for a fresh "
                  f"level, or see terrain-cascade.md, Rerunning from a stage), rerun the level with "
                  f"`python -m topography_training.water.run level --run {args.out / 'hydrology' / 'run-4m'} "
                  f"--coarse {args.out / 'hydrology' / '32m'}`, then delete manifest-failed.json, set "
                  f"hydrology_4m_done false in progress.json and rerun this command (it resumes after "
                  f"L3 from l2.npz)", file=sys.stderr)
            return 2
    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    if args.collage:
        src = _collage_source(args.collage)
    elif args.layout:
        src = _layout_source(layout.load(args.layout))
    else:
        src = _region_source(args.release, args.region)
    if src is None:
        print(f"error: region {args.region} not in the release", file=sys.stderr)
        return 2
    hyd = args.out / "hydrology"

    def save():
        cascade.save_json(progress_path, progress)

    if progress is None:
        # Disk: world layers and raw L3 (44 B a 4 m cell), the 4 m water work (54 B) and
        # erosion routings (28 B), with margin.
        n4 = (src.coarse.shape[0] * 4 if args.l3_km is None else args.l3_km * 32) * 8
        if args.l3_km != 0:
            hydrology.check_disk(args.out, 130.0 * n4 * n4, "the chain")
        args.out.mkdir(parents=True)
        hyd.mkdir()
        progress = {"params": params, "times": {}, "l2_done": False, "l3_bands_done": 0, "l3_band_s": [],
                    "hydrology_4m_done": False}
        save()  # so a crash during L1, L2 or the 32 m level resumes (run_level resumes itself)
    if not progress.get("l2_done"):
        h128, sea128, lake128, raw32 = _l1_l2(args, src, device, hyd, progress["times"])
        _png(args.out / src.reference[0], *src.reference[1:])
        if args.l3_km != 0:  # kept for a resumed run
            np.savez(args.out / "l2.npz", h128=h128, sea128=sea128, lake128=lake128, raw32=raw32)
            progress["l2_done"] = True
            save()
    else:
        print(f"resuming after L3 band {progress['l3_bands_done']}", flush=True)
        f = np.load(args.out / "l2.npz")
        h128, sea128, lake128, raw32 = f["h128"], f["sea128"], f["lake128"], f["raw32"]
    times = progress["times"]
    w32 = hydrology.read(hyd / "32m")
    h32 = np.array(w32.ground)
    sea32, wet32, river32 = w32.kind("sea"), w32.wet, w32.kind("river")
    lake32 = wet32 & ~sea32 & ~river32
    count = {k: sum(b["kind"] == k for b in w32.bodies) for k in ("lake", "river")}
    print(f"L2 {h32.shape} " + ("from the collage" if src.l2 is not None else f"in {times.get('l2_s', 0):.0f} s")
          + f", hydrology {times['hydrology_32m_s']:.0f} s; {count['lake']} lakes ({lake32.mean():.2%}), "
          f"{count['river']} rivers", flush=True)
    _reach(src, h32, 32.0, args.out, "l2")
    if args.l3_km == 0:
        _png(args.out / "l1.png", h128, 128, sea128 | lake128)
        _png(args.out / "l2.png", h32, 32, wet32)
        np.savez_compressed(args.out / "arrays.npz", l1=h128, l2=h32, l2_raw=raw32)
        (args.out / "manifest.json").write_text(json.dumps({
            "kind": "cascade_chain_l2", "source": src.name, "steps": args.steps, "seed": args.seed,
            "water": {"model": "hydrology v3", "32m": json.loads((hyd / "run-32m" / "level.json").read_text())},
            "river_km2": args.stream_km2, "hydrology": {"32m": "hydrology/32m"}, "times": times,
            "runs": _runs(args, ("l1", "l2")), "collage": _collage_path(args),
            "class_runs": _class_runs(args)}, indent=1))
        print(f"wrote {args.out}", flush=True)
        return 0

    # L3: the whole world, as the layout specifies, or a square (16 km: 512 cells of 32 m,
    # 4,096 of 4 m). Sea, lakes and streams are water, as in its lidar training data.
    n32 = h32.shape[0] if args.l3_km is None else args.l3_km * 1024 // 32
    y0 = x0 = (h32.shape[0] - n32) // 2
    if centre is not None:  # kept inside the map
        x0, y0 = (int(np.clip(round(c / 32.0 - n32 / 2), 0, h32.shape[0] - n32)) for c in centre)
    n4, sl = n32 * 8, np.s_[y0 : y0 + n32, x0 : x0 + n32]
    raw = _l3(args, src, w32, x0, y0, n32, device, args.out, progress, save)
    times["l3_s"] = sum(progress["l3_band_s"])
    times["l3_band_s"] = progress["l3_band_s"]
    del w32

    # Hydrology v3 at 4 m on the kept raw L3 surface, guided by the 32 m level (coarse guide).
    raw.flush()
    del raw
    if not progress["hydrology_4m_done"]:
        from ..water import grids

        t = time.time()
        run4 = hyd / "run-4m"
        g = run4 / "grid"
        if not (g / "grid.json").exists() or "moisture" not in json.loads((g / "grid.json").read_text()):
            grids.write_grid(g, None, 4.0, src.origin, src.class_grid(4, sl, (n32, n32)),
                             offset_m=(x0 * 32.0, y0 * 32.0), classes_cell_m=32.0,
                             provenance=None if src.provenance32 is None else src.provenance32[sl],
                             provenance_cell_m=32.0, elevation_link=args.out / "l3-raw.f32", size=(n4, n4))
            if src.provenance32 is not None:
                _region_layer(g, None if src.region32 is None else src.region32[sl])
            grids.add_moisture(g, hyd / "32m")
        code4 = _run_level(run4, coarse=hyd / "32m")
        _link_dir(hyd / "4m", "run-4m/out")
        times["hydrology_4m_s"] = time.time() - t
        progress["hydrology_4m_done"] = True
        progress["hydrology_4m_passed"] = code4 == 0
        save()
    w4 = hydrology.read(hyd / "4m")
    ground = w4.ground

    def wet_rows(a, b):
        return np.isfinite(w4.surface[a:b])

    count = {k: sum(b["kind"] == k for b in w4.bodies) for k in ("lake", "river")}
    wet_share = sum(int(wet_rows(a, b).sum()) for a, b in _rows(n4)) / n4**2
    print(f"L3 {(n4, n4)} in {times['l3_s']:.0f} s, hydrology {times['hydrology_4m_s']:.0f} s; "
          f"{count['lake']} lakes, {count['river']} rivers, water {wet_share:.2%}", flush=True)

    # Pictures: generated levels; the 4 m surface averaged to at most 4,096 cells a side.
    _png(args.out / "l1.png", h128, 128, sea128 | lake128)
    _png(args.out / "l2.png", h32, 32, wet32)
    _png(args.out / "l2-l3-area.png", h32[sl], 32, wet32[sl], 1024)
    over, wover, k = _preview(ground, wet_rows, n4, 4096)
    _png(args.out / "l3.png", over, 4.0 * k, wover, 2048)
    del over, wover
    c = n4 // 2
    _png(args.out / "l3-detail.png", np.asarray(ground[c - 512 : c + 512, c - 512 : c + 512]), 4,
         wet_rows(c - 512, c + 512)[:, c - 512 : c + 512], 1024)
    # The 4 m surface is the world's elevation.f32.
    np.savez_compressed(args.out / "arrays.npz", l1=h128, l2=h32, l2_raw=raw32)

    # A 4 m world output for `topography export heightmap` and `export view`.
    s = 32.0
    ox, oy = src.origin[0] + x0 * s, src.origin[1] - y0 * s
    world_input = args.out / "world-input"
    world_input.mkdir(exist_ok=True)
    (world_input / "world.json").write_text(json.dumps(
        {"kind": "cascade_chain_input", "name": f"chain-{src.name}", "crs": src.crs, "origin": [ox, oy]}, indent=1))
    # Hydrology layers are hard links to its 4 m output (the 128 m moisture to the 32 m one).
    digests = hydrology.link(hyd / "4m", args.out, hydrology.WORLD_LAYERS)
    listing = hydrology.world_bodies(json.loads((hyd / "4m" / "bodies.json").read_text())["bodies"],
                                     "the 4 m hydrology's bodies; dry basins from dry_min_km2")
    (args.out / "bodies.json").write_text(json.dumps(listing, indent=1))
    digests["bodies.json"] = hydrology.sha256(args.out / "bodies.json")
    digests |= hydrology.link(hyd / "32m", args.out, ("moisture.f32",))
    moisture = json.loads((hyd / "32m" / "grid.json").read_text())["moisture"]
    desc = {}
    if src.region32 is not None:
        if n32 == h32.shape[0]:  # the whole map: the collage's description measured at 4 m
            d = description.measure(src.l0.spec, ground, 4.0, "l3", _nearest(src.region32, 8))
        else:  # on the carved 32 m, and the L3 square at 4 m
            d = description.measure(src.l0.spec, h32, 32.0, "l2", src.region32)
            d["finer"] = [{"level": "l3"} | description.measure_window(src.l0.spec, ground, 4.0,
                                                                       (x0 * 32.0, y0 * 32.0))]
        description.write(args.out, d)
        del d
        digests["description.json"] = hydrology.sha256(args.out / "description.json")
        desc = {"description": "description.json"}

    # Cover of the final surface, classes and water, written row block by row block.
    cls32 = src.classes_at(4)
    cover32 = None if src.l0 is not None else [c & ~sea32 for c in src.cover(h32, 4)]

    def cover_rows(which: int):
        for a, b in _rows(n4):
            if cover32 is not None:  # a real region's 32 m cover
                c32 = cover32[which]
                yield np.where(_nearest(c32[y0 + a // 8 : y0 + -(-b // 8), x0 : x0 + n32], 8)[: b - a], 2, 1)
                continue
            # Terrain measures need about 300 m of margin.
            top, bot = max(a - COVER_MARGIN_4M, 0), min(b + COVER_MARGIN_4M, n4)
            got = src.cover(np.asarray(ground[top:bot]), 32, y0 * 32.0 + top * 4.0, x0 * 32.0)[which]
            yield np.where(got[a - top : b - top], 2, 1)

    def class_rows():
        for a, b in _rows(n4):
            yield _nearest(cls32[y0 + a // 8 : y0 + -(-b // 8), x0 : x0 + n32], 8)[: b - a]

    digests["water.u8"] = _write_layer(args.out / "water.u8", (np.where(wet_rows(a, b), 255, 0) for a, b in _rows(n4)))
    digests["snow-ice.u8"] = _write_layer(args.out / "snow-ice.u8", cover_rows(0))
    digests["vegetation.u8"] = _write_layer(args.out / "vegetation.u8", cover_rows(1))
    if not isinstance(cls32, int):  # authored layouts: the class per 4 m cell, for L4
        digests["classes.u8"] = _write_layer(args.out / "classes.u8", class_rows())
    if src.classes32 is not None:  # a collage's dithered classes
        digests["biome.u8"] = _write_layer(args.out / "biome.u8", class_rows())
    elif src.l0 is not None:  # biomes for viewing and export, borders warped like the cover
        digests["biome.u8"] = _write_layer(args.out / "biome.u8", (
            src.sampler().classes(y0 * 32.0 + a * 4.0, x0 * 32.0, b - a, n4, 4.0) for a, b in _rows(n4)))
    prov = {}
    if src.provenance32 is not None:
        digests["provenance.u8"] = _write_layer(args.out / "provenance.u8", [src.provenance32[sl]])
        prov = {"provenance": {"file": "provenance.u8", "cell_m": 32.0}}
    levels = {k: json.loads((hyd / f"run-{k}" / "level.json").read_text()) for k in ("32m", "4m")}
    passed = all(v["passed"] for v in levels.values())
    name = "manifest.json" if passed else "manifest-failed.json"
    (args.out / name).write_text(json.dumps({
        "schema_version": 1, "kind": "world_preview", "world_input": str(world_input.resolve()),
        "spacing_m": 4.0, "size": [n4, n4],
        "method": "cascade chain L3 over a collage's 32 m surface" if args.collage else
        f"cascade chain L1 -> L2 -> L3 from {'an authored' if src.l0 else 'a derived'} layout",
        "water_fill": hydrology.WATER_FILL,
        "hydrology": {"32m": "hydrology/32m", "4m": "hydrology/4m", "bodies": "bodies.json"},
        "moisture": moisture | {"file": "moisture.f32"}, **desc, **prov,
        "source": src.name, "layout": str((args.layout or args.collage).resolve()) if src.l0 else None,
        "l3_window_m": [x0 * 32.0, y0 * 32.0, n32 * 32.0, n32 * 32.0],
        "l3_bands": {"band_m": args.l3_band_m, "halo_m": args.l3_halo_m,
                     "count": len(cascade.bands(n4, args.l3_band_m // 4, args.l3_halo_m // 4)),
                     "windows": progress.get("l3_windows", {})},
        "class_id": src.classes if isinstance(src.classes, int) else None, "steps": args.steps,
        "guidance": args.guidance, "guidance_classes": args.guidance_classes,
        "water": {"model": "hydrology v3", **levels}, "l3_river_clamp": args.l3_river_clamp,
        "river_km2": args.stream_km2, "times": times,
        "runs": _runs(args, ("l1", "l2", "l3")), "collage": _collage_path(args),
        "class_runs": _class_runs(args),
        "layers": digests}, indent=1))
    # The raw L3 surface always stays: the 4 m water can be run again on it without diffusing.
    if passed:  # only now the run cannot be resumed any more
        (args.out / "l2.npz").unlink(missing_ok=True)
        cascade.drop_overlaps(args.out, 1 << 30)
    print(f"wrote {args.out}" + ("" if passed else " (FAILED: see hydrology/run-*/level.json)"), flush=True)
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
