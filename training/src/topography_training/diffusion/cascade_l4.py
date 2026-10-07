"""Banded L4 generation: refine a 4 m world (cascade chain output) to 1 m.

python -m topography_training.diffusion.cascade_l4 --run <l4 run dir> --world <4 m world dir> --out <dir>
    [--km N] [options]

The 1 m field is generated in bands of rows with a halo above and below, so host memory
stays bounded at any world size. Bands share their starting noise where they overlap
and are cross-faded over the halo. Output layers are memory-mapped and filled band by
band; rerunning with the same arguments resumes after the last finished band. Water
cells (sea, lakes, streams) keep the smooth 4 m surface, since L4 learned land only.
Writes a 1 m world output (manifest.json + layers) for `topography export heightmap`,
plus overview.png and detail.png hillshades.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import time
from pathlib import Path

import numpy as np

from . import cascade
from .data import smooth, soft_cover

NOISE_ROWS = 256  # starting noise is drawn per block of rows, so overlapping bands agree
MARGIN_4M = 16  # 4 m context cells around a band for the smoothing
WATER_SIGMA_M = 3.0  # rounds 4 m water corners at 1 m


def _noise(seed: int, g0: int, g1: int, cols: int):
    import torch

    parts = []
    for block in range(g0 // NOISE_ROWS, -(-g1 // NOISE_ROWS)):
        gen = torch.Generator(device="cpu").manual_seed(seed * 1_000_003 + block)
        parts.append(torch.randn((NOISE_ROWS, cols), generator=gen))
    rows = torch.cat(parts)
    off = g0 - g0 // NOISE_ROWS * NOISE_ROWS
    return rows[off : off + g1 - g0][None, None]


def _rows_4m(a: np.ndarray, r0: int, r1: int, c0: int, c1: int) -> np.ndarray:
    """A 4 m block with indices beyond the field repeating its edge."""
    rows = np.clip(np.arange(r0, r1), 0, a.shape[0] - 1)
    cols = np.clip(np.arange(c0, c1), 0, a.shape[1] - 1)
    return a[np.ix_(rows, cols)]


def _hash(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        while chunk := f.read(1 << 24):
            h.update(chunk)
    return h.hexdigest()


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--run", type=Path, required=True, help="L4 run directory")
    p.add_argument("--world", type=Path, required=True, help="4 m world output (cascade_chain)")
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--km", type=int, help="central square side in units of 1,024 m (power of two); default whole world")
    p.add_argument("--class-id", type=int, help="terrain class when the world manifest has none")
    p.add_argument("--band-m", type=int, default=2048)
    p.add_argument("--halo-m", type=int, default=256)
    p.add_argument("--steps", type=int, default=12)
    p.add_argument("--stride", type=int, default=192)
    p.add_argument("--batch", type=int, default=16)
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--guidance", type=float, default=1.0, help="class guidance scale (1: off)")
    args = p.parse_args(argv)

    import torch

    from .cascade_chain import _load, _png
    from .model import sample_field

    world = json.loads((args.world / "manifest.json").read_text())
    if world.get("kind") != "world_preview" or world.get("spacing_m") != 4.0:
        print("error: --world must be a 4 m world output", file=sys.stderr)
        return 2
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
    off4 = (n4 - c4) // 2
    n = c4 * 4

    params = {"run": str(args.run.resolve()), "world": str(args.world.resolve()), "km": args.km,
              "class_id": class_id, "band_m": args.band_m, "halo_m": args.halo_m, "steps": args.steps,
              "stride": args.stride, "seed": args.seed}
    if args.guidance != 1.0:
        params["guidance"] = args.guidance
    progress_path = args.out / "progress.json"
    if args.out.exists():
        if (args.out / "manifest.json").exists():
            print("error: output is already complete", file=sys.stderr)
            return 2
        if not progress_path.exists() or json.loads(progress_path.read_text())["params"] != params:
            print("error: output directory exists with other arguments", file=sys.stderr)
            return 2
        done = json.loads(progress_path.read_text())["bands_done"]
        times = json.loads(progress_path.read_text())["band_s"]
        mode = "r+"
    else:
        args.out.mkdir(parents=True)
        done, times, mode = 0, [], "w+"

    def layer(name, dtype):
        return np.memmap(args.out / name, dtype=dtype, mode=mode, shape=(n, n))

    elev, water_out = layer("elevation.f32", "<f4"), layer("water.u8", np.uint8)
    snow_out, veg_out = layer("snow-ice.u8", np.uint8), layer("vegetation.u8", np.uint8)

    def read4(name, dtype):
        return np.fromfile(args.world / name, dtype=dtype).reshape(n4, n4)[off4 : off4 + c4, off4 : off4 + c4]

    h4 = read4("elevation.f32", "<f4")
    water4 = read4("water.u8", np.uint8) == 255
    snow4, veg4 = read4("snow-ice.u8", np.uint8) == 2, read4("vegetation.u8", np.uint8) == 2
    classes4 = read4("classes.u8", np.uint8) if per_cell else None

    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    model, config = _load(args.run, device)
    level = cascade.LEVELS["l4"]
    if config.get("level") != "l4":
        print("error: --run is not an L4 run", file=sys.stderr)
        return 2
    scale, window = config["residual_scale_m"], config["crop"]
    from . import legend

    n_classes, lut = legend.model_lut(config)
    m, half = MARGIN_4M, args.halo_m // 2
    bands = list(range(0, n, args.band_m))
    print(f"{n} x {n} at 1 m in {len(bands)} bands; resuming after band {done}" if done else
          f"{n} x {n} at 1 m in {len(bands)} bands", flush=True)

    for k in range(done, len(bands)):
        t = time.time()
        a = bands[k]
        b = min(a + args.band_m, n)
        g0, g1 = max(a - args.halo_m, 0), min(b + args.halo_m, n)
        # 4 m rows covering the band with a margin; the 1 m fields are cropped back to it.
        r0, r1 = g0 // 4 - m, g1 // 4 + m
        crop = np.s_[m * 4 : -m * 4, m * 4 : -m * 4]
        coarse = cascade.refine_surface(level, _rows_4m(h4, r0, r1, -m, c4 + m))[crop]
        water = cascade.refine_mask(_rows_4m(water4, r0, r1, -m, c4 + m), 4, WATER_SIGMA_M, keep_thin=True)[crop]

        def cover(mask):
            return soft_cover(np.repeat(np.repeat(_rows_4m(mask, r0, r1, -m, c4 + m), 4, 0), 4, 1))[crop]

        snow, veg = cover(snow4), cover(veg4)
        classes = class_id
        if per_cell:
            classes = np.repeat(np.repeat(_rows_4m(classes4, r0, r1, -m, c4 + m), 4, 0), 4, 1)[crop].astype(np.int64)
        # Legend v3 classes as the run's model classes (older runs see subclasses as parents).
        classes = int(lut[classes]) if isinstance(classes, (int, np.integer)) else lut[classes].astype(np.int64)
        field = cascade.LazyField(level, coarse, [np.where(water, 255, 0).astype(np.uint8), snow, veg], classes,
                                  n_classes)
        res = sample_field(model, field, steps=args.steps, window=window, stride=args.stride, batch=args.batch,
                           noise=_noise(args.seed, g0, g1, n), guidance=args.guidance)[0, 0].numpy()
        # L4 learned land only: water keeps the 4 m surface, with a soft shore.
        land = 1.0 - np.clip(smooth(water.astype(np.float32), 2.0), 0, 1)
        h = (coarse + res * scale * land).astype(np.float32)
        del res, field

        # Rows this band owns; the first `halo` rows are cross-faded with the band above.
        w0, w1 = (a - half if k else 0), (b + half if b < n else n)
        if k:
            rows = np.arange(w0, a + half)
            fade = ((rows - w0 + 0.5) / args.halo_m)[:, None].astype(np.float32)
            elev[w0 : a + half] = (1 - fade) * elev[w0 : a + half] + fade * h[w0 - g0 : a + half - g0]
            elev[a + half : w1] = h[a + half - g0 : w1 - g0]
        else:
            elev[w0:w1] = h[w0 - g0 : w1 - g0]
        water_out[a:b] = np.where(water[a - g0 : b - g0], 255, 0)
        snow_out[a:b] = np.where(snow[a - g0 : b - g0] >= 128, 2, 1)
        veg_out[a:b] = np.where(veg[a - g0 : b - g0] >= 128, 2, 1)
        for mm in (elev, water_out, snow_out, veg_out):
            mm.flush()
        times.append(time.time() - t)
        progress_path.write_text(json.dumps({"params": params, "bands_done": k + 1, "band_s": times}))
        print(f"band {k + 1}/{len(bands)} rows {a}-{b} in {times[-1]:.0f} s", flush=True)

    # Pictures: the whole area averaged to 8 m and a 1,024 m square at 1 m in the middle.
    over = np.concatenate([cascade.block_mean(np.asarray(elev[y : y + 1024]), 8) for y in range(0, n, 1024)])
    wover = np.concatenate([cascade.block_mean(np.asarray(water_out[y : y + 1024], np.float32), 8)
                            for y in range(0, n, 1024)]) > 127
    _png(args.out / "overview.png", over, 8.0, wover, 2048)
    c, r = n // 2, min(512, n // 2)
    _png(args.out / "detail.png", np.asarray(elev[c - r : c + r, c - r : c + r]), 1.0,
         np.asarray(water_out[c - r : c + r, c - r : c + r]) == 255, 1024)

    src = json.loads((Path(world["world_input"]) / "world.json").read_text())
    ox, oy = src["origin"][0] + off4 * 4.0, src["origin"][1] - off4 * 4.0
    world_input = args.out / "world-input"
    world_input.mkdir(exist_ok=True)
    (world_input / "world.json").write_text(json.dumps(
        src | {"name": f"{src.get('name', 'world')}-l4", "origin": [ox, oy]}, indent=1))
    names = ("elevation.f32", "water.u8", "snow-ice.u8", "vegetation.u8")
    (args.out / "manifest.json").write_text(json.dumps({
        "schema_version": 1, "kind": "world_preview", "world_input": str(world_input.resolve()),
        "spacing_m": 1.0, "size": [n, n], "class_id": class_id,
        "method": "banded L4 over a cascade chain 4 m world",
        "water_fill": "water keeps the smooth 4 m surface",
        "source_world": str(args.world.resolve()), "params": params, "band_s": times,
        "layers": {name: _hash(args.out / name) for name in names}}, indent=1))
    print(f"done: wrote {args.out} in {sum(times) / 60:.0f} min of bands", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
