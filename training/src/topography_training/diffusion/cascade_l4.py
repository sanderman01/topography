"""Banded L4 generation: refine a 4 m world (cascade chain output) to 1 m.

python -m topography_training.diffusion.cascade_l4 --run <l4 run dir> --world <4 m world dir> --out <dir>
    [--km N] [options]

The 1 m field is generated in bands of rows with a halo above and below, so host memory
stays bounded at any world size. Bands share their starting noise where they overlap
and are cross-faded over the halo. Each band then gets its water from the Rust
`topography world hydrology` at 1 m, north to south: it holds the world's 4 m network fixed
(`--coarse`) and takes the previous band's rows (`--north`); the seam between them is
checked and a failure stops the run. Water cells keep the refined 4 m ground (L4 learned
land only) under the hydrology surface; bands and windows whose 4 m ground is all deep sea
water keep it without diffusion. `--guidance-classes` limits class guidance to windows
holding those classes. `--km` with `--centre` places the square anywhere. Output layers are
memory-mapped and filled band by band; rerunning with the same arguments resumes after the
last finished band.
Writes a 1 m world output (manifest.json + layers, water-surface.f32, water-body.u32 and
bodies.json) for `topography export heightmap`, plus overview.png and detail.png hillshades.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
import time
from pathlib import Path

import numpy as np

from . import cascade, hydrology
from .data import smooth, soft_cover

MARGIN_4M = 16  # 4 m context cells around a band for the smoothing
WATER_SIGMA_M = 3.0  # rounds 4 m water corners at 1 m
# Body fields that hold for the whole world (the rest are per band grid). Dry basins (not in
# the raster) keep their largest band area; tiny ones stay out (hydrology.world_bodies).
BODY_FIELDS = ("id", "kind", "level_m", "coarse", "parent", "over_budget", "endorheic", "ice")
# Disk per 1 m cell: elevation, the raw surface while running, water surface and body (4 B
# each), water, snow and vegetation (1 B each).
DISK_BYTES = 19


def _rows_4m(a: np.ndarray, r0: int, r1: int, c0: int, c1: int) -> np.ndarray:
    """A 4 m block with indices beyond the field repeating its edge."""
    rows = np.clip(np.arange(r0, r1), 0, a.shape[0] - 1)
    cols = np.clip(np.arange(c0, c1), 0, a.shape[1] - 1)
    return a[np.ix_(rows, cols)]


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
    args = p.parse_args(argv)
    if args.centre is not None and args.km is None:
        p.error("--centre needs --km")

    import torch

    from .cascade_chain import _load, _png
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
    # Hydrology halos are as wide as the diffusion halo and must lie in final rows.
    if args.band_m % 64 or args.halo_m % 64 or args.halo_m * 3 // 2 > args.band_m:
        print("error: band and halo must be multiples of 64 m, the halo at most 2/3 of a band", file=sys.stderr)
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
              "stride": args.stride, "seed": args.seed}
    if args.guidance != 1.0:
        params["guidance"] = args.guidance
        if args.guidance_classes:
            params["guidance_classes"] = sorted(args.guidance_classes)
    if args.centre is not None:
        params["window_4m"] = [col4, row4]
    if args.class_runs is not None:
        params["class_runs"] = str(args.class_runs.resolve())
    progress_path = args.out / "progress.json"
    hyd = args.out / "hydrology"
    if args.out.exists():
        if (args.out / "manifest.json").exists():
            (args.out / "elevation-raw.f32").unlink(missing_ok=True)  # left by a crash while finishing
            print("error: output is already complete", file=sys.stderr)
            return 2
        if not progress_path.exists() or json.loads(progress_path.read_text())["params"] != params:
            print("error: output directory exists with other arguments", file=sys.stderr)
            return 2
        progress = json.loads(progress_path.read_text())
        mode = "r+"
    else:
        # Plus two band directories of about 22 B a cell while the run goes.
        band_cells = n * (args.band_m + 2 * args.halo_m)
        hydrology.check_disk(args.out, DISK_BYTES * n * n + 2 * 22 * band_cells, "the L4 run")
        args.out.mkdir(parents=True)
        hyd.mkdir()
        progress = {"params": params, "bands_done": 0, "band_s": [], "hydrology_done": 0, "hydrology_s": [],
                    "bodies": {}}
        mode = "w+"

    def layer(name, dtype):
        return np.memmap(args.out / name, dtype=dtype, mode=mode, shape=(n, n))

    # Diffusion writes the raw surface (cross-faded between bands); hydrology carves it. It is
    # removed last, once the manifest is written.
    raw_path = args.out / "elevation-raw.f32"
    raw = layer(raw_path.name, "<f4") if mode == "w+" or raw_path.exists() else None
    elev, water_out = layer("elevation.f32", "<f4"), layer("water.u8", np.uint8)
    surface_out, body_out = layer("water-surface.f32", "<f4"), layer("water-body.u32", "<u4")
    snow_out, veg_out = layer("snow-ice.u8", np.uint8), layer("vegetation.u8", np.uint8)

    def read4(name, dtype):
        # Memory-mapped: a band reads only its rows.
        mm = np.memmap(args.world / name, dtype=dtype, mode="r", shape=(n4, n4))
        return mm[row4 : row4 + c4, col4 : col4 + c4]

    h4 = read4("elevation.f32", "<f4")
    water4, snow4, veg4 = read4("water.u8", np.uint8), read4("snow-ice.u8", np.uint8), read4("vegetation.u8", np.uint8)
    classes4 = read4("classes.u8", np.uint8) if per_cell else None

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
    bands = cascade.bands(n, args.band_m, args.halo_m)
    done = progress["bands_done"]
    print(f"{n} x {n} at 1 m in {len(bands)} bands; resuming after band {done}" if done else
          f"{n} x {n} at 1 m in {len(bands)} bands", flush=True)

    # Band grids sit in the 4 m hydrology grid's plane, offset by the crop.
    origin, east, south = g4["origin"], off_m[0] + col4 * 4.0, off_m[1] + row4 * 4.0
    bodies = progress["bodies"]

    def save():
        cascade.save_json(progress_path, progress)

    def hydro(j: int):
        """1 m hydrology of band j (its raw rows and halos are final), checked against band j - 1."""
        t = time.time()
        a, b = bands[j]
        top, bot = max(a - args.halo_m, 0), min(b + args.halo_m, n)
        cls = classes4[top // 4 : bot // 4] if per_cell else np.full(((bot - top) // 4, c4), class_id, np.uint8)
        grid = hydrology.write_grid(hyd / "grid", np.asarray(raw[top:bot]), 1.0, origin, cls,
                                    offset_m=(east, south + top), core_rows=(a - top, b - top), classes_cell_m=4.0)
        out, north = hyd / f"band-{j:03d}", (hyd / f"band-{j - 1:03d}" if j else None)
        hydrology.run(grid, out, coarse=coarse_dir, north=north)
        if north is not None:
            hydrology.seam(north, out)
        shutil.copy(out / "report.json", hyd / f"report-{j:03d}.json")
        o = hydrology.read(out, slice(a - top, b - top))
        elev[a:b], surface_out[a:b], body_out[a:b] = o.ground, o.surface, o.body
        water_out[a:b] = np.where(np.isfinite(o.surface), 255, 0)
        for mm in (elev, surface_out, body_out, water_out):
            mm.flush()
        # A body's parent is where it really ends: the band whose core holds its mouth or exit.
        core = range(a - top, b - top)
        for entry in o.bodies:
            fields = {k: entry[k] for k in BODY_FIELDS if k in entry}
            key = str(entry["id"])
            if entry["kind"] == "dry":  # areas count the band grid; keep the largest
                area = max(entry.get("area_km2", 0.0), bodies.get(key, {}).get("area_km2", 0.0))
                if area >= hydrology.WORLD_DRY_MIN_KM2:
                    bodies[key] = fields | {"area_km2": area}
                continue
            end = entry.get("mouth_cell") or entry.get("exit_cell")
            if end is not None and end[1] in core:
                bodies[key] = fields
            else:
                bodies.setdefault(key, fields)
        progress["hydrology_done"] = j + 1
        progress["hydrology_s"].append(time.time() - t)
        save()
        if north is not None:  # only the latest band is needed, as the next band's north
            shutil.rmtree(north)
        wet = np.isfinite(o.surface).mean()
        print(f"hydrology {j + 1}/{len(bands)} rows {a}-{b} in {progress['hydrology_s'][-1]:.0f} s, water {wet:.2%}",
              flush=True)

    def hydro_until(k: int):
        while progress["hydrology_done"] < k:
            hydro(progress["hydrology_done"])

    hydro_until(done - 1)
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
            # L4 learned land only: water keeps the refined 4 m ground, with a soft shore; windows
            # of deep water only are not diffused.
            land = 1.0 - np.clip(smooth(water.astype(np.float32), 2.0), 0, 1)
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
        snow_out[a:b] = np.where(snow[a - g0 : b - g0] >= 128, 2, 1)
        veg_out[a:b] = np.where(veg[a - g0 : b - g0] >= 128, 2, 1)
        for mm in (snow_out, veg_out):
            mm.flush()
        progress["band_s"].append(time.time() - t)
        progress["bands_done"] = k + 1
        save()
        cascade.drop_overlaps(args.out, k)
        print(f"band {k + 1}/{len(bands)} rows {a}-{b} in {progress['band_s'][-1]:.0f} s{note}", flush=True)
        # Free the band's fields before Rust runs; the band above is now final down to its halo.
        del coarse, water, snow, veg, classes, extra, h, guide
        hydro_until(k)
    hydro_until(len(bands))

    # World body list: areas from the delivered cells; dry basins keep no area.
    count = np.zeros(1, np.int64)
    for y in range(0, n, 1024):
        c = np.bincount(np.asarray(body_out[y : y + 1024]).ravel())
        count = np.pad(count, (0, max(len(c) - len(count), 0)))
        count[: len(c)] += c
    listed = []
    for entry in bodies.values():
        if entry["kind"] == "dry":
            listed.append(entry)
            continue
        cells = int(count[entry["id"]]) if entry["id"] < len(count) else 0
        if cells:
            listed.append(entry | {"area_km2": cells / 1e6})
    (args.out / "bodies.json").write_text(json.dumps(hydrology.world_bodies(
        listed, "merged from the 1 m bands: water areas count delivered cells, dry basin areas their "
        "largest band grid; parents from the band holding each end"), indent=1))

    # Pictures: the whole area averaged to 8 m and a 1,024 m square at 1 m in the middle.
    over = np.concatenate([cascade.block_mean(np.asarray(elev[y : y + 1024]), 8) for y in range(0, n, 1024)])
    wover = np.concatenate([cascade.block_mean(np.asarray(water_out[y : y + 1024], np.float32), 8)
                            for y in range(0, n, 1024)]) > 127
    _png(args.out / "overview.png", over, 8.0, wover, 2048)
    c, r = n // 2, min(512, n // 2)
    _png(args.out / "detail.png", np.asarray(elev[c - r : c + r, c - r : c + r]), 1.0,
         np.asarray(water_out[c - r : c + r, c - r : c + r]) == 255, 1024)

    src = json.loads((Path(world["world_input"]) / "world.json").read_text())
    ox, oy = src["origin"][0] + col4 * 4.0, src["origin"][1] - row4 * 4.0
    world_input = args.out / "world-input"
    world_input.mkdir(exist_ok=True)
    (world_input / "world.json").write_text(json.dumps(
        src | {"name": f"{src.get('name', 'world')}-l4", "origin": [ox, oy]}, indent=1))
    names = ("elevation.f32", "water.u8", "water-surface.f32", "water-body.u32", "bodies.json",
             "snow-ice.u8", "vegetation.u8")
    digests = {name: hydrology.sha256(args.out / name) for name in names}
    extra = {}
    if (args.world / "moisture.f32").exists():  # 128 m moisture, as the 4 m world's
        digests |= hydrology.link(args.world, args.out, ("moisture.f32",))
        extra["moisture"] = world["moisture"]
    if "description" in world:  # carried through as the chain measured it
        shutil.copy(args.world / world["description"], args.out / "description.json")
        shutil.copy(args.world / "description.md", args.out / "description.md")
        digests["description.json"] = hydrology.sha256(args.out / "description.json")
        extra["description"] = "description.json"
    if torch.cuda.is_available():  # this process only; a resumed run counts its last part
        extra["peak_vram_gb"] = round(torch.cuda.max_memory_reserved() / 2**30, 2)
    extra["deep_sea_bands"] = progress.get("deep_sea_bands", [])
    extra["windows"] = progress.get("windows", {})  # of diffused bands: deep-sea skipped, guided
    (args.out / "manifest.json").write_text(json.dumps({
        "schema_version": 1, "kind": "world_preview", "world_input": str(world_input.resolve()),
        "spacing_m": 1.0, "size": [n, n], "class_id": class_id,
        "method": "banded L4 over a cascade chain 4 m world, 1 m hydrology per band",
        "water_fill": hydrology.WATER_FILL, **extra,
        "source_world": str(args.world.resolve()), "params": params, "band_s": progress["band_s"],
        "hydrology_s": progress["hydrology_s"], "layers": digests}, indent=1))
    # Only now the run cannot be resumed any more: drop the raw surface and the last band.
    del raw
    raw_path.unlink(missing_ok=True)
    cascade.drop_overlaps(args.out, 1 << 30)
    for d in (hyd / f"band-{len(bands) - 1:03d}", hyd / "grid"):  # band reports stay
        shutil.rmtree(d, ignore_errors=True)
    print(f"done: wrote {args.out} in {(sum(progress['band_s']) + sum(progress['hydrology_s'])) / 60:.0f} min",
          flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
