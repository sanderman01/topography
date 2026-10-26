"""Layouts as a collage of real GLO-30 regions (docs/plans/fujix-v1.md).

python -m topography_training.diffusion.collage <layout.toml> --release <context release.json> --out <new-dir>

Each region's ground comes from pieces of real regions, cut out, rotated, scaled and
remapped in height, then blended across region borders with a Laplacian pyramid; classes
are dithered across the same borders. Writes an L0 directory that `layout.load` reads, the
32 m surface (arrays.npz: l1, l2) and pictures. Positions are metres from the north-west
corner, x east, y south; rotations are clockwise on the map.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import time
import tomllib
from dataclasses import dataclass
from pathlib import Path

import numpy as np

from . import cascade, description, layout, legend

CELL_M = 32.0  # GLO-30's resolution, and the L2 level
L0_CELL_M = 128.0
MAX_SCALE = 1.5  # the layout spec's scale budget
FEATHER_M = 2000.0  # a piece fades out over this distance inside its crop edge
PYRAMID_LEVELS = 6  # 32 m to 1 km
OCEAN_BORDER_M = 3000.0
OCEAN_SHELF_M = 1200.0  # the seabed falls most of the way to its edge depth over about this distance
SINK_M = 1000.0  # ground past the island outline sinks to the seabed over this distance
ISLAND_CLOSE_M = 4000.0  # gaps between region outlines narrower than this stay land
SHELF_M = -20.0  # sea floor at the island's outline
EDGE_LIMIT_SLOPE = 2.0  # m per m past the coast margin, so the edge limit seldom clamps land there


@dataclass
class Piece:
    source: str
    at: tuple[float, float]  # world position of the crop centre
    size_m: tuple[float, float]  # crop width (east) and height (south) in source metres
    offset_m: tuple[float, float] = (0.0, 0.0)  # crop centre from the source centre, east and south
    rotate_deg: float = 0.0
    scale: float = 1.0  # world metres per source metre
    remap: tuple[float, float, float, float] | None = None  # source lo, hi -> world lo, hi (linear)
    tilt_m_per_km: tuple[float, float] = (0.0, 0.0)  # added slope, east and south
    class_id: int | None = None  # the piece's class instead of the region's


@dataclass
class Region:
    id: str
    name: str
    class_id: int
    outline: list
    border_m: float
    forest: float
    pieces: list[Piece]


@dataclass
class Source:
    height: np.ndarray
    classes: np.ndarray  # legend v3 per cell
    class_id: int
    spacing_m: float


def validate(spec: dict) -> None:
    """Clear errors for layout mistakes that would otherwise fail deep inside the build."""
    if spec.get("schema_version") != 2 or spec.get("kind") != "world_layout":
        raise ValueError('not a layout v2 (schema_version = 2, kind = "world_layout")')
    if "flight" not in spec:
        raise ValueError("a layout v2 needs [flight] with clearance_m and bands")
    try:
        names, tops, _ = layout.flight(spec)
    except (KeyError, TypeError) as e:
        raise ValueError(f"[flight] needs clearance_m and bands of {{ name, top_m }} ({e})") from None
    if any(b <= a for a, b in zip(tops, tops[1:])):
        raise ValueError(f"[flight] band tops must rise from band to band: {tops}")
    if len(names) > len(layout.BAND_RGB):
        raise ValueError(f"[flight] has {len(tops)} bands; at most {len(layout.BAND_RGB) - 1} have colours")
    for p in spec.get("passes", []):
        if p.get("band") not in names[:-1]:
            raise ValueError(f"pass {p.get('id')}: band {p.get('band')!r} is not a [flight] band "
                             f"({', '.join(names[:-1])})")
    for f in spec.get("features", []):
        if f.get("kind") == "volcano" and "radius_m" not in f:
            raise ValueError(f"volcano {f.get('id')}: needs radius_m")
        if ("caldera_diameter_m" in f) != ("caldera_floor_m" in f):
            raise ValueError(f"{f.get('id')}: give both caldera_diameter_m and caldera_floor_m, or neither")


def parse(spec: dict) -> list[Region]:
    validate(spec)
    out = []
    for r in spec["regions"]:
        pieces = []
        for p in r["pieces"]:
            piece = Piece(p["source"], tuple(p["at"]), tuple(p["size_m"]), tuple(p.get("offset_m", (0.0, 0.0))),
                          float(p.get("rotate_deg", 0.0)), float(p.get("scale", 1.0)),
                          tuple(p["remap"]) if "remap" in p else None, tuple(p.get("tilt_m_per_km", (0.0, 0.0))),
                          p.get("class_id"))
            if not 1 / MAX_SCALE <= piece.scale <= MAX_SCALE:
                raise ValueError(f"{r['id']}: {p['source']} scale {piece.scale} is outside 1/{MAX_SCALE} to {MAX_SCALE}")
            if piece.remap is not None and (len(piece.remap) != 4 or piece.remap[0] == piece.remap[1]):
                raise ValueError(f"{r['id']}: {p['source']} remap needs [source lo, source hi, world lo, world hi] "
                                 f"with lo != hi, got {list(piece.remap)}")
            pieces.append(piece)
        out.append(Region(r["id"], r["name"], int(r["class_id"]), r["outline"], float(r.get("border_m", 3000.0)),
                          float(r.get("forest", 0.0)), pieces))
    return out


def sources(release: Path, ids: set[str]) -> dict[str, Source]:
    found = {r.name: r for r in cascade.load_release(release, ids)}
    missing = sorted(ids - found.keys())
    if missing:
        raise ValueError(f"not in the release: {', '.join(missing)}")
    out = {}
    for name, r in found.items():
        h = np.nan_to_num(r.height, nan=float(np.nanmedian(r.height))).astype(np.float32)
        cls = legend.cell_classes(r.class_id, h, r.snow, r.water, float(r.spacing_m))
        out[name] = Source(h, cls, int(r.class_id), float(r.spacing_m))
    return out


def check_crops(regions: list[Region], srcs: dict[str, Source]) -> None:
    for r in regions:
        for p in r.pieces:
            half = srcs[p.source].height.shape[0] * srcs[p.source].spacing_m / 2
            if abs(p.offset_m[0]) + p.size_m[0] / 2 > half or abs(p.offset_m[1]) + p.size_m[1] / 2 > half:
                raise ValueError(f"{r.id}: the {p.source} crop reaches past its source ({2 * half / 1000:.0f} km)")


def grid(n: int) -> tuple[np.ndarray, np.ndarray]:
    c = (np.arange(n, dtype=np.float32) + 0.5) * CELL_M
    return np.meshgrid(c, c)  # x east along columns, y south along rows


def place(p: Piece, src: Source, x: np.ndarray, y: np.ndarray):
    """The piece's heights, classes and weight on the world grid (weight 0 outside its crop)."""
    from scipy import ndimage

    t = np.radians(p.rotate_deg)
    dx, dy = (x - p.at[0]) / p.scale, (y - p.at[1]) / p.scale
    u = np.cos(t) * dx + np.sin(t) * dy  # undo the clockwise rotation: source metres from the crop centre
    v = -np.sin(t) * dx + np.cos(t) * dy
    edge = np.minimum(p.size_m[0] / 2 - np.abs(u), p.size_m[1] / 2 - np.abs(v)) * p.scale
    w = np.clip(edge / FEATHER_M, 0, 1)
    w = (w * w * (3 - 2 * w)).astype(np.float32)
    on = w > 0
    n_src = src.height.shape[0]
    coords = [n_src / 2 + (p.offset_m[1] + v[on]) / src.spacing_m - 0.5,
              n_src / 2 + (p.offset_m[0] + u[on]) / src.spacing_m - 0.5]
    h = np.zeros(x.shape, np.float32)
    cls = np.zeros(x.shape, np.uint8)
    h[on] = ndimage.map_coordinates(src.height, coords, order=1, mode="nearest")
    cls[on] = ndimage.map_coordinates(src.classes, coords, order=0, mode="nearest")
    if p.remap:
        a, b, c, d = p.remap
        h[on] = c + (h[on] - a) * (d - c) / (b - a)
    tx, ty = p.tilt_m_per_km
    h[on] += ((x[on] - p.at[0]) * tx + (y[on] - p.at[1]) * ty) / 1000
    return h, cls, w


