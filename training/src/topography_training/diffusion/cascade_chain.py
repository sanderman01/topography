"""Chain the cascade levels: L1 (128 m) -> L2 (32 m) -> L3 (4 m) from a layout.

python -m topography_training.diffusion.cascade_chain --l1-run <dir> --l2-run <dir> --l3-run <dir>
    (--release <context release.json> --region <id> | --layout <l0-dir>) --out <new-dir> [options]

The layout (envelope, relief, classes) is either derived from a real GLO-30 region, as an
author's L0 would give it, with snow/ice and vegetation from the same region; or an
authored L0 from `topography world layout`, with a class per cell, its lakes, and
snow/ice and vegetation from its snowline, treeline and forest share. L1 and L2 cover the
whole field; L3 the whole field too, or a central square with `--l3-km` (0 stops after L2). Terrain continues under water: after
each level the sea is the ground below 0 m connected to the field edge. After L1 the
land is made to drain (lakes kept, other depressions carved), and streams traced on the
L2 surface are water for L3.
Writes l1/l2/l3 hillshades, the real region for comparison, arrays.npz and a 4 m world
output (manifest.json + layers) for `topography export heightmap`.
"""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import json
import sys
import time
from pathlib import Path

import numpy as np

from . import cascade, hydrology, layout, legend, paint
from .cascade_eval import shade
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


def _load(run: Path, device):
    from safetensors.torch import load_file

    from .model import Denoiser

    config = json.loads((run / "config.json").read_text())
    model = Denoiser(config["arch"], config["conditioning"]).to(device).eval()
    model.load_state_dict(load_file(str(_latest(run)), device=str(device)))
    return model, config


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
    water: hydrology.Water
    snow: np.ndarray | None  # real 32 m cover, or None to derive it from the surface
    vegetation: np.ndarray | None
    reference: tuple[str, np.ndarray, float, np.ndarray]  # picture name, surface, cell, water
    crs: str
    origin: tuple[float, float]  # north-west corner of the field
    marks_from: np.ndarray  # 128 m surface whose valleys and peaks are the L1 landmarks
    l0: layout.L0 | None = None
    cover_sampler: object = None

    def classes_at(self, k: int):
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
        return np.array([hydrology.WATER.get(c, hydrology.WATER[0]).dry_share > 0
                         for c in range(NUM_CLASSES)])[self.classes_at(k)]

    def craters_at(self, k: int):
        """Painted crater cells, whose hollows always stay dry (authored layouts)."""
        if isinstance(self.classes, int):
            return None
        return np.isin(self.classes_at(k), CRATER_CLASSES)

    def sinks_at(self, k: int):
        """Cells whose dry basins swallow streams (karst); None where there are none."""
        sinks = np.array([hydrology.WATER.get(c, hydrology.WATER[0]).sinks for c in range(NUM_CLASSES)])
        if isinstance(self.classes, int):
            return np.True_ if sinks[self.classes] else None
        return sinks[self.classes_at(k)]

    def streams_at(self, k: int, water: hydrology.Water):
        """Stream start area and width: the class profile per cell for authored layouts."""
        if self.l0 is None or water is not self.water:
            return water.stream_km2, water.stream_width
        profiles = [hydrology.WATER.get(c, hydrology.WATER[0]) for c in range(NUM_CLASSES)]
        c = self.classes_at(k)
        return (np.array([w.stream_km2 for w in profiles], np.float32)[c],
                np.array([w.stream_width for w in profiles], np.float32)[c])


def _region_source(release: Path, region: str) -> Source | None:
    regions = cascade.load_release(release, {region})
    if not regions:
        return None
    r = regions[0]
    meta = next(m for m in json.loads(release.read_text())["regions"] if m["id"] == region)
    f1 = cascade.l1_fields(r)
    sea = r.water | cascade.open_sea(np.nan_to_num(r.height))
    return Source(region, f1.coarse, f1.extra, r.class_id, hydrology.WATER.get(r.class_id, hydrology.WATER[0]),
                  r.snow, r.vegetation, ("real.png", np.nan_to_num(r.height), 32.0, sea), meta["crs"],
                  tuple(meta["origin"]), f1.target)


