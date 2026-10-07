"""Lakes and streams on a generated surface: priority-flood depression filling with a
tiny gradient (Barnes et al. 2014) gives drainage order, lakes and flow accumulation.

Game layers, not hydraulics: lakes are filled depressions above a size and depth, and
streams follow drainage areas above a threshold with a width growing with that area.
"""

from __future__ import annotations

import heapq
from dataclasses import dataclass

import numpy as np

EPSILON_M = 1e-3  # gradient across filled flats so every cell drains
STREAM_MIN_AREA_KM2 = 4.0
# Width (m) = STREAM_WIDTH_M * sqrt(drainage area / km2), capped.
STREAM_WIDTH_M = 2.5
STREAM_MAX_WIDTH_M = 60.0


@dataclass(frozen=True)
class Water:
    """How much water a terrain class gets: lakes kept after L1 and small lakes after L2
    (shares of the land), the drainage area where streams begin and their width (m per
    sqrt(km2) of drainage area; dry washes are narrow). Judgment values
    from the inland water share of the real context regions (2026-10-03): about 0.5% in
    dunes, canyons and badlands, 3-5% in fjords, archipelagos and glacier country."""

    lake_share: float
    tarn_share: float
    stream_km2: float
    stream_width: float = 2.5
    dry_share: float = 0.0  # closed dry basins kept after L2: dolines, craters, interdune hollows
    sinks: bool = False  # streams end in its dry basins (karst); elsewhere they flow on to the sea


# By class id (0: unlabelled). Dry classes keep valley floors as dry washes.
WATER = {
    0: Water(0.010, 0.002, 8.0),
    1: Water(0.003, 0.000, 20.0, 1.5, 0.010, True),  # karst: streams sink
    2: Water(0.002, 0.000, 50.0, 1.0),  # gorges and canyons
    3: Water(0.020, 0.005, 4.0),  # fjords and glacial troughs
    4: Water(0.010, 0.002, 10.0, 2.5, 0.005),  # calderas and craters
    5: Water(0.020, 0.003, 4.0),  # archipelagos
    6: Water(0.006, 0.005, 4.0),  # alpine ridges and cirques
    7: Water(0.002, 0.000, 50.0, 1.0),  # badlands and mesas
    8: Water(0.001, 0.000, 200.0, 0.5, 0.005),  # dunes
    9: Water(0.010, 0.001, 4.0),  # braided rivers
    10: Water(0.015, 0.005, 4.0),  # glaciers and ice fields
    11: Water(0.005, 0.002, 10.0, 2.5, 0.003),  # volcanic cones and lava fields
    12: Water(0.008, 0.001, 4.0),  # coastal cliffs
}
# Legend v3 subclasses take their parent's water (plain the default).
WATER |= {13: WATER[2], 14: WATER[1], 15: WATER[5], 16: WATER[8], 17: WATER[11], 18: WATER[0]}

# Tarns, in cells of the surface drained (32 m at L2: about 0.4 to 26 ha), at least 2 m
# deep and compact (bounding-box side squared over area).
TARN_CELLS = (4, 256)
TARN_MIN_DEPTH_M = 2.0
TARN_MAX_ELONGATION = 4.0


def flood(h: np.ndarray, outlet: np.ndarray):
    """Filled surface, downstream neighbour per cell (-1 at outlets) and visit order.

    Outlets are sea cells and the field edge; every other cell drains to one of them."""
    rows, cols = h.shape
    flat = h.ravel().astype(np.float64)
    filled = flat.copy()
    parent = np.full(flat.size, -1, np.int64)
    seen = np.zeros(flat.size, bool)
    edge = np.zeros(h.shape, bool)
    edge[0, :] = edge[-1, :] = edge[:, 0] = edge[:, -1] = True
    heap = []
    for i in np.flatnonzero((outlet | edge).ravel()):
        seen[i] = True
        heap.append((filled[i], int(i)))
    heapq.heapify(heap)
    order = []
    offsets = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (-1, 1), (1, -1), (1, 1)]
    push, pop = heapq.heappush, heapq.heappop
    while heap:
        z, i = pop(heap)
        order.append(i)
        r, c = divmod(i, cols)
        for dr, dc in offsets:
            rr, cc = r + dr, c + dc
            if 0 <= rr < rows and 0 <= cc < cols:
                j = rr * cols + cc
                if not seen[j]:
                    seen[j] = True
                    parent[j] = i
                    if filled[j] <= z:
                        filled[j] = z + EPSILON_M
                    push(heap, (filled[j], j))
    return filled.reshape(h.shape), parent, np.array(order, np.int64)


