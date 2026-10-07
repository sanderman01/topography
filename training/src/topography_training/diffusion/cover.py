"""Snow/ice and vegetation from an authored layout, following the terrain.

python -m topography_training.diffusion.cover --world <world output> --layout <L0 dir>

The layout's forest share and snowline are read bilinearly, and its class map by
nearest cell, at positions displaced by a smooth noise field (about 150 m). The snow
and tree lines then bend with the ground (a game judgment, not a climate model):
snow lies lower on pole-facing slopes and in gullies, sheds from steep faces and
lingers less on ridges; trees reach higher in sheltered gullies and on sun-facing
slopes, thin out on steep ground and near their limit, and leave patchy edges. The
chain uses this for its 32 m and 4 m cover, and this command redraws the cover of a
finished world (for example after L4, at 1 m) and updates its manifest.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

import numpy as np

from . import layout

METHOD = ("cover v2: layout forest share, snowline and classes at ~150 m warped positions; snow and tree "
          "lines shifted by aspect, slope, curvature and noise")
WARP_M = 150.0
WARP_CELL_M = 32.0
GLACIER = 10  # glaciers and ice fields are snow/ice wherever painted
FEATURE_M = 16.0  # terrain features are measured on a grid of about this size
# Judgment values (m): how far each effect moves the snow and tree lines.
SNOW_ASPECT_M, SNOW_HOLLOW_M, SNOW_NOISE_M = 200.0, 150.0, 120.0
TREE_ASPECT_M, TREE_HOLLOW_M, TREE_NOISE_M = 80.0, 120.0, 70.0
TREE_FADE_M = 150.0  # forest thins over this band below the tree line
SNOW_STEEP_DEG = (40.0, 55.0)  # snow sheds from faces steeper than this range
TREE_STEEP_DEG = (32.0, 45.0)


def warp_field(extent_m: float, seed: int = 0) -> np.ndarray:
    """Smooth displacement (2, M, M) in metres on a WARP_CELL_M grid over the layout."""
    from scipy import ndimage

    m = max(2, int(np.ceil(extent_m / WARP_CELL_M)) + 1)
    d = ndimage.gaussian_filter(np.random.default_rng(seed).standard_normal((2, m, m)), (0, 4, 4))
    return (d / d.std() * WARP_M).astype(np.float32)


def noise_field(extent_m: float, sigma_cells: float, seed: int) -> np.ndarray:
    """Unit-std smooth noise on a WARP_CELL_M grid over the layout."""
    from scipy import ndimage

    m = max(2, int(np.ceil(extent_m / WARP_CELL_M)) + 1)
    d = ndimage.gaussian_filter(np.random.default_rng(seed).standard_normal((m, m)), sigma_cells)
    return (d / d.std()).astype(np.float32)


def _ramp(x: np.ndarray, lo: float, hi: float) -> np.ndarray:
    return np.clip((x - lo) / (hi - lo), 0, 1)


def terrain_features(h: np.ndarray, cell_m: float, pole_sign: float):
    """(slope deg, pole-facing -1..1, hollow -1..1) of a surface, measured on a ~16 m grid
    and smoothed over ~50-150 m so lines bend with landforms, not every bump."""
    from scipy import ndimage

    from .data import smooth

    k = max(1, int(round(FEATURE_M / cell_m)))
    rows, cols = h.shape[0] // k * k, h.shape[1] // k * k
    hb = np.nan_to_num(h[:rows, :cols]).reshape(rows // k, k, cols // k, k).mean(axis=(1, 3), dtype=np.float32)
    g = cell_m * k
    hs = smooth(hb, 50.0 / g)
    gy, gx = np.gradient(hs, g)
    slope = np.degrees(np.arctan(np.hypot(gx, gy)))
    # Row 0 is north: ground rising southward (gy > 0) faces north.
    pole = pole_sign * gy / np.maximum(np.hypot(gx, gy), 1e-3) * _ramp(slope, 2.0, 15.0)
    # Concave ground (gullies, hollows, cirques) is positive; convex ridges negative.
    lap = smooth(hb, 150.0 / g) - smooth(hb, 40.0 / g)
    hollow = np.tanh(lap / 15.0)

    def up(a):  # back to the input grid
        z = [(np.arange(n) + 0.5) / k - 0.5 for n in h.shape]
        yy, xx = np.meshgrid(*z, indexing="ij")
        return ndimage.map_coordinates(a, [yy, xx], order=1, mode="nearest").astype(np.float32)

    return up(slope), up(pole), up(hollow)


class Sampler:
    """Layout grids read at cell centres of any grid aligned with the layout's corner."""

    def __init__(self, l0: layout.L0):
        self.l0 = l0
        self.extent_m = l0.cell_m * l0.cls.shape[0]
        self.warp = warp_field(self.extent_m)
        # Broad (~300 m) and patchy (~100 m) noise, fixed in layout coordinates.
        self.broad = noise_field(self.extent_m, 5.0, 1)
        self.patchy = noise_field(self.extent_m, 1.5, 2)
        self.pole_sign = -1.0 if str(l0.spec.get("hemisphere", "north")).lower().startswith("s") else 1.0

    def _coords(self, y0_m: float, x0_m: float, rows: int, cols: int, cell_m: float):
        """Warped positions of cell centres in layout cells (centre of cell i at i)."""
        from scipy import ndimage

        ym = (y0_m + (np.arange(rows, dtype=np.float32) + 0.5) * cell_m)[:, None]
        xm = (x0_m + (np.arange(cols, dtype=np.float32) + 0.5) * cell_m)[None, :]
        ym, xm = np.broadcast_to(ym, (rows, cols)), np.broadcast_to(xm, (rows, cols))
        w = [ym / WARP_CELL_M, xm / WARP_CELL_M]
        dy = ndimage.map_coordinates(self.warp[0], w, order=1, mode="nearest").astype(np.float32)
        dx = ndimage.map_coordinates(self.warp[1], w, order=1, mode="nearest").astype(np.float32)
        c = self.l0.cell_m
        return [(ym + dy) / c - 0.5, (xm + dx) / c - 0.5]

    def read(self, grid: np.ndarray, coords, order: int) -> np.ndarray:
        from scipy import ndimage

        return ndimage.map_coordinates(grid, coords, order=order, mode="nearest", output=grid.dtype
                                       if order == 0 else np.float32)

    def classes(self, y0_m, x0_m, rows, cols, cell_m) -> np.ndarray:
        return self.read(self.l0.cls, self._coords(y0_m, x0_m, rows, cols, cell_m), 0)

    def _noise(self, grid: np.ndarray, y0_m, x0_m, rows, cols, cell_m) -> np.ndarray:
        from scipy import ndimage

        ym = (y0_m + (np.arange(rows, dtype=np.float32) + 0.5) * cell_m)[:, None] / WARP_CELL_M
        xm = (x0_m + (np.arange(cols, dtype=np.float32) + 0.5) * cell_m)[None, :] / WARP_CELL_M
        w = [np.broadcast_to(ym, (rows, cols)), np.broadcast_to(xm, (rows, cols))]
        return ndimage.map_coordinates(grid, w, order=1, mode="nearest").astype(np.float32)

    def fields(self, h: np.ndarray, y0_m: float, x0_m: float, cell_m: float) -> dict:
        """Snow/ice and vegetation masks and the local tree line (m) of the block of
        surface h whose north-west corner lies (y0_m, x0_m) from the layout's. Blocks of a
        bigger surface need about 300 m of margin for the terrain measures."""
        rows, cols = h.shape
        xy = self._coords(y0_m, x0_m, rows, cols, cell_m)
        land = h >= 0
        snowline = self.read(self.l0.snowline, xy, 1)
        forest = self.read(self.l0.forest.astype(np.float32), xy, 1) / 255
        cls = self.read(self.l0.cls, xy, 0)
        slope, pole, hollow = terrain_features(h, cell_m, self.pole_sign)
        broad = self._noise(self.broad, y0_m, x0_m, rows, cols, cell_m)
        patchy = self._noise(self.patchy, y0_m, x0_m, rows, cols, cell_m)
        line = snowline - SNOW_ASPECT_M * pole - SNOW_HOLLOW_M * hollow + SNOW_NOISE_M * broad
        shed = _ramp(slope, *SNOW_STEEP_DEG) > 0.5 + 0.3 * patchy
        snow = land & (((h > line) & ~shed) | (cls == GLACIER))
        treeline = (self.l0.spec["treeline_m"] - TREE_ASPECT_M * pole + TREE_HOLLOW_M * hollow
                    + TREE_NOISE_M * broad).astype(np.float32)
        # Forest share sets how much is wooded; it thins toward the tree line and on steep ground.
        share = forest * _ramp(treeline - h, 0.0, TREE_FADE_M) * (1 - _ramp(slope, *TREE_STEEP_DEG))
        veg = land & ~snow & (share + 0.25 * patchy > 0.5) & (share > 0.05)
        return {"snow": snow, "vegetation": veg, "treeline": treeline, "slope": slope}

    def cover(self, h: np.ndarray, y0_m: float, x0_m: float, cell_m: float) -> tuple[np.ndarray, np.ndarray]:
        f = self.fields(h, y0_m, x0_m, cell_m)
        return f["snow"], f["vegetation"]


