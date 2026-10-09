"""Form channels: per-cell shape descriptors of terrain (docs/plans/biome-forms-v1.md).

The 2026-10-07 review found that generated biomes match real roughness but not real
shape: too few flats, peaks and narrow hollows, and texture with no direction. Form
channels tell a model about shape directly. Training takes them from the true surface of
each window; generation takes them from a per-class table of real medians (stored in the
run config), with the dune direction set by the layout's wind. Channels, each smoothed
over FORM_SIGMA of the window:

0. log2 of residual roughness (truth minus coarse surface) over the run's residual scale;
1. flat share (slope under 3 degrees), as -1..1;
2. peak density (local maxima standing out of their surroundings by twice the local
   roughness), as -1..1;
3. hollow share (cells three local roughnesses below the highest ground around them), as
   -1..1;
4-5. texture direction: coherence times cos and sin of twice the gradient angle
   (0, 0 is "no direction" or unknown).

A dropped (unknown) form is all zeros, as for the other dropout groups.
"""

from __future__ import annotations

import numpy as np

COUNT = 6
START = 7  # first form channel: after shape, slopes, height and the three layout extras
LAYOUTS = {6: 7, 3: 8}  # base conditioning layout -> the same with form channels
BASE = {v: k for k, v in LAYOUTS.items()}
FORM_SIGMA = 1 / 8  # smoothing, as a share of the window side
FLAT_DEG = 3.0
PEAK_PROMINENCE = 2.0  # in local residual sd: a peak stands this far above its surroundings
HOLLOW_DEPTH = 3.0  # in local residual sd below the local high ground
DIRECTIONAL = (16,)  # classes whose texture direction the layout's wind sets (crest-train dunes)


def descriptors(h: np.ndarray, coarse: np.ndarray, cell_m: float, scale_m: float, size: int) -> np.ndarray:
    """(COUNT, H, W) form channels of a true surface over its coarse surface."""
    from scipy import ndimage

    from .data import smooth

    h = np.where(np.isfinite(h), h, coarse).astype(np.float32)
    s = max(1.0, FORM_SIGMA * size)
    res = h - coarse
    sd = np.sqrt(smooth(res * res, s))
    rough = np.log2(np.maximum(sd, 0.01 * scale_m) / scale_m)
    gy, gx = np.gradient(h, cell_m)
    flat = smooth((np.hypot(gx, gy) < np.tan(np.radians(FLAT_DEG))).astype(np.float32), s)
    # Peaks and hollows relative to the local roughness, so they describe shape, not height.
    k = max(3, int(round(s / 2)) | 1)
    top = (res == ndimage.maximum_filter(res, size=k)) & (res - ndimage.minimum_filter(res, size=2 * k + 1)
                                                         > PEAK_PROMINENCE * sd)
    peaks = np.clip(smooth(top.astype(np.float32), s) * k * k / 2, 0, 1)  # two peaks per k x k cells is 1
    hollow = smooth((ndimage.maximum_filter(res, size=2 * k + 1) - res > HOLLOW_DEPTH * sd).astype(np.float32), s)
    ry, rx = np.gradient(res)
    jxx, jyy, jxy = smooth(rx * rx, s), smooth(ry * ry, s), smooth(rx * ry, s)
    den = np.maximum(jxx + jyy, 1e-9)
    c2, s2 = (jxx - jyy) / den, 2 * jxy / den  # coherence times cos/sin of twice the angle
    out = np.stack([rough, flat * 2 - 1, peaks * 2 - 1, hollow * 2 - 1, c2, s2]).astype(np.float32)
    return np.nan_to_num(out)


def rotate(cond: np.ndarray, k: int, flip: int) -> np.ndarray:
    """Direction channels after np.rot90 by k and an optional mirror of the columns:
    a quarter turn adds 180 degrees to twice the angle; a mirror negates the angle."""
    c, s = cond[START + 4].copy(), cond[START + 5].copy()
    if k % 2:
        c, s = -c, -s
    if flip:
        s = -s
    cond[START + 4], cond[START + 5] = c, s
    return cond


def table(sampler, windows: int = 6, seed: int = 0) -> dict[str, list[float]]:
    """Median form channels per model class over land cells of training windows, taken
    per cell class (a canyon region's plateau counts as plain, its sea as nothing);
    direction kept only as its strength. Used for generation from a class map."""
    from .cascade import class_start_of_cond

    rng = np.random.default_rng(seed)
    out: dict[int, list[np.ndarray]] = {}
    for regions in sampler._groups().values():
        for idxs in regions.values():
            for i in rng.choice(idxs, size=min(windows, len(idxs)), replace=False):
                h, c, cond = sampler.window(int(i))
                f = cond[START : START + COUNT]
                cls = cond[class_start_of_cond(cond, sampler.n_classes):].argmax(0)
                land = np.isfinite(h) & (h > 0)
                for m in np.unique(cls[land]):
                    cells = land & (cls == m)
                    if cells.sum() < 0.05 * cells.size:
                        continue
                    v = [float(np.median(f[j][cells])) for j in range(4)]
                    v.append(float(np.median(np.hypot(f[4], f[5])[cells])))
                    out.setdefault(int(m), []).append(np.array(v))
    return {str(c): np.median(np.stack(v), 0).round(4).tolist() for c, v in sorted(out.items()) if c > 0}


def from_classes(classes: np.ndarray, tab: dict, wind_from_deg: float | None, sigma_cells: float) -> np.ndarray:
    """(COUNT, H, W) form channels for a map of model classes from a run's table, blended
    across class borders; directional classes take the wind's axis, others none."""
    from .data import smooth

    out = np.zeros((COUNT,) + classes.shape, np.float32)
    weight = np.zeros(classes.shape, np.float32)
    axis = None
    if wind_from_deg is not None:  # gradients run along the wind across transverse crests
        a = np.radians(float(wind_from_deg))
        gx, gy = np.sin(a), -np.cos(a)  # east, and south as image rows grow
        axis = (gx * gx - gy * gy, 2 * gx * gy)
    for c in np.unique(classes):
        v = tab.get(str(int(c)))
        if v is None:
            continue
        w = smooth((classes == c).astype(np.float32), sigma_cells)
        for j in range(4):
            out[j] += w * v[j]
        if axis is not None and int(c) in DIRECTIONAL:
            out[4] += w * v[4] * axis[0]
            out[5] += w * v[4] * axis[1]
        weight += w
    return out / np.maximum(weight, 1e-6)


def main(argv=None) -> int:
    """Recompute and store the form table of a run (generation only; weights unchanged)."""
    import argparse
    import json
    from pathlib import Path

    from . import cascade, legend

    p = argparse.ArgumentParser(description="recompute a run's per-class form table")
    p.add_argument("--run-dir", type=Path, required=True)
    args = p.parse_args(argv)
    path = args.run_dir / "config.json"
    config = json.loads(path.read_text())
    level = cascade.LEVELS[config["level"]]
    n, lut = legend.model_lut(config)
    regions = cascade.load_regions(Path(config["release"]), lazy=True)
    s = cascade.Sampler(regions, "train", level, config["crop"], lut=lut, n_classes=n,
                        form_scale=config["residual_scale_m"])
    old = config["forms"].pop("table")
    config["forms"].setdefault("previous_tables", []).append(old)
    config["forms"]["table"] = table(s)
    config["forms"]["table_method"] = "land cells, per cell class"
    path.write_text(json.dumps(config, indent=2))
    print(json.dumps(config["forms"]["table"]))
    return 0


if __name__ == "__main__":
    import sys

    sys.exit(main())