def fill_smooth(a: np.ndarray, known: np.ndarray) -> np.ndarray:
    """Unknown cells take a smooth average of the known ones (pull-push normalised convolution,
    coarse to fine); known cells stay exact."""
    if known.all():
        return a
    if not known.any():
        return np.zeros_like(a)
    num, den = [np.where(known, a, 0).astype(np.float32)], [known.astype(np.float32)]
    while min(num[-1].shape) > 4:
        num.append(_down(num[-1]))
        den.append(_down(den[-1]))
    est = num[-1] / np.maximum(den[-1], 1e-12)
    for nu, de in zip(reversed(num[:-1]), reversed(den[:-1])):
        # Finer averages take over where enough known cells are near.
        t = np.clip(2 * de, 0, 1)
        est = t * nu / np.maximum(de, 1e-12) + (1 - t) * _up(est, nu.shape)
    return np.where(known, a, est).astype(np.float32)


def region_surface(r: Region, srcs: dict[str, Source], x: np.ndarray, y: np.ndarray, noise: np.ndarray):
    """Heights (filled smoothly past the pieces), classes dithered by piece weight, and covered cells."""
    num = np.zeros(x.shape, np.float32)
    den = np.zeros(x.shape, np.float32)
    # The region's own class wherever no piece reaches.
    classes, weights = [np.full(x.shape, r.class_id, np.uint8)], [np.full(x.shape, 1e-4, np.float32)]
    for p in r.pieces:
        src = srcs[p.source]
        h, c, w = place(p, src, x, y)
        num += w * h
        den += w
        # The source's own class becomes the region's (or the piece's); glacier, alpine and plain cells stay.
        classes.append(np.where(c == src.class_id, p.class_id or r.class_id, c).astype(np.uint8))
        weights.append(w)
    covered = den > 1e-3
    return fill_smooth(num / np.maximum(den, 1e-6), covered), dither(classes, weights, noise), covered


