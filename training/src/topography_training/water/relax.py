"""Bank relaxation inside the 1 m channel corridor (hydrology v3), on the GPU.

Only dry corridor cells (corridor.u8 == 1) move, and only down. Each of ITERATIONS Jacobi
steps smooths their change from raw toward the mean change of their eight neighbours (the
1 m detail stays, so no planar facets form) and cuts back any slope to a neighbour steeper
than max(LIMIT_DEG, the cell's raw slope), never below the water beside them, their
receiver + RECEIVER_M (strictly above it, so drainage still holds and no flat forms; review
I7), or raw - MAX_LOWER_M tapered to 0 over TAPER_CELLS at the corridor's edge.
A per-neighbour cone still lets a cell's Horn slope pass raw, so a repair then puts back (to
the input) every changed bank that fails the Rust `walls` check (Horn slope >
max(WALL_DEG, raw)) and the moving neighbours of every carved cell that now fails it, and
raises lowered cells back above their receivers, until nothing fails (R11, I7).
The stencil is shifted slices of an edge-padded tensor combined in a fixed order, never
conv2d (whose MIOpen algorithm depends on the shape), with deterministic algorithms on; no
atomics, no reductions across cells. A Jacobi step reaches one cell and a repair pass three,
so a cell's result depends only on cells within EDGE_EFFECT of it, and tiles with
HALO >= EDGE_EFFECT agree on their overlaps.
"""

from __future__ import annotations

import math

import numpy as np
import torch
from scipy import ndimage

