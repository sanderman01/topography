"""Generate held-out comparisons, terrain statistics and a blind Yes/No review page.

python -m topography_training.diffusion.evaluate --run-dir <dir> --out <new-dir> [options]

Writes metrics.json, review.html (blind, shuffled real/generated hillshades),
key.json (which image is which; keep closed until votes are in), arrays.npz and
seam.png (MultiDiffusion strip).
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import numpy as np


def hillshade(h: np.ndarray, water: np.ndarray | None = None) -> np.ndarray:
    """RGB uint8 hillshade (sun from north-west, 45 degrees); invalid cells grey."""
    filled = np.where(np.isnan(h), np.nanmean(h), h)
    gy, gx = np.gradient(filled)
    az, alt = np.radians(315), np.radians(45)
    slope = np.arctan(np.hypot(gx, gy))
    aspect = np.arctan2(-gx, gy)
    shade = np.sin(alt) * np.cos(slope) + np.cos(alt) * np.sin(slope) * np.cos(az - aspect)
    g = (np.clip(shade, 0, 1) * 255).astype(np.uint8)
    rgb = np.stack([g, g, g], axis=-1)
    if water is not None:
        rgb[water] = (rgb[water] * np.array([0.4, 0.6, 1.0])).astype(np.uint8)
    rgb[np.isnan(h)] = (128, 128, 128)
    return rgb


def terrain_stats(h: np.ndarray, valid: np.ndarray) -> dict:
    gy, gx = np.gradient(np.where(valid, h, np.nanmean(h)))
    slope = np.degrees(np.arctan(np.hypot(gx, gy)))[valid]
    lap = (np.roll(h, 1, 0) + np.roll(h, -1, 0) + np.roll(h, 1, 1) + np.roll(h, -1, 1) - 4 * h)[valid]
    lap = lap[np.isfinite(lap)]
    return {
        "slope_mean_deg": float(slope.mean()),
        "slope_p90_deg": float(np.percentile(slope, 90)),
        "curvature_std_m": float(lap.std()) if lap.size else 0.0,
        "relief_m": float(np.nanmax(h[valid]) - np.nanmin(h[valid])),
    }


def _latest(run: Path) -> Path:
    weights = sorted(run.glob("ema-*.safetensors"))
    if not weights:
        raise FileNotFoundError("no EMA weights in run directory")
    return weights[-1]


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--run-dir", type=Path, required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--weights", type=Path)
    p.add_argument("--count", type=int, default=16, help="generated patches (same number of real)")
    p.add_argument("--steps", type=int, default=32)
    p.add_argument("--guidance", type=float, default=1.0)
    p.add_argument("--seed", type=int, default=0)
    args = p.parse_args(argv)

    import torch
    from PIL import Image
    from safetensors.torch import load_file

    from . import data
    from .model import Denoiser, sample_field

    if args.out.exists():
        print("error: output directory exists", file=sys.stderr)
        return 2
    config = json.loads((args.run_dir / "config.json").read_text())
    weights = args.weights or _latest(args.run_dir)
    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    model = Denoiser(config["arch"]).to(device).eval()
    model.load_state_dict(load_file(str(weights), device=str(device)))
    scale, size = config["residual_scale_m"], config["crop"]
    smooth_m = config.get("coarse_smooth_m", 0.0)
    tiles = data.load_release(Path(config["release"]))
    val = data.CropSampler(tiles, data.VALIDATION, size, stride=size, min_valid=0.8)
    train = data.CropSampler(tiles, data.TRAIN, size, stride=size // 2, min_valid=0.8)
    # Masked dry ground shows as grey holes that give real patches away in the blind review.
    val.positions = [(ti, y, x) for ti, y, x in val.positions
                     if (np.isnan(tiles[ti].height[y : y + size, x : x + size])
                         & ~tiles[ti].water[y : y + size, x : x + size]).mean() <= 0.01]

    # Spread picks over tiles: round-robin through each tile's validation windows.
    by_tile: dict[int, list[int]] = {}
    for idx, (ti, _, _) in enumerate(val.positions):
        by_tile.setdefault(ti, []).append(idx)
    picks, rnd = [], 0
    while len(picks) < args.count and any(rnd < len(v) for v in by_tile.values()):
        picks += [v[rnd] for v in by_tile.values() if rnd < len(v)]
        rnd += 1
    picks = picks[: args.count]

    args.out.mkdir(parents=True)
    rng = np.random.default_rng(args.seed)
    items, arrays, stats = [], {}, []
    for n, idx in enumerate(picks):
        ti, y, x = val.positions[idx]
        t = tiles[ti]
        sl = np.s_[y : y + size, x : x + size]
        real, coarse, water = t.height[sl], data.crop_coarse(t.height, y, x, size, sigma=smooth_m), t.water[sl]
        cond = data.condition(coarse, water, np.full((size, size), t.class_id))
        out = sample_field(model, torch.from_numpy(cond)[None].to(device), steps=args.steps,
                           window=size, stride=size, seed=args.seed + n, guidance=args.guidance)
        out = out[0].float().cpu().numpy()
        gen = coarse + out[0] * scale
        gen_valid = ~water
        gen_h = np.where(gen_valid, gen, np.nan)
        arrays[f"real_{n}"], arrays[f"gen_{n}"] = real, gen_h
        arrays[f"gen_snow_{n}"], arrays[f"gen_veg_{n}"] = out[1] > 0, out[2] > 0
        real_valid = ~np.isnan(real) & ~water
        stats.append({"tile": t.name, "class_id": t.class_id,
                      "real": terrain_stats(real, real_valid), "generated": terrain_stats(gen_h, gen_valid)})
        # Show real water with the same outline as generated water; fill the few remaining gaps.
        real_shown = np.where(water, np.nan, real)
        if (np.isnan(real_shown) & ~water).any():
            real_shown = np.where(water, np.nan, data.fill_nan(real_shown))
        for kind, h in (("real", real_shown), ("generated", gen_h)):
            items.append({"kind": kind, "tile": t.name, "class_id": t.class_id, "img": hillshade(h, water)})
        print(f"{n + 1}/{len(picks)} {t.name}", flush=True)

    # Memorization check: distance to the nearest training crop (4x downsampled residual).
    def small(r):
        r = np.nan_to_num(r - np.nanmean(r))
        return r.reshape(size // 4, 4, size // 4, 4).mean(axis=(1, 3)).ravel()

    bank_idx = rng.choice(len(train.positions), size=min(3000, len(train.positions)), replace=False)
    bank = []
    for i in bank_idx:
        ti, y, x = train.positions[i]
        tt = tiles[ti]
        bank.append(small(tt.height[y : y + size, x : x + size] - data.crop_coarse(tt.height, y, x, size, sigma=smooth_m)))
    bank = np.stack(bank)

    def nearest(h, coarse):
        v = small(h - coarse)
        return float(np.sqrt(((bank - v) ** 2).mean(axis=1)).min())

    for n, idx in enumerate(picks):
        ti, y, x = val.positions[idx]
        c = data.crop_coarse(tiles[ti].height, y, x, size, sigma=smooth_m)
        stats[n]["nearest_train_rms_m"] = {"real": nearest(arrays[f"real_{n}"], c),
                                           "generated": nearest(arrays[f"gen_{n}"], c)}

    seam = _seam_strip(model, tiles, size, scale, smooth_m, device, args, Image)
    np.savez_compressed(args.out / "arrays.npz", **arrays)

    order = rng.permutation(len(items))
    key = []
    cards = []
    for pos, i in enumerate(order):
        it = items[i]
        name = f"patch-{pos:03d}.png"
        Image.fromarray(it["img"]).resize((size * 2, size * 2), Image.NEAREST).save(args.out / name)
        key.append({"id": pos, "file": name, "kind": it["kind"], "tile": it["tile"], "class_id": it["class_id"]})
        cards.append(f'<figure><img src="{name}" alt="patch {pos}"><figcaption>#{pos} '
                     f'<label><input type="radio" name="p{pos}" value="yes">Yes</label> '
                     f'<label><input type="radio" name="p{pos}" value="no">No</label></figcaption></figure>')
    (args.out / "key.json").write_text(json.dumps(key, indent=1))
    (args.out / "review.html").write_text(_page(cards))
    metrics = {"schema_version": 1, "weights": str(weights), "steps": args.steps,
               "guidance": args.guidance, "patches": stats, "seam": seam}
    (args.out / "metrics.json").write_text(json.dumps(metrics, indent=1))
    print(f"wrote {args.out}", flush=True)
    return 0


def _seam_strip(model, tiles, size, scale, smooth_m, device, args, Image):
    """Generate a 2x4-window strip with MultiDiffusion; compare slopes on window joins."""
    import torch

    from . import data
    from .model import sample_field

    h, w, stride = size * 2, size * 4, size // 2
    for t in tiles:
        if t.height.shape[0] >= h and t.height.shape[1] >= w:
            break
    else:
        return None
    coarse, water = data.crop_coarse(t.height, 0, 0, (h, w), sigma=smooth_m), t.water[:h, :w]
    cond = data.condition(coarse, water, np.full((h, w), t.class_id))
    out = sample_field(model, torch.from_numpy(cond)[None].to(device), steps=args.steps,
                       window=size, stride=stride, seed=args.seed + 999, guidance=args.guidance)
    gen = coarse + out[0, 0].float().cpu().numpy() * scale
    Image.fromarray(hillshade(gen, water)).save(args.out / "seam.png")
    dx = np.abs(np.diff(gen, axis=1))
    joins = [c - 1 for c in range(stride, w, stride)]
    return {"tile": t.name, "join_to_overall_slope_ratio": float(dx[:, joins].mean() / dx.mean())}


def _page(cards: list[str]) -> str:
    body = "\n".join(cards)
    return f"""<!doctype html><html><head><meta charset="utf-8"><title>Terrain blind review</title>
<style>body{{font-family:sans-serif;background:#222;color:#eee;margin:16px}}
main{{display:flex;flex-wrap:wrap;gap:12px}}figure{{margin:0}}img{{width:384px;max-width:100%;display:block}}
textarea{{width:100%;height:6em}}</style></head><body>
<h1>Does this look like good game terrain?</h1>
<p>Some patches are real, some generated. Answer Yes/No for each, then copy the result below.</p>
<main>{body}</main>
<h2>Result</h2><textarea id="out" readonly></textarea>
<script>
const out=document.getElementById('out');
document.addEventListener('change',()=>{{const r={{}};
document.querySelectorAll('input:checked').forEach(i=>r[i.name.slice(1)]=i.value);
out.value=JSON.stringify(r);}});
</script></body></html>"""


if __name__ == "__main__":
    sys.exit(main())
