"""Game layers at 2 m for export v2 (docs/plans/fujix-v1.md §4): vegetation, snow and talus
densities, temperature, moisture, and top-2 biome and land-cover blends, written to <world>/game/.

python -m topography_training.diffusion.game_layers --world <world output> [--layout <L0>]

Biome is the layout's terrain class (warped as cover reads it), each class's share smoothed
over about 64 m; a texel keeps its two largest shares as ids a and b and a's share of the two.
Land cover holds ESA WorldCover codes, a recorded judgment call, not observed cover: water 80,
snow/ice 70 and forest 10 follow the water, snow and vegetation densities; bare rock (60) ramps
in around 35 degrees; above the tree line (blended in over ± 100 m) grassland (30) gives way to
bare ground over 400 m; dunes, badlands, glacier ground, volcanoes and lava are bare from gentle
slopes up and stay bare above the tree line; other open land takes its class's typical cover;
riparian (21, an extension code) lines lakes and larger rivers where the vegetation is thinner
than a riparian band. Each rule claims an interval of one uniform value, so each class's share
is the analytic length of its interval, mixed over the two biomes; the top two are kept.
Talus sheds from steep rock and settles below it under the angle of repose, most in hollows
and gullies. Temperature is the layout region's sea-level value, lapsed with height, slightly
milder at the coast; moisture is the 128 m hydrology field times local wetness.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import tomllib
from pathlib import Path

import numpy as np
from scipy import ndimage

from . import cover, layout

METHOD = ("game layers v2: cover v2 fields at 2 m, moisture-weighted forest, soft snow and glacier edges (none on "
          "land-cover water), snow/ice on frozen lakes and glaciers, talus shed from steep rock and settled below "
          "it, riparian band on lakes and larger rivers, temperature from layout regions with lapse and coast, "
          "moisture as the 128 m field times local wetness; biome and land cover as top-2 ids and weight from "
          "smoothed shares and analytic ramp intervals")
MOIST_DRY, MOIST_WET = 0.15, 0.45  # forest density ramps in over this moisture range
SNOW_SOFT_M = 80.0  # snow cover ramps from 0 to 1 across the bent snow line ± this
SNOW_SHED_DEG = (40.0, 55.0)  # as cover.SNOW_STEEP_DEG
# Talus: rock sheds debris that settles below it, under the angle of repose.
TALUS_SOURCE_DEG = (35.0, 45.0)  # steep rock ramps in as a debris source
TALUS_REACH_M = 40.0  # gaussian sigma of the source share and its height around a texel
TALUS_SUPPLY = 0.15  # source share for full supply
TALUS_DROP_M = (3.0, 20.0)  # settles this far below the nearby source height
TALUS_REPOSE_DEG = (33.0, 40.0)  # none on slopes steeper than the angle of repose
TALUS_HOLLOW_M = (6.0, 30.0)  # concavity: wide minus narrow gaussian of the ground
TALUS_HOLLOW_RAMP = (-0.5, 1.5)  # m of concavity over which talus collects fully
TALUS_GULLY_KM2 = (1e-3, 0.03)  # discharge of rills and gullies that collect talus (log ramp)
TALUS_OPEN = 0.3  # share of talus on planar or convex ground below a source
TALUS_CHANNEL_KM2 = (0.5, 2.0)  # none in stream channels
TALUS_GAIN = 1.5  # overall scale: canyon test mean near the old slope band's (0.16)
# Riparian band along lakes and larger rivers, where vegetation is otherwise thinner.
RIPARIAN = 21  # land-cover extension code (not WorldCover)
RIPARIAN_Q_KM2 = 2.0  # rivers from this discharge
RIPARIAN_WIDTH_M = (10.0, 40.0)  # band width: small sources to large
RIPARIAN_WIDE_KM2 = (2.0, 200.0)  # river discharge over which the band widens (log ramp)
RIPARIAN_WIDE_LAKE_KM2 = (0.01, 1.0)  # lake area over which the band widens (log ramp)
RIPARIAN_DENSITY = 0.25  # vegetation density inside the band
RIPARIAN_ABOVE_M = (2.0, 6.0)  # fades out at this height above the nearest water surface
RIPARIAN_SLOPE_DEG = (20.0, 35.0)  # fades out on steep banks
# Temperature (°C): region sea-level value, lapse with height, milder at the coast.
TEMPERATURE_DEFAULT_C = 12.0  # regions without `temperature_c`, and layouts without regions
LAPSE_C_PER_M = 0.0065
COAST_WARM_C = 1.0  # added at the coast, fading inland
COAST_FADE_M = 5000.0
REGION_BLEND_M = 1500.0  # gaussian sigma of region temperatures across borders
TEMPERATURE_OFFSET_C, TEMPERATURE_STEP_C = -30.0, 0.25  # °C = code · step + offset
# Moisture: 128 m field × local wetness, never below the riparian band's.
MOIST_TWI = (4.0, 14.0)  # topographic wetness ln(a / tan β) over which local wetness rises
MOIST_LOCAL = (0.7, 1.3)  # local wetness factor from dry spurs to wet hollows
MOIST_POLE = 0.1  # extra factor on pole-facing slopes (and less on sun-facing ones)
MOIST_RIPARIAN = 0.6  # moisture inside the riparian band
GLACIER_SHARE = (0.2, 0.8)  # glacier snow ramps in over this smoothed glacier share (~±55 m of the border)
BIOME_MIX_M = 64.0  # gaussian sigma of each class's share across biome borders
ALPINE_BAND_M = 400.0  # grass gives way to bare ground over this band above the tree line
TREELINE_EDGE_M = 100.0  # open cover turns alpine over ± this around the tree line (ecotone)
ROCK_RAMP_DEG = (30.0, 40.0)  # bare rock ramps in around 35 degrees
BARE_CLASS_DEG = (10.0, 20.0)  # bare classes are bare from gentle slopes up
WATER_WET = 0.25  # land-cover water (and no snow) at this wet share: keeps 1 m rivers whole at 2 m
TREE, SHRUB, GRASS, BARE, SNOW, WATER = 10, 20, 30, 60, 70, 80  # ESA WorldCover codes
COVER_CODES = (TREE, SHRUB, GRASS, BARE, SNOW, WATER, RIPARIAN)
# Open (non-forest, non-snow) land by terrain class id (0: unlabelled).
OPEN = np.full(256, GRASS, np.uint8)
OPEN[[2, 7, 13, 19]] = SHRUB  # gorges, badlands, canyons: scrub between bare slopes
OPEN[[8, 10, 11, 16, 17]] = BARE  # dunes, glacier ground, stratovolcanoes, lava fields
BARE_CLASSES = (7, 8, 10, 11, 16, 17)
WORLDCOVER_RGB = {10: "006400", 20: "ffbb22", 30: "ffff4c", 40: "f096ff", 50: "fa0000", 60: "b4b4b4",
                  70: "f0f0f0", 80: "0064c8", 90: "0096a0", 95: "00cf75", 100: "fae6a0", RIPARIAN: "6fbf3f"}
# Body kinds (bodies.json); `glacier` is ice ground, not water.
KINDS = {"sea": 1, "lake": 2, "river": 3, "ice": 4, "dry": 5, "glacier": 6}
DENSITIES = ("vegetation-density", "snow-cover", "talus-density")
CLIMATE = ("temperature", "moisture")
CLASSES = ("biome", "land-cover")
NAMES = DENSITIES + CLIMATE + tuple(f"{c}-{p}" for c in CLASSES for p in ("a", "b", "weight"))
ROWS = 256  # output rows per block; see the docs for measured peaks
LATTICE = "value (i, j) is the area mean of the footprint centred on world sample (i·2^L, j·2^L)"
ENCODINGS = {
    "vegetation-density": "fraction = code / 255; 1.0 is continuous canopy",
    "snow-cover": "fraction = code / 255",
    "talus-density": "fraction = code / 255",
    "temperature": f"°C = code · {TEMPERATURE_STEP_C} + ({TEMPERATURE_OFFSET_C}); mean annual, judgment call",
    "moisture": "0..1 = code / 255",
    "biome-a": "terrain-v3 id with the largest share", "biome-b": "terrain-v3 id with the second share (= a if none)",
    "biome-weight": "share of a in a + b = code / 255 (>= 128)",
    "land-cover-a": "WorldCover code (21 riparian) with the largest share",
    "land-cover-b": "WorldCover code with the second share (= a if none)",
    "land-cover-weight": "share of a in a + b = code / 255 (>= 128)",
}


def tent_down(a, above=None):
    """One level of the exporter's area mean (export/pyramid.rs): weights 1/4, 1/2, 1/4 over
    samples 2i-1, 2i, 2i+1 in each axis; index -1 clamps to 0."""
    def axis0(x, first):
        p = np.concatenate([first[None], x], 0)
        return 0.25 * p[0:-1:2] + 0.5 * p[1::2] + 0.25 * p[2::2]
    a = np.asarray(a, np.float32)
    a = axis0(a, a[0] if above is None else np.asarray(above, np.float32))
    return axis0(a.T, a.T[0]).T


def max_down(a, above=None):
    """Maximum over the same footprint as tent_down."""
    def axis0(x, first):
        p = np.concatenate([first[None], x], 0)
        return np.maximum(np.maximum(p[0:-1:2], p[1::2]), p[2::2])
    a = axis0(a, a[0] if above is None else above)
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


def _bodies(world: Path) -> tuple[np.ndarray, np.ndarray]:
    """Lookup tables by world body id: kind code (KINDS, 0 none) and lake area (km²)."""
    path = world / "bodies.json"
    if not path.exists():
        return np.zeros(1, np.uint8), np.zeros(1, np.float32)
    bodies = json.loads(path.read_text())["bodies"]
    n = max((b["id"] for b in bodies), default=0) + 1
    kind, area = np.zeros(n, np.uint8), np.zeros(n, np.float32)
    for b in bodies:
        kind[b["id"]] = KINDS.get(b.get("kind"), 0)
        area[b["id"]] = b.get("area_km2", 0.0)
    return kind, area


def _region_temperatures(layout_dir: Path, spec: dict) -> tuple[dict, str]:
    """Sea-level temperature (°C) by region id: the layout snapshot's `temperature_c`, else the
    collage's source layout file's (a game-only key, so adding it needs no collage rebuild)."""
    temps = {r["id"]: float(r["temperature_c"]) for r in spec.get("regions", []) if "temperature_c" in r}
    source = "layout snapshot"
    meta = layout_dir / "collage.json"
    if meta.exists():
        src = Path(json.loads(meta.read_text())["layout"])
        if src.exists():
            live = tomllib.loads(src.read_text())
            extra = {r["id"]: float(r["temperature_c"]) for r in live.get("regions", [])
                     if "temperature_c" in r and r["id"] not in temps}
            if extra:
                temps.update(extra)
                source = str(src.resolve())
    return temps, source


def _temperature_grid(layout_dir: Path, spec: dict) -> tuple[np.ndarray, float, dict]:
    """Sea-level temperature on the collage's region grid (layout coordinates), blended across
    region borders; sea cells take the nearest region's. Uniform default without a region map."""
    temps, source = _region_temperatures(layout_dir, spec)
    meta = layout_dir / "collage.json"
    info = {"default_c": TEMPERATURE_DEFAULT_C, "source": source}
    if not meta.exists():
        info["regions"] = {}
        return np.full((2, 2), TEMPERATURE_DEFAULT_C, np.float32), float("inf"), info
    e = json.loads(meta.read_text())["region32"]
    w, h = e["size"]
    region = np.fromfile(layout_dir / e["file"], np.uint8).reshape(h, w)
    lut = np.array([TEMPERATURE_DEFAULT_C] + [temps.get(r, TEMPERATURE_DEFAULT_C) for r in e["legend"]], np.float32)
    idx = ndimage.distance_transform_edt(region == 0, return_distances=False, return_indices=True)
    t = lut[region[idx[0], idx[1]]]
    del idx
    cell = float(e["cell_m"])
    info["regions"] = {r: float(lut[i + 1]) for i, r in enumerate(e["legend"])}
    return ndimage.gaussian_filter(t, REGION_BLEND_M / cell), cell, info


