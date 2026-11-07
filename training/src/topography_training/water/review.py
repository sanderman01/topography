"""Review figures for hydrology v3: hillshade crops (raw | result with water tinted and lake
exits ringed | cut depth), a tinted overview with the crop boxes, lake-exit contact sheets and
the coarse guide, a level's water against the next coarser level's, with a figure per mismatch.

Works on a v3 world, a driver window run (grid/ and out/) or a bare water output directory
(with --raw). Crops outside the processed grid are skipped. In the coarse guide a large, deep
coarse lake away from inner edges more than 5 m off its coarse level, gone or doubled is a hard
failure (ruling B15); the drivers withhold the result on one.

python -m topography_training.water.review <world | run dir | out dir> [--dest D] [--raw F] [--coarse coarse out dir]
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np
from scipy import ndimage

# The owner's spots (spec "Review"), east and north in metres.
SPOTS = ((25645, 24400), (25307, 24585), (20071, 30501), (27169, 44546), (43966, 21633), (49249, 16381),
         (47962, 36851), (48148, 36810), (48810, 19452), (51610, 20478), (42581, 56193))
CROP_M = 512
CROP_PX = 512
GAP_PX = 8
BAR_PX = 40
LEVEL_GROSS_M = 5.0
DOUBLED_SHARE = 0.25
RIVER_NEAR_M = 100.0


@dataclass(frozen=True)
class Area:
    """A carried review spot: one point (centred), a box (two corners) or a line (two ends)."""

    name: str
    points: tuple[tuple[int, int], ...]
    mark: str = "none"  # "box", "ends" or "none"
    size_m: int = CROP_M


# The controller's Task 17 list (carry-later.md, progress.md); E/N in metres. The straight-share
# runs (two longest per window) and thalweg_beside cells come from the Task 13 Rust probe
# windows, whose origin sits about 2 cells from the driver's windows at the same centre.
AREAS = (
    Area("parallel-river-E49236-N16093", ((49236, 16093), (49426, 16135)), "box"),
    Area("zigzag-gully-E26013-N24172", ((26013, 24172), (26078, 24272)), "box"),
    Area("south-lake-outflow-E25300-N24080", ((25300, 24080),)),
    Area("straight-trough-E25621-N24419", ((25621, 24419), (25937, 24077)), "ends"),
    Area("straight-trough-E49531-N16864", ((49531, 16864), (49582, 16759)), "ends"),
    Area("doubled-river-E49551-N16630", ((49551, 16630), (49663, 16538)), "ends"),
    Area("striped-artefact-E24990-N25040", ((24990, 25040), (25110, 25190)), "box"),
    Area("straight-share-E25307-a", ((25053, 25235), (25106, 25182)), "ends", 256),
    Area("straight-share-E25307-b", ((25841, 24175), (25894, 24121)), "ends", 256),
    Area("straight-share-E20071-a", ((20402, 30869), (20304, 30814)), "ends", 256),
    Area("straight-share-E20071-b", ((20263, 30269), (20182, 30224)), "ends", 256),
    Area("straight-share-E43966-a", ((43832, 21353), (43805, 21448)), "ends", 256),
    Area("straight-share-E43966-b", ((43579, 21857), (43621, 21777)), "ends", 256),
    Area("thalweg-beside-E49909-N16285", ((49909, 16285), (49913, 16288)), "box", 128),
    Area("thalweg-beside-E44829-N21770", ((44829, 21770), (44832, 21772)), "box", 128),
)
# The dam spot: flow arrows and lake outlet rings (carry-later.md).
DAM = (25645, 24400)
DAM_M, DAM_PX = 256, 1024
# Dry cells get a flow arrow from this discharge on (wet and thalweg cells always).
ARROW_KM2 = 2e-5
STRAIGHT_RUNS, RUN_M = 20, 256
FAILURE_M, FAILURE_CROPS = 128, 10
# Exit crops span at least EXIT_M and EXIT_CELLS: 128 m at 1 m and 4 m, 1,024 m at 32 m.
EXIT_M, EXIT_CELLS, EXIT_PX, SHEET_COLS, SHEET_ROWS = 128, 32, 256, 6, 8


def exit_cells(spacing_m: float) -> int:
    """Cells along a lake-exit crop's side at this spacing."""
    return max(EXIT_CELLS, int(round(EXIT_M / spacing_m)))
GUIDE_MIN_M, GUIDE_MAX_M, GUIDE_PX = 256, 4096, 768
OVERVIEW_PX = 2048
# The cut panel saturates at this lowering or raising; a square-root ramp shows 5-20 cm cuts.
CUT_M = 0.5

LAKE, RIVER = (0.45, 0.68, 1.0), (0.0, 0.15, 0.7)
CUT_LOW, CUT_HIGH = (0.85, 0.1, 0.1), (0.1, 0.3, 0.95)
MARK, OPEN_EXIT, CLOSED = (255, 220, 0), (40, 220, 60), (255, 40, 40)
THALWEG, WET, DRY = (230, 30, 30), (255, 220, 0), (255, 255, 255)
CARRY, RUNS, DAM_BOX = (255, 140, 0), (230, 0, 230), (0, 220, 220)
# D8 receiver codes in flow.u8's low 4 bits: (dx, dy).
D8 = {1: (0, -1), 2: (0, 1), 3: (-1, 0), 4: (1, 0), 5: (-1, -1), 6: (1, -1), 7: (-1, 1), 8: (1, 1)}


def _c255(c: tuple) -> tuple[int, int, int]:
    return tuple(int(v * 255) for v in c)


WATER_LEGEND = [("swatch", _c255(LAKE), "lake/sea"), ("swatch", _c255(RIVER), "river"),
                ("ring", OPEN_EXIT, "open lake exit"), ("ring", CLOSED, "closed lake low point")]
CROP_LEGEND = WATER_LEGEND + [("box", MARK, "review area"),
                              ("ramp", None, f"cut: red lowered, blue raised, sqrt ramp to {CUT_M:g} m")]