def accumulate(parent: np.ndarray, order: np.ndarray) -> np.ndarray:
    """Upstream cell count per cell (itself included)."""
    acc = np.ones(parent.size, np.float64)
    for i in order[::-1]:
        p = parent[i]
        if p >= 0:
            acc[p] += acc[i]
    return acc


def streams(h: np.ndarray, water: np.ndarray, cell_m: float, min_km2: float = STREAM_MIN_AREA_KM2,
            width_m: float = STREAM_WIDTH_M, outlet: np.ndarray | None = None):
    """Stream cells, downstream neighbour (flat index) and width (m) on a draining surface.

    Flow ends at `outlet` (default: all water). With only the sea and sinks as outlets,
    flow passes through lakes to their spill point. A stream, once started, continues to
    its outlet, so a larger threshold downstream (another class) does not cut it."""
    _, parent, order = flood(h, water if outlet is None else outlet)
    area_km2 = accumulate(parent, order).reshape(h.shape) * cell_m * cell_m / 1e6
    flow = (area_km2 >= min_km2).ravel()
    for i in order[::-1]:  # upstream first
        p = parent[i]
        if flow[i] and p >= 0:
            flow[p] = True
    stream = flow.reshape(h.shape) & ~water
    width = np.clip(width_m * np.sqrt(area_km2), 0, STREAM_MAX_WIDTH_M).astype(np.float32)
    return stream, parent.reshape(h.shape), width


def smooth_paths(stream: np.ndarray, parent: np.ndarray, width: np.ndarray, iterations: int = 6) -> np.ndarray:
    """Stream cell positions (2, rows, cols) in cells, pulled toward the line through their
    downstream and main upstream neighbours, so streams lose the 8-direction grid steps.
    Mouths, sources and junction cells of equal branches stay put."""
    rows, cols = stream.shape
    pos = np.stack(np.mgrid[0:rows, 0:cols]).astype(np.float32)
    idx = np.flatnonzero(stream.ravel())
    if not len(idx):
        return pos
    par = parent.ravel()[idx]
    ok = (par >= 0) & stream.ravel()[np.maximum(par, 0)]
    # Main upstream neighbour: the widest stream cell draining into each cell.
    up = np.full(rows * cols, -1, np.int64)
    best = np.zeros(rows * cols, np.float32)
    w = width.ravel()[idx]
    for i, p, wi, o in zip(idx, par, w, ok):
        if o and wi > best[p]:
            best[p], up[p] = wi, i
    child = up[idx]
    move = ok & (child >= 0)
    a, b, c = idx[move], par[move], child[move]
    flat = pos.reshape(2, -1)
    for _ in range(iterations):
        flat[:, a] = 0.5 * flat[:, a] + 0.25 * (flat[:, b] + flat[:, c])
    return pos


def draw_streams(stream, parent, width, k: int, cell_m: float, y0: int, x0: int, n: int,
                 pos: np.ndarray | None = None) -> np.ndarray:
    """Stream mask of the n x n window at (y0, x0), at k times the resolution: each stream
    cell joined to its downstream neighbour by a line as wide as its stream, at `pos`
    (see `smooth_paths`) or the cell centres."""
    cols = stream.shape[1]
    fine = np.zeros((n * k, n * k), bool)
    fine_m = cell_m / k
    ys, xs = np.nonzero(stream[y0 : y0 + n, x0 : x0 + n])
    for y, x in zip(ys + y0, xs + x0):
        p = parent[y, x]
        py, px = divmod(int(p), cols) if p >= 0 else (y, x)
        (sy, sx), (ey, ex) = ((pos[:, y, x], pos[:, py, px]) if pos is not None else ((y, x), (py, px)))
        r = max(0.5, width[y, x] / 2 / fine_m)
        for t in np.linspace(0, 1, max(2, int(k * 1.5))):
            cy = (sy + (ey - sy) * t - y0 + 0.5) * k
            cx = (sx + (ex - sx) * t - x0 + 0.5) * k
            ya, yb = int(max(cy - r, 0)), int(min(cy + r + 1, n * k))
            xa, xb = int(max(cx - r, 0)), int(min(cx + r + 1, n * k))
            if ya >= yb or xa >= xb:
                continue
            gy, gx = np.ogrid[ya:yb, xa:xb]
            fine[ya:yb, xa:xb] |= (gy + 0.5 - cy) ** 2 + (gx + 0.5 - cx) ** 2 <= r * r
    return fine


_OFFSETS = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (-1, 1), (1, -1), (1, 1)]


