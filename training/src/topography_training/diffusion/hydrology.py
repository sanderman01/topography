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


# By class id (0: unlabelled). Dry classes keep valley floors as dry washes.
WATER = {
    0: Water(0.010, 0.002, 8.0),
    1: Water(0.003, 0.000, 20.0, 1.5),  # karst: streams sink
    2: Water(0.002, 0.000, 50.0, 1.0),  # gorges and canyons
    3: Water(0.020, 0.005, 4.0),  # fjords and glacial troughs
    4: Water(0.010, 0.002, 10.0),  # calderas and craters
    5: Water(0.020, 0.003, 4.0),  # archipelagos
    6: Water(0.006, 0.005, 4.0),  # alpine ridges and cirques
    7: Water(0.002, 0.000, 50.0, 1.0),  # badlands and mesas
    8: Water(0.001, 0.000, 200.0, 0.5),  # dunes
    9: Water(0.010, 0.001, 4.0),  # braided rivers
    10: Water(0.015, 0.005, 4.0),  # glaciers and ice fields
    11: Water(0.005, 0.002, 10.0),  # volcanic cones and lava fields
    12: Water(0.008, 0.001, 4.0),  # coastal cliffs
}

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
            width_m: float = STREAM_WIDTH_M):
    """Stream cells, downstream neighbour (flat index) and width (m) on a draining surface."""
    _, parent, order = flood(h, water)
    area_km2 = accumulate(parent, order).reshape(h.shape) * cell_m * cell_m / 1e6
    stream = (area_km2 >= min_km2) & ~water
    width = np.clip(width_m * np.sqrt(area_km2), 0, STREAM_MAX_WIDTH_M).astype(np.float32)
    return stream, parent.reshape(h.shape), width


def draw_streams(stream, parent, width, k: int, cell_m: float, y0: int, x0: int, n: int) -> np.ndarray:
    """Stream mask of the n x n window at (y0, x0), at k times the resolution: each stream
    cell joined to its downstream neighbour by a line as wide as its stream."""
    cols = stream.shape[1]
    fine = np.zeros((n * k, n * k), bool)
    fine_m = cell_m / k
    ys, xs = np.nonzero(stream[y0 : y0 + n, x0 : x0 + n])
    for y, x in zip(ys + y0, xs + x0):
        p = parent[y, x]
        py, px = divmod(int(p), cols) if p >= 0 else (y, x)
        r = max(0.5, width[y, x] / 2 / fine_m)
        for t in np.linspace(0, 1, max(2, int(k * 1.5))):
            cy = (y + (py - y) * t - y0 + 0.5) * k
            cx = (x + (px - x) * t - x0 + 0.5) * k
            ya, yb = int(max(cy - r, 0)), int(min(cy + r + 1, n * k))
            xa, xb = int(max(cx - r, 0)), int(min(cx + r + 1, n * k))
            if ya >= yb or xa >= xb:
                continue
            gy, gx = np.ogrid[ya:yb, xa:xb]
            fine[ya:yb, xa:xb] |= (gy + 0.5 - cy) ** 2 + (gx + 0.5 - cx) ** 2 <= r * r
    return fine


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
          keep_at: np.ndarray | None = None):
    """Make a generated surface drain like real terrain.

    Depressions are kept as lakes (flat at their spill level) until they cover
    `lake_share` of the land: the largest by volume, or with `tarns` the deepest compact
    small ones (cirque tarns and kettles rather than valley-floor flats). Every other
    depression is carved through and the cut is widened into a valley floor. Depressions
    touching `keep_at` (authored lakes) are always kept and count toward the share.
    Returns (surface, lake mask, lake level)."""
    from scipy import ndimage

    from .data import smooth

    filled, depth, labels, count = _hollows(h, sea)
    keep = np.zeros(h.shape, bool)
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
        keep = np.isin(labels, _pick([k for k in order if k + 1 not in forced], area, budget) + forced)
    work = np.where(keep, filled, h)
    cut = work - carve(work, sea | keep)
    widened = np.maximum(cut, smooth(_maxpool3(cut).astype(np.float32), widen_sigma))
    out = np.where(keep, filled, work - widened).astype(np.float32)
    return out, keep, np.where(keep, filled, np.nan).astype(np.float32)
