"""Lakes and streams on a generated surface: priority-flood depression filling with a
tiny gradient (Barnes et al. 2014) gives drainage order, lakes and flow accumulation.

Game layers, not hydraulics: lakes are filled depressions above a size and depth, and
streams follow drainage areas above a threshold with a width growing with that area.
"""

from __future__ import annotations

import heapq

import numpy as np

EPSILON_M = 1e-3  # gradient across filled flats so every cell drains
STREAM_MIN_AREA_KM2 = 4.0
# Width (m) = STREAM_WIDTH_M * sqrt(drainage area / km2), capped.
STREAM_WIDTH_M = 2.5
STREAM_MAX_WIDTH_M = 60.0


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


def _components(mask: np.ndarray) -> tuple[np.ndarray, int]:
    """4-connected labels (0 = background) by iterative minimum propagation."""
    labels = np.where(mask, np.arange(1, mask.size + 1).reshape(mask.shape), 0)
    big = mask.size + 1
    while True:
        work = np.where(mask, labels, big)
        best = work.copy()
        best[1:] = np.minimum(best[1:], work[:-1])
        best[:-1] = np.minimum(best[:-1], work[1:])
        best[:, 1:] = np.minimum(best[:, 1:], work[:, :-1])
        best[:, :-1] = np.minimum(best[:, :-1], work[:, 1:])
        best = np.where(mask, best, 0)
        if (best == labels).all():
            break
        labels = best
    ids, labels = np.unique(labels, return_inverse=True)
    return labels.reshape(mask.shape), len(ids) - 1


def streams(h: np.ndarray, water: np.ndarray, cell_m: float):
    """Stream cells, downstream neighbour (flat index) and width (m) on a draining surface."""
    _, parent, order = flood(h, water)
    area_km2 = accumulate(parent, order).reshape(h.shape) * cell_m * cell_m / 1e6
    stream = (area_km2 >= STREAM_MIN_AREA_KM2) & ~water
    width = np.clip(STREAM_WIDTH_M * np.sqrt(area_km2), 0, STREAM_MAX_WIDTH_M).astype(np.float32)
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


def drain(h: np.ndarray, sea: np.ndarray, lake_share: float, widen_sigma: float):
    """Make a generated surface drain like real terrain.

    The largest depressions by volume are kept as lakes (flat at their spill level) until
    they cover `lake_share` of the land; every other depression is carved through and the
    cut is widened into a valley floor. Returns (surface, lake mask, lake level)."""
    from .data import smooth

    filled, _, _ = flood(h, sea)
    depth = filled - h
    labels, count = _components((depth > 0.05) & ~sea)
    keep = np.zeros(h.shape, bool)
    if count:
        flat = labels.ravel()
        area = np.bincount(flat, minlength=count + 1)[1:]
        volume = np.bincount(flat, weights=depth.ravel(), minlength=count + 1)[1:]
        budget = lake_share * (~sea).sum()
        chosen, total = [], 0
        for k in np.argsort(-volume):
            if total + area[k] > budget:
                continue
            chosen.append(k + 1)
            total += area[k]
        keep = np.isin(labels, chosen)
    work = np.where(keep, filled, h)
    cut = work - carve(work, sea | keep)
    widened = np.maximum(cut, smooth(_maxpool3(cut).astype(np.float32), widen_sigma))
    out = np.where(keep, filled, work - widened).astype(np.float32)
    return out, keep, np.where(keep, filled, np.nan).astype(np.float32)