ITERATIONS = 48
RADIUS = 1
TAPER_CELLS = 8
REPAIR_PASSES = 32  # more is a hard failure (the canyon probe settles in 8)
REPAIR_RADIUS = 3  # a pass: Horn slope (1), the carved cell's neighbours (2), their donors (3)
# How deep a window edge can change results.
EDGE_EFFECT = ITERATIONS * RADIUS + TAPER_CELLS + REPAIR_PASSES * REPAIR_RADIUS
HALO = EDGE_EFFECT + 8
STRIP = HALO - EDGE_EFFECT  # overlap cells outside a tile's core that are still exact
LIMIT_DEG = 38.0
ALPHA = 0.25
MAX_LOWER_M = 2.5  # the 2 m channel cap + 0.5 m (the change budget's max)
RECEIVER_M = 0.001  # a lowered cell stays at least this far above its receiver (strict)
WALL_DEG = 40.0  # the Rust `walls` check: a changed cell no steeper than max(this, raw)
CHANGED_M = 0.01  # where a cell counts as changed
# (row, column) offsets in flow.u8 order: 1 N, 2 S, 3 W, 4 E, 5 NW, 6 NE, 7 SW, 8 SE.
OFFSETS = ((-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (-1, 1), (1, -1), (1, 1))
# Horn weights (x east, y south) of each neighbour: edges 2/8, corners 1/8 on each axis.
HORN = tuple((dx * (2 - abs(dy)) / 8.0, dy * (2 - abs(dx)) / 8.0) for dy, dx in OFFSETS)

assert HALO >= ITERATIONS * RADIUS and STRIP > 0


def _shift(a: torch.Tensor, dy: int, dx: int) -> torch.Tensor:
    """a[y + dy, x + dx] with the edges repeated."""
    p = torch.nn.functional.pad(a[None, None], (1, 1, 1, 1), mode="replicate")[0, 0]
    h, w = a.shape
    return p[1 + dy : 1 + dy + h, 1 + dx : 1 + dx + w]


def _slope2(z: torch.Tensor, spacing_m: float) -> torch.Tensor:
    """Squared Horn gradient, in float64 as the Rust `walls` check computes it."""
    nbs = [_shift(z.double(), dy, dx) for dy, dx in OFFSETS]
    gx = sum(HORN[k][0] * n for k, n in enumerate(nbs)) / spacing_m
    gy = sum(HORN[k][1] * n for k, n in enumerate(nbs)) / spacing_m
    return gx * gx + gy * gy


def relax(ground: np.ndarray, raw: np.ndarray, surface: np.ndarray, corridor: np.ndarray, flow: np.ndarray,
          spacing_m: float = 1.0, device: str | None = None) -> np.ndarray:
    """The relaxed ground (float32) of one window."""
    move_np = corridor == 1
    if not move_np.any():
        return np.asarray(ground, np.float32).copy()
    dev = torch.device(device or ("cuda" if torch.cuda.is_available() else "cpu"))
    torch.use_deterministic_algorithms(True)
    t = lambda a, dtype=torch.float32: torch.from_numpy(np.ascontiguousarray(a)).to(dev, dtype)  # noqa: E731
    g, r = t(np.asarray(ground, np.float32)), t(np.asarray(raw, np.float32))
    ws = t(np.where(np.isfinite(surface), surface, -np.inf).astype(np.float32))
    move = t(move_np, torch.bool)
    direction = t(np.asarray(flow) & 15, torch.int64)
    dist = [spacing_m * (math.sqrt(2.0) if dy and dx else 1.0) for dy, dx in OFFSETS]
    # Static limits: the slope allowed (never past raw), the water beside, the taper.
    raw_slope = torch.stack([(r - _shift(r, dy, dx)) / d for (dy, dx), d in zip(OFFSETS, dist)]).amax(0)
    limit = torch.clamp(raw_slope, min=math.tan(math.radians(LIMIT_DEG)))
    bank = torch.stack([_shift(ws, dy, dx) for dy, dx in OFFSETS]).amax(0)
    edge = ndimage.distance_transform_cdt(corridor > 0, metric="chessboard").astype(np.float32)
    taper = t(np.clip(edge / TAPER_CELLS, 0.0, 1.0))
    floor = torch.maximum(bank, r - taper * MAX_LOWER_M)
    weights = [1.0 / d for d in dist]
    total = sum(weights)

    def receiver(z: torch.Tensor) -> torch.Tensor:
        recv = torch.full_like(z, -math.inf)
        for k, (dy, dx) in enumerate(OFFSETS):
            recv = torch.where(direction == k + 1, _shift(z, dy, dx), recv)
        return recv

    raw_mean = sum(w * _shift(r, dy, dx) for w, (dy, dx) in zip(weights, OFFSETS)) / total
    u = g
    for _ in range(ITERATIONS):
        nbs = [_shift(u, dy, dx) for dy, dx in OFFSETS]
        # Fixed-order sum of shifted slices (no conv2d): the same bits in any window.
        mean = sum(w * n for w, n in zip(weights, nbs)) / total
        # Smooth the lowering, not the ground: the 1 m detail stays, so no planar facets form.
        smooth = u + ALPHA * ((mean - u) - (raw_mean - r))
        cone = torch.stack([n + limit * d for n, d in zip(nbs, dist)]).amin(0)
        target = torch.minimum(u, torch.minimum(smooth, cone))
        low = torch.minimum(torch.maximum(floor, receiver(u) + RECEIVER_M), u)  # strictly above the receiver; never raise
        u = torch.where(move, torch.maximum(target, low), u)

    # Repair toward the input (only ever up, so it ends). Walls limit: max(WALL_DEG, raw);
    # a carved cell already steeper keeps its input slope as its limit.
    wall2 = torch.clamp(_slope2(r, spacing_m), min=math.tan(math.radians(WALL_DEG)) ** 2)
    carved = ~move & ((g - r).abs() > CHANGED_M)
    limit2 = torch.where(carved, torch.maximum(wall2, _slope2(g, spacing_m)), wall2)
    for _ in range(REPAIR_PASSES):
        bad = ((u - r).abs() > CHANGED_M) & (_slope2(u, spacing_m) > limit2)
        beside = torch.stack([_shift(bad & carved, dy, dx) for dy, dx in OFFSETS]).any(0)
        z = torch.where(move & (bad | beside), g, u)
        lift = torch.minimum(g, receiver(z) + RECEIVER_M)
        z = torch.where(move & (z < lift), lift, z)
        if torch.equal(z, u):
            return u.cpu().numpy()
        u = z
    raise RuntimeError(f"bank relaxation: walls/receiver repair did not settle in {REPAIR_PASSES} passes")
