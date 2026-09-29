"""EDM-preconditioned U-Net and MultiDiffusion sampler (Karras et al. 2022)."""

from __future__ import annotations

import torch
from torch import nn

from .data import channels, class_start

SIGMA_DATA = 0.5
DEFAULT_ARCH = {
    "block_out_channels": [64, 128, 256, 384, 512],
    "layers_per_block": 2,
    "attention_levels": 2,
}


class Denoiser(nn.Module):
    """D(x; sigma, cond) with EDM input/output/skip scaling."""

    def __init__(self, arch: dict | None = None, conditioning: int = 1):
        super().__init__()
        cond_channels, target_channels = channels(conditioning)
        self.conditioning, self.target_channels = conditioning, target_channels
        from diffusers import UNet2DModel

        arch = {**DEFAULT_ARCH, **(arch or {})}
        levels = len(arch["block_out_channels"])
        attn = arch["attention_levels"]
        self.unet = UNet2DModel(
            in_channels=target_channels + cond_channels,
            out_channels=target_channels,
            block_out_channels=tuple(arch["block_out_channels"]),
            layers_per_block=arch["layers_per_block"],
            down_block_types=tuple(
                "AttnDownBlock2D" if i >= levels - attn else "DownBlock2D" for i in range(levels)
            ),
            up_block_types=tuple(
                "AttnUpBlock2D" if i < attn else "UpBlock2D" for i in range(levels)
            ),
            attention_head_dim=64,
            norm_num_groups=32,
        )

    def forward(self, x, sigma, cond):
        sigma = sigma.reshape(-1, 1, 1, 1).to(x.dtype)
        s2 = sigma**2 + SIGMA_DATA**2
        c_skip = SIGMA_DATA**2 / s2
        c_out = sigma * SIGMA_DATA / s2.sqrt()
        c_in = 1 / s2.sqrt()
        c_noise = 250.0 * sigma.log().flatten()
        f = self.unet(torch.cat([c_in * x, cond], dim=1), c_noise).sample
        return c_skip * x + c_out * f.to(x.dtype)


def edm_loss(model, target, cond, mask, p_mean=-1.2, p_std=1.2):
    """Masked EDM loss; mask weights each target channel's known cells."""
    sigma = (torch.randn(target.shape[0], device=target.device) * p_std + p_mean).exp()
    noisy = target + torch.randn_like(target) * sigma.reshape(-1, 1, 1, 1)
    denoised = model(noisy, sigma, cond)
    weight = ((sigma**2 + SIGMA_DATA**2) / (sigma * SIGMA_DATA) ** 2).reshape(-1, 1, 1, 1)
    err = weight * (denoised.float() - target) ** 2 * mask
    return err.sum() / mask.sum().clamp(min=1)


def sigma_schedule(steps: int, sigma_min=0.002, sigma_max=80.0, rho=7.0) -> list[float]:
    i = torch.arange(steps, dtype=torch.float64)
    s = (sigma_max ** (1 / rho) + i / (steps - 1) * (sigma_min ** (1 / rho) - sigma_max ** (1 / rho))) ** rho
    return s.tolist() + [0.0]


def _weights(size: int, device) -> torch.Tensor:
    """Separable tent weights so overlapping windows blend smoothly."""
    ramp = torch.minimum(torch.arange(size) + 0.5, size - torch.arange(size) - 0.5)
    w = (ramp / ramp.max()).clamp(min=1e-3).to(device)
    return (w[:, None] * w[None, :])[None, None]


def _starts(length: int, window: int, stride: int) -> list[int]:
    if length <= window:
        return [0]
    s = list(range(0, length - window + 1, stride))
    return s if s[-1] == length - window else s + [length - window]


@torch.no_grad()
def denoise_field(model, x, sigma, cond, window, stride, batch, guidance=1.0, negative=None):
    """One MultiDiffusion evaluation: average window predictions over the field.

    Guidance pushes away from `negative` (same shape as cond) or, without it, from
    the class-dropped condition."""
    _, _, h, w = x.shape
    out = torch.zeros_like(x)
    total = torch.zeros((1, 1, h, w), device=x.device)
    wt = _weights(min(window, h, w), x.device)
    boxes = [(y, xx) for y in _starts(h, window, stride) for xx in _starts(w, window, stride)]
    for i in range(0, len(boxes), batch):
        part = boxes[i : i + batch]
        size = min(window, h, w)
        xs = torch.cat([x[:, :, y : y + size, xx : xx + size] for y, xx in part])
        cs = torch.cat([cond[:, :, y : y + size, xx : xx + size] for y, xx in part])
        # Coarse-relative channel is re-centred per window, as in training.
        cs[:, 0] -= cs[:, 0].mean(dim=(1, 2), keepdim=True)
        if negative is not None:
            ns = torch.cat([negative[:, :, y : y + size, xx : xx + size] for y, xx in part])
            ns[:, 0] -= ns[:, 0].mean(dim=(1, 2), keepdim=True)
        sig = torch.full((len(part),), sigma, device=x.device)
        with torch.autocast(x.device.type, dtype=torch.bfloat16, enabled=x.device.type == "cuda"):
            d = model(xs, sig, cs).float()
            if guidance != 1.0:
                if negative is not None:
                    uncond = ns
                else:
                    uncond, k = cs.clone(), class_start(cs)
                    uncond[:, k:] = 0
                    uncond[:, k] = 1
                du = model(xs, sig, uncond).float()
                d = du + guidance * (d - du)
        for j, (y, xx) in enumerate(part):
            out[:, :, y : y + size, xx : xx + size] += d[j : j + 1] * wt
            total[:, :, y : y + size, xx : xx + size] += wt
    return out / total


@torch.no_grad()
def sample_field(model, cond, steps=32, window=256, stride=128, batch=8, seed=0, guidance=1.0, negative=None):
    """Heun EDM sampling over a field of any size. cond: [1, C, H, W] on device."""
    gen = torch.Generator(device="cpu").manual_seed(seed)
    _, _, h, w = cond.shape
    sigmas = sigma_schedule(steps)
    x = torch.randn((1, model.target_channels, h, w), generator=gen).to(cond.device) * sigmas[0]
    for i in range(steps):
        s, s_next = sigmas[i], sigmas[i + 1]
        d = (x - denoise_field(model, x, s, cond, window, stride, batch, guidance, negative)) / s
        x_next = x + (s_next - s) * d
        if s_next > 0:
            d2 = (x_next - denoise_field(model, x_next, s_next, cond, window, stride, batch, guidance, negative)) / s_next
            x_next = x + (s_next - s) * 0.5 * (d + d2)
        x = x_next
    return x


def parameter_count(model: nn.Module) -> int:
    return sum(p.numel() for p in model.parameters())
