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
ISLAND_CLOSE_M = 4000.0  # gaps between region outlines narrower than this stay land
SHELF_M = -20.0  # sea floor at the island's outline


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


def parse(spec: dict) -> list[Region]:
    if spec.get("schema_version") != 2 or spec.get("kind") != "world_layout":
        raise ValueError('not a layout v2 (schema_version = 2, kind = "world_layout")')
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


def fill_nearest(a: np.ndarray) -> np.ndarray:
    from scipy import ndimage

    bad = ~np.isfinite(a)
    if not bad.any():
        return a
    if bad.all():
        return np.zeros_like(a)
    idx = ndimage.distance_transform_edt(bad, return_distances=False, return_indices=True)
    return a[tuple(idx)]


def region_surface(r: Region, srcs: dict[str, Source], x: np.ndarray, y: np.ndarray):
    """Heights (filled past the pieces with the nearest piece value), classes and covered cells."""
    num = np.zeros(x.shape, np.float32)
    den = np.zeros(x.shape, np.float32)
    best = np.zeros(x.shape, np.float32)
    cls = np.full(x.shape, r.class_id, np.uint8)
    for p in r.pieces:
        src = srcs[p.source]
        h, c, w = place(p, src, x, y)
        num += w * h
        den += w
        # The source's own class becomes the region's (or the piece's); glacier, alpine and plain cells stay.
        c = np.where(c == src.class_id, p.class_id or r.class_id, c).astype(np.uint8)
        better = w > best
        cls[better] = c[better]
        best = np.maximum(best, w)
    covered = den > 1e-3
    return fill_nearest(np.where(covered, num / np.maximum(den, 1e-6), np.nan)).astype(np.float32), cls, covered


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
    return (SHELF_M + (edge - SHELF_M) * np.sqrt(out_m / max(float(out_m.max()), 1.0))).astype(np.float32)


def edge_limit(n: int, spec: dict) -> np.ndarray:
    """Highest ground per cell: the seabed at the map edge rising to the shelf at the coast margin."""
    margin = float(spec.get("coast_margin_m", 4000.0))
    edge = float(spec.get("seabed_edge_m", -200.0))
    c = (np.arange(n) + 0.5) * CELL_M
    e = np.minimum(c, n * CELL_M - c)
    d = np.minimum(e[:, None], e[None, :])
    return np.where(d < margin, edge + (SHELF_M - edge) * d / margin, SHELF_M + (d - margin) * 2.0).astype(np.float32)


def _down(a: np.ndarray) -> np.ndarray:
    from scipy import ndimage

    return ndimage.zoom(ndimage.gaussian_filter(a, 1.0), 0.5, order=1)


def _up(a: np.ndarray, shape) -> np.ndarray:
    from scipy import ndimage

    return ndimage.zoom(a, (shape[0] / a.shape[0], shape[1] / a.shape[1]), order=1)


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


def build(layout_path: Path, release: Path, out: Path) -> int:
    t0 = time.time()
    text = layout_path.read_text()
    spec = tomllib.loads(text)
    regions = parse(spec)
    n = int(spec["extent_m"] / CELL_M)
    srcs = sources(release, {p.source for r in regions for p in r.pieces})
    check_crops(regions, srcs)
    out.mkdir(parents=True)
    x, y = grid(n)
    sds, surfaces, classes, covered = [], [], [], []
    for r in regions:
        sds.append(description.polygon_sd(r.outline, n, CELL_M))
        h, c, cov = region_surface(r, srcs, x, y)
        surfaces.append(h)
        classes.append(c)
        covered.append(cov)
    isd = island_sd(sds)
    land_w = [weight(sd, r.border_m) for sd, r in zip(sds, regions)]
    h32 = blend(surfaces + [ocean(isd, spec)], land_w + [weight(-isd, OCEAN_BORDER_M)])
    h32 = np.minimum(h32, edge_limit(n, spec))
    total = np.maximum(np.sum(land_w, axis=0), 1e-6)
    forest32 = (sum(w * r.forest for w, r in zip(land_w, regions)) / total).astype(np.float32)
    cls32 = classes[0]  # replaced by the dither in Task 4
    write_l0(out, text, spec, h32, cls32, forest32)
    cls32.tofile(out / "class32.u8")
    np.savez_compressed(out / "arrays.npz", l1=cascade.block_mean(h32, int(L0_CELL_M / CELL_M)), l2=h32)
    l0 = layout.load(out)
    layout.overview(l0, out / "overview.png")
    (out / "collage.json").write_text(json.dumps({
        "kind": "world_collage", "schema_version": 1, "layout": str(layout_path.resolve()),
        "layout_sha256": hashlib.sha256(text.encode()).hexdigest(), "release": str(release.resolve()),
        "sources": sorted(srcs), "cell_m": CELL_M, "time_s": round(time.time() - t0, 1)}, indent=1))
    print(f"wrote {out} in {time.time() - t0:.0f} s; land {(h32 >= 0).mean():.1%}", flush=True)
    return 0


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("layout", type=Path, help="layout v2 TOML")
    p.add_argument("--release", type=Path, required=True, help="context release.json")
    p.add_argument("--out", type=Path, required=True, help="new output directory")
    args = p.parse_args(argv)
    if args.out.exists():
        print("error: output directory exists", file=sys.stderr)
        return 2
    return build(args.layout, args.release, args.out)


if __name__ == "__main__":
    sys.exit(main())
