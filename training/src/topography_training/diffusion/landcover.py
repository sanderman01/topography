"""Land cover for a generated world: ESA WorldCover class codes derived from its layers.

python -m topography_training.diffusion.landcover --world <world output> [--treeline-m M]

Generated worlds have no observed land cover, so this is a recorded judgment call, not a
measurement: water is 80, snow/ice 70 and forest 10 (from the world's own layers); steep
ground is bare rock (60); above the treeline grassland (30) gives way to bare ground;
dunes, badlands, lava and glacier forelands are bare; other open land takes its terrain
class's typical cover. Writes land-cover.u8 into the world and adds it to the manifest,
so `topography export heightmap` exports it as a layer.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

import numpy as np

METHOD = "landcover v1: rules from water, snow/ice, vegetation, slope, treeline and terrain class (borders warped ~150 m)"
TREE, SHRUB, GRASS, BARE, SNOW, WATER = 10, 20, 30, 60, 70, 80
STEEP_DEG = 35.0  # bare rock above this slope
ALPINE_BAND_M = 400.0  # grassland this far above the treeline, bare ground beyond

# Open (non-forest, non-snow) land by terrain class id (0: unlabelled).
OPEN = np.full(256, GRASS, np.uint8)
OPEN[[2, 7]] = SHRUB  # gorges and canyons, badlands: scrub between bare slopes
OPEN[[8, 10, 11]] = BARE  # dunes, glacier forelands, lava fields
BARE_CLASSES = (7, 8, 10, 11)  # bare below the steep limit too, except where forested


def classify(h: np.ndarray, water: np.ndarray, snow: np.ndarray, veg: np.ndarray, classes: np.ndarray,
             cell_m: float, treeline_m: float) -> np.ndarray:
    """WorldCover codes for one block of rows (masks are booleans)."""
    gy, gx = np.gradient(h, cell_m)
    slope = np.degrees(np.arctan(np.hypot(gx, gy)))
    out = OPEN[classes]
    out = np.where(np.isin(classes, BARE_CLASSES) & (slope > 15), BARE, out)
    out = np.where(h > treeline_m, np.where(h > treeline_m + ALPINE_BAND_M, BARE, GRASS), out)
    out = np.where(slope > STEEP_DEG, BARE, out)
    out = np.where(veg, TREE, out)
    out = np.where(snow, SNOW, out)
    return np.where(water, WATER, out).astype(np.uint8)


def _classes(world: Path, manifest: dict, n: int, cell: float):
    """Class per cell: (memory-mapped map, its cells per world cell factor k, offset of the
    world inside it in world cells; L4 crops sit in the middle of their 4 m world)."""
    for d in (world, Path(manifest.get("source_world", world))):
        p = d / "classes.u8"
        if p.exists():
            m = int(round(p.stat().st_size ** 0.5))
            k = int(round(json.loads((d / "manifest.json").read_text())["spacing_m"] / cell))
            return np.memmap(p, np.uint8, "r", shape=(m, m)), k, (m * k - n) // 2
    cid = manifest.get("class_id")
    return (int(cid) if isinstance(cid, int) else 0), 1, 0


WARP_M = 150.0  # class borders follow smooth noise this far instead of the painted grid
WARP_CELL_M = 32.0


def _warp_field(n: int, cell: float, seed: int = 0) -> np.ndarray:
    """Smooth displacement (2, N, N) in metres on a WARP_CELL_M grid over the world."""
    from scipy import ndimage

    m = max(2, int(np.ceil(n * cell / WARP_CELL_M)) + 1)
    d = ndimage.gaussian_filter(np.random.default_rng(seed).standard_normal((2, m, m)), (0, 4, 4))
    return (d / d.std() * WARP_M).astype(np.float32)


def _warped(cls: np.ndarray, k: int, off: int, a: int, b: int, n: int, cell: float, warp: np.ndarray):
    """Classes of world rows a..b looked up at noise-displaced positions, so painted
    class borders become irregular instead of following the class grid."""
    from scipy import ndimage

    ys, xs = np.mgrid[a:b, 0:n].astype(np.float32)
    s = cell / WARP_CELL_M
    dy = ndimage.map_coordinates(warp[0], [ys * s, xs * s], order=1, mode="nearest")
    dx = ndimage.map_coordinates(warp[1], [ys * s, xs * s], order=1, mode="nearest")
    m = cls.shape[0]
    cy = np.clip(((ys + dy / cell + off) // k).astype(np.int64), 0, m - 1)
    cx = np.clip(((xs + dx / cell + off) // k).astype(np.int64), 0, m - 1)
    r0, r1 = int(cy.min()), int(cy.max()) + 1
    return np.asarray(cls[r0:r1])[cy - r0, cx]


def add(world: Path, treeline_m: float, rows: int = 1024) -> Path:
    manifest_path = world / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    n = manifest["size"][0]
    cell = float(manifest["spacing_m"])

    def layer(name, dtype):
        return np.memmap(world / name, dtype, "r", shape=(n, n))

    h, water = layer("elevation.f32", "<f4"), layer("water.u8", np.uint8)
    snow, veg = layer("snow-ice.u8", np.uint8), layer("vegetation.u8", np.uint8)
    cls, k, off = _classes(world, manifest, n, cell)
    warp = _warp_field(n, cell)
    out = np.memmap(world / "land-cover.u8", np.uint8, "w+", shape=(n, n))
    for y in range(0, n, rows):
        a, b = max(y - 1, 0), min(y + rows + 1, n)  # one-row margin for the slope
        if isinstance(cls, int):
            c = np.full((b - a, n), cls, np.uint8)
        else:
            c = _warped(cls, k, off, a, b, n, cell, warp)
        lc = classify(np.asarray(h[a:b], np.float32), water[a:b] == 255, snow[a:b] == 2, veg[a:b] == 2, c,
                      cell, treeline_m)
        out[y : min(y + rows, n)] = lc[y - a : y - a + min(rows, n - y)]
    out.flush()
    del out
    digest = hashlib.sha256()
    with (world / "land-cover.u8").open("rb") as f:
        while chunk := f.read(1 << 24):
            digest.update(chunk)
    manifest.setdefault("layers", {})["land-cover.u8"] = digest.hexdigest()
    manifest["land_cover"] = {"method": METHOD, "treeline_m": treeline_m, "codes": "ESA WorldCover"}
    manifest_path.write_text(json.dumps(manifest, indent=1))
    return world / "land-cover.u8"


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--world", type=Path, required=True, help="world output (cascade_chain or cascade_l4)")
    p.add_argument("--treeline-m", type=float, default=1000.0, help="the layout's treeline_m")
    args = p.parse_args(argv)
    if not (args.world / "manifest.json").exists():
        print("error: --world has no manifest.json", file=sys.stderr)
        return 2
    path = add(args.world, args.treeline_m)
    counts = np.bincount(np.memmap(path, np.uint8, "r")[:: 97], minlength=101)
    share = {int(c): round(float(counts[c]) / counts.sum(), 3) for c in np.nonzero(counts)[0]}
    print(f"wrote {path}; share by code (sampled): {share}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
