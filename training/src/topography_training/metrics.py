"""Physical reconstruction errors on paired scalar Float32 metre grids."""
import math

import numpy as np


def compare_terrain(height: np.ndarray, decoded: np.ndarray, valid: np.ndarray,
                    spacing_m: list[float]) -> dict:
    """Compare [row,column] metres; height/slope metrics exclude masked cells/pairs."""
    if (height.dtype != np.dtype("float32") or decoded.dtype != np.dtype("float32")
            or height.ndim != 2 or decoded.shape != height.shape
            or valid.shape != height.shape or valid.dtype != np.dtype("bool")):
        raise ValueError("expected paired Float32 grids and aligned Boolean validity")
    if len(spacing_m) != 2 or any(not math.isfinite(n) or n <= 0 for n in spacing_m):
        raise ValueError("invalid metric spacing")
    if not np.any(valid):
        raise ValueError("all-masked numeric input")
    if not np.all(np.isfinite(height[valid])) or not np.all(np.isfinite(decoded[valid])):
        raise ValueError("nonfinite valid elevation")
    error = decoded[valid].astype(np.float64) - height[valid].astype(np.float64)
    sum_squared = maximum = 0.0
    pairs = nonflat = newly_flat = 0
    for axis, spacing in [(0, spacing_m[1]), (1, spacing_m[0])]:
        use = valid[1:, :] & valid[:-1, :] if axis == 0 else valid[:, 1:] & valid[:, :-1]
        original_delta = np.diff(height.astype(np.float64), axis=axis)[use]
        decoded_delta = np.diff(decoded.astype(np.float64), axis=axis)[use]
        slope_error = (decoded_delta - original_delta) / spacing
        if slope_error.size:
            maximum = max(maximum, float(np.max(np.abs(slope_error))))
            sum_squared += float(np.sum(slope_error**2))
        pairs += int(use.sum())
        nonflat += int(np.count_nonzero(original_delta != 0))
        newly_flat += int(np.count_nonzero((original_delta != 0) & (decoded_delta == 0)))
    return {"max_height_error_m": float(np.max(np.abs(error))),
            "height_rmse_m": float(np.sqrt(np.mean(error**2))),
            "max_slope_component_error_m_per_m": maximum if pairs else None,
            "slope_component_rmse_m_per_m": math.sqrt(sum_squared / pairs) if pairs else None,
            "valid_neighbor_pairs": pairs, "nonflat_pairs": nonflat,
            "newly_flat_pairs": newly_flat,
            "bit_identical_valid_samples": bool(np.array_equal(height[valid].view(np.uint32), decoded[valid].view(np.uint32)))}