def _at_layout(grid: np.ndarray, cell_m: float, ym: np.ndarray, xm: np.ndarray) -> np.ndarray:
    """A layout-aligned grid read bilinearly at layout metres (rows ym, columns xm)."""
    if not np.isfinite(cell_m):
        return np.full((len(ym), len(xm)), grid.flat[0], np.float32)
    fy = np.broadcast_to((ym / cell_m - 0.5)[:, None], (len(ym), len(xm)))
    fx = np.broadcast_to((xm / cell_m - 0.5)[None, :], (len(ym), len(xm)))
    return ndimage.map_coordinates(grid, [fy, fx], order=1, mode="nearest").astype(np.float32)


def talus(h, slope2, q, s):
    """Talus density: debris shed by steep rock settles below it under the angle of repose, most
    in hollows and gullies, none in channels or on the rock itself."""
    ramp = cover._ramp
    src = ramp(slope2, *TALUS_SOURCE_DEG)
    gs = ndimage.gaussian_filter(src, TALUS_REACH_M / s)
    hr = ndimage.gaussian_filter(src * h, TALUS_REACH_M / s) / np.maximum(gs, 1e-6)  # nearby rock height
    supply = np.clip(gs / TALUS_SUPPLY, 0, 1)
    below = ramp(hr - h, *TALUS_DROP_M)
    del src, gs, hr
    hollow = ndimage.gaussian_filter(h, TALUS_HOLLOW_M[1] / s) - ndimage.gaussian_filter(h, TALUS_HOLLOW_M[0] / s)
    lq = np.log10(np.maximum(q, 1e-9))
    shape = np.maximum(ramp(hollow, *TALUS_HOLLOW_RAMP), ramp(lq, *np.log10(TALUS_GULLY_KM2)))
    t = (supply * below * (1 - ramp(slope2, *TALUS_REPOSE_DEG)) * (TALUS_OPEN + (1 - TALUS_OPEN) * shape)
         * (1 - ramp(q, *TALUS_CHANNEL_KM2)))
    return np.clip(TALUS_GAIN * t, 0, 1).astype(np.float32)


