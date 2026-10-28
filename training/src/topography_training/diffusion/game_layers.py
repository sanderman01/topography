"""Game layers at 2 m for export v2 (docs/plans/fujix-v1.md §4): vegetation, snow and talus
densities, and dithered biome and land cover, written to <world>/game/.

python -m topography_training.diffusion.game_layers --world <world output> [--layout <L0>]
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

import numpy as np
from scipy import ndimage

from . import cover, layout

METHOD = "game layers v1: cover v2 fields at 2 m, moisture-weighted forest, soft snow, talus below rock"
MOIST_DRY, MOIST_WET = 0.15, 0.45  # forest density ramps in over this moisture range
SNOW_SOFT_M = 80.0  # snow cover ramps from 0 to 1 across the bent snow line ± this
SNOW_SHED_DEG = (40.0, 55.0)  # as cover.SNOW_STEEP_DEG
ROCK_DEG = 40.0  # cliff faces that shed scree
TALUS_DEG = ((22.0, 30.0), (38.0, 45.0))  # scree lies on slopes rising over the first band, gone by the second
TALUS_REACH_M = 20.0  # gaussian sigma of rock share spreading onto the slopes below
NAMES = ("vegetation-density", "snow-cover", "talus-density")
ROWS = 256  # about 3.4 GB peak at 65,536² (32,768 game texels wide)
LATTICE = "value (i, j) is the area mean of the footprint centred on world sample (i·2^L, j·2^L)"


def tent_down(a, above=None):
    """One level of the exporter's area mean (export/pyramid.rs): weights 1/4, 1/2, 1/4 over
    samples 2i-1, 2i, 2i+1 in each axis; index -1 clamps to 0."""
    def axis0(x, first):
        p = np.concatenate([first[None], x], 0)
        return 0.25 * p[0:-1:2] + 0.5 * p[1::2] + 0.25 * p[2::2]
    a = np.asarray(a, np.float32)
    a = axis0(a, a[0] if above is None else np.asarray(above, np.float32))
    return axis0(a.T, a.T[0]).T


def _layout(manifest: dict) -> Path | None:
    """The world's authored layout, if any: recorded by `cover`, the chain, or its source world."""
    for m in (manifest, json.loads((Path(manifest["source_world"]) / "manifest.json").read_text())
              if "source_world" in manifest else {}):
        path = m.get("layout") or m.get("cover", {}).get("layout")
        if path:
            return Path(path)
    return None


def _moisture(world: Path, manifest: dict, origin, rows: np.ndarray, cols: np.ndarray, step: int,
              c0: float) -> np.ndarray:
    """Moisture bilinear from the world's grid at game texels (exporter lattice positions)."""
    e = manifest["moisture"]
    mw, mh = e["size"]
    ms, (nx, ny) = float(e["spacing_m"]), e["nw_m"]
    grid = np.fromfile(world / e["file"], "<f4").reshape(mh, mw)
    y = origin[1] - (rows * step + 0.5) * c0
    x = origin[0] + (cols * step + 0.5) * c0
    fy = np.broadcast_to(((ny - y) / ms - 0.5)[:, None], (len(rows), len(cols)))
    fx = np.broadcast_to(((x - nx) / ms - 0.5)[None, :], (len(rows), len(cols)))
    return ndimage.map_coordinates(grid, [fy, fx], order=1, mode="nearest").astype(np.float32)


def _fields(sampler, h, wet, moist, y0_m, x0_m, s):
    f = sampler.fields(h, y0_m, x0_m, s)
    slope2 = np.degrees(np.arctan(np.hypot(*np.gradient(h, s))))  # local slope for scree
    glacier = f["classes"] == cover.GLACIER  # layout classes, warped as cover reads them
    snow = np.where(glacier, 1.0, cover._ramp(h - f["snowline"], -SNOW_SOFT_M, SNOW_SOFT_M)
                    * (1 - cover._ramp(f["slope"], *SNOW_SHED_DEG)))
    snow *= (h >= 0) & (wet < 0.5)
    veg = f["share"] * cover._ramp(moist, MOIST_DRY, MOIST_WET) * (1 - snow) * (1 - wet)
    rock = (slope2 > ROCK_DEG).astype(np.float32)
    near = np.clip(ndimage.gaussian_filter(rock, TALUS_REACH_M / s) / 0.25, 0, 1)
    (a, b), (c, d) = TALUS_DEG
    talus = near * cover._ramp(slope2, a, b) * (1 - cover._ramp(slope2, c, d)) * (1 - snow) * (1 - wet)
    return {"vegetation-density": veg, "snow-cover": snow, "talus-density": talus}


