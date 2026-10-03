"""Chain the cascade levels: L1 (128 m) -> L2 (32 m) -> L3 (4 m) from one region's layout.

python -m topography_training.diffusion.cascade_chain --l1-run <dir> --l2-run <dir> --l3-run <dir>
    --release <context release.json> --region <id> --out <new-dir> [options]

The layout (envelope, relief, class) is derived from a real GLO-30 region, as an
author's L0 would give it; snow/ice and vegetation come from the same region. L1 and L2
cover the whole region, L3 a square in its middle. Terrain continues under water: after
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

from . import cascade, hydrology
from .cascade_eval import shade
from .data import NUM_CLASSES
from .evaluate import _latest

# L3 learned land only (lidar stops at the shore); deeper than this its seabed is the L2 surface.
L3_SEABED_M = 5.0


def _load(run: Path, device):
    from safetensors.torch import load_file

    from .model import Denoiser

    config = json.loads((run / "config.json").read_text())
    model = Denoiser(config["arch"], config["conditioning"]).to(device).eval()
    model.load_state_dict(load_file(str(_latest(run)), device=str(device)))
    return model, config


def _condition(level, coarse, extra, class_id) -> np.ndarray:
    """Like cascade.condition, but filled in place so big fields are not copied."""
    h, w = coarse.shape
    cond = np.zeros((4 + len(extra) + NUM_CLASSES, h, w), np.float32)
    cond[0] = coarse
    cond[1], cond[2] = cascade.slopes(coarse, level.cell_m)
    cond[3] = coarse / 4096.0
    for i, e in enumerate(extra):
        cond[4 + i] = cascade.decode(e)
    cond[4 + len(extra) + class_id] = 1.0
    return cond


def _sample(model, config, coarse, cond, steps, seed, batch):
    import torch

    from .model import sample_field

    size = config["crop"]
    out = sample_field(model, torch.from_numpy(cond)[None], steps=steps, window=size, stride=size // 2,
                       batch=batch, seed=seed)
    return coarse + out[0, 0].numpy() * config["residual_scale_m"]


def _spread(level: np.ndarray, mask: np.ndarray, cells: int) -> np.ndarray:
    """Lake levels carried `cells` cells beyond their mask, for rounded shores."""
    out = np.where(mask, level, -np.inf).astype(np.float32)
    for _ in range(cells):
        out = np.maximum(out, hydrology._maxpool3(out))
    return out


def _nearest(a: np.ndarray, k: int) -> np.ndarray:
    return np.repeat(np.repeat(a, k, axis=0), k, axis=1)


def _png(path: Path, h: np.ndarray, cell_m: float, sea: np.ndarray, size: int = 1536):
    from PIL import Image

    img = Image.fromarray(shade(h, cell_m, sea))
    img.resize((size, size), Image.LANCZOS if h.shape[0] > size else Image.NEAREST).save(path)


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    for level in ("l1", "l2", "l3"):
        p.add_argument(f"--{level}-run", type=Path, required=True)
    p.add_argument("--release", type=Path, required=True, help="context release with the layout region")
    p.add_argument("--region", required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--l3-km", type=int, default=16, help="side of the central L3 square in units of 1,024 m (power of two)")
    p.add_argument("--steps", type=int, default=12)
    p.add_argument("--batch", type=int, default=16)
    p.add_argument("--seed", type=int, default=0)
    # Water amounts default to the region class's profile (hydrology.WATER).
    p.add_argument("--lake-share", type=float, help="share of land kept as lakes after L1")
    p.add_argument("--tarn-share", type=float, help="share of land kept as small lakes after L2")
    p.add_argument("--stream-km2", type=float, help="drainage area where streams begin")
    args = p.parse_args(argv)

    import torch

    if args.out.exists():
        print("error: output directory exists", file=sys.stderr)
        return 2
    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    regions = [r for r in cascade.load_release(args.release) if r.name == args.region]
    if not regions:
        print(f"error: region {args.region} not in the release", file=sys.stderr)
        return 2
    r = regions[0]
    manifest = json.loads(args.release.read_text())
    meta = next(m for m in manifest["regions"] if m["id"] == args.region)
    profile = hydrology.WATER.get(r.class_id, hydrology.WATER[0])
    overrides = {"lake_share": args.lake_share, "tarn_share": args.tarn_share, "stream_km2": args.stream_km2}
    water = dataclasses.replace(profile, **{k: v for k, v in overrides.items() if v is not None})
    args.out.mkdir(parents=True)
    times = {}

    # L1: the whole region from its derived layout, then drainage: the biggest
    # depressions become lakes, the rest are carved through like real valleys.
    t = time.time()
    model, config = _load(args.l1_run, device)
    level = cascade.LEVELS["l1"]
    if config["conditioning"] != level.conditioning:
        print("error: the L1 run uses a retired layout", file=sys.stderr)
        return 2
    f1 = cascade.l1_fields(r)
    cond = _condition(level, f1.coarse, f1.extra, r.class_id)
    h128 = _sample(model, config, f1.coarse, cond, args.steps, args.seed, args.batch)
    h128, lake128, level128 = hydrology.drain(h128, cascade.open_sea(h128), water.lake_share, 1.0)
    sea128 = cascade.open_sea(h128)
    lake128 &= ~sea128
    times["l1_s"] = time.time() - t
    print(f"L1 {h128.shape} in {times['l1_s']:.0f} s; sea {sea128.mean():.1%}, lakes {lake128.mean():.1%}", flush=True)

    # L2: the whole region, seabed included; lakes are the only water it is told about.
    t = time.time()
    level = cascade.LEVELS["l2"]
    model, config = _load(args.l2_run, device)
    coarse = cascade.refine_surface(level, h128)
    lake32 = cascade.refine_mask(lake128, 4, 2.0)
    lake_level32 = _spread(_nearest(np.nan_to_num(level128), 4), _nearest(lake128, 4), 4)
    extra = [np.where(lake32, 255, 0).astype(np.uint8), cascade.soft_u8(r.snow, 1.0),
             cascade.soft_u8(r.vegetation, 1.0)]
    h32 = _sample(model, config, coarse, _condition(level, coarse, extra, r.class_id), args.steps,
                  args.seed + 1, args.batch)
    sea32 = cascade.open_sea(h32)
    lake32 &= ~sea32
    h32[lake32] = lake_level32[lake32]
    raw32 = h32.copy()
    # Small lakes (tarns, kettles) from L2's own hollows; the rest are carved through.
    h32, tarn32, _ = hydrology.drain(h32, sea32 | lake32, water.tarn_share, 1.0, tarns=True)
    sea32 = cascade.open_sea(h32)
    lake32 = (lake32 | tarn32) & ~sea32
    snow32, veg32 = r.snow & ~sea32, r.vegetation & ~sea32
    stream32, parent32, width32 = hydrology.streams(h32, sea32 | lake32, 32.0, water.stream_km2, water.stream_width)
    times["l2_s"] = time.time() - t
    print(f"L2 {h32.shape} in {times['l2_s']:.0f} s; tarns {tarn32.mean():.2%}, streams {stream32.sum()} cells", flush=True)

    # L3: a central square from the L2 surface (with a margin for the smoothing). Sea,
    # lakes and streams are water, as in its lidar training data.
    t = time.time()
    level = cascade.LEVELS["l3"]
    model, config = _load(args.l3_run, device)
    n32 = args.l3_km * 1024 // 32  # 16 -> 512 cells of 32 m, 4,096 of 4 m
    m = 16
    y0 = x0 = (h32.shape[0] - n32) // 2
    patch = h32[y0 - m : y0 + n32 + m, x0 - m : x0 + n32 + m]
    coarse = cascade.refine_surface(level, patch)[m * 8 : -m * 8, m * 8 : -m * 8]
    sl = np.s_[y0 : y0 + n32, x0 : x0 + n32]
    sea4 = cascade.refine_mask(sea32[sl], 8, 6.0)
    lake4 = cascade.refine_mask(lake32[sl], 8, 6.0) & ~sea4
    level4 = _spread(_nearest(h32[sl], 8), _nearest(lake32[sl], 8), 12)
    stream4 = hydrology.draw_streams(stream32, parent32, width32, 8, 32.0, y0, x0, n32) & ~sea4 & ~lake4
    snow4, veg4 = _nearest(snow32[sl], 8), _nearest(veg32[sl], 8)
    extra = [np.where(sea4 | lake4 | stream4, 255, 0).astype(np.uint8), cascade.soft_u8(snow4, 1.0),
             cascade.soft_u8(veg4, 1.0)]
    cond = _condition(level, coarse, extra, r.class_id)
    del extra
    h4 = _sample(model, config, coarse, cond, args.steps, args.seed + 2, args.batch).astype(np.float32)
    del cond
    w = np.clip(-coarse / L3_SEABED_M, 0, 1)
    h4 = ((1 - w) * h4 + w * coarse).astype(np.float32)
    sea4 = cascade.open_sea(h4)
    lake4 &= ~sea4
    stream4 &= ~sea4
    h4[lake4] = level4[lake4]
    water4 = sea4 | lake4 | stream4
    times["l3_s"] = time.time() - t
    print(f"L3 {h4.shape} in {times['l3_s']:.0f} s", flush=True)

    # Pictures: generated levels and the real region at the same framing.
    _png(args.out / "l1.png", h128, 128, sea128 | lake128)
    _png(args.out / "l2.png", h32, 32, sea32 | lake32 | stream32)
    _png(args.out / "real.png", np.nan_to_num(r.height), 32, r.water | cascade.open_sea(np.nan_to_num(r.height)))
    _png(args.out / "l2-l3-area.png", h32[sl], 32, (sea32 | lake32 | stream32)[sl], 1024)
    _png(args.out / "l3.png", h4, 4, water4, 2048)
    c = h4.shape[0] // 2
    _png(args.out / "l3-detail.png", h4[c - 512 : c + 512, c - 512 : c + 512], 4, water4[c - 512 : c + 512, c - 512 : c + 512], 1024)
    np.savez_compressed(args.out / "arrays.npz", l1=h128, l2=h32, l2_raw=raw32, l3=h4)

    # A 4 m world output for `topography export heightmap` and `export view`.
    s = 32.0
    ox, oy = meta["origin"][0] + x0 * s, meta["origin"][1] - y0 * s
    world_input = args.out / "world-input"
    world_input.mkdir()
    (world_input / "world.json").write_text(json.dumps(
        {"kind": "cascade_chain_input", "name": f"chain-{args.region}", "crs": meta["crs"], "origin": [ox, oy]}, indent=1))
    layers = {
        "elevation.f32": h4.astype("<f4").tobytes(),
        "water.u8": np.where(water4, 255, 0).astype(np.uint8).tobytes(),
        "snow-ice.u8": np.where(snow4, 2, 1).astype(np.uint8).tobytes(),
        "vegetation.u8": np.where(veg4, 2, 1).astype(np.uint8).tobytes(),
    }
    digests = {}
    for name, data in layers.items():
        (args.out / name).write_bytes(data)
        digests[name] = hashlib.sha256(data).hexdigest()
    (args.out / "manifest.json").write_text(json.dumps({
        "schema_version": 1, "kind": "world_preview", "world_input": str(world_input.resolve()),
        "spacing_m": 4.0, "size": [h4.shape[1], h4.shape[0]],
        "method": "cascade chain L1 -> L2 -> L3 from a derived layout", "water_fill": "ground continues under the sea; lakes flat at their spill level",
        "region": args.region, "class_id": r.class_id, "steps": args.steps, "water": vars(water), "times": times,
        "runs": {k: str(getattr(args, f"{k}_run").resolve()) for k in ("l1", "l2", "l3")},
        "layers": digests}, indent=1))
    print(f"wrote {args.out}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