def riparian(h, slope2, src, width, surf, s):
    """Riparian band membership (0-1): within the nearest source's width of lake or river water,
    not high above its surface, off steep banks. None where no texel is a source."""
    if not src.any():
        return np.zeros(h.shape, np.float32)
    ramp = cover._ramp
    d, idx = ndimage.distance_transform_edt(~src, sampling=s, return_indices=True)
    d = d.astype(np.float32)
    w = width[idx[0], idx[1]]
    above = h - surf[idx[0], idx[1]]
    del idx
    z = (1 - ramp(d, 0.5 * w, w)) * (1 - ramp(above, *RIPARIAN_ABOVE_M)) * (1 - ramp(slope2, *RIPARIAN_SLOPE_DEG))
    return z.astype(np.float32)


def _fields(sampler, h, wet, ice, moist, glacier, q, src, width, surf, y0_m, x0_m, s):
    """`glacier` is the smoothed glacier class share at the texels; `ice` frozen lakes and glaciers."""
    ramp = cover._ramp
    f = sampler.fields(h, y0_m, x0_m, s)
    slope2 = np.degrees(np.arctan(np.hypot(*np.gradient(h, s))))  # local slope for scree and banks
    snow = np.maximum(ramp(glacier, *GLACIER_SHARE),
                      ramp(h - f["snowline"], -SNOW_SOFT_M, SNOW_SOFT_M) * (1 - ramp(f["slope"], *SNOW_SHED_DEG)))
    snow *= (h >= 0) & (wet < WATER_WET)  # no snow where land cover is water
    snow = np.maximum(snow, ice)  # frozen lakes and glaciers are snow/ice whatever the ground's height
    forest = f["share"] * ramp(moist, MOIST_DRY, MOIST_WET) * (1 - snow) * (1 - wet)
    zone = riparian(h, slope2, src, width, surf, s) * (1 - snow) * (wet < WATER_WET)
    rip = RIPARIAN_DENSITY * zone
    # Riparian land cover where the band is denser than the forest, fading as forest reaches it.
    rip_lc = np.minimum(zone * np.clip(1 - forest / RIPARIAN_DENSITY, 0, 1), 1 - forest - snow)
    tal = talus(h, slope2, q, s) * (1 - snow) * (1 - wet)
    # Topographic wetness ln(a / tan β), a the specific catchment from discharge (km²) per texel width.
    a = np.maximum(q * 1e6, s * s) / s
    twi = np.log(a / np.maximum(np.tan(np.radians(slope2)), 1e-3))
    lo, hi = MOIST_LOCAL
    local = lo + (hi - lo) * ramp(twi, *MOIST_TWI) + MOIST_POLE * f["pole"]
    moisture = np.maximum(np.clip(moist * local, 0, 1), MOIST_RIPARIAN * zone)
    moisture = np.where(wet >= WATER_WET, 1.0, moisture)
    return {"vegetation-density": np.maximum(forest, rip), "snow-cover": snow, "talus-density": tal,
            "moisture": moisture, "slope2": slope2, "treeline": f["treeline"], "forest": forest,
            "riparian": rip_lc}


