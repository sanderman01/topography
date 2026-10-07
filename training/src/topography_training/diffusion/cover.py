"""Snow/ice and vegetation from an authored layout without its 128 m grid steps.

python -m topography_training.diffusion.cover --world <world output> --layout <L0 dir>

The layout's forest share and snowline are read bilinearly, and its class map by
nearest cell. All three are read at positions displaced by a smooth noise field (about
150 m), so borders follow the terrain and the noise instead of the painted grid. The
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

METHOD = "cover v1: layout forest share and snowline read bilinearly, classes by nearest cell, at ~150 m warped positions"
WARP_M = 150.0
WARP_CELL_M = 32.0
GLACIER = 10  # glaciers and ice fields are snow/ice wherever painted


def warp_field(extent_m: float, seed: int = 0) -> np.ndarray:
    """Smooth displacement (2, M, M) in metres on a WARP_CELL_M grid over the layout."""
    from scipy import ndimage

    m = max(2, int(np.ceil(extent_m / WARP_CELL_M)) + 1)
    d = ndimage.gaussian_filter(np.random.default_rng(seed).standard_normal((2, m, m)), (0, 4, 4))
    return (d / d.std() * WARP_M).astype(np.float32)


class Sampler:
    """Layout grids read at cell centres of any grid aligned with the layout's corner."""

    def __init__(self, l0: layout.L0):
        self.l0 = l0
        self.extent_m = l0.cell_m * l0.cls.shape[0]
        self.warp = warp_field(self.extent_m)

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

    def cover(self, h: np.ndarray, y0_m: float, x0_m: float, cell_m: float) -> tuple[np.ndarray, np.ndarray]:
        """Snow/ice and vegetation masks of the block of surface h whose north-west corner
        lies (y0_m, x0_m) from the layout's."""
        rows, cols = h.shape
        xy = self._coords(y0_m, x0_m, rows, cols, cell_m)
        land = h >= 0
        snowline = self.read(self.l0.snowline, xy, 1)
        forest = self.read(self.l0.forest.astype(np.float32), xy, 1)
        cls = self.read(self.l0.cls, xy, 0)
        snow = land & ((h > snowline) | (cls == GLACIER))
        veg = land & ~snow & (forest >= 128) & (h < self.l0.spec["treeline_m"])
        return snow, veg


def _digest(path: Path) -> str:
    d = hashlib.sha256()
    with path.open("rb") as f:
        while chunk := f.read(1 << 24):
            d.update(chunk)
    return d.hexdigest()


def redraw(world: Path, layout_dir: Path, rows: int = 1024) -> None:
    """Rewrite snow-ice.u8 and vegetation.u8 (2 present, 1 absent) of a world from its layout."""
    manifest_path = world / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    n, cell = manifest["size"][0], float(manifest["spacing_m"])
    sampler = Sampler(layout.load(layout_dir))
    off = (sampler.extent_m - n * cell) / 2  # crops sit in the middle of the layout
    h = np.memmap(world / "elevation.f32", "<f4", "r", shape=(n, n))
    snow_out = np.memmap(world / "snow-ice.u8", np.uint8, "r+", shape=(n, n))
    veg_out = np.memmap(world / "vegetation.u8", np.uint8, "r+", shape=(n, n))
    for y in range(0, n, rows):
        b = min(y + rows, n)
        snow, veg = sampler.cover(np.asarray(h[y:b]), off + y * cell, off, cell)
        snow_out[y:b] = np.where(snow, 2, 1)
        veg_out[y:b] = np.where(veg, 2, 1)
    snow_out.flush()
    veg_out.flush()
    del snow_out, veg_out
    for name in ("snow-ice.u8", "vegetation.u8"):
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
    print(f"redrew snow-ice.u8 and vegetation.u8 in {args.world}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