def weight(sd: np.ndarray, border_m: float) -> np.ndarray:
    """Soft membership: 0.5 on the outline, about 0.98 half a border inside."""
    return (1 / (1 + np.exp(-np.clip(8 * sd / border_m, -60, 60)))).astype(np.float32)


def island_sd(sds: list[np.ndarray]) -> np.ndarray:
    """Signed distance to the union of region outlines, with narrow gaps between them closed."""
    from scipy import ndimage

    inside = np.max(sds, axis=0) > 0
    grown = ndimage.distance_transform_edt(~inside) * CELL_M <= ISLAND_CLOSE_M / 2
    closed = ndimage.distance_transform_edt(grown) * CELL_M > ISLAND_CLOSE_M / 2
    return ((ndimage.distance_transform_edt(closed) - ndimage.distance_transform_edt(~closed)) * CELL_M).astype(
        np.float32)


def ocean(isd: np.ndarray, spec: dict) -> np.ndarray:
    edge = float(spec.get("seabed_edge_m", -200.0))
    out_m = np.maximum(-isd, 0)
    # Zero slope at the outline, so land that runs past it meets no crease there.
    return (SHELF_M + (edge - SHELF_M) * (1 - np.exp(-((out_m / OCEAN_SHELF_M) ** 2)))).astype(np.float32)


def sink_outside(h: np.ndarray, isd: np.ndarray, sea: np.ndarray) -> np.ndarray:
    """Ground the blend's coarse bands carry past the island outline sinks to the seabed;
    sea already deeper than the seabed (pieces' own sea) is left alone."""
    t = np.clip(-isd / SINK_M, 0, 1)
    t = t * t * (3 - 2 * t)
    return (h - t * np.maximum(h - sea, 0)).astype(np.float32)


def edge_limit(n: int, spec: dict) -> np.ndarray:
    """Highest ground per cell: the seabed at the map edge rising to the shelf at the coast margin."""
    margin = float(spec.get("coast_margin_m", 4000.0))
    edge = float(spec.get("seabed_edge_m", -200.0))
    c = (np.arange(n) + 0.5) * CELL_M
    e = np.minimum(c, n * CELL_M - c)
    d = np.minimum(e[:, None], e[None, :])
    rise = edge + (SHELF_M - edge) * d / margin
    return np.where(d < margin, rise, SHELF_M + (d - margin) * EDGE_LIMIT_SLOPE).astype(np.float32)


def _down(a: np.ndarray) -> np.ndarray:
    from scipy import ndimage

    return ndimage.zoom(ndimage.gaussian_filter(a, 1.0), 0.5, order=1)


def _up(a: np.ndarray, shape) -> np.ndarray:
    """Bilinear upsampling, blurred so repeated use leaves no kinks on the coarse grid."""
    from scipy import ndimage

    return ndimage.gaussian_filter(ndimage.zoom(a, (shape[0] / a.shape[0], shape[1] / a.shape[1]), order=1), 1.0)


# (wavelength, displacement std) in m. Kept gentle so the warp never folds: det(I + grad d) > 0.3.
WARP_OCTAVES = ((18000.0, 620.0), (6000.0, 80.0))
WARP_SEED = 7


