"""Blue-noise dithered class borders (docs/plans/fujix-v1.md).

Collage layouts dither region classes across their borders; training dithers the hard
borders of its per-cell labels the same way, so the models see at training time the
class maps that layouts give them.
"""

from __future__ import annotations

from functools import lru_cache

import numpy as np

TRAIN_HARD_SHARE = 0.25  # share of training windows that keep hard borders
NOISE_TILE = 256  # training noise: one tileable blue-noise tile, read at random offsets


def blue_noise(n: int, seed: int = 0) -> np.ndarray:
    """Uniform 0-1 values with little low-frequency content, so a dithered border reads as a fine mix."""
    from scipy import ndimage

    w = np.random.default_rng(seed).random((n, n), dtype=np.float32)
    hp = w - ndimage.gaussian_filter(w, 1.5, mode="wrap")
    r = np.empty(n * n, np.float32)
    r[np.argsort(hp, axis=None)] = (np.arange(n * n, dtype=np.float32) + 0.5) / (n * n)
    return r.reshape(n, n)


def weight(sd: np.ndarray, border_m: float) -> np.ndarray:
    """Soft membership: 0.5 on the outline, about 0.98 half a border inside."""
    return (1 / (1 + np.exp(-np.clip(8 * sd / border_m, -60, 60)))).astype(np.float32)


def dither(classes: list[np.ndarray], weights: list[np.ndarray], noise: np.ndarray) -> np.ndarray:
    """Each cell takes one region's class, with probability equal to that region's share."""
    total = np.maximum(np.sum(weights, axis=0), 1e-6)
    acc = np.zeros(noise.shape, np.float32)
    # Where every weight has vanished (far sea), the largest one's class.
    out = np.take_along_axis(np.stack(classes), np.argmax(weights, axis=0)[None], 0)[0]
    chosen = np.zeros(noise.shape, bool)
    for c, w in zip(classes, weights):
        acc += w / total
        pick = ~chosen & (noise < acc)
        out[pick] = c[pick]
        chosen |= pick
    return out


def dither_labels(cells: np.ndarray, cell_m: float, band_m: float, noise: np.ndarray) -> np.ndarray:
    """A label grid with its hard borders dithered across a band about `band_m` wide."""
    from scipy import ndimage

    present = np.unique(cells)
    if band_m <= 0 or len(present) < 2:
        return cells
    weights = []
    for c in present:
        inside = cells == c
        sd = (ndimage.distance_transform_edt(inside) - ndimage.distance_transform_edt(~inside)) * cell_m
        weights.append(weight(sd, band_m))
    return dither([np.full(cells.shape, c, cells.dtype) for c in present], weights, noise)


@lru_cache(maxsize=1)
def _tile() -> np.ndarray:
    return _tile_seeded(2)


def noise_at(rng: np.random.Generator, shape) -> np.ndarray:
    """Blue noise of any shape, from the tile at a random offset (it wraps seamlessly)."""
    t = _tile()
    oy, ox = (int(v) for v in rng.integers(NOISE_TILE, size=2))
    rows = (oy + np.arange(shape[0])) % NOISE_TILE
    cols = (ox + np.arange(shape[1])) % NOISE_TILE
    return t[np.ix_(rows, cols)]


def train_band(rng: np.random.Generator, max_m: float) -> float:
    """A training window's band width: hard borders for some windows, else uniform up to `max_m`."""
    if max_m <= 0 or rng.random() < TRAIN_HARD_SHARE:
        return 0.0
    return float(rng.uniform(0.0, max_m))


@lru_cache(maxsize=4)
def _tile_seeded(seed: int) -> np.ndarray:
    return blue_noise(NOISE_TILE, seed=seed)


def noise_window(y0: int, x0: int, rows: int, cols: int, seed: int) -> np.ndarray:
    """Blue noise at world cell indices: the same cell always gets the same value."""
    t = _tile_seeded(seed)
    return t[np.ix_((y0 + np.arange(rows)) % NOISE_TILE, (x0 + np.arange(cols)) % NOISE_TILE)]
