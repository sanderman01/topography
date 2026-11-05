"""Review figures for hydrology v3: hillshade crops (the raw surface beside the result, water
tinted, lake exits marked) and the coarse guide, the 1 m water against the 4 m world's.

Works on a v3 world, a driver window run (grid/ and out/) or a bare water output directory
(with --raw). Crops outside the processed grid are skipped. The coarse guide is a figure in
Plan A (ruling R17); a lake more than 5 m off its 4 m level, gone or doubled counts as a hard
failure in it (spec "Checks").

python -m topography_training.water.review <world | run dir | out dir> [--dest D] [--raw F] [--coarse 4m dir]
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from dataclasses import dataclass
from pathlib import Path

import numpy as np
from scipy import ndimage

# The owner's spots (spec "Review"), east and north in metres.
SPOTS = ((25645, 24400), (25307, 24585), (20071, 30501), (27169, 44546), (43966, 21633), (49249, 16381),
         (47962, 36851), (48148, 36810), (48810, 19452), (51610, 20478), (42581, 56193))
CROP_M = 512
CROP_PX = 512
GAP_PX = 8
LEVEL_GROSS_M = 5.0
DOUBLED_SHARE = 0.25
RIVER_NEAR_M = 100.0
RIVER_KM2_4M = 0.5


@dataclass(frozen=True)
class Area:
    """A carried review spot: one point (centred), a box (two corners) or a line (two ends)."""

    name: str
    points: tuple[tuple[int, int], ...]
    mark: str = "none"  # "box", "ends" or "none"
    size_m: int = CROP_M


# The controller's Task 17 list (carry-later.md, progress.md); E/N in metres. Straight-share
# runs and thalweg_beside clusters are the Task 13 probe results, converted to world metres.
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
# The dam spot: flow arrows and lake outlet markers (carry-later.md).
DAM = (25645, 24400)
DAM_M, DAM_PX = 96, 768
# Dry cells get a flow arrow from this discharge on (wet and thalweg cells always).
ARROW_KM2 = 2e-5
STRAIGHT_RUNS, RUN_M = 20, 256
FAILURE_M, FAILURE_CROPS = 128, 10
EXIT_M, EXIT_PX, SHEET_COLS, SHEET_ROWS = 128, 256, 6, 8

# Water tints by kind (1 lake or sea, 2 river) and marker colours.
TINT = {1: (0.2, 0.45, 1.0), 2: (0.1, 0.95, 1.0)}
MARK, FULL_EXIT, CLOSED = (255, 220, 0), (40, 230, 60), (255, 40, 40)
# D8 receiver codes in flow.u8's low 4 bits: (dx, dy).
D8 = {1: (0, -1), 2: (0, 1), 3: (-1, 0), 4: (1, 0), 5: (-1, -1), 6: (1, -1), 7: (-1, 1), 8: (1, 1)}


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

    def mm(self, name: str | Path, dtype) -> np.ndarray:
        path = name if isinstance(name, Path) else self.out / name
        return np.memmap(path, dtype, "r", shape=(self.h, self.w))

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


def _illum(z: np.ndarray, cell_m: float) -> np.ndarray:
    """Hillshade (north-west sun at 45 degrees) in 0..1, NaN where the ground is."""
    gy, gx = np.gradient(np.asarray(z, np.float64), cell_m)
    az, alt = np.radians(315), np.radians(45)
    slope = np.arctan(np.hypot(gx, gy))
    aspect = np.arctan2(-gx, gy)
    return np.clip(np.sin(alt) * np.cos(slope) + np.cos(alt) * np.sin(slope) * np.cos(az - aspect), 0, 1)


def _limits(i: np.ndarray) -> tuple[float, float]:
    """The contrast stretch: 1st to 99th percentile."""
    if not np.isfinite(i).any():
        return 0.0, 1.0
    lo, hi = np.nanpercentile(i, [1, 99])
    return (float(lo), float(hi)) if hi - lo > 1e-6 else (0.0, 1.0)


def _rgb(i: np.ndarray, lim: tuple[float, float], kinds: np.ndarray | None) -> np.ndarray:
    g = np.nan_to_num(np.clip((i - lim[0]) / (lim[1] - lim[0]), 0, 1), nan=0.5)
    rgb = np.repeat(g[..., None], 3, axis=2)
    if kinds is not None:
        for k, col in TINT.items():
            m = kinds == k
            rgb[m] = 0.3 * rgb[m] + 0.7 * np.array(col) * (0.55 + 0.45 * rgb[m])
    return (rgb * 255).astype(np.uint8)


def _up(a: np.ndarray, scale: int) -> np.ndarray:
    return a if scale == 1 else np.repeat(np.repeat(a, scale, 0), scale, 1)


def _join(left: np.ndarray, right: np.ndarray) -> np.ndarray:
    return np.concatenate([left, np.full((left.shape[0], GAP_PX, 3), 255, np.uint8), right], axis=1)


def _panels(L: Layers, win: tuple[int, int, int, int], scale: int, pair: bool) -> np.ndarray:
    """Raw beside the result (or the result alone), one contrast stretch from the raw."""
    r0, c0, h, w = win
    sl = (slice(r0, r0 + h), slice(c0, c0 + w))
    res = _illum(np.asarray(L.mm("elevation.f32", "<f4")[sl]), L.spacing)
    kinds = _kinds(_kind_table(L.bodies), np.asarray(L.mm("water-body.u32", "<u4")[sl]))
    if not pair:
        return _up(_rgb(res, _limits(res), kinds), scale)
    raw = _illum(np.asarray(L.mm(L.raw, "<f4")[sl]), L.spacing)
    lim = _limits(raw)
    return _join(_up(_rgb(raw, lim, None), scale), _up(_rgb(res, lim, kinds), scale))


def _window(L: Layers, c: int, r: int, cells: int) -> tuple[int, int, int, int] | None:
    """A cells² window around (c, r) kept inside the grid; None when (c, r) is outside it."""
    if not (0 <= c < L.w and 0 <= r < L.h):
        return None
    ch, cw = min(cells, L.h), min(cells, L.w)
    return min(max(r - ch // 2, 0), L.h - ch), min(max(c - cw // 2, 0), L.w - cw), ch, cw


def _exits(L: Layers, win: tuple[int, int, int, int]) -> list[tuple[int, int, tuple]]:
    """Lake outlet markers in the window: a full lake's exit cell, a closed lake's lowest cell."""
    r0, c0, h, w = win
    marks = []
    body = None
    for b in L.bodies:
        if b["kind"] != "lake":
            continue
        if b.get("exit_cell") is not None:
            c, r = b["exit_cell"]
            if r0 <= r < r0 + h and c0 <= c < c0 + w:
                marks.append((c, r, FULL_EXIT))
            continue
        if body is None:
            body = np.asarray(L.mm("water-body.u32", "<u4")[r0 : r0 + h, c0 : c0 + w])
            ground = np.asarray(L.mm("elevation.f32", "<f4")[r0 : r0 + h, c0 : c0 + w])
        inside = body == b["id"]
        if inside.any():
            r, c = np.unravel_index(np.argmin(np.where(inside, ground, np.inf)), inside.shape)
            marks.append((c0 + int(c), r0 + int(r), CLOSED))
    return marks


def _crop(L: Layers, path: Path, centre: tuple[int, int], size_m: float, target_px: int = CROP_PX,
          box: tuple[int, int, int, int] | None = None, ends: tuple = (), arrows: bool = False,
          pair: bool = True) -> Path | None:
    """One crop centred on cell `centre`; `box` (c0, r0, c1, r1) and `ends` [(c, r)] are marked
    on both panels, lake outlets and flow arrows on the result. None when outside the grid."""
    from PIL import Image, ImageDraw

    cells = max(1, int(round(size_m / L.spacing)))
    win = _window(L, *centre, cells)
    if win is None:
        return None
    r0, c0, h, w = win
    scale = max(1, int(round(target_px / cells)))
    img = Image.fromarray(_panels(L, win, scale, pair))
    draw = ImageDraw.Draw(img)
    right = w * scale + GAP_PX if pair else 0

    def px(x0: int, c: float, r: float) -> tuple[float, float]:
        return x0 + (c - c0 + 0.5) * scale, (r - r0 + 0.5) * scale

    for x0 in {0, right}:
        if box is not None:
            a, b = px(x0, box[0] - 0.5, box[1] - 0.5), px(x0, box[2] + 0.5, box[3] + 0.5)
            draw.rectangle([a, b], outline=MARK)
        for c, r in ends:
            x, y = px(x0, c, r)
            draw.ellipse([x - 5, y - 5, x + 5, y + 5], outline=MARK, width=2)
    if arrows:
        flow = np.asarray(L.mm("flow.u8", np.uint8)[r0 : r0 + h, c0 : c0 + w])
        kinds = _kinds(_kind_table(L.bodies), np.asarray(L.mm("water-body.u32", "<u4")[r0 : r0 + h, c0 : c0 + w]))
        q = np.asarray(L.mm("discharge.f32", "<f4")[r0 : r0 + h, c0 : c0 + w])
        shown = (flow & 15 > 0) & ((kinds > 0) | (flow & 16 > 0) | (q >= ARROW_KM2))
        for r, c in zip(*np.nonzero(shown)):
            dx, dy = D8.get(int(flow[r, c] & 15), (0, 0))
            x, y = px(right, c0 + c, r0 + r)
            tip = (x + dx * 0.4 * scale, y + dy * 0.4 * scale)
            col = (230, 30, 30) if flow[r, c] & 16 else MARK if kinds[r, c] else (0, 0, 0)
            draw.line([(x - dx * 0.3 * scale, y - dy * 0.3 * scale), tip], fill=col)
            draw.rectangle([tip[0] - 1, tip[1] - 1, tip[0] + 1, tip[1] + 1], fill=col)
    for c, r, col in _exits(L, win):
        x, y = px(right, c, r)
        rad = max(4, scale)
        draw.ellipse([x - rad, y - rad, x + rad, y + rad], outline=col, width=2)
    path.parent.mkdir(parents=True, exist_ok=True)
    img.save(path)
    return path


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

    lakes = sorted((b for b in L.bodies if b["kind"] == "lake" and b.get("exit_cell") is not None),
                   key=lambda b: (-b.get("area_km2", 0.0), b["id"]))
    cells = max(1, int(round(EXIT_M / L.spacing)))
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
            thumb = Image.fromarray(_panels(L, win, scale, pair=False))
            td = ImageDraw.Draw(thumb)
            x, y = (c - win[1] + 0.5) * scale, (r - win[0] + 0.5) * scale
            td.ellipse([x - 6, y - 6, x + 6, y + 6], outline=CLOSED, width=2)
            ox, oy = (i % SHEET_COLS) * (side + 4), (i // SHEET_COLS) * (side + cap + 4)
            sheet.paste(thumb, (ox, oy + cap))
            e, n = L.en(c, r)
            draw.text((ox + 2, oy + 1), f"lake {b['id']}  {b.get('level_m', 0):.1f} m  E{e} N{n}", fill=(0, 0, 0))
        path = dest / f"sheet-{s // per + 1:02d}.png"
        path.parent.mkdir(parents=True, exist_ok=True)
        sheet.save(path)
        out.append(path)
    return out


def crops(src: Path, dest: Path, spots=SPOTS, areas=AREAS, dam: tuple[int, int] | None = DAM,
          raw: Path | None = None) -> list[Path]:
    """Every review crop of a world, window run or water output under `dest`: the owner's spots
    (spots/E<e>-N<n>.png, 512 m), the carried areas (carry/), the dam flow overlay (dam/), the
    longest straight runs (straight-runs/), clusters of failing-check examples (failures/) and the
    lake-exit contact sheets (lake-exits/). Spots outside the grid are skipped."""
    L = open_layers(src, raw)
    out: list[Path | None] = []
    for e, n in spots:
        out.append(_crop(L, dest / "spots" / f"E{e}-N{n}.png", L.cell(e, n), CROP_M))
    for a in areas:
        cs = [L.cell(e, n) for e, n in a.points]
        lo = (min(c for c, _ in cs), min(r for _, r in cs))
        hi = (max(c for c, _ in cs), max(r for _, r in cs))
        centre = ((lo[0] + hi[0]) // 2, (lo[1] + hi[1]) // 2)
        out.append(_crop(L, dest / "carry" / f"{a.name}.png", centre, a.size_m,
                         box=(*lo, *hi) if a.mark == "box" else None, ends=tuple(cs) if a.mark == "ends" else ()))
    if dam is not None:
        out.append(_crop(L, dest / "dam" / f"flow-E{dam[0]}-N{dam[1]}.png", L.cell(*dam), DAM_M, DAM_PX,
                         arrows=True))
    figures = L.report.get("figures", {})
    for i, run in enumerate(figures.get("longest_straight_runs", [])[:STRAIGHT_RUNS]):
        a, b = run["from"], run["to"]
        mid = ((a[0] + b[0]) // 2, (a[1] + b[1]) // 2)
        e, n = L.en(*mid)
        out.append(_crop(L, dest / "straight-runs" / f"{i + 1:02d}-E{e}-N{n}.png", mid, RUN_M,
                         ends=(tuple(a), tuple(b))))
    for check, cells in sorted(L.report.get("examples", {}).items()):
        for j, g in enumerate(_clusters(cells, FAILURE_M / 2 / L.spacing)[:FAILURE_CROPS]):
            lo = (min(c for c, _ in g), min(r for _, r in g))
            hi = (max(c for c, _ in g), max(r for _, r in g))
            mid = ((lo[0] + hi[0]) // 2, (lo[1] + hi[1]) // 2)
            e, n = L.en(*mid)
            out.append(_crop(L, dest / "failures" / f"{check}-{j + 1:02d}-E{e}-N{n}.png", mid, FAILURE_M,
                             box=(*lo, *hi)))
    out += _lake_sheets(L, dest / "lake-exits")
    return [p for p in out if p is not None]


def window_pair(run_dir: Path, dest: Path, raw: Path | None = None, max_px: int = 2048) -> Path:
    """The whole window, raw beside the result (block means down to at most `max_px` a side;
    a block is a river if any cell is, else water if any cell is)."""
    from PIL import Image

    L = open_layers(run_dir, raw)
    k = max(1, math.ceil(max(L.w, L.h) / max_px))
    h, w = L.h // k * k, L.w // k * k
    table = _kind_table(L.bodies)

    def blocks(a: np.ndarray, how) -> np.ndarray:
        return how(a.reshape(a.shape[0] // k, k, w // k, k), axis=(1, 3))

    raw_m, res_m, kinds = [], [], []
    step = max(k, 1024 // k * k)
    for y in range(0, h, step):
        rows = slice(y, min(y + step, h))
        raw_m.append(blocks(np.asarray(L.mm(L.raw, "<f4")[rows, :w], np.float64), np.mean))
        res_m.append(blocks(np.asarray(L.mm("elevation.f32", "<f4")[rows, :w], np.float64), np.mean))
        kb = _kinds(table, np.asarray(L.mm("water-body.u32", "<u4")[rows, :w]))
        kinds.append(np.where(blocks(kb == 2, np.any), 2, np.where(blocks(kb == 1, np.any), 1, 0)))
    raw_i = _illum(np.concatenate(raw_m), L.spacing * k)
    res_i = _illum(np.concatenate(res_m), L.spacing * k)
    lim = _limits(raw_i)
    dest.parent.mkdir(parents=True, exist_ok=True)
    Image.fromarray(_join(_rgb(raw_i, lim, None), _rgb(res_i, lim, np.concatenate(kinds)))).save(dest)
    return dest


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


def coarse_guide(world: Path, coarse: Path, fine_dir: Path | None = None) -> dict:
    """The fine water against the coarse (4 m) world's over the fine grid's footprint.
    `fine_dir` holds the fine water-body.u32, bodies.json and grid.json (the world itself when
    None). Coarse lakes that the footprint cuts are not compared; coarse river cells within
    100 m of a footprint edge inside the coarse grid are not counted."""
    fine_dir = fine_dir or world
    cg, fg = _json(coarse / "grid.json"), _json(fine_dir / "grid.json")
    cw, ch = cg["size"]
    fw, fh = fg["size"]
    cs, fs = float(cg.get("spacing_m", 1.0)), float(fg.get("spacing_m", 1.0))
    k = int(round(cs / fs))
    (cex, cny), (fex, fny) = _nw(cg), _nw(fg)
    x0, y0 = (fex - cex) / cs, (cny - fny) / cs
    if k < 1 or abs(k * fs - cs) > 1e-6 or x0 != int(x0) or y0 != int(y0):
        raise ValueError(f"{fine_dir} is not aligned to the {cs:g} m grid of {coarse}")
    x0, y0 = int(x0), int(y0)
    pw, ph = fw // k, fh // k
    if x0 < 0 or y0 < 0 or x0 + pw > cw or y0 + ph > ch:
        raise ValueError(f"{fine_dir} lies outside {coarse}")
    cb = np.memmap(coarse / "water-body.u32", "<u4", "r", shape=(ch, cw))
    cq = np.memmap(coarse / "discharge.f32", "<f4", "r", shape=(ch, cw))
    fb = np.memmap(fine_dir / "water-body.u32", "<u4", "r", shape=(fh, fw))
    ckinds, clevels = _bodies(coarse)
    fkinds, flevels = _bodies(fine_dir)
    c_lake, c_river = _lookup(ckinds, "lake"), _lookup(ckinds, "river")
    f_lake, f_river = _lookup(fkinds, "lake"), _lookup(fkinds, "river")
    # Edges of the footprint that are not edges of the coarse grid cut lakes and rivers.
    inner = (y0 > 0, x0 > 0, y0 + ph < ch, x0 + pw < cw)
    # Coarse lake boxes over the footprint, footprint-relative, by streaming rows.
    boxes: dict[int, list[int]] = {}
    for y in range(0, ph, 1024):
        rows = np.asarray(cb[y0 + y : y0 + min(y + 1024, ph), x0 : x0 + pw])
        lab = np.where(_is(c_lake, rows), rows, 0).astype(np.int64)
        for i, sl in enumerate(ndimage.find_objects(lab), 1):
            if sl is None:
                continue
            b = boxes.setdefault(i, [ph, pw, -1, -1])
            b[:] = [min(b[0], y + sl[0].start), min(b[1], sl[1].start),
                    max(b[2], y + sl[0].stop - 1), max(b[3], sl[1].stop - 1)]
    diff, over, gone, doubled, cut = {}, [], [], [], 0
    for i, (r0, c0, r1, c1) in sorted(boxes.items()):
        if (inner[0] and r0 == 0) or (inner[1] and c0 == 0) or (inner[2] and r1 == ph - 1) or \
                (inner[3] and c1 == pw - 1):
            cut += 1
            continue
        mask = np.asarray(cb[y0 + r0 : y0 + r1 + 1, x0 + c0 : x0 + c1 + 1]) == i
        fine = np.asarray(fb[r0 * k : (r1 + 1) * k, c0 * k : (c1 + 1) * k])
        inside = np.kron(mask, np.ones((k, k), bool))
        ids, counts = np.unique(fine[inside], return_counts=True)
        lakes = [(int(c), int(j)) for j, c in zip(ids, counts) if _is(f_lake, np.array([j]))[0]]
        if not lakes:
            gone.append(i)
            continue
        area = int(inside.sum())
        if sum(1 for c, _ in lakes if c > DOUBLED_SHARE * area) >= 2:
            doubled.append(i)
        _, best = max(lakes)
        d = abs(flevels[best] - clevels[i])
        diff[str(i)] = round(d, 3)
        if d > LEVEL_GROSS_M:
            over.append(i)
    # Coarse rivers: a fine river cell within 100 m (a k x k block any, then a square dilation).
    near = int(round(RIVER_NEAR_M / cs))
    has = np.zeros((ph, pw), bool)
    step = max(1, 512 // k) * k
    for y in range(0, ph * k, step):
        rows = np.asarray(fb[y : min(y + step, ph * k), : pw * k])
        riv = _is(f_river, rows)
        has[y // k : y // k + rows.shape[0] // k] = riv.reshape(rows.shape[0] // k, k, pw, k).any(axis=(1, 3))
    has = ndimage.maximum_filter(has, size=2 * near + 1)
    keep = np.zeros((ph, pw), bool)
    keep[near if inner[0] else 0 : ph - near if inner[2] else ph,
         near if inner[1] else 0 : pw - near if inner[3] else pw] = True
    hits = total = 0
    for y in range(0, ph, 1024):
        rows = np.asarray(cb[y0 + y : y0 + min(y + 1024, ph), x0 : x0 + pw])
        q = np.asarray(cq[y0 + y : y0 + min(y + 1024, ph), x0 : x0 + pw])
        sel = _is(c_river, rows) & (q >= RIVER_KM2_4M) & keep[y : y + rows.shape[0]]
        total += int(sel.sum())
        hits += int((sel & has[y : y + rows.shape[0]]).sum())
    return {
        "lakes_compared": len(boxes) - cut, "lakes_cut_by_window": cut, "level_diff_m": diff,
        "lakes_over_5m": over, "lakes_gone": gone, "lakes_doubled": doubled,
        "coarse_river_cells": total,
        "coarse_river_share_with_fine_river_within_100m": hits / total if total else 1.0,
        "hard_failures": len(over) + len(gone) + len(doubled),
    }


GUIDE_SUMMARY = ("hard_failures", "lakes_compared", "lakes_over_5m", "lakes_gone", "lakes_doubled",
                 "coarse_river_share_with_fine_river_within_100m")


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("path", type=Path, help="v3 world, window run directory or water output directory")
    p.add_argument("--dest", type=Path, help="default: <world>/hydrology/1m/review or <run>/review")
    p.add_argument("--raw", type=Path, help="the raw surface, when not beside the output")
    p.add_argument("--coarse", type=Path, help="a 4 m hydrology directory for the coarse guide")
    a = p.parse_args(argv)
    L = open_layers(a.path, a.raw)
    world = L.out.parent.name == "hydrology"
    dest = a.dest or (L.out / "review" if world else (a.path if L.out != a.path else L.out) / "review")
    paths = crops(a.path, dest, raw=a.raw)
    if not world:
        paths.append(window_pair(a.path, dest / "window.png", a.raw))
    for path in paths:
        print(path)
    if a.coarse:
        guide = coarse_guide(a.path, a.coarse, fine_dir=L.out)
        (dest / "coarse-guide.json").write_text(json.dumps(guide, indent=1))
        print(json.dumps({k: guide[k] for k in GUIDE_SUMMARY}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