def cover_shares(biome, h, slope, treeline, forest, rip, snow, wet) -> np.ndarray:
    """Share of each COVER_CODES class at texels of one biome id array. Each rule claims an
    interval of a uniform u (later rules win): snow [0, snow), trees (1 - forest, 1], riparian
    just below the trees; between them rock, then the alpine or bare-class ramps, then the
    biome's open cover. Water at a wet share >= WATER_WET is the whole texel."""
    ramp = cover._ramp
    lo = snow
    hi = 1 - forest - rip

    def below(x):  # length of [lo, hi) ∩ [0, x)
        return np.clip(np.minimum(x, hi) - lo, 0, None)

    r_rock = ramp(slope, *ROCK_RAMP_DEG)
    bare_class = np.isin(biome, BARE_CLASSES)
    r_class = np.where(bare_class, ramp(slope, *BARE_CLASS_DEG), ramp(h - treeline, -TREELINE_EDGE_M, TREELINE_EDGE_M))
    r_alpine = np.where(bare_class, 1.0, ramp(h - treeline, 0.0, ALPINE_BAND_M))  # bare part of the class rule
    rock = below(r_rock)
    upto_class = below(np.maximum(r_class, r_rock))
    class_bare = below(np.maximum(np.minimum(r_alpine, r_class), r_rock)) - rock
    class_grass = upto_class - rock - class_bare
    open_ = np.clip(hi - lo, 0, None) - upto_class
    out = np.zeros((len(COVER_CODES),) + np.shape(h), np.float32)
    k = {c: i for i, c in enumerate(COVER_CODES)}
    out[k[SNOW]] = snow
    out[k[TREE]] = forest
    out[k[RIPARIAN]] = rip
    out[k[BARE]] = rock + class_bare
    out[k[GRASS]] = class_grass
    open_code = OPEN[biome]
    for c in (SHRUB, GRASS, BARE):
        out[k[c]] += np.where(open_code == c, open_, 0)
    water = wet >= WATER_WET
    out[:, water] = 0
    out[k[WATER], water] = 1
    return out


