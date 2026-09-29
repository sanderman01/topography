"""Generate 1 m terrain and cover masks for a world from its coarse inputs.

--scale f makes a quick low-detail version at f m spacing: the world is shrunk f times in
all three axes, generated at model scale and scaled back, so detail features come out f
times larger. Conditioning stays in host memory; only window batches go to the GPU.

python -m topography_training.diffusion.generate --run-dir <dir> --world <world-input-dir> --out <new-dir>

World input directory (little-endian raw arrays, row-major, north-up):
  world.json       {"schema_version": 1, "kind": "diffusion_world_input",
                    "size_blocks": [cols, rows], "block_m": 32}
  coarse.f32       32 m block elevation in metres [rows, cols]
  classes.u8       32 m block legend class, 0 = unknown [rows, cols]
  water.u8         optional 1 m water mask, 255 = water [rows*32, cols*32]
  snow-ice.u8, vegetation.u8
                   1 m cover, 2 present / 1 absent; required by conditioning layout 2
  land-cover.u8    optional 1 m WorldCover classes, passed through unchanged
Output: elevation.f32, snow-ice.u8 and vegetation.u8 (2 present / 1 absent),
water.u8, land-cover.u8 when given, and manifest.json. Water cells hold the coarse surface as fill. Layout 2
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
    p.add_argument("--negative", choices=["class", "ice"], default="class",
                   help="guide away from the class-dropped condition or from the same cover without ice")
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--scale", type=int, default=1, choices=[1, 2, 4, 8],
                   help="quick low-detail run: shrink the world this many times in all three axes, "
                        "generate, and scale back up; output spacing is this many metres")
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
    f = args.scale
    h, w = rows * data.COARSE_M, cols * data.COARSE_M
    hf, wf = h // f, w // f

    def layer(name: str) -> np.ndarray | None:
        path = args.world / name
        return np.memmap(path, "u1", "r", shape=(h, w)) if path.exists() else None

    def majority(full: np.ndarray | None, code: int) -> np.ndarray:
        """Cells where most of each f x f block has `code`; row bands bound memory."""
        if full is None:
            return np.zeros((hf, wf), bool)
        out = np.empty((hf, wf), bool)
        for r in range(0, hf, 512):
            band = np.asarray(full[r * f : (r + 512) * f]) == code
            out[r : r + 512] = band.reshape(-1, f, wf, f).mean(axis=(1, 3)) >= 0.5
        return out

    water = majority(layer("water.u8"), 255)

    config = json.loads((args.run_dir / "config.json").read_text())
    weights = args.weights or _latest(args.run_dir)
    device = torch.device("cuda")
    conditioning = config.get("conditioning", 1)
    model = Denoiser(config["arch"], conditioning).to(device).eval()
    model.load_state_dict(load_file(str(weights), device="cuda"))

    # Model space: f m cells treated as 1 m and heights divided by f, so the model sees a
    # smaller world with the same shapes; the result is scaled back up by f.
    smooth_m = config.get("coarse_smooth_m", 0.0)
    if f == 1:
        coarse = data.coarse_surface(coarse_blocks, smooth_m)
    else:
        t = torch.from_numpy(coarse_blocks.astype(np.float64) / f)[None, None]
        up = torch.nn.functional.interpolate(t, scale_factor=data.COARSE_M // f, mode="bicubic", align_corners=False)
        coarse = up[0, 0].numpy().astype(np.float32)
        coarse = data.smooth(coarse, smooth_m / f) if smooth_m > 0 else coarse
    if "water_smooth_m" in config:
        water = data.smooth_mask(water, config["water_smooth_m"])
    class_map = np.repeat(np.repeat(classes, data.COARSE_M // f, 0), data.COARSE_M // f, 1)
    snow = veg = None
    if conditioning != 1:
        if layer("snow-ice.u8") is None or layer("vegetation.u8") is None:
            print("error: world input lacks snow-ice.u8/vegetation.u8; rebuild it", file=sys.stderr)
            return 2
        snow, veg = majority(layer("snow-ice.u8"), 2), majority(layer("vegetation.u8"), 2)
        soft = config.get("cover_soft_m", data.COVER_SOFT_M)
        soft_snow, soft_veg = data.soft_cover(snow, soft), data.soft_cover(veg, soft)

    class WorldField:
        """Conditioning windows built on demand from compact host arrays."""

        def __init__(self, soft_snow=None):
            self.shape, self.device, self.snow = (hf, wf), torch.device("cpu"), soft_snow

        def windows(self, boxes, size, dev):
            parts = []
            for y, x in boxes:
                win = np.s_[y : y + size, x : x + size]
                cover = () if conditioning == 1 else (self.snow[win], soft_veg[win])
                parts.append(data.condition(coarse[win], water[win], class_map[win], *cover))
            return torch.from_numpy(np.stack(parts)).to(dev)

    negative = None
    if args.negative == "ice":
        if conditioning == 1:
            print("error: ice guidance needs conditioning layout 2", file=sys.stderr)
            return 2
        negative = WorldField(np.zeros_like(soft_snow))
    cond = WorldField(None if conditioning == 1 else soft_snow)
    start = time.time()
    out = sample_field(model, cond, steps=args.steps, window=config["crop"], stride=args.stride,
                       batch=args.batch, seed=args.seed, guidance=args.guidance, negative=negative)[0].float().numpy()
    elevation = (np.where(water, coarse, coarse + out[0] * config["residual_scale_m"]) * f).astype("<f4")
    layers = {
        "elevation.f32": elevation.tobytes(),
        "snow-ice.u8": (np.where(out[1] > 0, 2, 1) if conditioning == 1 else np.where(water | ~snow, 1, 2)).astype("u1").tobytes(),
        "vegetation.u8": (np.where(out[2] > 0, 2, 1) if conditioning == 1 else np.where(water | ~veg, 1, 2)).astype("u1").tobytes(),
        "water.u8": np.where(water, 255, 0).astype("u1").tobytes(),
    }
    cover_codes = layer("land-cover.u8")
    if cover_codes is not None:
        layers["land-cover.u8"] = np.ascontiguousarray(cover_codes[f // 2 :: f, f // 2 :: f]).tobytes()
    args.out.mkdir(parents=True)
    for name, blob in layers.items():
        (args.out / name).write_bytes(blob)
    manifest = {
        "schema_version": 1,
        "kind": "diffusion_world_output",
        "size": [wf, hf],
        "spacing_m": float(f),
        "scale": f,
        "water_fill": "coarse surface",
        "weights": str(weights),
        "weights_sha256": hashlib.sha256(Path(weights).read_bytes()).hexdigest(),
        "world_input": str(args.world.resolve()),
        "steps": args.steps, "stride": args.stride, "guidance": args.guidance, "negative": args.negative, "seed": args.seed,
        "seconds": round(time.time() - start, 1),
        "layers": {n: hashlib.sha256(b).hexdigest() for n, b in layers.items()},
    }
    (args.out / "manifest.json").write_text(json.dumps(manifest, indent=1))
    print(f"wrote {args.out} in {manifest['seconds']} s", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
