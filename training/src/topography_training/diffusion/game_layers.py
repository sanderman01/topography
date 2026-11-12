"""Game layers at 2 m for export v2 (docs/plans/fujix-v1.md §4): vegetation, snow and talus
densities, and dithered biome and land cover, written to <world>/game/.

python -m topography_training.diffusion.game_layers --world <world output> [--layout <L0>]

Biome is the layout's terrain class (warped as cover reads it), each class's share smoothed
over about 64 m and dithered with blue noise. Land cover holds ESA WorldCover codes, a
recorded judgment call, not observed cover: water 80, snow/ice 70 and forest 10 follow the
water, snow and vegetation densities; bare rock (60) ramps in around 35 degrees; above the
tree line grassland (30) gives way to bare ground over 400 m; dunes, badlands, glacier
ground, volcanoes and lava are bare from gentle slopes up and stay bare above the tree line;
other open land takes its class's typical cover. One blue-noise value per texel turns each ramp
into that share of texels, so edges read as a fine mix.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

import numpy as np
from scipy import ndimage

from . import borders, cover, layout

METHOD = ("game layers v1: cover v2 fields at 2 m, moisture-weighted forest, soft snow (none on land-cover water), "
          "snow/ice on frozen lakes, talus below rock; biome and land cover dithered from smoothed shares and ramps")
MOIST_DRY, MOIST_WET = 0.15, 0.45  # forest density ramps in over this moisture range
SNOW_SOFT_M = 80.0  # snow cover ramps from 0 to 1 across the bent snow line ± this
SNOW_SHED_DEG = (40.0, 55.0)  # as cover.SNOW_STEEP_DEG
ROCK_DEG = 40.0  # cliff faces that shed scree
TALUS_DEG = ((22.0, 30.0), (38.0, 45.0))  # scree lies on slopes rising over the first band, gone by the second
TALUS_REACH_M = 20.0  # gaussian sigma of rock share spreading onto the slopes below
BIOME_MIX_M = 64.0  # gaussian sigma of each class's share across biome borders
ALPINE_BAND_M = 400.0  # grass gives way to bare ground over this band above the tree line
ROCK_RAMP_DEG = (30.0, 40.0)  # bare rock ramps in around 35 degrees
BARE_CLASS_DEG = (10.0, 20.0)  # bare classes are bare from gentle slopes up
WATER_WET = 0.25  # land-cover water (and no snow) at this wet share, undithered: keeps 1 m rivers whole at 2 m
BIOME_SEED, LANDCOVER_SEED = 5, 6  # blue-noise tiles, distinct from training's and the collage's
TREE, SHRUB, GRASS, BARE, SNOW, WATER = 10, 20, 30, 60, 70, 80  # ESA WorldCover codes
# Open (non-forest, non-snow) land by terrain class id (0: unlabelled).
OPEN = np.full(256, GRASS, np.uint8)
OPEN[[2, 7, 13, 19]] = SHRUB  # gorges, badlands, canyons: scrub between bare slopes
OPEN[[8, 10, 11, 16, 17]] = BARE  # dunes, glacier ground, stratovolcanoes, lava fields
BARE_CLASSES = (7, 8, 10, 11, 16, 17)
WORLDCOVER_RGB = {10: "006400", 20: "ffbb22", 30: "ffff4c", 40: "f096ff", 50: "fa0000", 60: "b4b4b4",
                  70: "f0f0f0", 80: "0064c8", 90: "0096a0", 95: "00cf75", 100: "fae6a0"}
DENSITIES = ("vegetation-density", "snow-cover", "talus-density")
CLASSES = ("biome", "land-cover")
NAMES = DENSITIES + CLASSES
ROWS = 256  # about 3.7 GB peak at 65,536² (32,768 game texels wide; extrapolated from 2,048² runs)
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


def _layout_offset(origin, extent_m: float, size_m: float) -> tuple[float, float]:
    """(south, east) metres of a world's north-west corner from the layout's, whose own corner
    is at (0, extent) in the sketch plane (cascade_chain); raises if the world leaves the layout."""
    oy, ox = extent_m - float(origin[1]), float(origin[0])
    if min(oy, ox) < -0.5 or max(oy, ox) + size_m > extent_m + 0.5:
        raise ValueError(f"world origin {origin} ({size_m:g} m square) is outside the {extent_m:g} m layout")
    return oy, ox


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


def _ice_ids(world: Path) -> np.ndarray:
    """World body ids of frozen lakes (bodies.json kind `ice`)."""
    path = world / "bodies.json"
    if not path.exists():
        return np.zeros(0, np.uint32)
    bodies = json.loads(path.read_text())["bodies"]
    return np.array([b["id"] for b in bodies if b.get("kind") == "ice"], np.uint32)


def _fields(sampler, h, wet, ice, moist, y0_m, x0_m, s):
    f = sampler.fields(h, y0_m, x0_m, s)
    slope2 = np.degrees(np.arctan(np.hypot(*np.gradient(h, s))))  # local slope for scree
    glacier = f["classes"] == cover.GLACIER  # layout classes, warped as cover reads them
    snow = np.where(glacier, 1.0, cover._ramp(h - f["snowline"], -SNOW_SOFT_M, SNOW_SOFT_M)
                    * (1 - cover._ramp(f["slope"], *SNOW_SHED_DEG)))
    snow *= (h >= 0) & (wet < WATER_WET)  # no snow where land cover is water
    snow = np.maximum(snow, ice)  # a frozen lake is snow/ice whatever its bed's height
    veg = f["share"] * cover._ramp(moist, MOIST_DRY, MOIST_WET) * (1 - snow) * (1 - wet)
    rock = (slope2 > ROCK_DEG).astype(np.float32)
    near = np.clip(ndimage.gaussian_filter(rock, TALUS_REACH_M / s) / 0.25, 0, 1)
    (a, b), (c, d) = TALUS_DEG
    talus = near * cover._ramp(slope2, a, b) * (1 - cover._ramp(slope2, c, d)) * (1 - snow) * (1 - wet)
    return {"vegetation-density": veg, "snow-cover": snow, "talus-density": talus, "slope2": slope2,
            "treeline": f["treeline"]}


def land_cover(biome, h, slope, treeline, veg, snow, wet, u):
    """WorldCover codes: each density or ramp becomes that share of texels via noise u; water is hard."""
    ramp = cover._ramp
    out = OPEN[biome]
    bare_class = np.isin(biome, BARE_CLASSES)
    out = np.where(bare_class & (ramp(slope, *BARE_CLASS_DEG) > u), BARE, out)
    alpine = np.where(ramp(h - treeline, 0.0, ALPINE_BAND_M) > u, BARE, GRASS)
    out = np.where(~bare_class & (h > treeline), alpine, out)
    out = np.where(ramp(slope, *ROCK_RAMP_DEG) > u, BARE, out)
    # Trees from the top of the noise and snow from the bottom: veg <= 1 - snow keeps them disjoint.
    out = np.where(veg > 1 - u, TREE, out)
    out = np.where(snow > u, SNOW, out)
    return np.where(wet >= WATER_WET, WATER, out).astype(np.uint8)


def _lerp_index(pos: np.ndarray, n: int):
    """Indices and weights of linear interpolation at grid positions, clamped at the edges."""
    pos = np.clip(pos, 0, n - 1)
    i0 = np.minimum(np.floor(pos).astype(np.intp), n - 1)
    return i0, np.minimum(i0 + 1, n - 1), (pos - i0).astype(np.float32)


def _biome(shares, rows: np.ndarray, cols, u: np.ndarray) -> np.ndarray:
    """Dithered classes at game texel rows: shares read bilinearly (separably), accumulated
    class by class, as borders.dither does but holding one share at a time."""
    n32 = shares[0][1].shape[0]
    r0, r1, tr = _lerp_index(rows, n32)
    c0, c1, tc = cols
    acc = np.zeros(u.shape, np.float32)
    out = np.zeros(u.shape, np.uint8)
    done = np.zeros(u.shape, bool)
    for k, share in shares:
        s = share[r0] * (1 - tr)[:, None] + share[r1] * tr[:, None]
        acc += s[:, c0] * (1 - tc) + s[:, c1] * tc
        pick = ~done & (u < acc)
        out[pick] = k
        done |= pick
    out[~done] = shares[-1][0]  # rounding at acc ≈ 1
    return out


def write(world: Path, layout_dir: Path | None = None, rows: int = ROWS) -> dict:
    """Write game/<name>.u8 at 2 m (the world's spacing if coarser): densities as round(255 × fraction),
    biome as terrain-v3 ids (0 unlabelled), land cover as WorldCover codes. Records them in
    the manifest; returns manifest["game"]."""
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
        raise ValueError("world has no moisture entry; regenerate it with hydrology v3")
    origin = json.loads((Path(manifest["world_input"]) / "world.json").read_text())["origin"]
    sampler = cover.Sampler(layout.load(layout_dir))
    oy, ox = _layout_offset(origin, sampler.extent_m, n * c0)
    ground = np.memmap(world / "elevation.f32", "<f4", "r", shape=(n, n))
    surface = np.memmap(world / "water-surface.f32", "<f4", "r", shape=(n, n))
    body = np.memmap(world / "water-body.u32", "<u4", "r", shape=(n, n))
    ice_ids = _ice_ids(world)
    (world / "game").mkdir(exist_ok=True)
    # New files replaced into place: world copies may hard-link the original's layers.
    tmps = {k: world / "game" / f"{k}.u8.tmp" for k in NAMES}
    for t in tmps.values():
        t.unlink(missing_ok=True)
    outs = {k: np.memmap(tmps[k], np.uint8, "w+", shape=(mg, mg)) for k in NAMES}
    m = int(np.ceil(cover.MARGIN_M / s))
    align = max(1, int(round(cover.FEATURE_M / s)))  # cover.terrain_features block size
    cols = np.arange(mg)
    # Biome shares on a 32 m grid over the world; texels read them at their centres.
    n32 = int(np.ceil(n * c0 / 32.0))
    c32 = sampler.classes(oy, ox, n32, n32, 32.0)
    shares = [(int(k), ndimage.gaussian_filter((c32 == k).astype(np.float32), BIOME_MIX_M / 32.0))
              for k in np.unique(c32)]
    del c32

    def to32(i):  # texel index to 32 m grid position (cell centre i at i)
        return (i * step + 0.5) * c0 / 32.0 - 0.5

    lerp_cols = _lerp_index(to32(cols), n32)
    for y in range(0, mg, rows):
        b = min(y + rows, mg)
        # Start on the feature grid so it stays put across blocks.
        g0, g1 = max((y - m) // align * align, 0), min(b + m, mg)
        # Fine rows step·g0 - 1 (the row above, at level 1) up to step·g1.
        r0 = max(step * g0 - (step - 1), 0)
        g = np.asarray(ground[r0 : step * g1], np.float32)
        frozen = np.isin(np.asarray(body[r0 : step * g1]), ice_ids)
        # Ice is a frozen lake's surface: snow/ice cover, not land-cover water.
        wet = ((np.asarray(surface[r0 : step * g1]) > g) & ~frozen).astype(np.float32)
        ice = frozen.astype(np.float32)
        if level:
            k = step * g0 - r0  # 1 when there is a row above
            h = tent_down(g[k:], g[0] if k else None)
            wet = tent_down(wet[k:], wet[0] if k else None)
            ice = tent_down(ice[k:], ice[0] if k else None)
        else:
            h = g
        moist = _moisture(world, manifest, origin, np.arange(g0, g1), cols, step, c0)
        f = _fields(sampler, h, wet, ice, moist, oy + g0 * s, ox, s)
        sl = slice(y - g0, b - g0)
        for k in DENSITIES:
            outs[k][y:b] = np.round(255 * np.clip(f[k][sl], 0, 1)).astype(np.uint8)
        biome = _biome(shares, to32(np.arange(y, b)), lerp_cols, borders.noise_window(y, 0, b - y, mg, BIOME_SEED))
        outs["biome"][y:b] = biome
        outs["land-cover"][y:b] = land_cover(
            biome, h[sl], f["slope2"][sl], f["treeline"][sl], f["vegetation-density"][sl], f["snow-cover"][sl],
            wet[sl], borders.noise_window(y, 0, b - y, mg, LANDCOVER_SEED))
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
                   "talus_deg": [list(t) for t in TALUS_DEG], "talus_reach_m": TALUS_REACH_M,
                   "biome_mix_m": BIOME_MIX_M, "alpine_band_m": ALPINE_BAND_M,
                   "rock_ramp_deg": list(ROCK_RAMP_DEG), "bare_class_deg": list(BARE_CLASS_DEG),
                   "water_wet": WATER_WET, "blue_noise_seeds": {"biome": BIOME_SEED, "land-cover": LANDCOVER_SEED},
                   "rows": rows},
        "layers": list(NAMES),
    }
    # Replace, not rewrite: world copies may hard-link files of the original.
    tmp = manifest_path.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(manifest, indent=1))
    os.replace(tmp, manifest_path)
    return manifest["game"]


def _area_mean(path: Path, mg: int, k: int, value) -> np.ndarray:
    """k×k means of value(uint8 rows) over an mg² layer, read in row bands to bound RAM."""
    a = np.memmap(path, np.uint8, "r", shape=(mg, mg))
    m = mg // k
    band = max(1, (1 << 24) // (k * mg))  # output rows per band: about 16 M texels read at once
    out = []
    for r in range(0, m, band):
        b = min(r + band, m)
        v = value(np.asarray(a[r * k : b * k, : m * k]))
        out.append(v.reshape(b - r, k, m, k, -1).mean(axis=(1, 3)))
    return np.concatenate(out)


def _png(world: Path, game: dict, out: Path, px: int = 1024) -> None:
    from PIL import Image

    mg = game["size"][0]
    k = max(1, mg // px)
    game_dir = world / "game"
    rgb = [_area_mean(game_dir / f"{name}.u8", mg, k, lambda v: v.astype(np.float32)[..., None])[..., 0]
           for name in ("snow-cover", "vegetation-density", "talus-density")]
    out.parent.mkdir(parents=True, exist_ok=True)
    Image.fromarray(np.round(np.stack(rgb, -1)).astype(np.uint8)).save(out)
    # Land cover in WorldCover colours, area-averaged so the dither reads as a blend.
    lut = np.zeros((256, 3), np.float32)
    for code, hexc in WORLDCOVER_RGB.items():
        lut[code] = [int(hexc[i : i + 2], 16) for i in (0, 2, 4)]
    c = _area_mean(game_dir / "land-cover.u8", mg, k, lambda v: lut[v])
    Image.fromarray(np.round(c).astype(np.uint8)).save(_landcover_png(out))


def _landcover_png(out: Path) -> Path:
    return out.with_name(f"{out.stem}-landcover{out.suffix}")


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--world", type=Path, required=True, help="world output (cascade_chain or cascade_l4)")
    p.add_argument("--layout", type=Path, help="L0 directory; default the one the world records")
    p.add_argument("--rows", type=int, default=ROWS, help="output rows per block (bounds RAM)")
    p.add_argument("--png", type=Path,
                   help="also write an RGB picture (snow R, vegetation G, talus B) and <stem>-landcover.png")
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
    for name in DENSITIES:
        v = np.memmap(args.world / "game" / f"{name}.u8", np.uint8, "r")[::97].astype(np.float32) / 255
        print(f"  {name}: mean {v.mean():.3f}, share > 0.5 {(v > 0.5).mean():.3f}")
    for name in CLASSES:
        counts = np.bincount(np.memmap(args.world / "game" / f"{name}.u8", np.uint8, "r")[::97], minlength=256)
        share = {int(c): round(float(counts[c] / counts.sum()), 3) for c in np.nonzero(counts)[0]}
        print(f"  {name}: share by code (sampled) {share}")
    if args.png:
        _png(args.world, game, args.png)
        print(f"wrote {args.png} and {_landcover_png(args.png)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