def top2(shares: np.ndarray, codes) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """(a, b, weight u8) from shares (K, ...): a the largest, b the second (= a when it is 0),
    weight = a's share of a + b × 255."""
    codes = np.asarray(codes, np.uint8)
    i1 = np.argmax(shares, 0)
    s1 = np.take_along_axis(shares, i1[None], 0)[0]
    rest = shares.copy()
    np.put_along_axis(rest, i1[None], -1.0, 0)
    i2 = np.argmax(rest, 0)
    s2 = np.maximum(np.take_along_axis(rest, i2[None], 0)[0], 0)
    a, b = codes[i1], codes[i2]
    b = np.where(s2 > 0, b, a)
    w = s1 / np.maximum(s1 + s2, 1e-9)
    return a, b, np.round(255 * w).astype(np.uint8)


def _lerp_index(pos: np.ndarray, n: int):
    """Indices and weights of linear interpolation at grid positions, clamped at the edges."""
    pos = np.clip(pos, 0, n - 1)
    i0 = np.minimum(np.floor(pos).astype(np.intp), n - 1)
    return i0, np.minimum(i0 + 1, n - 1), (pos - i0).astype(np.float32)


def _share_at(share: np.ndarray, rows: np.ndarray, cols) -> np.ndarray:
    """A 32 m share read bilinearly (separably) at game texel rows."""
    r0, r1, tr = _lerp_index(rows, share.shape[0])
    c0, c1, tc = cols
    s = share[r0] * (1 - tr)[:, None] + share[r1] * tr[:, None]
    return s[:, c0] * (1 - tc) + s[:, c1] * tc


def _biome_top2(shares, rows: np.ndarray, cols) -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    """Two largest biome shares at game texel rows, holding one class's share at a time:
    (a, b, share of a, share of b), b = a where only one class is present."""
    shape = (len(rows), len(cols[0]))
    s1, s2 = np.zeros(shape, np.float32), np.zeros(shape, np.float32)
    a, b = np.zeros(shape, np.uint8), np.zeros(shape, np.uint8)
    for k, share in shares:
        s = _share_at(share, rows, cols)
        first = s > s1
        second = ~first & (s > s2)
        s2 = np.where(first, s1, np.where(second, s, s2))
        b = np.where(first, a, np.where(second, k, b))
        s1 = np.where(first, s, s1)
        a = np.where(first, k, a).astype(np.uint8)
    b = np.where(s2 > 0, b, a).astype(np.uint8)
    return a, b, s1, s2