def warp_field(n: int, seed: int = WARP_SEED) -> tuple[np.ndarray, np.ndarray]:
    """Smooth fixed-seed displacements (east, south, m) that bend straight outlines."""
    from scipy import ndimage

    rng = np.random.default_rng(seed)
    coarse = 512.0  # noise is made at this spacing, then interpolated
    m = int(n * CELL_M / coarse)
    out = []
    for _ in range(2):
        d = np.zeros((n, n), np.float32)
        for wave, std in WARP_OCTAVES:
            w = ndimage.gaussian_filter(rng.standard_normal((m, m)), wave / (2 * np.pi * coarse), mode="wrap")
            d += ndimage.zoom((std * w / w.std()).astype(np.float32), n / m, order=3)
        out.append(d)
    return out[0], out[1]


def warp(a: np.ndarray, dx: np.ndarray, dy: np.ndarray) -> np.ndarray:
    """`a` sampled at each cell's displaced position."""
    from scipy import ndimage

    i, j = np.mgrid[0 : a.shape[0], 0 : a.shape[1]].astype(np.float32)
    return ndimage.map_coordinates(a, [i + dy / CELL_M, j + dx / CELL_M], order=1, mode="nearest").astype(np.float32)


def blend(surfaces: list[np.ndarray], weights: list[np.ndarray], levels: int = PYRAMID_LEVELS) -> np.ndarray:
    """Laplacian-pyramid blend: each band of detail mixes over the weights blurred to its
    scale, so broad shape changes across the whole border and fine detail over a strip."""
    total = np.maximum(np.sum(weights, axis=0), 1e-6)
    out = None
    for h, w in zip(surfaces, weights):
        gh, gw = [h], [w / total]
        for _ in range(levels - 1):
            gh.append(_down(gh[-1]))
            gw.append(_down(gw[-1]))
        lap = [gh[i] - _up(gh[i + 1], gh[i].shape) for i in range(levels - 1)] + [gh[-1]]
        mixed = [band * g for band, g in zip(lap, gw)]
        out = mixed if out is None else [a + b for a, b in zip(out, mixed)]
    h = out[-1]
    for band in reversed(out[:-1]):
        h = _up(h, band.shape) + band
    return h.astype(np.float32)


def local_relief(h: np.ndarray, radius_m: float = 1000.0) -> np.ndarray:
    from scipy import ndimage

    r = 2 * int(radius_m / CELL_M) + 1
    hi = ndimage.maximum_filter1d(ndimage.maximum_filter1d(h, r, 0), r, 1)
    lo = ndimage.minimum_filter1d(ndimage.minimum_filter1d(h, r, 0), r, 1)
    return (hi - lo).astype(np.float32)


def snowline(spec: dict, n: int, cell_m: float) -> np.ndarray:
    """Windward snowline upwind of the map centre, lee snowline downwind (moisture comes later)."""
    b = np.radians(float(spec.get("wind_from_deg", 225.0)))
    ux, uy = -np.sin(b), np.cos(b)  # downwind, east and south
    c = (np.arange(n, dtype=np.float32) + 0.5) * cell_m - n * cell_m / 2
    s = (c[None, :] * ux + c[:, None] * uy) / (n * cell_m / 2)
    t = np.clip((s + 0.3) / 0.6, 0, 1)
    t = t * t * (3 - 2 * t)
    lo, hi = float(spec["snowline_windward_m"]), float(spec["snowline_lee_m"])
    return (lo + (hi - lo) * t).astype(np.float32)


def write_l0(out: Path, text: str, spec: dict, h32: np.ndarray, cls32: np.ndarray, forest32: np.ndarray) -> None:
    """The L0 v1 directory that `layout.load` reads, at 128 m."""
    from scipy import ndimage

    k = int(L0_CELL_M / CELL_M)
    mid = cascade.block_mean(h32, k).astype(np.float32)
    n = mid.shape[0]
    land = mid >= 0
    layers = {
        "mid.f32": mid,
        "relief.f32": cascade.block_mean(local_relief(h32), k).astype(np.float32),
        "coast.f32": ((ndimage.distance_transform_edt(land) - ndimage.distance_transform_edt(~land)) * L0_CELL_M
                      ).astype(np.float32),
        "class.u8": legend.majority(cls32, k),
        "lake.u8": np.zeros((n, n), np.uint8),
        "lake-level.f32": np.zeros((n, n), np.float32),
        "snowline.f32": snowline(spec, n, L0_CELL_M),
        "forest.u8": np.clip(cascade.block_mean(forest32, k) * 255, 0, 255).astype(np.uint8),
    }
    for name, a in layers.items():
        a.astype("<f4" if name.endswith(".f32") else "u1").tofile(out / name)
    (out / "layout.json").write_text(json.dumps({
        "kind": "world_layout_l0", "schema_version": 1, "size": [n, n], "cell_m": L0_CELL_M,
        "layers": list(layers), "config_text": text}, indent=1))