def _digest(path: Path) -> str:
    d = hashlib.sha256()
    with path.open("rb") as f:
        while chunk := f.read(1 << 24):
            d.update(chunk)
    return d.hexdigest()


MARGIN_M = 320.0  # context rows around a block for the terrain measures


def redraw(world: Path, layout_dir: Path, rows: int = 2048) -> None:
    """Rewrite snow-ice.u8 and vegetation.u8 (2 present, 1 absent) of a world from its layout,
    and write biome.u8 (terrain class per cell, borders warped like the cover)."""
    manifest_path = world / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    n, cell = manifest["size"][0], float(manifest["spacing_m"])
    sampler = Sampler(layout.load(layout_dir))
    off = (sampler.extent_m - n * cell) / 2  # crops sit in the middle of the layout
    h = np.memmap(world / "elevation.f32", "<f4", "r", shape=(n, n))
    snow_out = np.memmap(world / "snow-ice.u8", np.uint8, "r+", shape=(n, n))
    veg_out = np.memmap(world / "vegetation.u8", np.uint8, "r+", shape=(n, n))
    biome_out = np.memmap(world / "biome.u8", np.uint8, "w+", shape=(n, n))
    m = int(np.ceil(MARGIN_M / cell))
    for y in range(0, n, rows):
        b = min(y + rows, n)
        a, e = max(y - m, 0), min(b + m, n)
        snow, veg = sampler.cover(np.asarray(h[a:e]), off + a * cell, off, cell)
        snow_out[y:b] = np.where(snow[y - a : b - a], 2, 1)
        veg_out[y:b] = np.where(veg[y - a : b - a], 2, 1)
        biome_out[y:b] = sampler.classes(off + y * cell, off, b - y, n, cell)
    for m in (snow_out, veg_out, biome_out):
        m.flush()
    del snow_out, veg_out, biome_out
    for name in ("snow-ice.u8", "vegetation.u8", "biome.u8"):
        manifest["layers"][name] = _digest(world / name)
    manifest["cover"] = {"method": METHOD, "layout": str(layout_dir.resolve())}
    manifest_path.write_text(json.dumps(manifest, indent=1))


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--world", type=Path, required=True)
    p.add_argument("--layout", type=Path, required=True, help="L0 directory (`topography world layout`)")
    args = p.parse_args(argv)
    if not (args.world / "manifest.json").exists():
        print("error: --world has no manifest.json", file=sys.stderr)
        return 2
    redraw(args.world, args.layout)
    print(f"redrew snow-ice.u8 and vegetation.u8 and wrote biome.u8 in {args.world}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