def extend_mouths(h: np.ndarray, water: np.ndarray, stream: np.ndarray, sinks: np.ndarray | None = None,
                  max_cells: int = 4000) -> np.ndarray:
    """Streams continued from where they stop short of water, cell by cell to the lowest
    unvisited neighbour, until they reach water (a refined coast or lake can leave a
    mouth stranded on land). Streams ending in `sinks` (karst) stay. Returns the new cells."""
    from scipy import ndimage

    rows, cols = h.shape
    labels, count = ndimage.label(stream & ~water, structure=np.ones((3, 3)))
    if not count:
        return np.zeros(h.shape, bool)
    near = ndimage.binary_dilation(water, np.ones((3, 3)))
    reached = np.zeros(count + 1, bool)
    reached[np.unique(labels[near & (labels > 0)])] = True
    if sinks is not None:
        reached[np.unique(labels[sinks & (labels > 0)])] = True
    lowest = ndimage.minimum_position(np.where(labels > 0, h, np.inf), labels, np.arange(1, count + 1))
    added = np.zeros(h.shape, bool)
    for k, (y, x) in enumerate(lowest, start=1):
        if reached[k]:
            continue
        seen = {(y, x)}
        path = []
        for _ in range(max_cells):
            best = None
            for dy, dx in _OFFSETS:
                yy, xx = y + dy, x + dx
                if 0 <= yy < rows and 0 <= xx < cols and (yy, xx) not in seen:
                    if best is None or h[yy, xx] < h[best]:
                        best = (yy, xx)
            if best is None:
                break
            y, x = best
            seen.add(best)
            if water[y, x] or (sinks is not None and sinks[y, x]):
                break
            path.append(best)
        else:
            continue  # no water within reach: leave this stream as it was
        for p in path:
            added[p] = True
    return added


def carve_streams(h: np.ndarray, stream: np.ndarray, water: np.ndarray) -> tuple[np.ndarray, int]:
    """Stream beds lowered so they descend all the way to the water they drain into
    (priority-flood carving along stream cells only). Returns (surface, cells lowered)."""
    rows, cols = h.shape
    out = h.astype(np.float64).ravel().copy()
    inside = stream.ravel() & ~water.ravel()
    seen = ~inside
    parent = np.full(out.size, -1, np.int64)
    from scipy import ndimage

    seeds = np.flatnonzero(inside & ndimage.binary_dilation(water, np.ones((3, 3))).ravel())
    seen[seeds] = True
    heap = [(out[i], int(i)) for i in seeds]
    heapq.heapify(heap)
    lowered = 0
    while heap:
        _, i = heapq.heappop(heap)
        r, c = divmod(i, cols)
        for dy, dx in _OFFSETS:
            rr, cc = r + dy, c + dx
            if 0 <= rr < rows and 0 <= cc < cols:
                j = rr * cols + cc
                if seen[j]:
                    continue
                seen[j] = True
                parent[j] = i
                target, k = out[j] - EPSILON_M, i
                while k >= 0 and out[k] > target:
                    out[k] = target
                    target -= EPSILON_M
                    lowered += 1
                    k = parent[k]
                heapq.heappush(heap, (out[j], j))
    return out.reshape(h.shape).astype(np.float32), lowered


def _maxpool3(a: np.ndarray) -> np.ndarray:
    p = np.pad(a, 1, mode="edge")
    return np.max([p[dy : dy + a.shape[0], dx : dx + a.shape[1]] for dy in range(3) for dx in range(3)], axis=0)


def carve(h: np.ndarray, outlet: np.ndarray) -> np.ndarray:
    """Priority-flood carving (Soille 2004): instead of filling a pit, lower the path from
    its spill cell back to the outlet so every cell drains."""
    rows, cols = h.shape
    hc = h.ravel().astype(np.float64).copy()
    parent = np.full(hc.size, -1, np.int64)
    seen = np.zeros(hc.size, bool)
    edge = np.zeros(h.shape, bool)
    edge[0, :] = edge[-1, :] = edge[:, 0] = edge[:, -1] = True
    heap = [(hc[i], int(i)) for i in np.flatnonzero((outlet | edge).ravel())]
    for _, i in heap:
        seen[i] = True
    heapq.heapify(heap)
    offsets = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (-1, 1), (1, -1), (1, 1)]
    push, pop = heapq.heappush, heapq.heappop
    while heap:
        _, i = pop(heap)
        r, c = divmod(i, cols)
        for dr, dc in offsets:
            rr, cc = r + dr, c + dc
            if 0 <= rr < rows and 0 <= cc < cols:
                j = rr * cols + cc
                if seen[j]:
                    continue
                seen[j] = True
                parent[j] = i
                target = hc[j] - EPSILON_M
                k = i
                while k >= 0 and hc[k] > target:
                    hc[k] = target
                    target -= EPSILON_M
                    k = parent[k]
                push(heap, (hc[j], j))
    return hc.reshape(h.shape)


