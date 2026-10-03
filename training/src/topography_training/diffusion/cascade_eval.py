"""Blind review and statistics for a cascade level on held-out context regions.

python -m topography_training.diffusion.cascade_eval --run-dir <dir> --out <new-dir> [options]

L1 generates each validation region whole (48 km at 128 m) from its derived layout; L2
L3 and L4 generate windows (8 km at 32 m, 1 km at 4 m, 256 m at 1 m) from the true surface of the level above. The review shows real terrain
for half of the items and generated terrain for the other half, never the same place
twice. Writes review.html, key.json (keep closed until votes are in), compare.html
(side by side, also closed), metrics.json and arrays.npz.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import numpy as np

from .evaluate import _latest, _page


def shade(h: np.ndarray, cell_m: float, sea: np.ndarray) -> np.ndarray:
    """RGB hillshade (north-west sun, 45 degrees) with sea tinted blue."""
    gy, gx = np.gradient(h, cell_m)
    az, alt = np.radians(315), np.radians(45)
    slope = np.arctan(np.hypot(gx, gy))
    aspect = np.arctan2(-gx, gy)
    s = np.sin(alt) * np.cos(slope) + np.cos(alt) * np.sin(slope) * np.cos(az - aspect)
    g = (np.clip(s, 0, 1) * 255).astype(np.uint8)
    rgb = np.stack([g, g, g], axis=-1)
    rgb[sea] = (rgb[sea] * np.array([0.35, 0.55, 1.0])).astype(np.uint8)
    return rgb


def stats(h: np.ndarray, cell_m: float) -> dict:
    gy, gx = np.gradient(h, cell_m)
    slope = np.degrees(np.arctan(np.hypot(gx, gy)))
    lap = (np.roll(h, 1, 0) + np.roll(h, -1, 0) + np.roll(h, 1, 1) + np.roll(h, -1, 1) - 4 * h)[1:-1, 1:-1]
    return {"slope_mean_deg": float(slope.mean()), "slope_p90_deg": float(np.percentile(slope, 90)),
            "curvature_std_m": float(lap.std()), "relief_m": float(h.max() - h.min()),
            "land_share": float((h >= 0).mean())}


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--run-dir", type=Path, required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--weights", type=Path)
    p.add_argument("--count", type=int, default=16, help="L2 review items of each kind")
    p.add_argument("--steps", type=int, default=32)
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--seed-check", type=int, default=8,
                   help="windows generated with one seed; their residual correlation shows whether the level above steers the output")
    args = p.parse_args(argv)

    import torch
    from PIL import Image
    from safetensors.torch import load_file

    from . import cascade
    from .model import Denoiser, sample_field

    if args.out.exists():
        print("error: output directory exists", file=sys.stderr)
        return 2
    config = json.loads((args.run_dir / "config.json").read_text())
    if config.get("kind") != "terrain_cascade_run":
        print("error: not a cascade run", file=sys.stderr)
        return 2
    level = cascade.LEVELS[config["level"]]
    weights = args.weights or _latest(args.run_dir)
    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    model = Denoiser(config["arch"], config["conditioning"]).to(device).eval()
    model.load_state_dict(load_file(str(weights), device=str(device)))
    scale, size = config["residual_scale_m"], config["crop"]
    regions = cascade.load_regions(Path(config["release"]))
    val = cascade.Sampler(regions, "validation", level, size, stride=size)
    train = cascade.Sampler(regions, "train", level, size, stride=size // 4)

    def generate(coarse, cond, seed):
        out = sample_field(model, torch.from_numpy(cond)[None].to(device), steps=args.steps,
                           window=size, stride=size // 2, seed=seed)
        return coarse + out[0, 0].float().cpu().numpy() * scale

    # Items: (name, class, real surface, coarse, generated surface).
    items = []
    if level.name == "l1":
        for n, (r, f) in enumerate(zip(val.regions, val.fields)):
            classes = np.full(f.coarse.shape, f.class_id, np.int64)
            cond = cascade.condition(level, f.coarse, [cascade.decode(e) for e in f.extra], classes)
            gen = generate(f.coarse, cond, args.seed + n)
            items.append((r.name, r.class_id, f.target, f.coarse, gen))
            print(f"{n + 1}/{len(val.regions)} {r.name}", flush=True)
    else:
        by_region: dict[int, list[int]] = {}
        for idx, (ri, y, x) in enumerate(val.positions):
            # Mostly-sea windows show little but a smooth seabed.
            if (val.fields[ri].target[y : y + size, x : x + size] < 0).mean() > 0.5:
                continue
            # Survey gaps would give real windows away.
            if np.isnan(val.fields[ri].target[y : y + size, x : x + size]).mean() > 0.005:
                continue
            by_region.setdefault(ri, []).append(idx)
        picks, rnd = [], 0
        while len(picks) < 2 * args.count and any(rnd < len(v) for v in by_region.values()):
            picks += [v[rnd] for v in by_region.values() if rnd < len(v)]
            rnd += 1
        for n, idx in enumerate(picks[: 2 * args.count]):
            real, coarse, cond = val.window(idx)
            real = np.where(np.isnan(real), coarse, real)
            r = val.regions[val.positions[idx][0]]
            items.append((r.name, r.class_id, real, coarse, generate(coarse, cond, args.seed + n)))
            print(f"{n + 1}/{2 * args.count} {r.name}", flush=True)

    # Same-seed check: residuals that correlate across different windows ignore their input.
    seed_check = None
    if level.name != "l1" and args.seed_check > 1:
        res = []
        for idx in picks[: args.seed_check]:
            _, coarse, cond = val.window(idx)
            res.append((generate(coarse, cond, args.seed) - coarse).ravel())
        corr = np.corrcoef(np.stack(res))
        seed_check = {"windows": len(res), "seed": args.seed,
                      "mean_residual_correlation": float(corr[np.triu_indices(len(res), 1)].mean())}
        print(f"same-seed residual correlation {seed_check['mean_residual_correlation']:.3f}", flush=True)

    # Memorization check: distance to the nearest training residual (4x downsampled).
    def small(r):
        r = r - r.mean()
        k = 4
        return r[: r.shape[0] // k * k, : r.shape[1] // k * k].reshape(r.shape[0] // k, k, -1, k).mean(axis=(1, 3))

    rng = np.random.default_rng(args.seed)
    bank = []
    for i in rng.choice(len(train.positions), size=min(3000, len(train.positions)), replace=False):
        real, coarse, _ = train.window(int(i))
        bank.append(small(np.where(np.isnan(real), coarse, real) - coarse).ravel())
    bank = np.stack(bank)

    def nearest(h, coarse):
        v = small((h - coarse)[:size, :size]).ravel()
        return float(np.sqrt(((bank - v) ** 2).mean(axis=1)).min())

    args.out.mkdir(parents=True)
    arrays, metrics, shown, pairs = {}, [], [], []
    zoom = max(1, 768 // max(items[0][2].shape))
    for n, (name, class_id, real, coarse, gen) in enumerate(items):
        arrays[f"real_{n}"], arrays[f"gen_{n}"] = real, gen
        metrics.append({"region": name, "class_id": class_id, "real": stats(real, level.cell_m),
                        "generated": stats(gen, level.cell_m),
                        "nearest_train_rms_m": {"real": nearest(real, coarse), "generated": nearest(gen, coarse)}})
        imgs = {k: Image.fromarray(shade(h, level.cell_m, cascade.open_sea(h))).resize(
            (h.shape[1] * zoom, h.shape[0] * zoom), Image.NEAREST) for k, h in (("real", real), ("generated", gen))}
        for k, img in imgs.items():
            img.save(args.out / f"{k}-{n:03d}.png")
        pairs.append((name, n))
        kind = "real" if n % 2 == 0 else "generated"
        shown.append({"kind": kind, "region": name, "class_id": class_id, "file": f"{kind}-{n:03d}.png"})

    order = rng.permutation(len(shown))
    key, cards = [], []
    for pos, i in enumerate(order):
        it = shown[i]
        key.append({"id": pos} | it)
        cards.append(f'<figure><img src="{it["file"]}" alt="item {pos}"><figcaption>#{pos} '
                     f'<label><input type="radio" name="p{pos}" value="yes">Yes</label> '
                     f'<label><input type="radio" name="p{pos}" value="no">No</label></figcaption></figure>')
    (args.out / "key.json").write_text(json.dumps(key, indent=1))
    span = items[0][2].shape[0] * level.cell_m
    span = f"{span / 1000:.0f} km" if span >= 1000 else f"{span} m"
    page = _page(cards).replace("<h1>Does this look like good game terrain?</h1>",
                                f"<h1>Does this look like real terrain? ({span}, {level.cell_m} m cells)</h1>")
    (args.out / "review.html").write_text(page.replace("width:384px", "width:560px"))
    rows = "".join(f'<h3>{name}</h3><div class="pair"><img src="real-{n:03d}.png"><img src="generated-{n:03d}.png"></div>'
                   for name, n in pairs)
    (args.out / "compare.html").write_text(
        '<!doctype html><meta charset="utf-8"><title>Real vs generated</title><style>body{background:#222;color:#eee;'
        'font-family:sans-serif}.pair{display:flex;gap:8px}.pair img{width:48%}</style>'
        f"<h1>Left real, right generated</h1>{rows}")
    np.savez_compressed(args.out / "arrays.npz", **arrays)
    (args.out / "metrics.json").write_text(json.dumps(
        {"schema_version": 1, "level": level.name, "weights": str(weights), "steps": args.steps,
         "same_seed": seed_check, "items": metrics},
        indent=1))
    print(f"wrote {args.out}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