def write(world: Path, layout_dir: Path | None = None, rows: int = ROWS) -> dict:
    """Write game/<name>.u8 (round(255 × fraction)) at 2 m (the world's spacing if coarser) and
    record them in the manifest; returns manifest["game"]."""
    manifest_path = world / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    n, c0 = manifest["size"][0], float(manifest["spacing_m"])
    level = 1 if c0 < 2.0 else 0
    step, mg = 1 << level, n >> level
    s = c0 * step
    layout_dir = layout_dir or _layout(manifest)
    if layout_dir is None:
        raise ValueError("world records no layout; pass --layout")
    if "moisture" not in manifest:
        raise ValueError("world has no moisture entry; regenerate it with hydrology v2")
    origin = json.loads((Path(manifest["world_input"]) / "world.json").read_text())["origin"]
    sampler = cover.Sampler(layout.load(layout_dir))
    off = (sampler.extent_m - n * c0) / 2  # crops sit in the middle of the layout
    ground = np.memmap(world / "elevation.f32", "<f4", "r", shape=(n, n))
    surface = np.memmap(world / "water-surface.f32", "<f4", "r", shape=(n, n))
    (world / "game").mkdir(exist_ok=True)
    # New files replaced into place: world copies may hard-link the original's layers.
    tmps = {k: world / "game" / f"{k}.u8.tmp" for k in NAMES}
    for t in tmps.values():
        t.unlink(missing_ok=True)
    outs = {k: np.memmap(tmps[k], np.uint8, "w+", shape=(mg, mg)) for k in NAMES}
    m = int(np.ceil(cover.MARGIN_M / s))
    align = max(1, int(round(cover.FEATURE_M / s)))  # cover.terrain_features block size
    cols = np.arange(mg)
    for y in range(0, mg, rows):
        b = min(y + rows, mg)
        # Start on the feature grid so it stays put across blocks.
        g0, g1 = max((y - m) // align * align, 0), min(b + m, mg)
        # Fine rows step·g0 - 1 (the row above, at level 1) up to step·g1.
        r0 = max(step * g0 - (step - 1), 0)
        g = np.asarray(ground[r0 : step * g1], np.float32)
        wet = (np.asarray(surface[r0 : step * g1]) > g).astype(np.float32)
        if level:
            k = step * g0 - r0  # 1 when there is a row above
            h = tent_down(g[k:], g[0] if k else None)
            wet = tent_down(wet[k:], wet[0] if k else None)
        else:
            h = g
        moist = _moisture(world, manifest, origin, np.arange(g0, g1), cols, step, c0)
        f = _fields(sampler, h, wet, moist, off + g0 * s, off, s)
        for k in NAMES:
            outs[k][y:b] = np.round(255 * np.clip(f[k][y - g0 : b - g0], 0, 1)).astype(np.uint8)
    for a in outs.values():
        a.flush()
    del outs
    for k in NAMES:
        os.replace(tmps[k], world / "game" / f"{k}.u8")
        manifest["layers"][f"game/{k}.u8"] = cover._digest(world / "game" / f"{k}.u8")
    manifest["game"] = {
        "spacing_m": s, "size": [mg, mg], "level": level, "lattice": LATTICE, "method": METHOD,
        "params": {"layout": str(Path(layout_dir).resolve()), "moist_dry": MOIST_DRY, "moist_wet": MOIST_WET,
                   "snow_soft_m": SNOW_SOFT_M, "snow_shed_deg": list(SNOW_SHED_DEG), "rock_deg": ROCK_DEG,
                   "talus_deg": [list(t) for t in TALUS_DEG], "talus_reach_m": TALUS_REACH_M, "rows": rows},
        "layers": list(NAMES),
    }
    # Replace, not rewrite: world copies may hard-link files of the original.
    tmp = manifest_path.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(manifest, indent=1))
    os.replace(tmp, manifest_path)
    return manifest["game"]


def _png(world: Path, game: dict, out: Path, px: int = 1024) -> None:
    from PIL import Image

    mg = game["size"][0]
    k = max(1, mg // px)
    rgb = []
    for name in ("snow-cover", "vegetation-density", "talus-density"):
        a = np.memmap(world / "game" / f"{name}.u8", np.uint8, "r", shape=(mg, mg))
        a = np.asarray(a[: mg // k * k, : mg // k * k], np.float32)
        rgb.append(a.reshape(mg // k, k, mg // k, k).mean(axis=(1, 3)))
    out.parent.mkdir(parents=True, exist_ok=True)
    Image.fromarray(np.round(np.stack(rgb, -1)).astype(np.uint8)).save(out)


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--world", type=Path, required=True, help="world output (cascade_chain or cascade_l4)")
    p.add_argument("--layout", type=Path, help="L0 directory; default the one the world records")
    p.add_argument("--rows", type=int, default=ROWS, help="output rows per block (bounds RAM)")
    p.add_argument("--png", type=Path, help="also write an RGB picture (snow R, vegetation G, talus B)")
    args = p.parse_args(argv)
    if not (args.world / "manifest.json").exists():
        print("error: --world has no manifest.json", file=sys.stderr)
        return 2
    try:
        game = write(args.world, args.layout, args.rows)
    except ValueError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    print(f"wrote game layers in {args.world}: {game['size']} at {game['spacing_m']} m, level {game['level']}")
    for name in NAMES:
        v = np.memmap(args.world / "game" / f"{name}.u8", np.uint8, "r")[::97].astype(np.float32) / 255
        print(f"  {name}: mean {v.mean():.3f}, share > 0.5 {(v > 0.5).mean():.3f}")
    if args.png:
        _png(args.world, game, args.png)
        print(f"wrote {args.png}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
