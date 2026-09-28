"""Generate 1 m terrain and cover masks for a world from its coarse inputs.

python -m topography_training.diffusion.generate --run-dir <dir> --world <world-input-dir> --out <new-dir>

World input directory (little-endian raw arrays, row-major, north-up):
  world.json       {"schema_version": 1, "kind": "diffusion_world_input",
                    "size_blocks": [cols, rows], "block_m": 32}
  coarse.f32       32 m block elevation in metres [rows, cols]
  classes.u8       32 m block legend class, 0 = unknown [rows, cols]
  water.u8         optional 1 m water mask, 255 = water [rows*32, cols*32]
  snow-ice.u8, vegetation.u8
                   1 m cover, 2 present / 1 absent; required by conditioning layout 2
Output: elevation.f32, snow-ice.u8 and vegetation.u8 (2 present / 1 absent),
water.u8 and manifest.json. Water cells hold the coarse surface as fill. Layout 2
generates terrain for the given cover and passes it through; layout 1 predicts cover.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import time
from pathlib import Path

import numpy as np


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--run-dir", type=Path, required=True)
    p.add_argument("--world", type=Path, required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--weights", type=Path)
    p.add_argument("--steps", type=int, default=32)
    p.add_argument("--stride", type=int, default=128)
    p.add_argument("--batch", type=int, default=8)
    p.add_argument("--guidance", type=float, default=1.0)
    p.add_argument("--seed", type=int, default=0)
    args = p.parse_args(argv)

    import torch
    from safetensors.torch import load_file

    from . import data
    from .evaluate import _latest
    from .model import Denoiser, sample_field

    if args.out.exists():
        print("error: output directory exists", file=sys.stderr)
        return 2
    spec = json.loads((args.world / "world.json").read_text())
    if spec.get("kind") != "diffusion_world_input" or spec.get("block_m") != data.COARSE_M:
        print("error: unsupported world input", file=sys.stderr)
        return 2
    cols, rows = spec["size_blocks"]
    coarse_blocks = np.fromfile(args.world / "coarse.f32", "<f4").reshape(rows, cols)
    classes = np.fromfile(args.world / "classes.u8", "u1").reshape(rows, cols)
    if classes.max() >= data.NUM_CLASSES or not np.isfinite(coarse_blocks).all():
        print("error: invalid classes or coarse elevation", file=sys.stderr)
        return 2
    h, w = rows * data.COARSE_M, cols * data.COARSE_M
    water_path = args.world / "water.u8"
    water = (np.fromfile(water_path, "u1").reshape(h, w) == 255) if water_path.exists() else np.zeros((h, w), bool)

    config = json.loads((args.run_dir / "config.json").read_text())
    weights = args.weights or _latest(args.run_dir)
    device = torch.device("cuda")
    conditioning = config.get("conditioning", 1)
    model = Denoiser(config["arch"], conditioning).to(device).eval()
    model.load_state_dict(load_file(str(weights), device="cuda"))

    coarse = data.coarse_surface(coarse_blocks, config.get("coarse_smooth_m", 0.0))
    if "water_smooth_m" in config:
        water = data.smooth_mask(water, config["water_smooth_m"])
    class_map = np.repeat(np.repeat(classes, data.COARSE_M, 0), data.COARSE_M, 1).astype(np.int64)
    if conditioning == 1:
        cond = data.condition(coarse, water, class_map)
    else:
        try:
            snow, veg = (np.fromfile(args.world / f"{n}.u8", "u1").reshape(h, w) for n in ("snow-ice", "vegetation"))
        except FileNotFoundError:
            print("error: world input lacks snow-ice.u8/vegetation.u8; rebuild it", file=sys.stderr)
            return 2
        soft = config.get("cover_soft_m", data.COVER_SOFT_M)
        cond = data.condition(coarse, water, class_map, data.soft_cover(snow == 2, soft), data.soft_cover(veg == 2, soft))
    cond = torch.from_numpy(cond)[None].to(device)
    start = time.time()
    out = sample_field(model, cond, steps=args.steps, window=config["crop"], stride=args.stride,
                       batch=args.batch, seed=args.seed, guidance=args.guidance)[0].float().cpu().numpy()
    elevation = np.where(water, coarse, coarse + out[0] * config["residual_scale_m"]).astype("<f4")
    layers = {
        "elevation.f32": elevation.tobytes(),
        "snow-ice.u8": (np.where(out[1] > 0, 2, 1).astype("u1") if conditioning == 1 else np.where(water, 1, snow).astype("u1")).tobytes(),
        "vegetation.u8": (np.where(out[2] > 0, 2, 1).astype("u1") if conditioning == 1 else np.where(water, 1, veg).astype("u1")).tobytes(),
        "water.u8": np.where(water, 255, 0).astype("u1").tobytes(),
    }
    args.out.mkdir(parents=True)
    for name, blob in layers.items():
        (args.out / name).write_bytes(blob)
    manifest = {
        "schema_version": 1,
        "kind": "diffusion_world_output",
        "size": [w, h],
        "spacing_m": 1.0,
        "water_fill": "coarse surface",
        "weights": str(weights),
        "weights_sha256": hashlib.sha256(Path(weights).read_bytes()).hexdigest(),
        "world_input": str(args.world.resolve()),
        "steps": args.steps, "stride": args.stride, "guidance": args.guidance, "seed": args.seed,
        "seconds": round(time.time() - start, 1),
        "layers": {n: hashlib.sha256(b).hexdigest() for n, b in layers.items()},
    }
    (args.out / "manifest.json").write_text(json.dumps(manifest, indent=1))
    print(f"wrote {args.out} in {manifest['seconds']} s", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