def _layout_source(l0: layout.L0) -> Source:
    """Envelope from the authored mid surface widened by its relief; water amounts are the
    land-weighted class profiles."""
    lo, hi = cascade.envelope(l0.mid)
    lo, hi = lo - l0.relief / 2, hi + l0.relief / 2
    land = l0.mid >= 0
    share = np.bincount(l0.cls[land], minlength=NUM_CLASSES) / max(int(land.sum()), 1)
    profiles = [hydrology.WATER.get(c, hydrology.WATER[0]) for c in range(NUM_CLASSES)]
    mean = {f: float(sum(share[c] * getattr(profiles[c], f) for c in range(NUM_CLASSES)))
            for f in ("lake_share", "tarn_share", "stream_km2", "stream_width", "dry_share")}
    extent = l0.cell_m * l0.mid.shape[0]
    return Source(l0.spec["name"], ((lo + hi) / 2).astype(np.float32), [((hi - lo) / 2048.0).astype(np.float32)],
                  l0.cls, hydrology.Water(**mean), None, None,
                  ("layout.png", l0.mid, l0.cell_m, (l0.mid < 0) | l0.lake), LAYOUT_CRS, (0.0, extent), l0.mid, l0)


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


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    for level in ("l1", "l2", "l3"):
        p.add_argument(f"--{level}-run", type=Path, required=True)
    p.add_argument("--release", type=Path, help="context release with the layout region")
    p.add_argument("--region", help="derive the layout from this real region")
    p.add_argument("--layout", type=Path, help="authored L0 (`topography world layout`) instead of --release/--region")
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--l3-km", type=int,
                   help="side of a central L3 square in units of 1,024 m (power of two); default the whole world; 0 stops after L2")
    p.add_argument("--steps", type=int, default=12)
    p.add_argument("--batch", type=int, default=16)
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--guidance", type=float, default=1.0, help="class guidance scale (1: off)")
    p.add_argument("--no-exemplars", action="store_true", help="authored layouts: do not hold real exemplars")
    # Water amounts default to the class profiles (hydrology.WATER).
    p.add_argument("--lake-share", type=float, help="share of land kept as lakes after L1")
    p.add_argument("--tarn-share", type=float, help="share of land kept as small lakes after L2")
    p.add_argument("--stream-km2", type=float, help="drainage area where streams begin")
    args = p.parse_args(argv)
    if (args.layout is None) == (args.release is None or args.region is None):
        p.error("give --layout, or --release and --region")

    import torch

    if args.out.exists():
        print("error: output directory exists", file=sys.stderr)
        return 2
    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    src = _layout_source(layout.load(args.layout)) if args.layout else _region_source(args.release, args.region)
    if src is None:
        print(f"error: region {args.region} not in the release", file=sys.stderr)
        return 2
    overrides = {"lake_share": args.lake_share, "tarn_share": args.tarn_share, "stream_km2": args.stream_km2}
    overrides = {k: v for k, v in overrides.items() if v is not None}
    water = dataclasses.replace(src.water, **overrides) if overrides else src.water
    args.out.mkdir(parents=True)
    times = {}

    # L1: the whole field from its layout, then drainage: the biggest depressions become
    # lakes, the rest are carved through like real valleys. Authored lakes are kept.
    t = time.time()
    model, config = _load(args.l1_run, device)
    level = cascade.LEVELS["l1"]
    if config["conditioning"] not in (5, 6, 7):
        print("error: the L1 run uses a retired layout", file=sys.stderr)
        return 2
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
    h128, lake128, level128, _ = hydrology.drain(h128, cascade.open_sea(h128), water.lake_share, 1.0,
                                                 keep_at=authored, dry_share=water.dry_share, dry_at=src.dry_at(1),
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
    model, config = _load(args.l2_run, device)
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
    sea32 = cascade.open_sea(h32)
    lake32 &= ~sea32
    h32[lake32] = lake_level32[lake32]
    raw32 = h32.copy()
    # Small lakes (tarns, kettles) from L2's own hollows; the rest are carved through.
    h32, tarn32, _, dry32 = hydrology.drain(h32, sea32 | lake32, water.tarn_share, 1.0, tarns=True,
                                            dry_share=water.dry_share, dry_at=src.dry_at(4),
                                            dry_keep_at=src.craters_at(4))
    sea32 = cascade.open_sea(h32)
    lake32 = (lake32 | tarn32) & ~sea32
    snow32, veg32 = src.cover(h32, 4)
    snow32, veg32 = snow32 & ~sea32, veg32 & ~sea32
    # Flow passes through lakes and dry basins to the sea; only karst basins swallow it.
    sinks = src.sinks_at(4)
    sinks32 = dry32 & sinks if sinks is not None else np.zeros_like(dry32)
    stream32, parent32, width32 = hydrology.streams(h32, sea32 | lake32, 32.0, *src.streams_at(4, water),
                                                    outlet=sea32 | sinks32)
    pos32 = hydrology.smooth_paths(stream32, parent32, width32)
    times["l2_s"] = time.time() - t
    print(f"L2 {h32.shape} in {times['l2_s']:.0f} s; tarns {tarn32.mean():.2%}, dry basins {dry32.mean():.2%}, streams {stream32.sum()} cells", flush=True)
    _reach(src, h32, 32.0, args.out, "l2")
    ref_name, ref_h, ref_cell, ref_water = src.reference
    _png(args.out / ref_name, ref_h, ref_cell, ref_water)
    if args.l3_km == 0:
        _png(args.out / "l1.png", h128, 128, sea128 | lake128)
        _png(args.out / "l2.png", h32, 32, sea32 | lake32 | stream32)
        np.savez_compressed(args.out / "arrays.npz", l1=h128, l2=h32, l2_raw=raw32)
        (args.out / "manifest.json").write_text(json.dumps({
            "kind": "cascade_chain_l2", "source": src.name, "steps": args.steps, "seed": args.seed,
            "water": vars(water), "times": times,
            "runs": {k: str(getattr(args, f"{k}_run").resolve()) for k in ("l1", "l2")}}, indent=1))
        print(f"wrote {args.out}", flush=True)
        return 0

    # L3: a central square from the L2 surface (with a margin for the smoothing). Sea,
    # lakes and streams are water, as in its lidar training data.
    t = time.time()
    level = cascade.LEVELS["l3"]
    model, config = _load(args.l3_run, device)
    # Default: the whole world, as the layout specifies; 16 -> 512 cells of 32 m, 4,096 of 4 m.
    n32 = h32.shape[0] if args.l3_km is None else args.l3_km * 1024 // 32
    m = 16
    y0 = x0 = (h32.shape[0] - n32) // 2
    # The smoothing margin repeats the edge where the square reaches the field's edge.
    patch = np.pad(h32, m, mode="edge")[y0 : y0 + n32 + 2 * m, x0 : x0 + n32 + 2 * m]
    coarse = cascade.refine_surface(level, patch)[m * 8 : -m * 8, m * 8 : -m * 8]
    sl = np.s_[y0 : y0 + n32, x0 : x0 + n32]
    sea4 = cascade.refine_mask(sea32[sl], 8, 6.0)
    lake4 = cascade.refine_mask(lake32[sl], 8, 6.0) & ~sea4
    level4 = _spread(_nearest(h32[sl], 8), _nearest(lake32[sl], 8), 12)
    stream4 = hydrology.draw_streams(stream32, parent32, width32, 8, 32.0, y0, x0, n32, pos32) & ~sea4 & ~lake4
    if src.l0 is not None:  # at 4 m from the layout, not 32 m steps
        snow4, veg4 = src.cover(coarse, 32, y0 * 32.0, x0 * 32.0)
    else:
        snow4, veg4 = _nearest(snow32[sl], 8), _nearest(veg32[sl], 8)
    extra = [np.where(sea4 | lake4 | stream4, 255, 0).astype(np.uint8), cascade.soft_u8(snow4, 1.0),
             cascade.soft_u8(veg4, 1.0)]
    classes32 = src.classes_at(4)
    classes4 = classes32 if isinstance(classes32, int) else _nearest(classes32[sl], 8)
    n_classes, model4 = _model_classes(config, classes4)
    extra = extra + _forms(config, classes4, coarse.shape, src)
    field = cascade.LazyField(level, coarse, extra, model4, n_classes)
    h4 = _sample(model, config, coarse, field, args.steps, args.seed + 2, args.batch,
                 guidance=args.guidance).astype(np.float32)
    del field, extra
    w = np.clip(-coarse / L3_SEABED_M, 0, 1)
    h4 = ((1 - w) * h4 + w * coarse).astype(np.float32)
    sea4 = cascade.open_sea(h4)
    lake4 &= ~sea4
    stream4 &= ~sea4
    h4[lake4] = level4[lake4]
    # The refined coast can leave mouths on land: continue them, then make every bed descend.
    sinks4 = _nearest(sinks32[sl], 8)
    mouths = hydrology.extend_mouths(h4, sea4 | lake4, stream4, sinks4)
    stream4 |= mouths
    h4, lowered = hydrology.carve_streams(h4, stream4, sea4 | lake4)
    print(f"L3 streams: {mouths.sum()} cells added to reach water, {lowered} bed cells lowered", flush=True)
    water4 = sea4 | lake4 | stream4
    if src.l0 is not None:  # cover of the final surface
        snow4, veg4 = src.cover(h4, 32, y0 * 32.0, x0 * 32.0)
    times["l3_s"] = time.time() - t
    print(f"L3 {h4.shape} in {times['l3_s']:.0f} s", flush=True)

    # Pictures: generated levels and the real region at the same framing.
    _png(args.out / "l1.png", h128, 128, sea128 | lake128)
    _png(args.out / "l2.png", h32, 32, sea32 | lake32 | stream32)
    _png(args.out / "l2-l3-area.png", h32[sl], 32, (sea32 | lake32 | stream32)[sl], 1024)
    _png(args.out / "l3.png", h4, 4, water4, 2048)
    c = h4.shape[0] // 2
    _png(args.out / "l3-detail.png", h4[c - 512 : c + 512, c - 512 : c + 512], 4, water4[c - 512 : c + 512, c - 512 : c + 512], 1024)
    np.savez_compressed(args.out / "arrays.npz", l1=h128, l2=h32, l2_raw=raw32, l3=h4)

    # A 4 m world output for `topography export heightmap` and `export view`.
    s = 32.0
    ox, oy = src.origin[0] + x0 * s, src.origin[1] - y0 * s
    world_input = args.out / "world-input"
    world_input.mkdir()
    (world_input / "world.json").write_text(json.dumps(
        {"kind": "cascade_chain_input", "name": f"chain-{src.name}", "crs": src.crs, "origin": [ox, oy]}, indent=1))
    layers = {
        "elevation.f32": h4.astype("<f4").tobytes(),
        "water.u8": np.where(water4, 255, 0).astype(np.uint8).tobytes(),
        "snow-ice.u8": np.where(snow4, 2, 1).astype(np.uint8).tobytes(),
        "vegetation.u8": np.where(veg4, 2, 1).astype(np.uint8).tobytes(),
    }
    if not isinstance(classes4, int):  # authored layouts: the class per 4 m cell, for L4
        layers["classes.u8"] = classes4.astype(np.uint8).tobytes()
    if src.l0 is not None:  # biomes for viewing and export, borders warped like the cover
        layers["biome.u8"] = src.sampler().classes(y0 * 32.0, x0 * 32.0, *h4.shape, 4.0).tobytes()
    digests = {}
    for name, data in layers.items():
        (args.out / name).write_bytes(data)
        digests[name] = hashlib.sha256(data).hexdigest()
    (args.out / "manifest.json").write_text(json.dumps({
        "schema_version": 1, "kind": "world_preview", "world_input": str(world_input.resolve()),
        "spacing_m": 4.0, "size": [h4.shape[1], h4.shape[0]],
        "method": f"cascade chain L1 -> L2 -> L3 from {'an authored' if src.l0 else 'a derived'} layout", "water_fill": "ground continues under the sea; lakes flat at their spill level",
        "source": src.name, "layout": str(args.layout.resolve()) if args.layout else None,
        "class_id": src.classes if isinstance(src.classes, int) else None, "steps": args.steps, "water": vars(water), "times": times,
        "runs": {k: str(getattr(args, f"{k}_run").resolve()) for k in ("l1", "l2", "l3")},
        "layers": digests}, indent=1))
    print(f"wrote {args.out}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