def _hollows(h: np.ndarray, sea: np.ndarray):
    """Filled surface, depth and labelled closed depressions (scipy labels) of a surface."""
    from scipy import ndimage

    filled, _, _ = flood(h, sea)
    depth = filled - h
    labels, count = ndimage.label((depth > 0.05) & ~sea)
    return filled, depth, labels, count


def _pick(order, area, budget) -> list[int]:
    chosen, total = [], 0
    for k in order:
        if total + area[k] <= budget:
            chosen.append(k + 1)
            total += area[k]
    return chosen


def drain(h: np.ndarray, sea: np.ndarray, lake_share: float, widen_sigma: float, tarns: bool = False,
          keep_at: np.ndarray | None = None, dry_share: float = 0.0, dry_at: np.ndarray | None = None,
          dry_keep_at: np.ndarray | None = None):
    """Make a generated surface drain like real terrain.

    Depressions are kept as lakes (flat at their spill level) until they cover
    `lake_share` of the land: the largest by volume, or with `tarns` the deepest compact
    small ones (cirque tarns and kettles rather than valley-floor flats). Every other
    depression is carved through and the cut is widened into a valley floor. Depressions
    touching `keep_at` (authored lakes) are always kept and count toward the share.
    With `dry_share`, the deepest compact remaining depressions centred in `dry_at` stay
    closed and dry (dolines, craters, interdune hollows) up to that share of the land;
    depressions centred in `dry_keep_at` (painted craters) always stay closed and dry.
    Returns (surface, lake mask, lake level, dry basin mask)."""
    from scipy import ndimage

    from .data import smooth

    filled, depth, labels, count = _hollows(h, sea)
    keep = np.zeros(h.shape, bool)
    dry = np.zeros(h.shape, bool)
    if count:
        idx = np.arange(1, count + 1)
        area = ndimage.sum(np.ones(h.shape), labels, idx)
        budget = lake_share * (~sea).sum()
        forced = [] if keep_at is None else [int(k) for k in np.unique(labels[keep_at & (labels > 0)])]
        budget -= sum(area[k - 1] for k in forced)
        if tarns:
            deepest = ndimage.maximum(depth, labels, idx)
            span = np.array([max(s[0].stop - s[0].start, s[1].stop - s[1].start) for s in ndimage.find_objects(labels)])
            ok = (area >= TARN_CELLS[0]) & (area <= TARN_CELLS[1]) & (deepest >= TARN_MIN_DEPTH_M)
            ok &= span**2 / area <= TARN_MAX_ELONGATION
            order = [k for k in np.argsort(-deepest) if ok[k]]
        else:
            order = np.argsort(-ndimage.sum(depth, labels, idx))
        if dry_at is not None:  # hollows of dry classes (craters, dolines, interdunes) stay dry, not lakes
            wet_ok = ndimage.mean(dry_at.astype(np.float32), labels, idx) <= 0.5
            order = [k for k in order if wet_ok[k]]
        lakes = _pick([k for k in order if k + 1 not in forced], area, budget) + forced
        keep = np.isin(labels, lakes)
        if dry_share > 0:
            deepest = ndimage.maximum(depth, labels, idx)
            span = np.array([max(s[0].stop - s[0].start, s[1].stop - s[1].start) for s in ndimage.find_objects(labels)])
            ok = (deepest >= TARN_MIN_DEPTH_M) & (span**2 / area <= TARN_MAX_ELONGATION)
            if dry_at is not None:
                ok &= ndimage.mean(dry_at.astype(np.float32), labels, idx) > 0.5
            order = [k for k in np.argsort(-deepest) if ok[k] and k + 1 not in lakes]
            dry = np.isin(labels, _pick(order, area, dry_share * (~sea).sum()))
        if dry_keep_at is not None:
            craters = ndimage.mean(dry_keep_at.astype(np.float32), labels, idx) > 0.5
            dry |= np.isin(labels, [k + 1 for k in np.flatnonzero(craters) if k + 1 not in lakes])
    work = np.where(keep, filled, h)
    cut = work - carve(work, sea | keep | dry)
    widened = np.maximum(cut, smooth(_maxpool3(cut).astype(np.float32), widen_sigma))
    widened[dry] = 0
    out = np.where(keep, filled, work - widened).astype(np.float32)
    return out, keep, np.where(keep, filled, np.nan).astype(np.float32), dry
