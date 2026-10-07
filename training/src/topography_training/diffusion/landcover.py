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

METHOD = "landcover v2: rules from water, snow/ice, vegetation, slope, local tree line and terrain class (as cover v2)"
TREE, SHRUB, GRASS, BARE, SNOW, WATER = 10, 20, 30, 60, 70, 80
STEEP_DEG = 35.0  # bare rock above this slope
ALPINE_BAND_M = 400.0  # grassland this far above the treeline, bare ground beyond

# Open (non-forest, non-snow) land by terrain class id (0: unlabelled).
OPEN = np.full(256, GRASS, np.uint8)
OPEN[[2, 7]] = SHRUB  # gorges and canyons, badlands: scrub between bare slopes
OPEN[[8, 10, 11]] = BARE  # dunes, glacier forelands, lava fields
BARE_CLASSES = (7, 8, 10, 11)  # bare below the steep limit too, except where forested


def classify(h: np.ndarray, water: np.ndarray, snow: np.ndarray, veg: np.ndarray, classes: np.ndarray,
             cell_m: float, treeline_m) -> np.ndarray:
    """WorldCover codes for one block of rows (masks are booleans; the tree line may vary
    per cell). Above the tree line grass gives way to bare ground, except on ground that
    is bare anyway (dunes, lava, badlands), which stays bare rather than turning to grass."""
    gy, gx = np.gradient(h, cell_m)
    slope = np.degrees(np.arctan(np.hypot(gx, gy)))
    out = OPEN[classes]
    bare_class = np.isin(classes, BARE_CLASSES)
    out = np.where(bare_class & (slope > 15), BARE, out)
    alpine = np.where(h > treeline_m + ALPINE_BAND_M, BARE, GRASS)
    out = np.where((h > treeline_m) & ~bare_class, alpine, out)
    out = np.where(slope > STEEP_DEG, BARE, out)
    out = np.where(veg, TREE, out)
    out = np.where(snow, SNOW, out)
    return np.where(water, WATER, out).astype(np.uint8)


def _layout(world: Path, manifest: dict):
    """The world's authored layout, if any: recorded by `cover`, the chain, or its source world."""
    for m in (manifest, json.loads((Path(manifest["source_world"]) / "manifest.json").read_text())
              if "source_world" in manifest else {}):
        path = m.get("layout") or m.get("cover", {}).get("layout")
        if path:
            return Path(path)
    return None


def add(world: Path, treeline_m: float | None = None, layout_dir: Path | None = None, rows: int = 1024) -> Path:
    manifest_path = world / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    n = manifest["size"][0]
    cell = float(manifest["spacing_m"])

    def layer(name, dtype):
        return np.memmap(world / name, dtype, "r", shape=(n, n))

    h, water = layer("elevation.f32", "<f4"), layer("water.u8", np.uint8)
    snow, veg = layer("snow-ice.u8", np.uint8), layer("vegetation.u8", np.uint8)
    layout_dir = layout_dir or _layout(world, manifest)
    sampler = None
    if layout_dir is not None:  # classes read the way `cover` reads them, borders warped alike
        from .cover import Sampler
        from .layout import load

        sampler = Sampler(load(layout_dir))
        off = (sampler.extent_m - n * cell) / 2
        treeline_m = treeline_m if treeline_m is not None else float(sampler.l0.spec["treeline_m"])
    cid = manifest.get("class_id")
    if treeline_m is None:
        treeline_m = 1000.0
    out = np.memmap(world / "land-cover.u8", np.uint8, "w+", shape=(n, n))
    from .cover import MARGIN_M

    margin = int(np.ceil(MARGIN_M / cell)) if sampler is not None else 1  # context for the terrain measures
    for y in range(0, n, rows):
        a, b = max(y - margin, 0), min(y + rows + margin, n)
        block = np.asarray(h[a:b], np.float32)
        line = treeline_m
        if sampler is not None:
            c = sampler.classes(off + a * cell, off, b - a, n, cell)
            line = sampler.fields(block, off + a * cell, off, cell)["treeline"]
        else:
            c = np.full((b - a, n), cid if isinstance(cid, int) else 0, np.uint8)
        lc = classify(block, water[a:b] == 255, snow[a:b] == 2, veg[a:b] == 2, c, cell, line)
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
    p.add_argument("--layout", type=Path, help="L0 directory; default the one the world records")
    p.add_argument("--treeline-m", type=float, help="default the layout's treeline_m, else 1000")
    args = p.parse_args(argv)
    if not (args.world / "manifest.json").exists():
        print("error: --world has no manifest.json", file=sys.stderr)
        return 2
    path = add(args.world, args.treeline_m, args.layout)
    counts = np.bincount(np.memmap(path, np.uint8, "r")[:: 97], minlength=101)
    share = {int(c): round(float(counts[c]) / counts.sum(), 3) for c in np.nonzero(counts)[0]}
    print(f"wrote {path}; share by code (sampled): {share}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
