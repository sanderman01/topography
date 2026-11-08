"""Banded L4 generation: refine a 4 m world (cascade chain output) to a raw 1 m surface.

python -m topography_training.diffusion.cascade_l4 --run <l4 run dir> --world <4 m world dir> --out <dir>
    [--km N] [options]

The 1 m field is generated in bands of rows with a halo above and below, so host memory
stays bounded at any world size. Bands share their starting noise where they overlap
and are cross-faded over the halo. Water cells keep the refined 4 m ground (L4 learned land
only); `--keep-river-residual` keeps the residual under 4 m rivers (zeroed under lakes and sea
only). Bands and windows whose 4 m ground is all deep sea water keep it without diffusion.
`--guidance-classes` limits class guidance to windows holding those classes. `--km` with
`--centre` places the square anywhere. Output layers are memory-mapped and filled band by
band; rerunning with the same arguments resumes after the last finished band.
Writes a raw 1 m world (manifest.json of kind world_raw, elevation-raw.f32, snow-ice.u8,
vegetation.u8) plus overview.png and detail.png hillshades. It has no water:
`python -m topography_training.water.run world --source <it> --out <world>` makes the water world.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
import time
from pathlib import Path
from types import SimpleNamespace

import numpy as np

from . import cascade, hydrology
from .data import smooth, soft_cover

MARGIN_4M = 16  # 4 m context cells around a band for the smoothing
WATER_SIGMA_M = 3.0  # rounds 4 m water corners at 1 m
DISK_BYTES = 6  # raw surface 4 B, snow and vegetation 1 B each
# The diffused surface: the input of `water.run world`.
RAW = "elevation-raw.f32"


def _rows_4m(a: np.ndarray, r0: int, r1: int, c0: int, c1: int) -> np.ndarray:
    """A 4 m block with indices beyond the field repeating its edge."""
    rows = np.clip(np.arange(r0, r1), 0, a.shape[0] - 1)
    cols = np.clip(np.arange(c0, c1), 0, a.shape[1] - 1)
    return a[np.ix_(rows, cols)]


def raw_manifest(out: Path, n: int, world_input: Path, source_world: Path, params: dict, extra: dict) -> dict:
    """The manifest of an L4 raw world: the diffused surface and cover, no water (ruling B20)."""
    digests = {name: hydrology.sha256(out / name) for name in (RAW, "snow-ice.u8", "vegetation.u8")}
    return {
        "schema_version": 1, "kind": "world_raw", "world_input": str(world_input.resolve()),
        "spacing_m": 1.0, "size": [n, n],
        "method": "banded L4 over a cascade chain 4 m world; water: python -m topography_training.water.run world",
        **extra, "source_world": str(source_world.resolve()), "params": params,
        "hydrology_input": {"file": RAW, "sha256": digests[RAW],
                            "rerun": "python -m topography_training.water.run world --source <this world> --out <new world>"},
        "layers": digests,
    }


def _finish(c: SimpleNamespace, extra: dict) -> None:
    """Pictures, world input and the raw manifest of a finished L4 run."""
    from .cascade_chain import _png

    n = c.n
    over = np.concatenate([cascade.block_mean(np.asarray(c.raw[y : y + 1024]), 8) for y in range(0, n, 1024)])
    _png(c.out / "overview.png", over, 8.0, np.zeros(over.shape, bool), 2048)
    m, r = n // 2, min(512, n // 2)
    _png(c.out / "detail.png", np.asarray(c.raw[m - r : m + r, m - r : m + r]), 1.0, np.zeros((2 * r, 2 * r), bool), 1024)
    src = json.loads((Path(c.world["world_input"]) / "world.json").read_text())
    ox, oy = src["origin"][0] + c.col4 * 4.0, src["origin"][1] - c.row4 * 4.0
    world_input = c.out / "world-input"
    world_input.mkdir(exist_ok=True)
    (world_input / "world.json").write_text(json.dumps(src | {"name": f"{src.get('name', 'world')}-l4",
                                                              "origin": [ox, oy]}, indent=1))
    if (c.world_dir / "moisture.f32").exists():
        hydrology.link(c.world_dir, c.out, ("moisture.f32",))
        extra["moisture"] = c.world["moisture"]
    if "description" in c.world:
        shutil.copy(c.world_dir / c.world["description"], c.out / "description.json")
        shutil.copy(c.world_dir / "description.md", c.out / "description.md")
        extra["description"] = "description.json"
    params = c.params | {"window_4m": [c.col4, c.row4]}
    manifest = raw_manifest(c.out, n, world_input, c.world_dir, params, extra | {"class_id": c.class_id,
                                                                               "band_s": c.band_s})
    cascade.save_json(c.out / "manifest.json", manifest)


def _setup(params: dict, out: Path) -> SimpleNamespace:
    """What the bands need from the run's parameters (as `main` checked them)."""
    world_dir = Path(params["world"])
    world = json.loads((world_dir / "manifest.json").read_text())
    coarse_dir = (world_dir / world["hydrology"]["4m"]).resolve()
    class_id = params["class_id"]
    per_cell = class_id == "per cell"
    n4 = world["size"][0]
    c4 = n4 if params["km"] is None else params["km"] * 256
    col4, row4 = params.get("window_4m", [(n4 - c4) // 2] * 2)
    g4 = json.loads((coarse_dir / "grid.json").read_text())
    off_m = g4.get("offset_m") or [0.0, 0.0]
    n = c4 * 4

    def read4(name, dtype):
        # Memory-mapped: a band reads only its rows.
        mm = np.memmap(world_dir / name, dtype=dtype, mode="r", shape=(n4, n4))
        return mm[row4 : row4 + c4, col4 : col4 + c4]

    c = SimpleNamespace(
        out=out, world_dir=world_dir, world=world, class_id=class_id, per_cell=per_cell, n4=n4, c4=c4,
        col4=col4, row4=row4, n=n, params=params, bands=cascade.bands(n, params["band_m"], params["halo_m"]),
        east=off_m[0] + col4 * 4.0, south=off_m[1] + row4 * 4.0, read4=read4,
        classes4=read4("classes.u8", np.uint8) if per_cell else None)
    if params.get("keep_river_residual"):
        # B32: the residual is zeroed under 4 m lakes and sea only; bodies from the raster (M5).
        bodies = json.loads((world_dir / world["hydrology"].get("bodies", "bodies.json")).read_text())["bodies"]
        c.body4 = read4("water-body.u32", "<u4")
        c.lake_sea_ids = np.array([b["id"] for b in bodies if b["kind"] in ("sea", "lake")], np.int64)
    return c


def _layers(c: SimpleNamespace, mode: str) -> None:
    def layer(name, dtype):
        return np.memmap(c.out / name, dtype=dtype, mode=mode, shape=(c.n, c.n))

    c.raw = layer(RAW, "<f4")
    c.snow_out, c.veg_out = layer("snow-ice.u8", np.uint8), layer("vegetation.u8", np.uint8)


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--run", type=Path, required=True, help="L4 run directory")
    p.add_argument("--class-runs", type=Path, help="directory of per-class expert runs (c1, c2, ...) mixed over --run")
    p.add_argument("--world", type=Path, required=True, help="4 m world output (cascade_chain)")
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--km", type=int, help="square side in units of 1,024 m (power of two); default whole world")
    p.add_argument("--centre", help="X,Y: centre of the --km square in map metres from the north-west corner "
                   "(y south); default the 4 m world's centre")
    p.add_argument("--class-id", type=int, help="terrain class when the world manifest has none")
    p.add_argument("--band-m", type=int, default=1024, help="band height; 1,024 m keeps a 64 km run under about 10 GB")
    p.add_argument("--halo-m", type=int, default=256)
    p.add_argument("--steps", type=int, default=12)
    p.add_argument("--stride", type=int, default=192)
    p.add_argument("--batch", type=int, default=16)
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--guidance", type=float, default=1.0, help="class guidance scale (1: off)")
    p.add_argument("--guidance-classes", type=int, nargs="+",
                   help="legend classes whose windows get --guidance; other windows none (default: all)")
    p.add_argument("--keep-river-residual", action="store_true",
                   help="keep the diffused residual under 4 m rivers (zeroed under lakes and sea only; B32 A/B)")
    args = p.parse_args(argv)
    if args.centre is not None and args.km is None:
        p.error("--centre needs --km")

    import torch

    from .cascade_chain import _load
    from .model import plan_windows, sample_field, world_noise

    world = json.loads((args.world / "manifest.json").read_text())
    if world.get("kind") != "world_preview" or world.get("spacing_m") != 4.0:
        print("error: --world must be a 4 m world output", file=sys.stderr)
        return 2
    if "4m" not in world.get("hydrology", {}):
        print("error: the world has no 4 m hydrology output; regenerate it with cascade_chain", file=sys.stderr)
        return 2
    coarse_dir = (args.world / world["hydrology"]["4m"]).resolve()
    class_id = args.class_id if args.class_id is not None else world.get("class_id")
    per_cell = class_id is None and (args.world / "classes.u8").exists()
    if per_cell:
        class_id = "per cell"
    if class_id is None:
        print("error: the world has no class_id or classes.u8; pass --class-id", file=sys.stderr)
        return 2
    if args.band_m % 64 or args.halo_m % 64 or args.halo_m > args.band_m:
        print("error: band and halo must be multiples of 64 m, the halo no wider than a band", file=sys.stderr)
        return 2
    n4 = world["size"][0]
    c4 = n4 if args.km is None else args.km * 256
    if c4 > n4 or c4 & (c4 - 1):
        print("error: --km must give a power-of-two square inside the world", file=sys.stderr)
        return 2
    # The world's 4 m grid in the map plane (an L3 square sits at its offset).
    g4 = json.loads((coarse_dir / "grid.json").read_text())
    off_m = g4.get("offset_m") or [0.0, 0.0]
    col4 = row4 = (n4 - c4) // 2
    if args.centre is not None:
        try:
            cx, cy = (float(v) for v in args.centre.split(","))
        except ValueError:
            print("error: --centre takes X,Y", file=sys.stderr)
            return 2
        col4, row4 = (int(np.clip(round((c - o) / 4.0 - c4 / 2), 0, n4 - c4)) for c, o in ((cx, off_m[0]), (cy, off_m[1])))
    n = c4 * 4

    params = {"run": str(args.run.resolve()), "world": str(args.world.resolve()), "km": args.km,
              "class_id": class_id, "band_m": args.band_m, "halo_m": args.halo_m, "steps": args.steps,
              "stride": args.stride, "seed": args.seed, "keep_river_residual": args.keep_river_residual}
    if args.guidance != 1.0:
        params["guidance"] = args.guidance
        if args.guidance_classes:
            params["guidance_classes"] = sorted(args.guidance_classes)
    if args.centre is not None:
        params["window_4m"] = [col4, row4]
    if args.class_runs is not None:
        params["class_runs"] = str(args.class_runs.resolve())
    progress_path = args.out / "progress.json"
    if args.out.exists():
        if (args.out / "manifest.json").exists():
            print("error: output is already complete", file=sys.stderr)
            return 2
        if not progress_path.exists() or json.loads(progress_path.read_text())["params"] != params:
            print("error: output directory exists with other arguments", file=sys.stderr)
            return 2
        progress = json.loads(progress_path.read_text())
        mode = "r+"
    else:
        hydrology.check_disk(args.out, DISK_BYTES * n * n, "the L4 run")
        args.out.mkdir(parents=True)
        progress = {"params": params, "bands_done": 0, "band_s": []}
        mode = "w+"

    c = _setup(params, args.out)
    # Diffusion writes the raw surface (cross-faded between bands); water.run reads it.
    _layers(c, mode)
    raw, h4 = c.raw, c.read4("elevation.f32", "<f4")
    water4, snow4, veg4 = c.read4("water.u8", np.uint8), c.read4("snow-ice.u8", np.uint8), c.read4("vegetation.u8", np.uint8)
    classes4 = c.classes4

    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    model, config = _load(args.run, device, args.class_runs)
    level = cascade.LEVELS["l4"]
    if config.get("level") != "l4":
        print("error: --run is not an L4 run", file=sys.stderr)
        return 2
    scale, window = config["residual_scale_m"], config["crop"]
    from . import legend

    n_classes, lut = legend.model_lut(config)
    m = MARGIN_4M
    bands = c.bands
    done = progress["bands_done"]
    print(f"{n} x {n} at 1 m in {len(bands)} bands; resuming after band {done}" if done else
          f"{n} x {n} at 1 m in {len(bands)} bands", flush=True)
    east, south = c.east, c.south

    def save():
        cascade.save_json(progress_path, progress)

    c.band_s = progress["band_s"]
    for k in range(done, len(bands)):
        t = time.time()
        a, b = bands[k]
        g0, g1 = max(a - args.halo_m, 0), min(b + args.halo_m, n)
        # 4 m rows covering the band with a margin; the 1 m fields are cropped back to it.
        r0, r1 = g0 // 4 - m, g1 // 4 + m
        crop = np.s_[m * 4 : -m * 4, m * 4 : -m * 4]
        ground4 = _rows_4m(h4, r0, r1, -m, c4 + m)
        # A band of deep sea only (with its margin) keeps the refined 4 m ground: L4 learned land only.
        deep = bool((ground4 < cascade.DEEP_SEA_M).all())
        coarse = cascade.refine_surface(level, ground4)[crop]
        del ground4
        water = cascade.refine_mask(_rows_4m(water4, r0, r1, -m, c4 + m) == 255, 4, WATER_SIGMA_M,
                                    keep_thin=True)[crop]
        lakes_sea = None
        if args.keep_river_residual:
            lakes_sea = cascade.refine_mask(np.isin(_rows_4m(c.body4, r0, r1, -m, c4 + m), c.lake_sea_ids), 4,
                                            WATER_SIGMA_M, keep_thin=True)[crop]

        def cover(mask):
            rows = _rows_4m(mask, r0, r1, -m, c4 + m) == 2
            return soft_cover(np.repeat(np.repeat(rows, 4, 0), 4, 1))[crop]

        snow, veg = cover(snow4), cover(veg4)
        classes = class_id
        if per_cell:
            classes = np.repeat(np.repeat(_rows_4m(classes4, r0, r1, -m, c4 + m), 4, 0), 4, 1)[crop]
        guide = None
        if args.guidance != 1.0 and args.guidance_classes:
            guide = np.isin(classes, args.guidance_classes)
            guide = (None if guide else False) if guide.ndim == 0 else guide
        # Legend v3 classes as the run's model classes (older runs see subclasses as parents).
        classes = int(lut[classes]) if isinstance(classes, (int, np.integer)) else lut[classes]
        extra = [np.where(water, 255, 0).astype(np.uint8), snow, veg]
        if deep:
            h = coarse
        elif "forms" in config:  # form channels from the run's per-class table (forms.py)
            from . import forms

            cmap = np.full(coarse.shape, classes, np.uint8) if np.ndim(classes) == 0 else classes.astype(np.uint8)
            extra += list(forms.from_classes(cmap, config["forms"]["table"], None,
                                             config["forms"]["sigma"] * window).astype(np.float16))
            del cmap
        note = " (deep sea, not diffused)"
        if not deep:
            # L4 learned land only: residual zeroed under water (B32: rivers optional), with a soft
            # shore; windows of deep water only are not diffused.
            land = 1.0 - np.clip(smooth((lakes_sea if args.keep_river_residual else water).astype(np.float32), 2.0), 0, 1)
            skip = (coarse < cascade.DEEP_SEA_M) & (land < 1e-3)
            plan = plan_windows(*coarse.shape, window, args.stride, skip, guide if guide is not False else None)
            stats = progress.setdefault("windows", {"total": 0, "skipped": 0, "guided": 0})
            stats["total"] += len(plan[1])
            stats["skipped"] += len(plan[2])
            g = 1.0 if guide is False else args.guidance
            stats["guided"] += 0 if g == 1.0 else len(plan[1]) if plan[3] is None else len(plan[3] - plan[2])
            note = f" ({len(plan[2])}/{len(plan[1])} windows deep sea)"
            field = cascade.LazyField(level, coarse, extra, classes, n_classes)
            # Noise of the map's 1 m grid, so a square previews the whole map at its place.
            wy, wx = int(south) + g0, int(east)
            res = sample_field(model, field, steps=args.steps, window=window, stride=args.stride, batch=args.batch,
                               noise=world_noise(args.seed, wy, wy + g1 - g0, wx, wx + n), guidance=g,
                               plan=plan)[0, 0].numpy()
            h = (coarse + res * scale * land).astype(np.float32)
            del res, field, land, skip
        else:
            progress.setdefault("deep_sea_bands", []).append(k)

        cascade.write_band(raw, h, bands, k, g0, args.halo_m, args.out)
        c.snow_out[a:b] = np.where(snow[a - g0 : b - g0] >= 128, 2, 1)
        c.veg_out[a:b] = np.where(veg[a - g0 : b - g0] >= 128, 2, 1)
        for mm in (c.snow_out, c.veg_out):
            mm.flush()
        progress["band_s"].append(time.time() - t)
        progress["bands_done"] = k + 1
        save()
        cascade.drop_overlaps(args.out, k)
        print(f"band {k + 1}/{len(bands)} rows {a}-{b} in {progress['band_s'][-1]:.0f} s{note}", flush=True)
        del coarse, water, lakes_sea, snow, veg, classes, extra, h, guide

    extra = {}
    if torch.cuda.is_available():  # this process only; a resumed run counts its last part
        extra["peak_vram_gb"] = round(torch.cuda.max_memory_reserved() / 2**30, 2)
    extra["deep_sea_bands"] = progress.get("deep_sea_bands", [])
    extra["windows"] = progress.get("windows", {})  # of diffused bands: deep-sea skipped, guided
    _finish(c, extra)
    # Only now the run cannot be resumed any more: drop the last band's overlap.
    cascade.drop_overlaps(args.out, 1 << 30)
    print(f"done: wrote {args.out} in {sum(progress['band_s']) / 60:.0f} min", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