DAM_LEGEND = CROP_LEGEND[:4] + [("line", THALWEG, "thalweg flow"), ("line", WET, "wet flow"),
                                ("line", DRY, f"dry flow, Q >= {ARROW_KM2:g} km2")]
OVERVIEW_LEGEND = WATER_LEGEND[:2] + [("box", MARK, "owner spot"), ("box", CARRY, "carried area"),
                                      ("box", DAM_BOX, "dam view"), ("box", RUNS, "straight run"),
                                      ("box", CLOSED, "failing check")]


def _json(path: Path) -> dict:
    return json.loads(path.read_text())


def _nw(grid: dict) -> tuple[float, float]:
    """East and north of the grid's north-west corner (driver windows add offset_m to the
    world origin; worlds and probe outputs give their own origin)."""
    w, h = grid["size"]
    s = float(grid.get("spacing_m", 1.0))
    origin = grid.get("origin", [0.0, h * s])
    off = grid.get("offset_m", [0.0, 0.0])
    return float(origin[0] + off[0]), float(origin[1] - off[1])


def _kind_table(bodies: list) -> np.ndarray:
    """Body id -> 0 dry, 1 lake or sea, 2 river."""
    table = np.zeros(max((b["id"] for b in bodies), default=0) + 1, np.uint8)
    for b in bodies:
        table[b["id"]] = {"lake": 1, "sea": 1, "river": 2}.get(b["kind"], 0)
    return table


def _kinds(table: np.ndarray, body: np.ndarray) -> np.ndarray:
    k = table[np.minimum(body, len(table) - 1)]
    k[body >= len(table)] = 0
    return k


def _is_open(b: dict) -> bool:
    """A lake that drains: by its endorheic flag, else by having an exit cell."""
    return not b["endorheic"] if "endorheic" in b else b.get("exit_cell") is not None


@dataclass
class Layers:
    """A water output (`out`) and the raw surface it was computed from."""

    out: Path
    raw: Path
    w: int
    h: int
    spacing: float
    nw: tuple[float, float]
    bodies: list
    report: dict
    table: np.ndarray = field(init=False)
    by_id: dict = field(init=False)

    def __post_init__(self):
        self.table = _kind_table(self.bodies)
        self.by_id = {b["id"]: b for b in self.bodies}

    def cell(self, e: float, n: float) -> tuple[int, int]:
        return int(math.floor((e - self.nw[0]) / self.spacing)), int(math.floor((self.nw[1] - n) / self.spacing))

    def en(self, c: float, r: float) -> tuple[int, int]:
        return int(round(self.nw[0] + c * self.spacing)), int(round(self.nw[1] - r * self.spacing))


def open_layers(path: Path, raw: Path | None = None) -> Layers:
    """A v3 world (hydrology/1m), a driver window run (out/, grid/) or a water output dir."""
    if (path / "hydrology" / "1m" / "report.json").exists():
        out, default = path / "hydrology" / "1m", path / "elevation-raw.f32"
    elif (path / "out" / "report.json").exists():
        out, default = path / "out", path / "grid" / "elevation.f32"
    elif (path / "report.json").exists():
        out, default = path, path.parent / "grid" / "elevation.f32"
    else:
        raise FileNotFoundError(f"{path} is not a v3 world, window run or water output")
    raw = raw or default
    g = _json(out / "grid.json")
    w, h = g["size"]
    if not raw.exists() or raw.stat().st_size != w * h * 4:
        raise FileNotFoundError(f"no {w}x{h} raw surface at {raw} (pass --raw)")
    return Layers(out, raw, w, h, float(g.get("spacing_m", 1.0)), _nw(g),
                  _json(out / "bodies.json")["bodies"], _json(out / "report.json"))


def default_dest(path: Path) -> Path:
    """Review figures go next to the input, never inside the Rust output: <world>/hydrology/review,
    <run>/review, or <out>-review beside a bare output directory."""
    if (path / "hydrology" / "1m" / "report.json").exists():
        return path / "hydrology" / "review"
    if (path / "out" / "report.json").exists():
        return path / "review"
    return path.parent / f"{path.name}-review"


# Reading and rendering.

Win = tuple[int, int, int, int]  # r0, c0, h, w


def _rows(path: Path, dtype, width: int, win: Win, k: int = 1) -> np.ndarray:
    """Every k-th row and column of window `win` of a `width`-wide raster, read row by row with
    no lasting memory map (a long-lived map of the 17 GB body raster kept every touched page
    in the driver's RSS)."""
    r0, c0, h, w = win
    dtype = np.dtype(dtype)
    with open(path, "rb") as f:
        if k == 1 and c0 == 0 and w == width:
            return np.fromfile(f, dtype, count=h * w, offset=r0 * width * dtype.itemsize).reshape(h, w)
        out = np.empty((len(range(0, h, k)), len(range(0, w, k))), dtype)
        for i, r in enumerate(range(r0, r0 + h, k)):
            f.seek((r * width + c0) * dtype.itemsize)
            out[i] = np.frombuffer(f.read(w * dtype.itemsize), dtype)[::k]
        return out


def _read(L: Layers, name: str | Path, dtype, win: Win, k: int = 1) -> np.ndarray:
    return _rows(name if isinstance(name, Path) else L.out / name, dtype, L.w, win, k)


