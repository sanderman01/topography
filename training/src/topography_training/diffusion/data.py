"""Training release loading, coarse surfaces, geographic split and crop sampling.

Arrays are row-major, north-up, 1 m cells. Heights are Float32 metres.
Cover masks use int8: +1 present, -1 absent, 0 unknown.
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from pathlib import Path

import numpy as np

COARSE_M = 32
SMOOTH_M = 16.0  # Gaussian sigma applied after bicubic upsampling of the coarse blocks
WATER_SMOOTH_M = 6.0  # rounds the 10 m land-cover steps in water masks
MARGIN_M = 96  # context around a crop for its coarse surface
BAND_M = 2048
VALIDATION_M = 512
BUFFER_M = 128
NUM_CLASSES = 13  # 0 = unknown, 1..12 legend classes (terrain legend v2)
TRAIN, VALIDATION = 1, 2


@dataclass
class Tile:
    name: str
    class_id: int
    height: np.ndarray  # float32 metres, NaN where not valid ground
    water: np.ndarray  # bool
    snow: np.ndarray  # int8
    vegetation: np.ndarray  # int8
    split: np.ndarray  # uint8: 0 buffer, 1 train, 2 validation


def _read(release: Path, layer: dict, shape: tuple[int, int]) -> np.ndarray:
    data = (release / layer["path"]).read_bytes()
    if hashlib.sha256(data).hexdigest() != layer["sha256"]:
        raise ValueError(f"release layer hash mismatch: {layer['path']}")
    dtype = {"float32_le": "<f4", "uint8": "u1"}[layer["dtype"]]
    return np.frombuffer(data, dtype=dtype).reshape(shape)


def _cover(codes: np.ndarray) -> np.ndarray:
    return np.select([codes == 2, codes == 1], [1, -1], 0).astype(np.int8)


def _halve(height, valid, water, snow, veg):
    """0.5 m to 1 m: mean of fully valid 2x2 blocks; masks by majority vote."""
    h, w = height.shape[0] // 2 * 2, height.shape[1] // 2 * 2

    def blocks(a):
        return a[:h, :w].reshape(h // 2, 2, w // 2, 2)

    valid2 = blocks(valid).all(axis=(1, 3))
    height2 = blocks(height.astype(np.float64)).mean(axis=(1, 3)).astype(np.float32)
    water2 = blocks(water).sum(axis=(1, 3)) >= 2

    def vote(m):
        s = blocks(m.astype(np.int16)).sum(axis=(1, 3))
        known = blocks(m != 0).any(axis=(1, 3))
        return np.where(known, np.where(s > 0, 1, -1), 0).astype(np.int8)

    return height2, valid2, water2, vote(snow), vote(veg)


def fill_nan(a: np.ndarray) -> np.ndarray:
    """Fill NaN cells by repeated 4-neighbour averaging of known cells."""
    a = a.copy()
    if np.isnan(a).all():
        raise ValueError("no valid cells to fill from")
    while np.isnan(a).any():
        p = np.pad(a, 1, constant_values=np.nan)
        n = np.stack([p[:-2, 1:-1], p[2:, 1:-1], p[1:-1, :-2], p[1:-1, 2:]])
        count = (~np.isnan(n)).sum(axis=0)
        mean = np.nansum(n, axis=0) / np.maximum(count, 1)
        grow = np.isnan(a) & (count > 0)
        a[grow] = mean[grow]
    return a


def coarse_blocks(height: np.ndarray) -> np.ndarray:
    """Mean of valid ground per 32 m block, with empty blocks filled."""
    h, w = height.shape
    if h % COARSE_M or w % COARSE_M:
        raise ValueError("tile size must be a multiple of 32 cells")
    b = height.reshape(h // COARSE_M, COARSE_M, w // COARSE_M, COARSE_M)
    with np.errstate(invalid="ignore"):
        count = (~np.isnan(b)).sum(axis=(1, 3))
        mean = np.nansum(b, axis=(1, 3), dtype=np.float64) / count
    mean[count < COARSE_M * COARSE_M // 4] = np.nan
    return fill_nan(mean.astype(np.float32))


def upsample_coarse(blocks: np.ndarray) -> np.ndarray:
    """Bicubic 32x upsampling of a 32 m block grid to 1 m cell centres."""
    import torch
    import torch.nn.functional as F

    t = torch.from_numpy(blocks.astype(np.float64))[None, None]
    up = F.interpolate(t, scale_factor=COARSE_M, mode="bicubic", align_corners=False)
    return up[0, 0].numpy().astype(np.float32)


def smooth(a: np.ndarray, sigma: float) -> np.ndarray:
    """Separable Gaussian blur with edge replication."""
    import torch
    import torch.nn.functional as F

    r = int(3 * sigma)
    k = torch.exp(-0.5 * (torch.arange(-r, r + 1, dtype=torch.float32) / sigma) ** 2)
    k /= k.sum()
    # float64 convolution is ~20x slower on CPU; centring keeps float32 precise.
    mean = float(a.mean())
    t = torch.from_numpy((a - mean).astype(np.float32))[None, None]
    t = F.conv2d(F.pad(t, (r, r, 0, 0), mode="replicate"), k.view(1, 1, 1, -1))
    t = F.conv2d(F.pad(t, (0, 0, r, r), mode="replicate"), k.view(1, 1, -1, 1))
    return (t[0, 0].numpy() + mean).astype(np.float32)


def coarse_surface(blocks: np.ndarray, sigma: float = SMOOTH_M) -> np.ndarray:
    """1 m coarse surface from 32 m blocks, low-passed so block edges leave no trace."""
    up = upsample_coarse(blocks)
    return smooth(up, sigma) if sigma > 0 else up


def smooth_mask(mask: np.ndarray, sigma: float = WATER_SMOOTH_M) -> np.ndarray:
    if sigma <= 0:
        return mask
    return smooth(mask.astype(np.float32), sigma) > 0.5


def crop_coarse(
    height: np.ndarray, y: int, x: int, size: int | tuple[int, int], oy: int = 0, ox: int = 0, sigma: float = SMOOTH_M
) -> np.ndarray:
    """Coarse surface for one crop from a block grid shifted by (oy, ox) cells.

    Random offsets keep the model from learning a fixed block alignment.
    """
    rows, cols = (size, size) if isinstance(size, int) else size
    ny, nx = (-(-(k + 2 * MARGIN_M + COARSE_M) // COARSE_M) * COARSE_M for k in (rows, cols))
    y0, x0 = y - MARGIN_M - oy, x - MARGIN_M - ox
    region = np.full((ny, nx), np.nan, np.float32)
    sy, sx = max(y0, 0), max(x0, 0)
    ey, ex = min(y0 + ny, height.shape[0]), min(x0 + nx, height.shape[1])
    region[sy - y0 : ey - y0, sx - x0 : ex - x0] = height[sy:ey, sx:ex]
    up = coarse_surface(coarse_blocks(region), sigma)
    return up[MARGIN_M + oy : MARGIN_M + oy + rows, MARGIN_M + ox : MARGIN_M + ox + cols]


def split_mask(origin_y: float, rows: int, cols: int) -> np.ndarray:
    """Geographic split from absolute northing bands (see plan)."""
    northing = origin_y - (np.arange(rows) + 0.5)
    pos = np.mod(northing, BAND_M)
    row = np.full(rows, TRAIN, np.uint8)
    row[pos < VALIDATION_M] = VALIDATION
    row[(pos >= VALIDATION_M) & (pos < VALIDATION_M + BUFFER_M)] = 0
    row[pos >= BAND_M - BUFFER_M] = 0
    return np.repeat(row[:, None], cols, axis=1)


def release_water_smooth_m(manifest: dict) -> float:
    """Water smoothing the loader applies; game_cover_v5 and later releases arrive smoothed."""
    policy = manifest.get("cover_policy", "")
    version = int(policy.removeprefix("game_cover_v")) if policy.startswith("game_cover_v") else 0
    return 0.0 if version >= 5 else WATER_SMOOTH_M


def load_release(manifest_path: Path) -> list[Tile]:
    manifest = json.loads(manifest_path.read_text())
    if manifest.get("kind") != "diffusion_training_release" or manifest.get("schema_version") not in (1, 2):
        raise ValueError("unsupported training release")
    water_smooth_m = release_water_smooth_m(manifest)
    release = manifest_path.parent
    tiles = []
    for t in manifest["tiles"]:
        cols, rows = t["grid"]["size"]
        spacing = t["grid"]["spacing_m"]
        layers = {k: _read(release, v, (rows, cols)) for k, v in t["layers"].items()}
        valid = layers["validity"] == 255
        if "artifacts" in layers:
            # Seams (4) are left out like missing ground; stripes and facets are already repaired.
            valid &= (layers["artifacts"] & 4) == 0
        height = layers["elevation"].astype(np.float32)
        water = layers["water"] == 255
        snow, veg = _cover(layers["snow-ice"]), _cover(layers["vegetation"])
        # Source transforms carry float noise (e.g. 1.0000000000003).
        if np.allclose(spacing, 0.5, rtol=0, atol=1e-6):
            height, valid, water, snow, veg = _halve(height, valid, water, snow, veg)
        elif not np.allclose(spacing, 1.0, rtol=0, atol=1e-6):
            raise ValueError(f"unsupported spacing {spacing}")
        height = np.where(valid, height, np.nan).astype(np.float32)
        r, c = height.shape[0] // COARSE_M * COARSE_M, height.shape[1] // COARSE_M * COARSE_M
        height, water, snow, veg = height[:r, :c], smooth_mask(water[:r, :c], water_smooth_m), snow[:r, :c], veg[:r, :c]
        tiles.append(
            Tile(
                name=t["title"],
                class_id=int(t["primary_class_id"]),
                height=height,
                water=water,
                snow=snow,
                vegetation=veg,
                split=split_mask(t["grid"]["origin"][1], r, c),
            )
        )
    return tiles


def condition(coarse: np.ndarray, water: np.ndarray, classes: np.ndarray) -> np.ndarray:
    """Conditioning channels [2 + 1 + NUM_CLASSES, H, W] for one window."""
    rel = (coarse - coarse.mean()) / 256.0
    absolute = coarse / 4096.0
    onehot = np.eye(NUM_CLASSES, dtype=np.float32)[classes].transpose(2, 0, 1)
    return np.concatenate(
        [rel[None], absolute[None], np.where(water, 1.0, -1.0)[None], onehot]
    ).astype(np.float32)


COND_CHANNELS = 3 + NUM_CLASSES
TARGET_CHANNELS = 3  # residual / scale, snow/ice, vegetation


class CropSampler:
    """Random crops lying entirely inside one split with enough valid ground."""

    def __init__(
        self, tiles: list[Tile], role: int, size: int, stride: int = 16, min_valid=0.5, smooth_m=SMOOTH_M
    ):
        self.tiles, self.size, self.smooth_m = tiles, size, smooth_m
        self.positions = []
        for i, t in enumerate(tiles):
            ok = _window_sum(t.split == role, size) == size * size
            dense = _window_sum(~np.isnan(t.height), size) >= min_valid * size * size
            ys, xs = np.nonzero((ok & dense)[::stride, ::stride])
            self.positions += [(i, y * stride, x * stride) for y, x in zip(ys, xs)]
        if not self.positions:
            raise ValueError("no crops available for this split")

    def crop(self, index: int, scale: float, rng: np.random.Generator, augment: bool):
        """Return (target [3,S,S], cond [C,S,S], loss mask [3,S,S])."""
        i, y, x = self.positions[index]
        t, s = self.tiles[i], self.size
        sl = np.s_[y : y + s, x : x + s]
        oy, ox = rng.integers(COARSE_M, size=2)
        h, c = t.height[sl], crop_coarse(t.height, y, x, s, int(oy), int(ox), self.smooth_m)
        valid = ~np.isnan(h) & ~t.water[sl]
        residual = np.where(valid, (h - c) / scale, 0.0)
        snow, veg = t.snow[sl], t.vegetation[sl]
        target = np.stack([residual, snow, veg]).astype(np.float32)
        mask = np.stack([valid, snow != 0, veg != 0]).astype(np.float32)
        classes = np.full((s, s), t.class_id, np.int64)
        cond = condition(c, t.water[sl], classes)
        if augment:
            k, flip = rng.integers(4), rng.integers(2)
            target, cond, mask = (_dihedral(a, k, flip) for a in (target, cond, mask))
        return target, cond, mask

    def batch(self, n: int, scale: float, rng: np.random.Generator, augment=True):
        picks = rng.integers(len(self.positions), size=n)
        parts = [self.crop(int(p), scale, rng, augment) for p in picks]
        return tuple(np.stack(a) for a in zip(*parts))


def _dihedral(a: np.ndarray, k: int, flip: int) -> np.ndarray:
    a = np.rot90(a, k, axes=(1, 2))
    return np.ascontiguousarray(a[:, :, ::-1] if flip else a)


def _window_sum(mask: np.ndarray, size: int) -> np.ndarray:
    """Sum over each size x size window whose top-left is (y, x); out of range is 0."""
    ii = np.pad(mask.astype(np.int64).cumsum(0).cumsum(1), ((1, 0), (1, 0)))
    h, w = mask.shape
    out = np.zeros((h, w), np.int64)
    if h >= size and w >= size:
        out[: h - size + 1, : w - size + 1] = (
            ii[size:, size:] - ii[:-size, size:] - ii[size:, :-size] + ii[:-size, :-size]
        )
    return out


def residual_scale(tiles: list[Tile], smooth_m: float = SMOOTH_M) -> float:
    """Fixed metre scale: twice the residual std on training ground, to 0.5 m."""
    parts = [
        (t.height - coarse_surface(coarse_blocks(t.height), smooth_m))[(t.split == TRAIN) & ~np.isnan(t.height) & ~t.water]
        for t in tiles
    ]
    std = float(np.concatenate(parts).std())
    return max(0.5, round(2 * std * 2) / 2)
