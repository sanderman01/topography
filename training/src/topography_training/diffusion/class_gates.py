"""Class gates for a cascade run (see docs/plans/class-conditioning-v1.md).

python -m topography_training.diffusion.class_gates --run-dir <dir> --out <new-dir> [options]

1. Class swap: on fixed windows, generate every class with the same seed and the
   window's own class with another seed. The class must change the residual more than
   the seed does (mean correlation between classes below the seed correlation).
2. Class recognition: a nearest-centroid classifier on residual features (relief,
   slope, curvature, skew, spectrum, anisotropy), fitted on real windows, must
   recognise generated residuals as their requested class. Real accuracy is measured
   leaving one region out; generated windows come from regions of other classes
   (swapped) and of the class itself (own). Gate: swapped recognition >= 70% of real.
3. Class roughness: correlation is blind to amplitude, so the residual roughness (std)
   generated for each class from the same windows must follow the real per-class
   roughness (rank correlation >= 0.7), with the coarse input shown and hidden.
4. A class grid: one window generated in every class, as a hillshade sheet.

Writes gates.json and class-grid.png; exit status 1 when a gate fails.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import numpy as np

CLASSES = list(range(1, 13))
RECOGNITION_SHARE = 0.7
ROUGHNESS_RANK = 0.7


def features(r: np.ndarray, cell_m: float) -> np.ndarray:
    """Scale-free description of a residual (generated or true minus coarse)."""
    r = r - r.mean()
    gy, gx = np.gradient(r, cell_m)
    g = np.hypot(gx, gy)
    lap = (np.roll(r, 1, 0) + np.roll(r, -1, 0) + np.roll(r, 1, 1) + np.roll(r, -1, 1) - 4 * r)[1:-1, 1:-1]
    sd = r.std() + 1e-6
    # Structure tensor coherence: 1 for parallel ridges (dunes), 0 for isotropic texture.
    jxx, jyy, jxy = (gx * gx).mean(), (gy * gy).mean(), (gx * gy).mean()
    coherence = np.sqrt((jxx - jyy) ** 2 + 4 * jxy**2) / (jxx + jyy + 1e-12)
    # Radially averaged power spectrum slope (log-log).
    f = np.abs(np.fft.rfft2(r)) ** 2
    ky = np.fft.fftfreq(r.shape[0])[:, None]
    kx = np.fft.rfftfreq(r.shape[1])[None, :]
    k = np.hypot(ky, kx)
    bins = np.logspace(np.log10(2 / r.shape[0]), np.log10(0.5), 12)
    idx = np.digitize(k.ravel(), bins)
    power = np.array([f.ravel()[idx == i].mean() if (idx == i).any() else np.nan for i in range(1, len(bins))])
    ok = np.isfinite(power) & (power > 0)
    slope = np.polyfit(np.log(bins[1:][ok]), np.log(power[ok]), 1)[0] if ok.sum() > 2 else 0.0
    return np.array([
        np.log(sd), ((r / sd) ** 3).mean(), ((r / sd) ** 4).mean(),
        np.log(g.mean() + 1e-6), np.log(np.percentile(g, 90) + 1e-6),
        np.log(lap.std() + 1e-6) - np.log(sd), coherence, slope,
    ])


class Centroids:
    """Nearest centroid in standardised feature space."""

    def __init__(self, x: np.ndarray, y: np.ndarray):
        self.mu, self.sd = x.mean(0), x.std(0) + 1e-6
        z = (x - self.mu) / self.sd
        self.classes = np.unique(y)
        self.c = np.stack([z[y == k].mean(0) for k in self.classes])

    def predict(self, x: np.ndarray) -> np.ndarray:
        z = (np.atleast_2d(x) - self.mu) / self.sd
        return self.classes[((z[:, None] - self.c[None]) ** 2).sum(-1).argmin(1)]


def _windows(sampler, per_region: int, rng) -> list[tuple]:
    """(region index, class, real, coarse, cond) for up to per_region mostly-land,
    fully surveyed windows of every region."""
    by_region: dict[int, list[int]] = {}
    for idx, (ri, y, x) in enumerate(sampler.positions):
        by_region.setdefault(ri, []).append(idx)
    out = []
    s = sampler.size
    for ri, idxs in by_region.items():
        f = sampler.fields[ri]
        good = [i for i in rng.permutation(idxs)
                if (lambda t: np.isfinite(t).all() and (t >= 0).mean() > 0.6)(
                    f.target[sampler.positions[i][1]: sampler.positions[i][1] + s,
                             sampler.positions[i][2]: sampler.positions[i][2] + s])]
        for i in good[:per_region]:
            real, coarse, cond = sampler.window(int(i))
            out.append((ri, sampler.regions[ri].class_id, real, coarse, cond))
    return out


def _with_class(cond: np.ndarray, class_id: int) -> np.ndarray:
    from .data import class_start

    c = cond.copy()
    k = class_start(c)
    c[k:] = 0
    c[k + class_id] = 1
    return c


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--run-dir", type=Path, required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--weights", type=Path)
    p.add_argument("--class-runs", type=Path,
                   help="directory of per-class fine-tuned runs c1..c12 over --run-dir (a ClassMixture)")
    p.add_argument("--steps", type=int, default=12)
    p.add_argument("--guidance", type=float, default=1.0)
    p.add_argument("--swap-windows", type=int, default=6)
    p.add_argument("--per-region", type=int, default=6, help="real windows per region for the classifier")
    p.add_argument("--generated", type=int, default=4, help="generated windows per class and kind")
    p.add_argument("--seed", type=int, default=0)
    args = p.parse_args(argv)

    import torch
    from PIL import Image
    from safetensors.torch import load_file

    from . import cascade
    from .cascade_eval import shade
    from .evaluate import _latest
    from .layout import NAMES, _font, _label
    from .model import Denoiser, sample_field

    if args.out.exists():
        print("error: output directory exists", file=sys.stderr)
        return 2
    config = json.loads((args.run_dir / "config.json").read_text())
    level = cascade.LEVELS[config["level"]]
    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    if args.class_runs:
        from .model import load_mixture

        runs = {c: args.class_runs / f"c{c}" for c in CLASSES if (args.class_runs / f"c{c}" / "config.json").exists()}
        model = load_mixture(args.run_dir, runs, device)
        print(f"mixture of {len(runs)} per-class runs over {args.run_dir.name}", flush=True)
    else:
        model = Denoiser(config["arch"], config["conditioning"]).to(device).eval()
        model.load_state_dict(load_file(str(args.weights or _latest(args.run_dir)), device=str(device)))
    scale, size, cell = config["residual_scale_m"], config["crop"], float(level.cell_m)
    regions = cascade.load_regions(Path(config["release"]))
    rng = np.random.default_rng(args.seed)
    windows = []
    for split in ("train", "validation"):
        try:
            windows += _windows(cascade.Sampler(regions, split, level, size), args.per_region, rng)
        except ValueError:
            pass

    def generate(coarse, cond, seed):
        out = sample_field(model, torch.from_numpy(cond)[None].to(device), steps=args.steps, window=size,
                           stride=size // 2, seed=seed, guidance=args.guidance)
        return out[0, 0].float().cpu().numpy() * scale

    # 1. Class swap.
    picks = [windows[i] for i in rng.choice(len(windows), size=min(args.swap_windows, len(windows)), replace=False)]
    cross, seeds, grid = [], [], None
    for n, (_, own, _, coarse, cond) in enumerate(picks):
        res = {c: generate(coarse, _with_class(cond, c), args.seed) for c in CLASSES}
        other = generate(coarse, _with_class(cond, own), args.seed + 1)
        corr = np.corrcoef(np.stack([res[c].ravel() for c in CLASSES]))
        cross.append(float(corr[np.triu_indices(len(CLASSES), 1)].mean()))
        seeds.append(float(np.corrcoef(res[own].ravel(), other.ravel())[0, 1]))
        if grid is None:
            grid = (coarse, res)
        print(f"swap {n + 1}/{len(picks)}: between classes {cross[-1]:.2f}, other seed {seeds[-1]:.2f}", flush=True)
    swap = {"mean_correlation_between_classes": float(np.mean(cross)),
            "mean_correlation_other_seed": float(np.mean(seeds)), "windows": len(picks)}
    swap["pass"] = swap["mean_correlation_between_classes"] < swap["mean_correlation_other_seed"]

    # 3. Class roughness (computed before recognition, which reuses the windows).
    def rank(v):
        return np.argsort(np.argsort(v)).astype(float)

    real_sd = {c: float(np.median([np.nanstd(r - co) for _, k, r, co, _ in windows if k == c]))
               for c in CLASSES if any(k == c for _, k, _, _, _ in windows)}
    shown = {c: [] for c in real_sd}
    hidden = {c: [] for c in real_sd}
    from .cascade import DROP_CHANNELS

    for _, _, _, coarse, cond in picks[:3]:
        for c in real_sd:
            shown[c].append(float(generate(coarse, _with_class(cond, c), args.seed).std()))
            k = _with_class(cond, c)
            k[list(DROP_CHANNELS[config["conditioning"]][0])] = 0
            hidden[c].append(float(generate(coarse, k, args.seed).std()))
    cls = sorted(real_sd)
    r_real = rank(np.array([real_sd[c] for c in cls]))
    rough = {"real_std_m": real_sd, "generated_std_m_coarse_shown": {c: float(np.mean(shown[c])) for c in cls},
             "generated_std_m_coarse_hidden": {c: float(np.mean(hidden[c])) for c in cls}}
    for key in ("coarse_shown", "coarse_hidden"):
        g = rank(np.array([rough[f"generated_std_m_{key}"][c] for c in cls]))
        rough[f"rank_correlation_{key}"] = float(np.corrcoef(r_real, g)[0, 1])
    rough["pass"] = min(rough["rank_correlation_coarse_shown"], rough["rank_correlation_coarse_hidden"]) >= ROUGHNESS_RANK
    print(f"roughness rank correlation: shown {rough['rank_correlation_coarse_shown']:.2f}, "
          f"hidden {rough['rank_correlation_coarse_hidden']:.2f}", flush=True)

    # 2. Class recognition.
    x = np.stack([features(np.nan_to_num(real - coarse), cell) for _, _, real, coarse, _ in windows])
    y = np.array([c for _, c, _, _, _ in windows])
    rid = np.array([ri for ri, _, _, _, _ in windows])
    hits = []
    for ri in np.unique(rid):
        keep = rid != ri
        if len(np.unique(y[keep])) < 2:
            continue
        hits += list(Centroids(x[keep], y[keep]).predict(x[~keep]) == y[~keep])
    real_acc = float(np.mean(hits))
    clf = Centroids(x, y)
    rec = {"swapped": [], "own": []}
    confusion = np.zeros((13, 13), int)
    for c in sorted(set(y)):
        for kind, pool in (("swapped", [w for w in windows if w[1] != c]), ("own", [w for w in windows if w[1] == c])):
            for j in rng.choice(len(pool), size=min(args.generated, len(pool)), replace=False):
                _, _, _, coarse, cond = pool[j]
                got = int(clf.predict(features(generate(coarse, _with_class(cond, c), args.seed + 2), cell))[0])
                rec[kind].append(got == c)
                if kind == "swapped":
                    confusion[c, got] += 1
        print(f"recognition class {c}: swapped {np.mean(rec['swapped'][-args.generated:]):.2f}", flush=True)
    recognition = {"real_accuracy_leave_region_out": real_acc,
                   "generated_swapped": float(np.mean(rec["swapped"])), "generated_own": float(np.mean(rec["own"])),
                   "classes": sorted(int(c) for c in set(y)), "confusion_swapped": confusion.tolist()}
    recognition["pass"] = recognition["generated_swapped"] >= RECOGNITION_SHARE * real_acc

    # 3. Class grid.
    args.out.mkdir(parents=True)
    coarse, res = grid
    tile = 256
    sheet = Image.new("RGB", (4 * (tile + 8) + 8, 3 * (tile + 30) + 40), (30, 30, 30))
    from PIL import ImageDraw

    d = ImageDraw.Draw(sheet)
    _label(d, (8, 8), f"{args.run_dir.name}: one window, every class, same seed", _font(18))
    for i, c in enumerate(CLASSES):
        h = coarse + res[c]
        img = Image.fromarray(shade(h, cell, cascade.open_sea(h))).resize((tile, tile))
        x0, y0 = 8 + (i % 4) * (tile + 8), 40 + (i // 4) * (tile + 30)
        sheet.paste(img, (x0, y0 + 22))
        _label(d, (x0, y0), f"{c} {NAMES[c]}", _font(14))
    sheet.save(args.out / "class-grid.png")

    result = {"run": str(args.run_dir.resolve()), "level": level.name, "steps": args.steps, "guidance": args.guidance,
              "class_swap": swap, "class_recognition": recognition, "class_roughness": rough,
              "pass": bool(swap["pass"] and recognition["pass"] and rough["pass"])}
    (args.out / "gates.json").write_text(json.dumps(result, indent=1))
    print(f"class swap: between classes {swap['mean_correlation_between_classes']:.2f} vs other seed "
          f"{swap['mean_correlation_other_seed']:.2f} -> {'PASS' if swap['pass'] else 'FAIL'}")
    print(f"recognition: real {real_acc:.2f}, generated swapped {recognition['generated_swapped']:.2f}, own "
          f"{recognition['generated_own']:.2f} -> {'PASS' if recognition['pass'] else 'FAIL'}")
    print(f"roughness: rank correlation shown {rough['rank_correlation_coarse_shown']:.2f}, hidden "
          f"{rough['rank_correlation_coarse_hidden']:.2f} -> {'PASS' if rough['pass'] else 'FAIL'}")
    return 0 if result["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
