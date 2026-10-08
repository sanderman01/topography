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
    """Median form channels per model class over the training windows (direction kept
    only as its strength), for generation from a class map."""
    rng = np.random.default_rng(seed)
    out: dict[int, list[np.ndarray]] = {}
    for cls, regions in sampler._groups().items():
        for ri, idxs in regions.items():
            for i in rng.choice(idxs, size=min(windows, len(idxs)), replace=False):
                h, c, cond = sampler.window(int(i))
                f = cond[START : START + COUNT]
                m = int(sampler.lut[cls])
                v = [float(np.median(f[j])) for j in range(4)]
                v.append(float(np.median(np.hypot(f[4], f[5]))))
                out.setdefault(m, []).append(np.array(v))
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