def blue_noise(n: int, seed: int = 0) -> np.ndarray:
    """Uniform 0-1 values with little low-frequency content, so a dithered border reads as a fine mix."""
    from scipy import ndimage

    w = np.random.default_rng(seed).random((n, n), dtype=np.float32)
    hp = w - ndimage.gaussian_filter(w, 1.5, mode="wrap")
    r = np.empty(n * n, np.float32)
    r[np.argsort(hp, axis=None)] = (np.arange(n * n, dtype=np.float32) + 0.5) / (n * n)
    return r.reshape(n, n)


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


DIP_SIGMA_M = 1000.0  # a dip about 4 km across reads as a broad saddle
DIP_MAX_M = 300.0
DIP_STEP_M = 50.0


def dip_passes(h: np.ndarray, spec: dict) -> tuple[np.ndarray, list[tuple[str, float, bool]]]:
    """Lower each pass that sits above its band with a broad dip; returns (h, [(id, depth, ok)])."""
    names = layout.flight(spec)[0]
    n = h.shape[0]
    c = (np.arange(n, dtype=np.float32) + 0.5) * CELL_M
    h = h.copy()
    dips = []
    for p in spec.get("passes", []):
        want = names.index(p["band"])
        bump = np.exp(-((c[None, :] - p["at"][0]) ** 2 + (c[:, None] - p["at"][1]) ** 2) / (2 * DIP_SIGMA_M**2))
        depth = 0.0
        while True:
            got = layout.band_at(layout.band_map(h, CELL_M, spec), CELL_M, p["at"], description.PASS_RADIUS_M)
            if got <= want or depth >= DIP_MAX_M:
                break
            h -= (DIP_STEP_M * bump).astype(np.float32)
            depth += DIP_STEP_M
        dips.append((p["id"], depth, got <= want))
    return h, dips


CALDERA_WALL_M = 450.0  # inner wall, from the floor's edge to the rim crest
CALDERA_FLANK = 0.75  # outer slope (m per m) where the rim stands above the cone
CALDERA_RIM_DROP_M = 120.0  # the rim crest runs from a third of this to this below the old summit
CALDERA_CREST_M = 300.0  # width of the crest band where ground detail is damped
CALDERA_SOFT_M = 80.0  # soft max height, so the raised rim meets the cone without a crease
CALDERA_DETAIL_M = 400.0  # ground detail finer than this is kept on the rim and flanks
CALDERA_FLOOR_RISE_M = 40.0  # the ice floor rises this much toward the walls
PLATEAU_M = 150.0  # the summit plateau: ground this close to the top, joined to it


def summit_plateau(h: np.ndarray, x: np.ndarray, y: np.ndarray, f: dict) -> tuple[float, float, float]:
    """Centroid (x, y) and height of the summit plateau within the volcano's radius."""
    from scipy import ndimage

    disc = np.hypot(x - f["at"][0], y - f["at"][1]) <= f["radius_m"]
    top = float(h[disc].max())
    j, i = np.unravel_index(int(np.argmax(np.where(disc, h, -np.inf))), h.shape)
    labels, _ = ndimage.label(disc & (h >= top - PLATEAU_M))
    on = labels == labels[j, i]
    return float(x[on].mean()), float(y[on].mean()), top


