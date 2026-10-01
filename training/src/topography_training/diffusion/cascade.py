"""Cascade levels trained on context regions (see terrain-cascade-v1.md).

L1 (128 m) refines the smooth mid-surface of an L0-like layout envelope; L2 (32 m)
refines the 128 m surface, both from GLO-30 regions at 32 m. L3 (4 m) refines the 32 m
surface from lidar regions at 4 m. Each generates a residual over the level above.
Regions are row-major and north-up; the split is by whole region (set in the release).
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from pathlib import Path

import numpy as np

from .data import NUM_CLASSES, _window_sum, smooth

# L0 layout scales: the envelope and land/sea are soft so the model, not the painter,
# draws ranges, valleys and coastlines.
ENVELOPE_M = 2048
LAND_SMOOTH_M = 1024
MIN_RANGE_M = 1.0  # unit-range shape channel floor for flat windows
SEA_SMOOTH_CELLS = 2.0
SEA_LEVEL_TOLERANCE_M = 8.0


@dataclass(frozen=True)
class Level:
    name: str
    cell_m: int
    conditioning: int  # layout id stored in run configs
    region_m: int  # cell size of the regions it trains on
    block: int = 0  # refining levels: coarse block size in cells
    smooth_cells: float = 0.0  # refining levels: smoothing after upsampling
    margin: int = 16  # refining levels: context cells around a window
    stride: int = 8  # spacing of candidate training windows, in cells


# Smoothing is half a block, like the 1 m model's 16 m on 32 m blocks.
LEVELS = {
    "l1": Level("l1", 128, 4, 32),
    "l2": Level("l2", 32, 3, 32, block=4, smooth_cells=2.0),
    "l3": Level("l3", 4, 3, 4, block=8, smooth_cells=4.0, margin=32, stride=32),
}
MIN_VALID = 0.9  # share of surveyed cells a training window needs


@dataclass
class Region:
    name: str
    class_id: int
    split: str
    spacing_m: int
    height: np.ndarray  # float32 metres, NaN where not surveyed
    water: np.ndarray  # bool
    snow: np.ndarray  # bool
    vegetation: np.ndarray  # bool


def _read(release: Path, layer: dict, shape) -> np.ndarray:
    data = (release / layer["path"]).read_bytes()
    if hashlib.sha256(data).hexdigest() != layer["sha256"]:
        raise ValueError(f"release layer hash mismatch: {layer['path']}")
    dtype = {"float32_le": "<f4", "uint8": "u1"}[layer["dtype"]]
    return np.frombuffer(data, dtype=dtype).reshape(shape)


def load_release(manifest_path: Path) -> list[Region]:
    manifest = json.loads(manifest_path.read_text())
    if manifest.get("kind") != "context_training_release" or manifest.get("schema_version") != 1:
        raise ValueError("unsupported context release")
    out = []
    for r in manifest["regions"]:
        shape = (r["size"][1], r["size"][0])
        layers = {k: _read(manifest_path.parent, v, shape) for k, v in r["layers"].items()}
        height = layers["height"].astype(np.float32)
        if "validity" in layers:
            height = np.where(layers["validity"] == 255, height, np.nan).astype(np.float32)
        out.append(Region(r["id"], int(r["class_id"]), r["split"], int(r["spacing_m"]), height,
                          layers["water"] == 255, layers["snow-ice"] == 2, layers["vegetation"] == 2))
    return out


def block_mean(a: np.ndarray, k: int) -> np.ndarray:
    h, w = a.shape[0] // k * k, a.shape[1] // k * k
    return a[:h, :w].reshape(h // k, k, w // k, k).mean(axis=(1, 3), dtype=np.float64).astype(np.float32)


def upsample(a: np.ndarray, k: int) -> np.ndarray:
    """Bicubic k-times upsampling to cell centres."""
    import torch
    import torch.nn.functional as F

    t = torch.from_numpy(a.astype(np.float64))[None, None]
    return F.interpolate(t, scale_factor=k, mode="bicubic", align_corners=False)[0, 0].numpy().astype(np.float32)


def _extreme(a: np.ndarray, size: int, maximum: bool) -> np.ndarray:
    import torch
    import torch.nn.functional as F

    t = torch.from_numpy(a if maximum else -a)[None, None]
    r = size // 2
    t = F.max_pool2d(F.pad(t, (r, r, r, r), mode="replicate"), size, stride=1)
    out = t[0, 0, : a.shape[0], : a.shape[1]].numpy()
    return out if maximum else -out


def envelope(h128: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """L0 envelope from a 128 m surface: local low and high over ~2 km, smoothed at 2 km."""
    cells = ENVELOPE_M // 128
    lo = smooth(_extreme(h128, cells, False), cells)
    hi = smooth(_extreme(h128, cells, True), cells)
    return lo, np.maximum(hi, lo)


def unit_range(c: np.ndarray) -> np.ndarray:
    lo, hi = float(c.min()), float(c.max())
    return ((c - lo) / max(hi - lo, MIN_RANGE_M) * 2 - 1).astype(np.float32)


def slopes(c: np.ndarray, cell_m: float) -> tuple[np.ndarray, np.ndarray]:
    """East and north gradients (m/m) of a north-up surface, clipped to +-2."""
    gy, gx = np.gradient(c, cell_m)
    return np.clip(gx, -2, 2).astype(np.float32), np.clip(-gy, -2, 2).astype(np.float32)


def soft(mask: np.ndarray, sigma_cells: float) -> np.ndarray:
    return (np.clip(smooth(mask.astype(np.float32), sigma_cells), 0, 1) * 2 - 1).astype(np.float32)


@dataclass
class Fields:
    """Full-region inputs of one level; channel 0 holds raw coarse metres."""

    target: np.ndarray  # true surface at the level's cell size
    coarse: np.ndarray | None  # None: computed per window
    extra: list[np.ndarray]  # conditioning channels after shape, slopes and absolute height
    class_id: int


def _nan_blocks(a: np.ndarray, k: int) -> np.ndarray:
    """Block means over surveyed cells; blocks under a quarter surveyed are filled."""
    from .data import fill_nan

    h, w = a.shape[0] // k * k, a.shape[1] // k * k
    b = a[:h, :w].reshape(h // k, k, w // k, k)
    count = (~np.isnan(b)).sum(axis=(1, 3))
    if count.min() == k * k:
        return b.mean(axis=(1, 3), dtype=np.float64).astype(np.float32)
    with np.errstate(invalid="ignore"):
        mean = (np.nansum(b, axis=(1, 3), dtype=np.float64) / count).astype(np.float32)
    mean[count < k * k // 4] = np.nan
    return fill_nan(mean)


def refine_coarse(level: Level, h: np.ndarray, y: int, x: int, size: int, oy: int = 0, ox: int = 0) -> np.ndarray:
    """Coarse surface of one window: block means on a grid shifted by (oy, ox) cells,
    upsampled and smoothed. Cells beyond the region repeat its edge."""
    k, m = level.block, level.margin
    n = -(-(size + 2 * m + k) // k) * k
    rows = np.clip(np.arange(y - m - oy, y - m - oy + n), 0, h.shape[0] - 1)
    cols = np.clip(np.arange(x - m - ox, x - m - ox + n), 0, h.shape[1] - 1)
    c = smooth(upsample(_nan_blocks(h[np.ix_(rows, cols)], k), k), level.smooth_cells)
    return c[m + oy : m + oy + size, m + ox : m + ox + size]


def refine_surface(level: Level, coarse: np.ndarray) -> np.ndarray:
    """Coarse surface of a whole field from the level above (generation)."""
    return smooth(upsample(coarse, level.block), level.smooth_cells)


def refine_fields(r: Region) -> Fields:
    """Per-cell inputs; the coarse surface is computed per window (see refine_coarse)."""
    extra = [np.where(r.water, 1.0, -1.0).astype(np.float32), soft(r.snow, 1.0), soft(r.vegetation, 1.0)]
    return Fields(r.height, None, extra, r.class_id)


def l1_fields(r: Region) -> Fields:
    h = block_mean(r.height, 4)
    water = block_mean(r.water.astype(np.float32), 4) > 0.5
    lo, hi = envelope(h)
    mid = (lo + hi) / 2
    extra = [((hi - lo) / 2048.0).astype(np.float32), soft(~water, LAND_SMOOTH_M / 128)]
    return Fields(h, mid.astype(np.float32), extra, r.class_id)


def _grow(mask: np.ndarray) -> np.ndarray:
    out = mask.copy()
    out[1:] |= mask[:-1]
    out[:-1] |= mask[1:]
    out[:, 1:] |= mask[:, :-1]
    out[:, :-1] |= mask[:, 1:]
    return out


def resolve_sea(h: np.ndarray, land: np.ndarray, coarse: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """Put generated sea at 0 m. `land` is the soft land/water condition (-1 water .. 1 land);
    only water where the coarse surface is below 100 m counts as sea, not mountain lakes.

    The model leaves the sea a few metres off zero, so the field is shifted by the median
    height on the layout's sea side; sea is then the low ground connected to that side,
    without single-cell specks, and is set to 0 m like GLO-30 sea."""
    zone = (land < 0) & (coarse < 100.0)
    if not zone.any():
        return h, np.zeros(h.shape, bool)
    h = h - float(np.median(h[zone]))
    # The generated sea surface wanders by about +-10 m; decide on a smoothed surface.
    low = smooth(h, SEA_SMOOTH_CELLS) <= SEA_LEVEL_TOLERANCE_M
    sea = zone & low
    while True:
        grown = _grow(sea) & low
        if (grown == sea).all():
            break
        sea = grown
    for _ in range(3):  # cells with at most one neighbour of their kind are specks
        land_n = sum(np.roll(~sea, s, a) for s in (1, -1) for a in (0, 1))
        sea = (sea & (land_n < 4)) | (~sea & (land_n <= 1))
    return np.where(sea, 0.0, h).astype(np.float32), sea


def condition(level: Level, coarse: np.ndarray, extra: list[np.ndarray], classes: np.ndarray) -> np.ndarray:
    """Conditioning for one window. Channel 0 is raw coarse metres here; `normalise`
    turns it into the per-window unit-range shape channel (training and generation)."""
    ex, ny = slopes(coarse, level.cell_m)
    parts = [coarse[None], ex[None], ny[None], (coarse / 4096.0)[None]] + [e[None] for e in extra]
    parts.append(np.eye(NUM_CLASSES, dtype=np.float32)[classes].transpose(2, 0, 1))
    return np.concatenate(parts).astype(np.float32)


def normalise(cond: np.ndarray) -> np.ndarray:
    cond = cond.copy()
    cond[0] = unit_range(cond[0])
    return cond


class Sampler:
    """Random windows of one split with the CropSampler interface of the 1 m model."""

    def __init__(self, regions: list[Region], split: str, level: Level, size: int, stride: int | None = None):
        self.level, self.size = level, size
        stride = stride or level.stride
        self.regions = [r for r in regions if r.split == split]
        if not self.regions:
            raise ValueError(f"no {split} regions")
        if any(r.spacing_m != level.region_m for r in self.regions):
            raise ValueError(f"{level.name} trains on {level.region_m} m regions")
        self.fields = [l1_fields(r) if level.name == "l1" else refine_fields(r) for r in self.regions]
        self.positions = []
        for i, f in enumerate(self.fields):
            rows, cols = f.target.shape
            valid = _window_sum(~np.isnan(f.target), size) >= MIN_VALID * size * size
            ys, xs = range(0, rows - size + 1, stride), range(0, cols - size + 1, stride)
            self.positions += [(i, y, x) for y in ys for x in xs if valid[y, x]]
        if not self.positions:
            raise ValueError("no window of this size has enough surveyed ground")

    def window(self, index: int, rng: np.random.Generator | None = None):
        """(true surface, coarse, cond with raw channel 0) of one window. Refining levels
        draw a random block offset so the model cannot learn a fixed alignment."""
        i, y, x = self.positions[index]
        f, s = self.fields[i], self.size
        sl = np.s_[y : y + s, x : x + s]
        if f.coarse is None:
            k = self.level.block
            oy, ox = (int(v) for v in rng.integers(k, size=2)) if rng is not None else (0, 0)
            c = refine_coarse(self.level, f.target, y, x, s, oy, ox)
        else:
            c = f.coarse[sl]
        classes = np.full((s, s), f.class_id, np.int64)
        return f.target[sl], c, condition(self.level, c, [e[sl] for e in f.extra], classes)

    def crop(self, index: int, scale: float, rng: np.random.Generator, augment: bool):
        h, c, cond = self.window(index, rng)
        valid = ~np.isnan(h)
        target = np.where(valid, (h - c) / scale, 0.0)[None].astype(np.float32)
        cond = normalise(cond)
        mask = valid[None].astype(np.float32)
        if augment:
            k, flip = rng.integers(4), rng.integers(2)
            target, cond, mask = (_dihedral(a, k, flip) for a in (target, cond, mask))
            cond = _rotate_slopes(cond, int(k), int(flip))
        return target, cond, mask

    def batch(self, n: int, scale: float, rng: np.random.Generator, augment=True):
        picks = rng.integers(len(self.positions), size=n)
        parts = [self.crop(int(p), scale, rng, augment) for p in picks]
        return tuple(np.stack(a) for a in zip(*parts))


def _dihedral(a: np.ndarray, k: int, flip: int) -> np.ndarray:
    a = np.rot90(a, k, axes=(1, 2))
    return np.ascontiguousarray(a[:, :, ::-1] if flip else a)


def _rotate_slopes(cond: np.ndarray, k: int, flip: int) -> np.ndarray:
    """Recompute slope channels after rotating the grid so they stay east/north."""
    # Channel 0 is already unit-range; its gradients point the same way as the metre field's.
    ex, ny = cond[1].copy(), cond[2].copy()
    for _ in range(k):  # np.rot90 turns counter-clockwise: east becomes north
        ex, ny = -ny, ex
    if flip:
        ex = -ex
    cond[1], cond[2] = ex, ny
    return cond


def residual_scale(sampler: Sampler, windows: int = 400) -> float:
    """Fixed metre scale: twice the residual std over training windows, to 0.5 m."""
    rng = np.random.default_rng(0)
    parts = []
    for p in rng.choice(len(sampler.positions), size=min(windows, len(sampler.positions)), replace=False):
        h, c, _ = sampler.window(int(p))
        parts.append((h - c)[~np.isnan(h)])
    std = float(np.concatenate(parts).std())
    return max(0.5, round(2 * std * 2) / 2)
