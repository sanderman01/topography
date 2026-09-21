"""Fixed small Float32 scalar autoencoder and fitting-window schedule for one trial."""
from __future__ import annotations

import random

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    import torch
    from torch import nn


def build_model() -> nn.Module:
    """Map N×1×H×W normalized heights through a four-channel quarter-grid latent."""
    from torch import nn
    return nn.Sequential(
        nn.Conv2d(1, 16, 3, padding=1), nn.SiLU(),
        nn.Conv2d(16, 32, 4, stride=2, padding=1), nn.SiLU(),
        nn.Conv2d(32, 4, 4, stride=2, padding=1),
        nn.Conv2d(4, 32, 3, padding=1), nn.SiLU(),
        nn.Upsample(scale_factor=2, mode="bilinear", align_corners=False),
        nn.Conv2d(32, 16, 3, padding=1), nn.SiLU(),
        nn.Upsample(scale_factor=2, mode="bilinear", align_corners=False),
        nn.Conv2d(16, 1, 3, padding=1),
    ).float()


def masked_loss(prediction: torch.Tensor, target: torch.Tensor, valid: torch.Tensor) -> torch.Tensor:
    """Mean squared scalar error; excluded targets/predictions contribute no gradient."""
    import torch
    if prediction.shape != target.shape or valid.shape != target.shape or valid.dtype != torch.bool:
        raise ValueError("loss shape/mask mismatch")
    if not valid.any():
        raise ValueError("all-masked fitting window")
    error = prediction[valid] - target[valid]
    if not torch.isfinite(error).all():
        raise ValueError("nonfinite fitting error")
    return error.square().mean()


def window_schedule(regions: list[dict]) -> list[dict]:
    """Select roles before cropping; round-robin fitting regions with seeded native offsets."""
    fitting = [r for r in regions if r["role"] == "calibration"]
    if not fitting or any(min(r["grid"]["size"]) < 512 for r in fitting):
        raise ValueError("no fitting regions or undersized fitting grid")
    rng = random.Random(0)
    return [{"step": step + 1, "prepared_sha256": r["prepared_sha256"],
             "column": rng.randrange(r["grid"]["size"][0] - 511),
             "row": rng.randrange(r["grid"]["size"][1] - 511)}
            for step in range(200) for r in [fitting[step % len(fitting)]]]