def calderas(h: np.ndarray, spec: dict, x: np.ndarray, y: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """A flat-floored caldera on each volcano with `caldera_diameter_m` and `caldera_floor_m`,
    centred on its summit plateau: a rim at about the summit height, raised over the cone where
    needed, and steep inner walls down to the floor. Returns (h, ice floor cells)."""
    from scipy import ndimage

    def smax(a, b):
        return (a + b + np.sqrt((a - b) ** 2 + CALDERA_SOFT_M**2)) / 2

    h, ice = h.copy(), np.zeros(h.shape, bool)
    for f in spec.get("features", []):
        if f["kind"] != "volcano" or "caldera_floor_m" not in f:
            continue
        cx, cy, top = summit_plateau(h, x, y, f)
        r, t = np.hypot(x - cx, y - cy), np.arctan2(y - cy, x - cx)
        near = r <= f["radius_m"]
        r, t = r[near], t[near]
        # An irregular oval rim, higher on one side, so it reads as no drawn ring.
        rr = f["caldera_diameter_m"] / 2 * (1 + 0.08 * np.cos(2 * (t - 0.6)) + 0.04 * np.cos(3 * t + 2.2)
                                            + 0.03 * np.cos(5 * t + 0.7))
        g = 0.7 * (1 - np.cos(t - 2.0)) / 2 + 0.3 * (1 - np.cos(3 * t + 1.0)) / 2
        rim = top - CALDERA_RIM_DROP_M * (1 + 2 * g) / 3
        floor = float(f["caldera_floor_m"]) + CALDERA_FLOOR_RISE_M * np.minimum(r / rr, 1) ** 2
        a = CALDERA_WALL_M / 3  # rounds the crest, so the outer flank starts level
        outer = rim - CALDERA_FLANK * (np.hypot(np.maximum(r - rr, 0), a) - a)
        smooth = ndimage.gaussian_filter(h, CALDERA_DETAIL_M / CELL_M)
        # Half the detail on the crest, so gullies notch the rim without opening the hollow.
        detail = (h - smooth)[near] * (1 - 0.5 * np.exp(-(((r - rr) / CALDERA_CREST_M) ** 2)))
        # 0 on the floor, rising over the inner wall to 1 at the crest and beyond.
        s = np.clip((r - rr + CALDERA_WALL_M) / CALDERA_WALL_M, 0, 1)
        s = s * s * (3 - 2 * s)
        h[near] = floor + (smax(smooth[near], outer) + detail - floor) * s
        ice[near] |= r < rr - CALDERA_WALL_M / 2
    return h, ice


SEAM_RATIO = 1.3  # slopes along a border this much steeper than inside both regions show a seam
AREA_TOLERANCE = 0.25
VOLCANO_MARGIN_M = 400.0
SITE_SLOPE_DEG = 5.0  # each region keeps a patch this flat for a settlement
SITE_KM2 = 0.5
CLAMP_NEAR_M = 150.0
CLAMP_CELLS = 20
CALDERA_TOLERANCE_M = 150.0  # caldera floor within this of its target, and at least this far below the rim
GLACIER_CLASS = 10  # terrain legend v3


def report(spec, regions, h, sds, land_w, covered, ice, dips) -> tuple[str, int]:
    lines, warnings = [], 0

    def check(ok: bool, text: str):
        nonlocal warnings
        warnings += not ok
        lines.append(("  ok   " if ok else "  WARN ") + text)

    from scipy import ndimage

    land = h >= 0
    lines.append(f"Collage at {CELL_M:.0f} m: land {land.mean():.1%} of the map")
    c = (np.arange(h.shape[0]) + 0.5) * CELL_M
    e = np.minimum(c, h.shape[0] * CELL_M - c)
    to_edge = np.minimum(e[:, None], e[None, :])
    margin = float(spec.get("coast_margin_m", 4000.0))
    edge_m = float(to_edge[land].min()) if land.any() else 0.0
    check(edge_m >= margin, f"land comes within {edge_m / 1000:.1f} km of the map edge")
    # Land pressed against the edge clamp makes a straight coast.
    clamped = int((land & ~ndimage.binary_erosion(land) & (to_edge < margin + CLAMP_NEAR_M)).sum())
    check(clamped <= CLAMP_CELLS, f"{clamped} coast cells within {CLAMP_NEAR_M / 1000:.2f} km of the coast margin")
    total = np.maximum(np.sum(land_w, axis=0), 1e-6)
    gy, gx = np.gradient(h, CELL_M)
    slope = np.hypot(gx, gy)
    flat = land & ~ice & (slope < np.tan(np.radians(SITE_SLOPE_DEG)))  # no settlements on caldera ice
    for i, r in enumerate(regions):
        inside = sds[i] > 0
        bare = (inside & ~covered[i]).sum() / max(int(inside.sum()), 1)
        check(bare <= 0.01, f"{r.id:<15} pieces cover {1 - bare:.1%} of the outline")
        labels, count = ndimage.label(flat & inside)
        site_km2 = float(np.bincount(labels.ravel())[1:].max()) * CELL_M**2 / 1e6 if count else 0.0
        check(site_km2 >= SITE_KM2, f"{r.id:<15} largest patch under {SITE_SLOPE_DEG:.0f}° is {site_km2:.2f} km²")
        want = spec["regions"][i].get("area_km2")
        got = float((inside & land).sum()) * CELL_M**2 / 1e6
        if want:
            check(abs(got - want) <= AREA_TOLERANCE * want, f"{r.id:<15} land {got:.0f} km² (target {want:.0f})")
        else:
            lines.append(f"  info {r.id:<15} land {got:.0f} km²")
    for i in range(len(regions)):
        for j in range(i + 1, len(regions)):
            a, b = land_w[i] / total, land_w[j] / total
            seam = (a > 0.25) & (b > 0.25) & land
            if seam.sum() < 50:
                continue
            core = ((a > 0.9) | (b > 0.9)) & land
            ratio = float(np.percentile(slope[seam], 99) / max(np.percentile(slope[core], 99), 1e-3))
            check(ratio <= SEAM_RATIO, f"seam {regions[i].id} / {regions[j].id}: steepest slopes {ratio:.2f}x the regions'")
    for f in spec.get("features", []):
        if f["kind"] != "volcano":
            continue
        d = np.hypot(*np.meshgrid((np.arange(h.shape[0]) + 0.5) * CELL_M - f["at"][0],
                                  (np.arange(h.shape[0]) + 0.5) * CELL_M - f["at"][1]))
        top, rest = float(h[d <= f["radius_m"]].max()), float(h[d > 1.5 * f["radius_m"]].max())
        check(top - rest >= VOLCANO_MARGIN_M, f"volcano {f['id']} {top:.0f} m, next highest {rest:.0f} m")
        if "caldera_floor_m" in f:
            c = description.caldera(h, CELL_M, f)
            want = float(f["caldera_floor_m"])
            if not c:
                check(False, f"caldera {f['id']}: no closed hollow within {f['caldera_diameter_m'] / 2000:.2f} km")
                continue
            ok = abs(c["floor_m"] - want) <= CALDERA_TOLERANCE_M and c["rim_m"] - c["floor_m"] >= CALDERA_TOLERANCE_M
            check(ok, f"caldera {f['id']}: floor {c['floor_m']:.0f} m (target {want:.0f}), lowest rim "
                      f"{c['rim_m']:.0f} m, closed {c['rim_m'] - c['floor_m']:.0f} m deep, "
                      f"{c['diameter_m'] / 1000:.1f} km across below the rim")
    for pid, depth, ok in dips:
        check(ok, f"pass {pid:<17} in its band" + (f" after a {depth:.0f} m dip" if depth else ""))
    lines.append(f"{warnings} warning(s)")
    return "\n".join(lines) + "\n", warnings


def sheet(out: Path, regions: list[Region], srcs: dict[str, Source], h32: np.ndarray, m_per_px: float = 64.0) -> Path:
    """Each piece's source crop beside the world around where it was placed, at one scale."""
    from PIL import Image, ImageDraw

    from .cascade_eval import shade

    rows = []
    for r in regions:
        for p in r.pieces:
            src = srcs[p.source]
            n_src, s = src.height.shape[0], src.spacing_m
            cx, cy = n_src / 2 + p.offset_m[0] / s, n_src / 2 + p.offset_m[1] / s
            hw, hh = int(p.size_m[0] / s / 2), int(p.size_m[1] / s / 2)
            step = max(1, round(m_per_px / s))
            sub = src.height[int(cy) - hh : int(cy) + hh : step, int(cx) - hw : int(cx) + hw : step]
            left = Image.fromarray(shade(sub, s * step, sub < 0))
            half = int(np.hypot(*p.size_m) * p.scale / 2 / CELL_M)
            i, j = int(p.at[0] / CELL_M), int(p.at[1] / CELL_M)
            k = max(1, round(m_per_px / CELL_M))
            win = h32[max(0, j - half) : j + half : k, max(0, i - half) : i + half : k]
            right = Image.fromarray(shade(win, CELL_M * k, win < 0))
            rows.append((f"{r.name}: {p.source}, {p.rotate_deg:.0f}° x{p.scale:g}", left, right))
    pad, font = 12, layout._font(18)
    width = max(l.width + rt.width for _, l, rt in rows) + 3 * pad
    height = sum(max(l.height, rt.height) + 36 for _, l, rt in rows) + pad
    img = Image.new("RGB", (width, height), (30, 30, 30))
    d = ImageDraw.Draw(img)
    y = pad
    for label, left, right in rows:
        layout._label(d, (pad, y), label, font)
        img.paste(left, (pad, y + 26))
        img.paste(right, (2 * pad + left.width, y + 26))
        y += max(left.height, right.height) + 36
    img.save(out / "pieces.png")
    return out / "pieces.png"


def partial_dir(out: Path) -> Path:
    return out.with_name(out.name + ".partial")


def build(layout_path: Path, release: Path, out: Path) -> int:
    """Builds into `<out>.partial`, renamed to `out` when complete and removed on an error."""
    import shutil

    t0 = time.time()
    text = layout_path.read_text()
    spec = tomllib.loads(text)
    regions = parse(spec)
    srcs = sources(release, {p.source for r in regions for p in r.pieces})
    check_crops(regions, srcs)
    work = partial_dir(out)
    work.mkdir(parents=True)
    try:
        h32 = _build(spec, text, regions, srcs, work)
        (work / "collage.json").write_text(json.dumps({
            "kind": "world_collage", "schema_version": 1, "layout": str(layout_path.resolve()),
            "layout_sha256": hashlib.sha256(text.encode()).hexdigest(), "release": str(release.resolve()),
            "sources": sorted(srcs), "cell_m": CELL_M, "time_s": round(time.time() - t0, 1)}, indent=1))
        work.rename(out)
    except BaseException:
        shutil.rmtree(work, ignore_errors=True)
        raise
    print(f"wrote {out} in {time.time() - t0:.0f} s; land {(h32 >= 0).mean():.1%}", flush=True)
    return 0


def _build(spec: dict, text: str, regions: list[Region], srcs: dict[str, Source], out: Path) -> np.ndarray:
    n = int(spec["extent_m"] / CELL_M)
    x, y = grid(n)
    sds, surfaces, classes, covered = [], [], [], []
    piece_noise = blue_noise(n, seed=1)  # independent of the region dither below
    dx, dy = warp_field(n)  # one warp for every outline, so borders stay shared
    for r in regions:
        sds.append(warp(description.polygon_sd(r.outline, n, CELL_M), dx, dy))
        h, c, cov = region_surface(r, srcs, x, y, piece_noise)
        surfaces.append(h)
        classes.append(c)
        covered.append(cov)
    isd = island_sd(sds)
    land_w = [weight(sd, r.border_m) for sd, r in zip(sds, regions)]
    sea = ocean(isd, spec)
    h32 = sink_outside(blend(surfaces + [sea], land_w + [weight(-isd, OCEAN_BORDER_M)]), isd, sea)
    h32 = np.minimum(h32, edge_limit(n, spec))
    h32, dips = dip_passes(h32, spec)
    h32, ice = calderas(h32, spec, x, y)
    total = np.maximum(np.sum(land_w, axis=0), 1e-6)
    forest32 = np.where(h32 >= 0, sum(w * r.forest for w, r in zip(land_w, regions)) / total, 0).astype(np.float32)
    forest32[ice] = 0
    cls32 = dither(classes, land_w, blue_noise(n))
    cls32[ice] = GLACIER_CLASS
    write_l0(out, text, spec, h32, cls32, forest32)
    cls32.tofile(out / "class32.u8")
    np.savez_compressed(out / "arrays.npz", l1=cascade.block_mean(h32, int(L0_CELL_M / CELL_M)), l2=h32)
    l0 = layout.load(out)
    layout.overview(l0, out / "overview.png")
    text_r, warnings = report(spec, regions, h32, sds, land_w, covered, ice, dips)
    (out / "report.txt").write_text(text_r)
    print(text_r, end="")
    text_b, bands, _ = layout.reach(h32, CELL_M, spec)
    (out / "reach-l2.txt").write_text(text_b)
    layout.reach_png(h32, CELL_M, bands, spec, out / "reach-l2.png")
    description.write(out, description.measure(spec, h32, CELL_M, "collage", [sd > 0 for sd in sds]))
    sheet(out, regions, srcs, h32)
    return h32


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("layout", type=Path, help="layout v2 TOML")
    p.add_argument("--release", type=Path, required=True, help="context release.json")
    p.add_argument("--out", type=Path, required=True, help="new output directory")
    args = p.parse_args(argv)
    for d in (args.out, partial_dir(args.out)):
        if d.exists():
            print(f"error: {d} exists" + (" (left by a killed build; remove it)" if d != args.out else ""),
                  file=sys.stderr)
            return 2
    return build(args.layout, args.release, args.out)


if __name__ == "__main__":
    sys.exit(main())
