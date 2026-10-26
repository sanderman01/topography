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
    out = classes[-1].copy()
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


SEAM_RATIO = 1.3  # slopes along a border this much steeper than inside both regions show a seam
AREA_TOLERANCE = 0.25
VOLCANO_MARGIN_M = 400.0
SITE_SLOPE_DEG = 5.0  # each region keeps a patch this flat for a settlement
SITE_KM2 = 0.5


def report(spec, regions, h, sds, land_w, covered, dips) -> tuple[str, int]:
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
    edge_m = float(np.minimum(e[:, None], e[None, :])[land].min()) if land.any() else 0.0
    check(edge_m >= float(spec.get("coast_margin_m", 4000.0)), f"land comes within {edge_m / 1000:.1f} km of the map edge")
    total = np.maximum(np.sum(land_w, axis=0), 1e-6)
    gy, gx = np.gradient(h, CELL_M)
    slope = np.hypot(gx, gy)
    flat = land & (slope < np.tan(np.radians(SITE_SLOPE_DEG)))
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
    h32, dips = dip_passes(h32, spec)
    total = np.maximum(np.sum(land_w, axis=0), 1e-6)
    forest32 = (sum(w * r.forest for w, r in zip(land_w, regions)) / total).astype(np.float32)
    cls32 = dither(classes, land_w, blue_noise(n))
    write_l0(out, text, spec, h32, cls32, forest32)
    cls32.tofile(out / "class32.u8")
    np.savez_compressed(out / "arrays.npz", l1=cascade.block_mean(h32, int(L0_CELL_M / CELL_M)), l2=h32)
    l0 = layout.load(out)
    layout.overview(l0, out / "overview.png")
    text_r, warnings = report(spec, regions, h32, sds, land_w, covered, dips)
    (out / "report.txt").write_text(text_r)
    print(text_r, end="")
    text_b, bands, _ = layout.reach(h32, CELL_M, spec)
    (out / "reach-l2.txt").write_text(text_b)
    layout.reach_png(h32, CELL_M, bands, spec, out / "reach-l2.png")
    description.write(out, description.measure(spec, h32, CELL_M, "collage"))
    sheet(out, regions, srcs, h32)
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