def _reduce(kinds: np.ndarray, k: int) -> np.ndarray:
    """k x k blocks: river if any cell is, else lake if any cell is."""
    h, w = -(-kinds.shape[0] // k) * k, -(-kinds.shape[1] // k) * k
    pad = np.zeros((h, w), np.uint8)
    pad[: kinds.shape[0], : kinds.shape[1]] = kinds
    b = pad.reshape(h // k, k, w // k, k)
    return np.where((b == 2).any(axis=(1, 3)), 2, np.where((b == 1).any(axis=(1, 3)), 1, 0)).astype(np.uint8)


def _kinds_at(L: Layers, win: Win, k: int = 1) -> np.ndarray:
    """Water kinds of the window at stride k, block-reduced so thin rivers survive (streamed)."""
    if k == 1:
        return _kinds(L.table, _read(L, "water-body.u32", "<u4", win))
    r0, c0, h, w = win
    step = k * max(1, 512 // k)
    return np.concatenate([_reduce(_kinds(L.table, _read(L, "water-body.u32", "<u4",
                                                         (y, c0, min(y + step, r0 + h) - y, w))), k)
                           for y in range(r0, r0 + h, step)])


def _illum(z: np.ndarray, cell_m: float) -> np.ndarray:
    """Hillshade (north-west sun at 45 degrees) in 0..1; NaN where the elevation is NaN."""
    gy, gx = np.gradient(np.asarray(z, np.float64), cell_m)
    az, alt = np.radians(315), np.radians(45)
    slope = np.arctan(np.hypot(gx, gy))
    aspect = np.arctan2(-gx, gy)
    return np.clip(np.sin(alt) * np.cos(slope) + np.cos(alt) * np.sin(slope) * np.cos(az - aspect), 0, 1)


def _limits(i: np.ndarray, dry: np.ndarray | None = None) -> tuple[float, float]:
    """The contrast stretch: 1st to 99th percentile (of the dry cells when there are any)."""
    v = i[dry] if dry is not None and dry.any() else i
    v = v[np.isfinite(v)]
    if v.size == 0:
        return 0.0, 1.0
    lo, hi = np.percentile(v, [1, 99])
    return (float(lo), float(hi)) if hi - lo > 1e-6 else (0.0, 1.0)


def _gray(i: np.ndarray, lim: tuple[float, float]) -> np.ndarray:
    return np.nan_to_num(np.clip((i - lim[0]) / (lim[1] - lim[0]), 0, 1), nan=0.5)


def _water(g: np.ndarray, kinds: np.ndarray, widen: bool) -> np.ndarray:
    """Gray with lakes light blue and rivers dark blue (widened to 3 px when a cell is 1 px)."""
    rgb = np.repeat(g[..., None], 3, axis=2)
    lake = kinds == 1
    river = ndimage.binary_dilation(kinds == 2) if widen else kinds == 2
    rgb[lake] = 0.35 * rgb[lake] + 0.65 * np.array(LAKE) * (0.6 + 0.4 * g[lake, None])
    rgb[river] = 0.2 * rgb[river] + 0.8 * np.array(RIVER)
    return rgb


def _cut(d: np.ndarray, g: np.ndarray) -> np.ndarray:
    """Result minus raw over a light hillshade: red lowered, blue raised."""
    base = np.repeat((0.7 + 0.3 * g)[..., None], 3, axis=2)
    t = np.nan_to_num(np.clip(d / CUT_M, -1, 1))
    a = np.sqrt(np.abs(t))[..., None]
    col = np.where((t < 0)[..., None], np.array(CUT_LOW), np.array(CUT_HIGH))
    return base * (1 - a) + col * a


def _u8(rgb: np.ndarray, scale: int) -> np.ndarray:
    a = (np.clip(rgb, 0, 1) * 255).astype(np.uint8)
    return a if scale == 1 else np.repeat(np.repeat(a, scale, 0), scale, 1)


def _compose(panels: list[np.ndarray], title: str, px_per_m: float, legend: list):
    """Panels side by side over a caption bar: title, scale bar, north arrow and legend."""
    from PIL import Image, ImageDraw

    h = panels[0].shape[0]
    xs, x = [], 0
    for p in panels:
        xs.append(x)
        x += p.shape[1] + GAP_PX
    img = Image.new("RGB", (max(x - GAP_PX, 1100), h + BAR_PX), "white")
    for p, x0 in zip(panels, xs):
        img.paste(Image.fromarray(p), (x0, 0))
    draw = ImageDraw.Draw(img)
    draw.text((4, h + 3), title, fill=(0, 0, 0))
    y = h + 26
    want = 120 / px_per_m
    nice = max(m * 10**p for p in range(-1, 6) for m in (1, 2, 5) if m * 10**p <= want) if want >= 0.1 else want
    length = nice * px_per_m
    draw.line([(4, y), (4 + length, y)], fill=(0, 0, 0), width=2)
    for xe in (4, 4 + length):
        draw.line([(xe, y - 4), (xe, y + 4)], fill=(0, 0, 0))
    draw.text((8 + length, y - 6), f"{nice:g} m", fill=(0, 0, 0))
    nx = 180
    draw.polygon([(nx, y - 10), (nx - 5, y + 4), (nx + 5, y + 4)], fill=(0, 0, 0))
    draw.text((nx + 8, y - 6), "N", fill=(0, 0, 0))
    x = 210
    for kind, col, label in legend:
        if kind == "swatch":
            draw.rectangle([x, y - 5, x + 12, y + 5], fill=col, outline=(0, 0, 0))
        elif kind == "ring":
            draw.ellipse([x, y - 6, x + 12, y + 6], outline=col, width=2)
        elif kind == "box":
            draw.rectangle([x, y - 5, x + 12, y + 5], outline=col, width=2)
        elif kind == "line":
            draw.line([(x, y), (x + 12, y)], fill=(0, 0, 0), width=4)
            draw.line([(x, y), (x + 12, y)], fill=col, width=2)
        elif kind == "ramp":
            for i in range(25):
                t = i / 12 - 1
                c = np.array(CUT_LOW if t < 0 else CUT_HIGH) * abs(t) + (1 - abs(t))
                draw.line([(x + i, y - 5), (x + i, y + 5)], fill=_c255(tuple(c)))
            x += 12
        draw.text((x + 16, y - 6), label, fill=(0, 0, 0))
        x += 16 + int(draw.textlength(label)) + 14
    return img, xs


def _window(L: Layers, c: int, r: int, cells: int) -> Win | None:
    """A cells² window around (c, r) kept inside the grid; None when (c, r) is outside it."""
    if not (0 <= c < L.w and 0 <= r < L.h):
        return None
    ch, cw = min(cells, L.h), min(cells, L.w)
    return min(max(r - ch // 2, 0), L.h - ch), min(max(c - cw // 2, 0), L.w - cw), ch, cw


def _rings(L: Layers, win: Win) -> list[tuple[int, int, tuple]]:
    """Lake rings in the window: an open lake's exit cell, a closed lake's lowest cell."""
    r0, c0, h, w = win
    marks = [(b["exit_cell"][0], b["exit_cell"][1], OPEN_EXIT) for b in L.bodies
             if b["kind"] == "lake" and _is_open(b) and b.get("exit_cell") is not None
             and r0 <= b["exit_cell"][1] < r0 + h and c0 <= b["exit_cell"][0] < c0 + w]
    body = _read(L, "water-body.u32", "<u4", win)
    ground = None
    for i in np.unique(body):
        b = L.by_id.get(int(i))
        if b is None or b["kind"] != "lake" or _is_open(b):
            continue
        if ground is None:
            ground = _read(L, "elevation.f32", "<f4", win)
        inside = body == i
        r, c = np.unravel_index(np.argmin(np.where(inside, ground, np.inf)), inside.shape)
        marks.append((c0 + int(c), r0 + int(r), CLOSED))
    return marks


def _arrows(draw, L: Layers, win: Win, scale: int, x0: int) -> None:
    """D8 flow from cell centre to receiver centre, outlined; a head on about every 7th cell."""
    r0, c0, h, w = win
    flow = _read(L, "flow.u8", np.uint8, win)
    kinds = _kinds(L.table, _read(L, "water-body.u32", "<u4", win))
    q = _read(L, "discharge.f32", "<f4", win)
    shown = (flow & 15 > 0) & ((kinds > 0) | (flow & 16 > 0) | (q >= ARROW_KM2))
    cells = list(zip(*np.nonzero(shown)))
    segs = []
    for r, c in cells:
        dx, dy = D8.get(int(flow[r, c] & 15), (0, 0))
        a = (x0 + (c + 0.5) * scale, (r + 0.5) * scale)
        b = (a[0] + dx * scale, a[1] + dy * scale)
        col = THALWEG if flow[r, c] & 16 else WET if kinds[r, c] else DRY
        segs.append((a, b, dx, dy, col, ((r0 + r) * 3 + (c0 + c) * 5) % 7 == 0))
    for a, b, *_ in segs:
        draw.line([a, b], fill=(0, 0, 0), width=4)
    for a, b, dx, dy, col, head in segs:
        draw.line([a, b], fill=col, width=2)
        if head:
            n = math.hypot(dx, dy)
            ux, uy = dx / n, dy / n
            tip = (b[0] - ux * 0.2 * scale, b[1] - uy * 0.2 * scale)
            base = (tip[0] - ux * 1.5 * scale, tip[1] - uy * 1.5 * scale)
            hw = 0.75 * scale
            draw.polygon([tip, (base[0] - uy * hw, base[1] + ux * hw), (base[0] + uy * hw, base[1] - ux * hw)],
                         fill=col, outline=(0, 0, 0))


def _crop(L: Layers, path: Path, centre: tuple[int, int], size_m: float, title: str, target_px: int = CROP_PX,
          box: tuple[int, int, int, int] | None = None, ends: tuple = (), arrows: bool = False,
          legend: list = CROP_LEGEND) -> tuple[Path | None, Win | None]:
    """raw | result | cut around cell `centre`, one contrast stretch from the result's dry cells.
    `box` (c0, r0, c1, r1) and `ends` [(c, r)] are marked on raw and result, lake rings and flow
    arrows on the result. (None, None) when the centre is outside the grid."""
    from PIL import ImageDraw

    cells = max(1, int(round(size_m / L.spacing)))
    win = _window(L, *centre, cells)
    if win is None:
        return None, None
    r0, c0, h, w = win
    k = max(1, math.ceil(cells / target_px))
    scale = max(1, int(round(target_px / cells))) if k == 1 else 1
    raw = _read(L, L.raw, "<f4", win, k)
    res = _read(L, "elevation.f32", "<f4", win, k)
    kinds = _kinds_at(L, win, k)
    ir, ie = _illum(raw, L.spacing * k), _illum(res, L.spacing * k)
    lim = _limits(ie, kinds == 0)
    gr, ge = _gray(ir, lim), _gray(ie, lim)
    panels = [_u8(np.repeat(gr[..., None], 3, axis=2), scale), _u8(_water(ge, kinds, scale < 2), scale),
              _u8(_cut(res.astype(np.float64) - raw, ge), scale)]
    e, n = L.en(*centre)
    img, xs = _compose(panels, f"{title}   centre E{e} N{n}   raw | result | cut (result - raw)",
                       scale / (L.spacing * k), legend)
    draw = ImageDraw.Draw(img)

    def px(x0: int, c: float, r: float) -> tuple[float, float]:
        return x0 + (c - c0 + 0.5) * scale / k, (r - r0 + 0.5) * scale / k

    for x0 in xs[:2]:
        if box is not None:
            a, b = px(x0, box[0] - 0.5, box[1] - 0.5), px(x0, box[2] + 0.5, box[3] + 0.5)
            draw.rectangle([a, b], outline=MARK, width=2)
        for c, r in ends:
            x, y = px(x0, c, r)
            draw.rectangle([x - 5, y - 5, x + 5, y + 5], outline=MARK, width=2)
    if arrows and k == 1:
        _arrows(draw, L, win, scale, xs[1])
    rad = max(6, 1.5 * scale)
    for c, r, col in _rings(L, win):
        x, y = px(xs[1], c, r)
        draw.ellipse([x - rad, y - rad, x + rad, y + rad], outline=(0, 0, 0), width=4)
        draw.ellipse([x - rad, y - rad, x + rad, y + rad], outline=col, width=2)
    path.parent.mkdir(parents=True, exist_ok=True)
    img.save(path)
    return path, win


def _clusters(cells: list, radius: float) -> list[list]:
    """Greedy grouping of [c, r] cells within `radius` of a group's first cell."""
    groups: list[list] = []
    for c, r in sorted(map(tuple, cells)):
        for g in groups:
            if abs(g[0][0] - c) <= radius and abs(g[0][1] - r) <= radius:
                g.append((c, r))
                break
        else:
            groups.append([(c, r)])
    return groups


def _lake_sheets(L: Layers, dest: Path) -> list[Path]:
    """Contact sheets of every lake exit (spec "Review", I8), largest lakes first: the result
    with water tinted, the exit ringed; captioned id, level and E/N."""
    from PIL import Image, ImageDraw

    lakes = sorted((b for b in L.bodies if b["kind"] == "lake" and _is_open(b) and b.get("exit_cell") is not None),
                   key=lambda b: (-b.get("area_km2", 0.0), b["id"]))
    cells = exit_cells(L.spacing)
    exit_m = cells * L.spacing
    scale = max(1, int(round(EXIT_PX / cells)))
    side, cap = cells * scale, 14
    per = SHEET_COLS * SHEET_ROWS
    out = []
    for s in range(0, len(lakes), per):
        page = lakes[s : s + per]
        rows = math.ceil(len(page) / SHEET_COLS)
        sheet = Image.new("RGB", (SHEET_COLS * (side + 4), rows * (side + cap + 4)), "white")
        draw = ImageDraw.Draw(sheet)
        for i, b in enumerate(page):
            c, r = b["exit_cell"]
            win = _window(L, c, r, cells)
            if win is None:
                continue
            res = _read(L, "elevation.f32", "<f4", win)
            kinds = _kinds(L.table, _read(L, "water-body.u32", "<u4", win))
            ie = _illum(res, L.spacing)
            thumb = Image.fromarray(_u8(_water(_gray(ie, _limits(ie, kinds == 0)), kinds, scale < 2), scale))
            td = ImageDraw.Draw(thumb)
            x, y = (c - win[1] + 0.5) * scale, (r - win[0] + 0.5) * scale
            td.ellipse([x - 7, y - 7, x + 7, y + 7], outline=(0, 0, 0), width=4)
            td.ellipse([x - 7, y - 7, x + 7, y + 7], outline=OPEN_EXIT, width=2)
            ox, oy = (i % SHEET_COLS) * (side + 4), (i // SHEET_COLS) * (side + cap + 4)
            sheet.paste(thumb, (ox, oy + cap))
            e, n = L.en(c, r)
            draw.text((ox + 2, oy + 1), f"lake {b['id']}  {b.get('level_m', 0):.1f} m  E{e} N{n}", fill=(0, 0, 0))
        img, _ = _compose([np.asarray(sheet)], f"lake exits {s + 1}-{s + len(page)} of {len(lakes)}, "
                          f"{exit_m:g} m each, largest lakes first", scale / L.spacing, WATER_LEGEND[:3])
        path = dest / f"sheet-{s // per + 1:02d}.png"
        path.parent.mkdir(parents=True, exist_ok=True)
        img.save(path)
        out.append(path)
    return out


def _overview(L: Layers, path: Path, marks: list[tuple[Win, tuple]]) -> Path:
    """The whole grid at a stride (water block-reduced), tinted, with the crop boxes marked."""
    from PIL import ImageDraw

    k = max(1, math.ceil(max(L.w, L.h) / OVERVIEW_PX))
    win = (0, 0, L.h, L.w)
    res = _read(L, "elevation.f32", "<f4", win, k)
    kinds = _kinds_at(L, win, k)
    ie = _illum(res, L.spacing * k)
    panel = _u8(_water(_gray(ie, _limits(ie, kinds == 0)), kinds, True), 1)
    (e0, n0), (e1, n1) = L.en(0, 0), L.en(L.w, L.h)
    img, _ = _compose([panel], f"overview E{e0}-{e1} N{n1}-{n0}, 1 px = {k * L.spacing:g} m", 1 / (L.spacing * k),
                      OVERVIEW_LEGEND)
    draw = ImageDraw.Draw(img)
    for (r0, c0, h, w), col in marks:
        draw.rectangle([c0 / k, r0 / k, (c0 + w) / k, (r0 + h) / k], outline=col, width=2)
    path.parent.mkdir(parents=True, exist_ok=True)
    img.save(path)
    return path


def crops(src: Path, dest: Path, spots=SPOTS, areas=AREAS, dam: tuple[int, int] | None = DAM,
          raw: Path | None = None) -> list[Path]:
    """Every review figure of a world, window run or water output under `dest`: the owner's spots
    (spots/E<e>-N<n>.png, 512 m), the carried areas (carry/), the dam flow view (dam/), the
    longest straight runs (straight-runs/), clusters of failing-check examples (failures/), the
    lake-exit contact sheets (lake-exits/) and overview.png with every crop box. Spots outside
    the grid are skipped."""
    L = open_layers(src, raw)
    out: list[Path] = []
    marks: list[tuple[Win, tuple]] = []

    def add(result: tuple[Path | None, Win | None], colour: tuple) -> None:
        if result[0] is not None:
            out.append(result[0])
            marks.append((result[1], colour))

    for e, n in spots:
        add(_crop(L, dest / "spots" / f"E{e}-N{n}.png", L.cell(e, n), CROP_M, f"spot E{e} N{n}"), MARK)
    for a in areas:
        cs = [L.cell(e, n) for e, n in a.points]
        lo = (min(c for c, _ in cs), min(r for _, r in cs))
        hi = (max(c for c, _ in cs), max(r for _, r in cs))
        centre = ((lo[0] + hi[0]) // 2, (lo[1] + hi[1]) // 2)
        add(_crop(L, dest / "carry" / f"{a.name}.png", centre, a.size_m, a.name,
                  box=(*lo, *hi) if a.mark == "box" else None, ends=tuple(cs) if a.mark == "ends" else ()), CARRY)
    if dam is not None:
        add(_crop(L, dest / "dam" / f"flow-E{dam[0]}-N{dam[1]}.png", L.cell(*dam), DAM_M, "dam flow", DAM_PX,
                  arrows=True, legend=DAM_LEGEND), DAM_BOX)
    figures = L.report.get("figures", {})
    for i, run in enumerate(figures.get("longest_straight_runs", [])[:STRAIGHT_RUNS]):
        a, b = run["from"], run["to"]
        mid = ((a[0] + b[0]) // 2, (a[1] + b[1]) // 2)
        e, n = L.en(*mid)
        add(_crop(L, dest / "straight-runs" / f"{i + 1:02d}-E{e}-N{n}.png", mid, RUN_M,
                  f"straight run {i + 1}, {run.get('length_m', 0):.0f} m", ends=(tuple(a), tuple(b))), RUNS)
    for check, cells in sorted(L.report.get("examples", {}).items()):
        for j, g in enumerate(_clusters(cells, FAILURE_M / 2 / L.spacing)[:FAILURE_CROPS]):
            lo = (min(c for c, _ in g), min(r for _, r in g))
            hi = (max(c for c, _ in g), max(r for _, r in g))
            mid = ((lo[0] + hi[0]) // 2, (lo[1] + hi[1]) // 2)
            e, n = L.en(*mid)
            add(_crop(L, dest / "failures" / f"{check}-{j + 1:02d}-E{e}-N{n}.png", mid, FAILURE_M,
                      f"{check} cluster {j + 1} ({len(g)} cells)", box=(*lo, *hi)), CLOSED)
    out += _lake_sheets(L, dest / "lake-exits")
    out.append(_overview(L, dest / "overview.png", marks))
    return out


def window_pair(run: Path, dest: Path) -> Path:
    """Raw hillshade beside the result's (work/carved.f32), water tinted on the right: a quick
    look at what a window or level run changed."""
    from PIL import Image

    g = _json(run / "grid" / "grid.json")
    w, h = g["size"]
    cell = float(g.get("spacing_m", 1.0))
    k = max(1, math.ceil(max(w, h) / 1024))

    def load(path: Path) -> np.ndarray:
        return np.fromfile(path, "<f4").reshape(h, w)[::k, ::k]

    raw, res = load(run / "grid" / "elevation.f32"), load(run / "work" / "carved.f32")
    wet = np.isfinite(load(run / "work" / "water-surface.f32"))

    def shade(z: np.ndarray) -> np.ndarray:
        gy, gx = np.gradient(z.astype(np.float64), cell * k)
        i = np.clip((1 - gx + gy) / np.sqrt(3 * (1 + gx**2 + gy**2)), 0, 1)  # light from the NW
        return (i * 255).astype(np.uint8)

    left = np.repeat(shade(raw)[..., None], 3, axis=2)
    right = np.repeat(shade(res)[..., None], 3, axis=2)
    right[wet] = (right[wet] * 0.4 + np.array([30, 90, 200]) * 0.6).astype(np.uint8)
    gap = np.full((left.shape[0], GAP_PX, 3), 255, np.uint8)
    dest.parent.mkdir(parents=True, exist_ok=True)
    Image.fromarray(np.concatenate([left, gap, right], axis=1)).save(dest)
    return dest


# The coarse guide.


def _bodies(dirpath: Path) -> tuple[dict[int, str], dict[int, float]]:
    bodies = _json(dirpath / "bodies.json")["bodies"]
    return ({b["id"]: b["kind"] for b in bodies},
            {b["id"]: float(b["level_m"]) for b in bodies if b.get("level_m") is not None})


def _lookup(kinds: dict[int, str], kind: str) -> np.ndarray:
    table = np.zeros(max(kinds, default=0) + 1, bool)
    for i, k in kinds.items():
        table[i] = k == kind
    return table


def _is(table: np.ndarray, ids: np.ndarray) -> np.ndarray:
    return table[np.minimum(ids, len(table) - 1)] & (ids < len(table))


def _align(cg: dict, fg: dict, where: str) -> tuple[int, int, int]:
    """Fine cells per coarse cell and the fine grid's NW corner in coarse cells (x, y)."""
    cs, fs = float(cg.get("spacing_m", 1.0)), float(fg.get("spacing_m", 1.0))
    k = int(round(cs / fs))
    (cex, cny), (fex, fny) = _nw(cg), _nw(fg)
    x0, y0 = (fex - cex) / cs, (cny - fny) / cs
    if k < 1 or abs(k * fs - cs) > 1e-6 or x0 != int(x0) or y0 != int(y0):
        raise ValueError(f"{where}: the fine grid is not aligned to the {cs:g} m grid")
    return k, int(x0), int(y0)


def coarse_guide(world: Path, coarse: Path, fine_dir: Path | None = None, min_km2: float = 0.05,
                 min_depth_m: float | None = None, edge_m: float = 1000.0,
                 coarse_river_km2: float | None = None) -> dict:
    """The fine water against the next coarser level's over the fine grid's footprint.
    `fine_dir` holds the fine water-body.u32, bodies.json and grid.json (the world itself when
    None). Coarse lakes that the footprint cuts are not compared; coarse river cells within
    100 m of a footprint edge inside the coarse grid are not counted. A compared lake can fail
    (B15) only when it covers at least `min_km2`, is deeper than `min_depth_m` (level minus its
    lowest coarse ground; default the fine level's breach cap + 1 m) and lies `edge_m` or more
    from every inner footprint edge; the others are counted as figures. `mismatches` lists each
    failing lake with its box in metres (E west, N north, E east, N south)."""
    from . import table as water_table

    fine_dir = fine_dir or world
    cg, fg = _json(coarse / "grid.json"), _json(fine_dir / "grid.json")
    cs_m, fs_m = float(cg.get("spacing_m", 1.0)), float(fg.get("spacing_m", 1.0))
    if min_depth_m is None:
        min_depth_m = float(water_table.level(fs_m)["breach_cap_m"]) + 1.0
    if coarse_river_km2 is None:
        coarse_river_km2 = float(water_table.level(cs_m)["river_km2"])
    min_cells = math.ceil(min_km2 * 1e6 / (cs_m * cs_m))
    edge_cells = int(math.ceil(edge_m / cs_m))
    cw, ch = cg["size"]
    fw, fh = fg["size"]
    cs = float(cg.get("spacing_m", 1.0))
    k, x0, y0 = _align(cg, fg, str(fine_dir))
    pw, ph = fw // k, fh // k
    if x0 < 0 or y0 < 0 or x0 + pw > cw or y0 + ph > ch:
        raise ValueError(f"{fine_dir} lies outside {coarse}")
    cex, cny = _nw(cg)
    # Rows [r0, r1) and columns [c0, c1), read per call (no lasting memory map).
    def cb(r0, r1, c0, c1):
        return _rows(coarse / "water-body.u32", "<u4", cw, (r0, c0, r1 - r0, c1 - c0))

    def cq(r0, r1, c0, c1):
        return _rows(coarse / "discharge.f32", "<f4", cw, (r0, c0, r1 - r0, c1 - c0))

    def fb(r0, r1, c0, c1):
        return _rows(fine_dir / "water-body.u32", "<u4", fw, (r0, c0, r1 - r0, c1 - c0))
    ckinds, clevels = _bodies(coarse)
    fkinds, flevels = _bodies(fine_dir)
    c_lake, c_river = _lookup(ckinds, "lake"), _lookup(ckinds, "river")
    f_lake, f_river = _lookup(fkinds, "lake"), _lookup(fkinds, "river")
    # Edges of the footprint that are not edges of the coarse grid cut lakes and rivers.
    inner = (y0 > 0, x0 > 0, y0 + ph < ch, x0 + pw < cw)
    # Coarse lake boxes over the footprint, footprint-relative, by streaming rows.
    boxes: dict[int, list[int]] = {}
    for y in range(0, ph, 1024):
        rows = cb(y0 + y, y0 + min(y + 1024, ph), x0, x0 + pw)
        lab =np.where(_is(c_lake, rows), rows, 0).astype(np.int64)
        for i, sl in enumerate(ndimage.find_objects(lab), 1):
            if sl is None:
                continue
            b = boxes.setdefault(i, [ph, pw, -1, -1])
            b[:] = [min(b[0], y + sl[0].start), min(b[1], sl[1].start),
                    max(b[2], y + sl[0].stop - 1), max(b[3], sl[1].stop - 1)]
    diff, over, gone, doubled, mismatches, cut = {}, [], [], [], [], 0
    small = shallow = near_edge = 0
    for i, (r0, c0, r1, c1) in sorted(boxes.items()):
        if (inner[0] and r0 == 0) or (inner[1] and c0 == 0) or (inner[2] and r1 == ph - 1) or \
                (inner[3] and c1 == pw - 1):
            cut += 1
            continue
        box_m = [cex + (x0 + c0) * cs, cny - (y0 + r0) * cs, cex + (x0 + c1 + 1) * cs, cny - (y0 + r1 + 1) * cs]
        mask = cb(y0 + r0, y0 + r1 + 1, x0 + c0, x0 + c1 + 1) == i
        # Only large, deep lakes away from inner edges can fail (B15); the rest are figures.
        cells = int(mask.sum())
        lowest = float(_rows(coarse / "elevation.f32", "<f4", cw,
                             (y0 + r0, x0 + c0, r1 - r0 + 1, c1 - c0 + 1))[mask].min())
        near = ((inner[0] and r0 < edge_cells) or (inner[1] and c0 < edge_cells)
                or (inner[2] and r1 >= ph - edge_cells) or (inner[3] and c1 >= pw - edge_cells))
        hard = True
        if cells < min_cells:
            small += 1
            hard = False
        elif clevels[i] - lowest <= min_depth_m:
            shallow += 1
            hard = False
        elif near:
            near_edge += 1
            hard = False
        fine = fb(r0 * k, (r1 + 1) * k, c0 * k, (c1 + 1) * k)
        inside = np.kron(mask, np.ones((k, k), bool))
        ids, counts = np.unique(fine[inside], return_counts=True)
        lakes = [(int(c), int(j)) for j, c in zip(ids, counts) if _is(f_lake, np.array([j]))[0]]
        found = []
        if not lakes:
            if hard:
                gone.append(i)
                found.append(("gone", None))
        else:
            area = int(inside.sum())
            _, best = max(lakes)
            if hard and sum(1 for c, _ in lakes if c > DOUBLED_SHARE * area) >= 2:
                doubled.append(i)
                found.append(("doubled", flevels[best]))
            d = abs(flevels[best] - clevels[i])
            diff[str(i)] = round(d, 3)
            if hard and d > LEVEL_GROSS_M:
                over.append(i)
                found.append(("over_5m", flevels[best]))
        for failure, level in found:
            mismatches.append({"lake": i, "failure": failure, "box_m": box_m, "coarse_level_m": clevels.get(i),
                               "fine_level_m": level})
    # Coarse rivers: a fine river cell within 100 m (a k x k block any, then a square dilation).
    near = int(round(RIVER_NEAR_M / cs))
    has = np.zeros((ph, pw), bool)
    step = max(1, 512 // k) * k
    for y in range(0, ph * k, step):
        rows = fb(y, min(y + step, ph * k), 0, pw * k)
        riv = _is(f_river, rows)
        has[y // k : y // k + rows.shape[0] // k] = riv.reshape(rows.shape[0] // k, k, pw, k).any(axis=(1, 3))
    has = ndimage.maximum_filter(has, size=2 * near + 1)
    keep = np.zeros((ph, pw), bool)
    keep[near if inner[0] else 0 : ph - near if inner[2] else ph,
         near if inner[1] else 0 : pw - near if inner[3] else pw] = True
    hits = total = 0
    for y in range(0, ph, 1024):
        rows = cb(y0 + y, y0 + min(y + 1024, ph), x0, x0 + pw)
        q = cq(y0 + y, y0 + min(y + 1024, ph), x0, x0 + pw)
        sel = _is(c_river, rows) & (q >= coarse_river_km2) & keep[y : y + rows.shape[0]]
        total += int(sel.sum())
        hits += int((sel & has[y : y + rows.shape[0]]).sum())
    return {
        "lakes_compared": len(boxes) - cut, "lakes_cut_by_window": cut, "level_diff_m": diff,
        "lakes_over_5m": over, "lakes_gone": gone, "lakes_doubled": doubled,
        "lakes_small": small, "lakes_shallow": shallow, "lakes_near_edge": near_edge,
        "min_cells": min_cells, "min_depth_m": min_depth_m, "edge_m": edge_m,
        "coarse_river_km2": coarse_river_km2,
        "coarse_river_cells": total,
        "coarse_river_share_with_fine_river_within_100m": hits / total if total else 1.0,
        "hard_failures": len(over) + len(gone) + len(doubled),
        "mismatches": mismatches,
    }


def guide_figures(src: Path, coarse: Path, guide: dict, dest: Path, raw: Path | None = None) -> list[Path]:
    """One figure per coarse-guide mismatch: the result with the 4 m water beside it with the
    1 m water, the 4 m lake's box marked (coarse-guide/lake-<id>-<failure>.png)."""
    from PIL import ImageDraw

    L = open_layers(src, raw)
    cg = _json(coarse / "grid.json")
    kc, x0, y0 = _align(cg, _json(L.out / "grid.json"), str(src))
    cw, ch = cg["size"]
    cb = np.memmap(coarse / "water-body.u32", "<u4", "r", shape=(ch, cw))
    ctable = _kind_table(_json(coarse / "bodies.json")["bodies"])
    out = []
    for m in guide.get("mismatches", []):
        e0, n0, e1, n1 = m["box_m"]
        size = min(max(max(e1 - e0, n0 - n1) * 1.3, GUIDE_MIN_M), GUIDE_MAX_M)
        centre = L.cell((e0 + e1) / 2, (n0 + n1) / 2)
        cells = max(1, int(round(size / L.spacing)))
        win = _window(L, *centre, cells)
        if win is None:
            continue
        r0, c0, h, w = win
        k = max(1, math.ceil(cells / GUIDE_PX))
        scale = max(1, int(round(GUIDE_PX / cells))) if k == 1 else 1
        res = _read(L, "elevation.f32", "<f4", win, k)
        fk = _kinds_at(L, win, k)
        rows, cols = r0 + np.arange(0, h, k), c0 + np.arange(0, w, k)
        ck = _kinds(ctable, np.asarray(cb[np.ix_(y0 + rows // kc, x0 + cols // kc)]))
        ie = _illum(res, L.spacing * k)
        g = _gray(ie, _limits(ie, fk == 0))
        widen = scale < 2
        lvl = "none" if m["fine_level_m"] is None else f"{m['fine_level_m']:.2f} m"
        e, n = L.en(*centre)
        img, xs = _compose([_u8(_water(g, ck, widen), scale), _u8(_water(g, fk, widen), scale)],
                           f"4 m lake {m['lake']}: {m['failure']}, 4 m level {m['coarse_level_m']:.2f} m, "
                           f"1 m level {lvl}   centre E{e} N{n}   4 m water | 1 m water",
                           scale / (L.spacing * k), WATER_LEGEND[:2] + [("box", MARK, "4 m lake box")])
        draw = ImageDraw.Draw(img)
        (bc0, br0), (bc1, br1) = L.cell(e0, n0), L.cell(e1, n1)
        for x in xs:
            draw.rectangle([x + (bc0 - c0) * scale / k, (br0 - r0) * scale / k,
                            x + (bc1 - c0) * scale / k, (br1 - r0) * scale / k], outline=MARK, width=2)
        path = dest / f"lake-{m['lake']}-{m['failure']}.png"
        path.parent.mkdir(parents=True, exist_ok=True)
        img.save(path)
        out.append(path)
    return out


GUIDE_SUMMARY = ("hard_failures", "lakes_compared", "lakes_over_5m", "lakes_gone", "lakes_doubled",
                 "lakes_small", "lakes_shallow", "lakes_near_edge",
                 "coarse_river_share_with_fine_river_within_100m")


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("path", type=Path, help="v3 world, window run directory or water output directory")
    p.add_argument("--dest", type=Path, help="default: next to the input (see default_dest)")
    p.add_argument("--raw", type=Path, help="the raw surface, when not beside the output")
    p.add_argument("--coarse", type=Path, help="the next coarser level's water output directory for the coarse guide")
    a = p.parse_args(argv)
    dest = a.dest or default_dest(a.path)
    paths = crops(a.path, dest, raw=a.raw)
    if a.coarse:
        guide = coarse_guide(a.path, a.coarse, fine_dir=open_layers(a.path, a.raw).out)
        (dest / "coarse-guide.json").write_text(json.dumps(guide, indent=1))
        paths += guide_figures(a.path, a.coarse, guide, dest / "coarse-guide", a.raw)
        print(json.dumps({k: guide[k] for k in GUIDE_SUMMARY}))
    for path in paths:
        print(path)
    return 0


if __name__ == "__main__":
    sys.exit(main())
