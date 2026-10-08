"""Class gates for a cascade run (see docs/plans/class-conditioning-v1.md).

python -m topography_training.diffusion.class_gates --run-dir <dir> --out <new-dir> [options]

Generated windows come from validation regions only; real windows of both splits fit
the classifier and give the real per-class roughness.

1. Class swap (reported, not gated): on fixed windows, generate every class with the
   same seed and the window's own class with another seed. With the coarse input shown,
   following it is correct at L2-L4, so a high correlation alone is not a failure.
2. Class recognition: a nearest-centroid classifier on residual features (relief,
   slope, curvature, skew, spectrum, anisotropy), fitted on real windows, must
   recognise generated residuals as their requested class. Real accuracy is measured
   leaving one region out; generated windows come from validation regions of the class
   itself (own, gated: >= 70% of real) and of other classes (swapped, reported).
3. Class roughness: correlation is blind to amplitude, so the residual roughness (std)
   generated for each class from the same windows and seeds must follow the real
   per-class roughness. Gated with the coarse input hidden (rank correlation >= 0.7);
   shown is reported. A bootstrap over windows gives a 90% interval.
4. A class grid: one window generated in every class, as a hillshade sheet.

--direction w samples with the class direction learned with the coarse input hidden
added while it is shown (model.denoise_field).

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
    """(region index, class, residual, load) for up to per_region mostly-land, fully
    surveyed windows of every region; load() rebuilds (coarse, cond), so only the
    residual is kept in memory (a 1 m library has thousands of windows)."""
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
            real, coarse, _ = sampler.window(int(i))
            out.append((ri, int(sampler.lut[sampler.regions[ri].class_id]), (real - coarse).astype(np.float32),
                        lambda i=int(i): sampler.window(i)[1:]))
    return out


def _with_class(cond: np.ndarray, class_id: int, n_classes: int, form_table: dict | None = None) -> np.ndarray:
    """A window's conditioning with another class; runs with form channels take that
    class's forms from their table, as generation does."""
    from .data import class_start

    c = cond.copy()
    if form_table is not None:
        from . import forms

        f = forms.from_classes(np.full(c.shape[1:], class_id, np.uint8), form_table, None, 1.0)
        c[forms.START : forms.START + forms.COUNT] = f
    k = class_start(c, n_classes)
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
    p.add_argument("--direction", type=float, default=0.0, help="weight of the hidden-coarse class direction")
    p.add_argument("--swap-windows", type=int, default=4)
    p.add_argument("--rough-windows", type=int, default=8, help="validation windows for the roughness gate")
    p.add_argument("--seeds", type=int, default=2, help="seeds per roughness window")
    p.add_argument("--per-region", type=int, default=6, help="real windows per region for the classifier")
    p.add_argument("--generated", type=int, default=8, help="generated windows per class and kind")
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
    from . import legend

    n_classes, lut = legend.model_lut(config)
    tab = config.get("forms", {}).get("table")
    form_scale = scale if tab is not None else None
    # The classes this model was trained on (legend v2 runs: 1-12).
    classes = CLASSES if n_classes == legend.LEGACY_CLASSES else sorted(set(int(c) for c in lut) - {0})
    regions = cascade.load_regions(Path(config["release"]), lazy=True)
    rng = np.random.default_rng(args.seed)
    windows = []
    for split in ("train", "validation"):
        try:
            got = _windows(cascade.Sampler(regions, split, level, size, lut=lut, n_classes=n_classes,
                                           form_scale=form_scale), args.per_region, rng)
        except ValueError:
            continue
        # Region indices are per split; offset validation ones so they stay distinct.
        windows += [(ri + (10_000 if split == "validation" else 0), *rest, split) for ri, *rest in got]
    held = [w for w in windows if w[4] == "validation"]
    if not held:
        print("error: no validation windows", file=sys.stderr)
        return 2
    from .cascade import DROP_CHANNELS

    hide = list(DROP_CHANNELS[config["conditioning"]][0])
    direction = (args.direction, hide) if args.direction else None

    def generate(coarse, cond, seed):
        out = sample_field(model, torch.from_numpy(cond)[None].to(device), steps=args.steps, window=size,
                           stride=size // 2, seed=seed, guidance=args.guidance, direction=direction)
        return out[0, 0].float().cpu().numpy() * scale

    def spread(pool, n):
        """n windows from pool, cycling through classes so every class is represented."""
        by = {}
        for w in rng.permutation(len(pool)):
            by.setdefault(pool[w][1], []).append(pool[w])
        out = []
        while len(out) < min(n, len(pool)):
            for c in sorted(by):
                if by[c] and len(out) < n:
                    out.append(by[c].pop())
        return out

    # 1. Class swap (reported).
    picks = spread(held, args.swap_windows)
    cross, seeds, grid = [], [], None
    for n, (_, own, _, load, _) in enumerate(picks):
        coarse, cond = load()
        res = {c: generate(coarse, _with_class(cond, c, n_classes, tab), args.seed) for c in classes}
        other = generate(coarse, _with_class(cond, own, n_classes, tab), args.seed + 1)
        corr = np.corrcoef(np.stack([res[c].ravel() for c in classes]))
        cross.append(float(corr[np.triu_indices(len(classes), 1)].mean()))
        seeds.append(float(np.corrcoef(res[own].ravel(), other.ravel())[0, 1]))
        if grid is None:
            grid = (coarse, res)
        print(f"swap {n + 1}/{len(picks)}: between classes {cross[-1]:.2f}, other seed {seeds[-1]:.2f}", flush=True)
    swap = {"mean_correlation_between_classes": float(np.mean(cross)),
            "mean_correlation_other_seed": float(np.mean(seeds)), "windows": len(picks), "gated": False}

    # 3. Class roughness (computed before recognition, which reuses the windows).
    def rank(v):
        return np.argsort(np.argsort(v)).astype(float)

    real_sd = {c: float(np.median([np.nanstd(r) for _, k, r, _, _ in windows if k == c]))
               for c in classes if any(w[1] == c for w in windows)}
    cls = sorted(real_sd)
    r_real = rank(np.array([real_sd[c] for c in cls]))
    rough_picks = spread(held, args.rough_windows)
    # std[key][window, seed, class]
    std = {key: np.zeros((len(rough_picks), args.seeds, len(cls))) for key in ("coarse_shown", "coarse_hidden")}
    for wi, (_, _, _, load, _) in enumerate(rough_picks):
        coarse, cond = load()
        for si in range(args.seeds):
            for ci, c in enumerate(cls):
                k = _with_class(cond, c, n_classes, tab)
                std["coarse_shown"][wi, si, ci] = generate(coarse, k, args.seed + 10 * si).std()
                k[hide] = 0
                std["coarse_hidden"][wi, si, ci] = generate(coarse, k, args.seed + 10 * si).std()
        print(f"roughness window {wi + 1}/{len(rough_picks)}", flush=True)
    rough = {"real_std_m": real_sd, "windows": len(rough_picks), "seeds": args.seeds}
    brng = np.random.default_rng(7)
    for key, v in std.items():
        mean = v.mean((0, 1))
        rough[f"generated_std_m_{key}"] = {c: float(m) for c, m in zip(cls, mean)}
        rough[f"rank_correlation_{key}"] = float(np.corrcoef(r_real, rank(mean))[0, 1])
        boot = [np.corrcoef(r_real, rank(v[brng.integers(0, len(v), len(v))].mean((0, 1))))[0, 1]
                for _ in range(500)]
        rough[f"rank_correlation_{key}_90"] = [float(np.nanpercentile(boot, 5)), float(np.nanpercentile(boot, 95))]
    rough["pass"] = rough["rank_correlation_coarse_hidden"] >= ROUGHNESS_RANK
    print(f"roughness rank correlation: shown {rough['rank_correlation_coarse_shown']:.2f}, "
          f"hidden {rough['rank_correlation_coarse_hidden']:.2f}", flush=True)

    # 2. Class recognition.
    x = np.stack([features(np.nan_to_num(r), cell) for _, _, r, _, _ in windows])
    y = np.array([w[1] for w in windows])
    rid = np.array([w[0] for w in windows])
    hits = []
    for ri in np.unique(rid):
        keep = rid != ri
        if len(np.unique(y[keep])) < 2:
            continue
        hits += list(Centroids(x[keep], y[keep]).predict(x[~keep]) == y[~keep])
    real_acc = float(np.mean(hits))
    clf = Centroids(x, y)
    rec = {"swapped": [], "own": []}
    confusion = np.zeros((n_classes, n_classes), int)
    for c in sorted(set(y)):
        for kind, pool in (("swapped", [w for w in held if w[1] != c]), ("own", [w for w in held if w[1] == c])):
            if not pool:
                continue
            for n, j in enumerate(rng.choice(len(pool), size=args.generated, replace=len(pool) < args.generated)):
                coarse, cond = pool[j][3]()
                got = int(clf.predict(features(generate(coarse, _with_class(cond, c, n_classes, tab), args.seed + 2 + n), cell))[0])
                rec[kind].append(got == c)
                confusion[c, got] += kind == "own"
        print(f"recognition class {c}: own {np.mean(rec['own'][-args.generated:]):.2f}", flush=True)
    recognition = {"real_accuracy_leave_region_out": real_acc,
                   "generated_swapped": float(np.mean(rec["swapped"])), "generated_own": float(np.mean(rec["own"])),
                   "classes": sorted(int(c) for c in set(y)), "confusion_own": confusion.tolist()}
    recognition["pass"] = recognition["generated_own"] >= RECOGNITION_SHARE * real_acc

    # 3. Class grid.
    args.out.mkdir(parents=True)
    coarse, res = grid
    tile = 256
    sheet = Image.new("RGB", (4 * (tile + 8) + 8, 3 * (tile + 30) + 40), (30, 30, 30))
    from PIL import ImageDraw

    d = ImageDraw.Draw(sheet)
    _label(d, (8, 8), f"{args.run_dir.name}: one window, every class, same seed", _font(18))
    for i, c in enumerate(classes):
        h = coarse + res[c]
        img = Image.fromarray(shade(h, cell, cascade.open_sea(h))).resize((tile, tile))
        x0, y0 = 8 + (i % 4) * (tile + 8), 40 + (i // 4) * (tile + 30)
        sheet.paste(img, (x0, y0 + 22))
        _label(d, (x0, y0), f"{c} {NAMES[c]}", _font(14))
    sheet.save(args.out / "class-grid.png")

    result = {"run": str(args.run_dir.resolve()), "level": level.name, "steps": args.steps, "guidance": args.guidance,
              "class_swap": swap, "class_recognition": recognition, "class_roughness": rough,
              "direction": args.direction, "pass": bool(recognition["pass"] and rough["pass"])}
    (args.out / "gates.json").write_text(json.dumps(result, indent=1))
    print(f"class swap (reported): between classes {swap['mean_correlation_between_classes']:.2f} vs other seed "
          f"{swap['mean_correlation_other_seed']:.2f}")
    print(f"recognition: real {real_acc:.2f}, generated own {recognition['generated_own']:.2f} -> "
          f"{'PASS' if recognition['pass'] else 'FAIL'} (swapped {recognition['generated_swapped']:.2f})")
    lo, hi = rough["rank_correlation_coarse_hidden_90"]
    print(f"roughness: rank correlation hidden {rough['rank_correlation_coarse_hidden']:.2f} [{lo:.2f}, {hi:.2f}] -> "
          f"{'PASS' if rough['pass'] else 'FAIL'} (shown {rough['rank_correlation_coarse_shown']:.2f})")
    return 0 if result["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