def write(world: Path, layout_dir: Path | None = None, rows: int = ROWS) -> dict:
    """Write game/<name>.u8 at 2 m (the world's spacing if coarser), see ENCODINGS. Records them
    in the manifest; returns manifest["game"]."""
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
    q_path = world / "hydrology" / "1m" / "discharge.f32"
    if not q_path.exists():
        raise ValueError(f"world has no {q_path.relative_to(world)}; run hydrology v3 at 1 m")
    origin = json.loads((Path(manifest["world_input"]) / "world.json").read_text())["origin"]
    l0 = layout.load(layout_dir)
    sampler = cover.Sampler(l0)
    oy, ox = _layout_offset(origin, sampler.extent_m, n * c0)
    ground = np.memmap(world / "elevation.f32", "<f4", "r", shape=(n, n))
    surface = np.memmap(world / "water-surface.f32", "<f4", "r", shape=(n, n))
    body = np.memmap(world / "water-body.u32", "<u4", "r", shape=(n, n))
    discharge = np.memmap(q_path, "<f4", "r", shape=(n, n))
    glacier_path = world / "glacier.u8"
    glacier_mask = np.memmap(glacier_path, np.uint8, "r", shape=(n, n)) if glacier_path.exists() else None
    kind_lut, area_lut = _bodies(world)
    t_grid, t_cell, t_info = _temperature_grid(Path(layout_dir), l0.spec)
    (world / "game").mkdir(exist_ok=True)
    # New files replaced into place: world copies may hard-link the original's layers.
    tmps = {k: world / "game" / f"{k}.u8.tmp" for k in NAMES}
    for t in tmps.values():
        t.unlink(missing_ok=True)
    outs = {k: np.memmap(tmps[k], np.uint8, "w+", shape=(mg, mg)) for k in NAMES}
    m = int(np.ceil(cover.MARGIN_M / s))
    align = max(1, int(round(cover.FEATURE_M / s)))  # cover.terrain_features block size
    cols = np.arange(mg)
    xm = ox + (cols * step + 0.5) * c0  # texel centres in layout metres
    # Biome shares on a 32 m grid over the world; texels read them at their centres.
    n32 = int(np.ceil(n * c0 / 32.0))
    c32 = sampler.classes(oy, ox, n32, n32, 32.0)
    shares = [(int(k), ndimage.gaussian_filter((c32 == k).astype(np.float32), BIOME_MIX_M / 32.0))
              for k in np.unique(c32)]
    del c32
    glacier32 = dict(shares).get(cover.GLACIER, np.zeros((n32, n32), np.float32))

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
        bid = np.asarray(body[r0 : step * g1])
        bid = np.where(bid < len(kind_lut), bid, 0)  # ids missing from bodies.json count as none
        kind = kind_lut[bid]
        ice1 = np.isin(kind, (KINDS["ice"], KINDS["glacier"]))
        if glacier_mask is not None:
            ice1 |= np.asarray(glacier_mask[r0 : step * g1]) > 0
        sf = np.asarray(surface[r0 : step * g1])
        # Ice is a frozen lake's surface and glaciers are ground: snow/ice cover, not land-cover water.
        wet1 = (sf > g) & ~ice1
        q1 = np.asarray(discharge[r0 : step * g1], np.float32)
        lake = wet1 & (kind == KINDS["lake"])
        river = wet1 & (kind == KINDS["river"]) & (q1 >= RIPARIAN_Q_KM2)
        # Band width by river discharge or lake area.
        big = np.where(river, cover._ramp(np.log10(np.maximum(q1, 1e-9)), *np.log10(RIPARIAN_WIDE_KM2)),
                       cover._ramp(np.log10(np.maximum(area_lut[bid], 1e-9)), *np.log10(RIPARIAN_WIDE_LAKE_KM2)))
        lo_w, hi_w = RIPARIAN_WIDTH_M
        src1 = lake | river
        width1 = np.where(src1, lo_w + (hi_w - lo_w) * big, 0).astype(np.float32)
        surf1 = np.where(src1, sf, -np.inf).astype(np.float32)
        del bid, kind, lake, river, big, sf
        if level:
            k = step * g0 - r0  # 1 when there is a row above

            def dn(a, f=tent_down):
                return f(a[k:], a[0] if k else None)

            h, wet, ice = dn(g), dn(wet1.astype(np.float32)), dn(ice1.astype(np.float32))
            q, src, width, surf = dn(q1, max_down), dn(src1, max_down), dn(width1, max_down), dn(surf1, max_down)
        else:
            h, wet, ice, q, src, width, surf = g, wet1.astype(np.float32), ice1.astype(np.float32), q1, src1, width1, surf1
        del g, wet1, ice1, q1, src1, width1, surf1
        moist = _moisture(world, manifest, origin, np.arange(g0, g1), cols, step, c0)
        glacier = _share_at(glacier32, to32(np.arange(g0, g1)), lerp_cols)
        f = _fields(sampler, h, wet, ice, moist, glacier, q, src, width, surf, oy + g0 * s, ox, s)
        del moist, glacier, q, src, width, surf
        sl = slice(y - g0, b - g0)
        for name in DENSITIES + ("moisture",):
            outs[name][y:b] = np.round(255 * np.clip(f[name][sl], 0, 1)).astype(np.uint8)
        ym = oy + (np.arange(y, b) * step + 0.5) * c0
        hb = h[sl]
        temp = (_at_layout(t_grid, t_cell, ym, xm) - LAPSE_C_PER_M * np.maximum(hb, 0)
                + COAST_WARM_C * (1 - cover._ramp(_at_layout(l0.coast, l0.cell_m, ym, xm), 0.0, COAST_FADE_M)))
        outs["temperature"][y:b] = np.clip(np.round((temp - TEMPERATURE_OFFSET_C) / TEMPERATURE_STEP_C), 0, 255)
        ba, bb, sa, sb = _biome_top2(shares, to32(np.arange(y, b)), lerp_cols)
        wa = sa / np.maximum(sa + sb, 1e-9)
        outs["biome-a"][y:b], outs["biome-b"][y:b] = ba, bb
        outs["biome-weight"][y:b] = np.round(255 * wa)
        args = (hb, f["slope2"][sl], f["treeline"][sl], f["forest"][sl], f["riparian"][sl],
                f["snow-cover"][sl], wet[sl])
        mix = wa * cover_shares(ba, *args) + (1 - wa) * cover_shares(bb, *args)
        la, lb, lw = top2(mix, COVER_CODES)
        outs["land-cover-a"][y:b], outs["land-cover-b"][y:b], outs["land-cover-weight"][y:b] = la, lb, lw
        del f, mix
    for a in outs.values():
        a.flush()
    del outs
    for k in NAMES:
        os.replace(tmps[k], world / "game" / f"{k}.u8")
        manifest["layers"][f"game/{k}.u8"] = cover._digest(world / "game" / f"{k}.u8")
    for old in CLASSES:  # the dithered class layers of game layers v1
        (world / "game" / f"{old}.u8").unlink(missing_ok=True)
        manifest["layers"].pop(f"game/{old}.u8", None)
    manifest["game"] = {
        "spacing_m": s, "size": [mg, mg], "level": level, "lattice": LATTICE, "method": METHOD,
        "encodings": ENCODINGS,
        "inputs": {"discharge": str(q_path.relative_to(world)),
                   "glacier": glacier_path.name if glacier_mask is not None else None},
        "params": {"layout": str(Path(layout_dir).resolve()), "moist_dry": MOIST_DRY, "moist_wet": MOIST_WET,
                   "snow_soft_m": SNOW_SOFT_M, "snow_shed_deg": list(SNOW_SHED_DEG),
                   "talus": {"source_deg": list(TALUS_SOURCE_DEG), "reach_m": TALUS_REACH_M, "supply": TALUS_SUPPLY,
                             "drop_m": list(TALUS_DROP_M), "repose_deg": list(TALUS_REPOSE_DEG),
                             "hollow_m": list(TALUS_HOLLOW_M), "hollow_ramp_m": list(TALUS_HOLLOW_RAMP),
                             "gully_km2": list(TALUS_GULLY_KM2), "open": TALUS_OPEN,
                             "channel_km2": list(TALUS_CHANNEL_KM2), "gain": TALUS_GAIN},
                   "riparian": {"code": RIPARIAN, "q_km2": RIPARIAN_Q_KM2, "width_m": list(RIPARIAN_WIDTH_M),
                                "wide_km2": list(RIPARIAN_WIDE_KM2), "wide_lake_km2": list(RIPARIAN_WIDE_LAKE_KM2),
                                "density": RIPARIAN_DENSITY, "above_m": list(RIPARIAN_ABOVE_M),
                                "slope_deg": list(RIPARIAN_SLOPE_DEG)},
                   "temperature": {**t_info, "lapse_c_per_m": LAPSE_C_PER_M, "coast_warm_c": COAST_WARM_C,
                                   "coast_fade_m": COAST_FADE_M, "region_blend_m": REGION_BLEND_M},
                   "moisture": {"twi": list(MOIST_TWI), "local": list(MOIST_LOCAL), "pole": MOIST_POLE,
                                "riparian": MOIST_RIPARIAN},
                   "biome_mix_m": BIOME_MIX_M, "glacier_share": list(GLACIER_SHARE),
                   "alpine_band_m": ALPINE_BAND_M, "treeline_edge_m": TREELINE_EDGE_M,
                   "rock_ramp_deg": list(ROCK_RAMP_DEG), "bare_class_deg": list(BARE_CLASS_DEG),
                   "water_wet": WATER_WET, "rows": rows},
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
        v = value(np.asarray(a[r * k : b * k, : m * k]), r * k, b * k)
        out.append(v.reshape(b - r, k, m, k, -1).mean(axis=(1, 3)))
    return np.concatenate(out)


def _png(world: Path, game: dict, out: Path, px: int = 1024) -> None:
    from PIL import Image

    mg = game["size"][0]
    k = max(1, mg // px)
    game_dir = world / "game"
    rgb = [_area_mean(game_dir / f"{name}.u8", mg, k, lambda v, *_: v.astype(np.float32)[..., None])[..., 0]
           for name in ("snow-cover", "vegetation-density", "talus-density")]
    out.parent.mkdir(parents=True, exist_ok=True)
    Image.fromarray(np.round(np.stack(rgb, -1)).astype(np.uint8)).save(out)
    # Land cover in WorldCover colours, a and b blended by weight and area-averaged.
    lut = np.zeros((256, 3), np.float32)
    for code, hexc in WORLDCOVER_RGB.items():
        lut[code] = [int(hexc[i : i + 2], 16) for i in (0, 2, 4)]
    lb, lw = (np.memmap(game_dir / f"land-cover-{p}.u8", np.uint8, "r", shape=(mg, mg)) for p in ("b", "weight"))

    def blend(v, r0, r1):
        w = np.asarray(lw[r0:r1, : v.shape[1]], np.float32)[..., None] / 255
        return w * lut[v] + (1 - w) * lut[np.asarray(lb[r0:r1, : v.shape[1]])]

    c = _area_mean(game_dir / "land-cover-a.u8", mg, k, blend)
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
    for name in DENSITIES + ("moisture",):
        v = np.memmap(args.world / "game" / f"{name}.u8", np.uint8, "r")[::97].astype(np.float32) / 255
        print(f"  {name}: mean {v.mean():.3f}, share > 0.5 {(v > 0.5).mean():.3f}")
    t = np.memmap(args.world / "game" / "temperature.u8", np.uint8, "r")[::97].astype(np.float32)
    t = t * TEMPERATURE_STEP_C + TEMPERATURE_OFFSET_C
    print(f"  temperature: {t.min():.1f} .. {t.max():.1f} °C, mean {t.mean():.1f}")
    for name in CLASSES:
        counts = np.bincount(np.memmap(args.world / "game" / f"{name}-a.u8", np.uint8, "r")[::97], minlength=256)
        share = {int(c): round(float(counts[c] / counts.sum()), 3) for c in np.nonzero(counts)[0]}
        w = np.memmap(args.world / "game" / f"{name}-weight.u8", np.uint8, "r")[::97]
        print(f"  {name}-a: share by code (sampled) {share}; weight < 255 on {(w < 255).mean():.3f}")
    if args.png:
        _png(args.world, game, args.png)
        print(f"wrote {args.png} and {_landcover_png(args.png)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
