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


class TensorField:
    """Conditioning held as one [1, C, H, W] tensor; the field lives on its device."""

    def __init__(self, cond: torch.Tensor):
        self.cond, self.shape, self.device = cond, tuple(cond.shape[-2:]), cond.device

    def windows(self, boxes, size, device):
        return torch.cat([self.cond[:, :, y : y + size, x : x + size] for y, x in boxes]).to(device)


def _shape(cs: torch.Tensor, conditioning: int) -> torch.Tensor:
    """Per-window channel 0, as in training: re-centred (1 m layouts) or unit range
    from raw coarse metres (cascade layouts)."""
    c = cs[:, 0]
    if conditioning >= 3:
        from .cascade import MIN_RANGE_M

        lo, hi = c.amin(dim=(1, 2), keepdim=True), c.amax(dim=(1, 2), keepdim=True)
        cs[:, 0] = (c - lo) / (hi - lo).clamp(min=MIN_RANGE_M) * 2 - 1
    else:
        cs[:, 0] = c - c.mean(dim=(1, 2), keepdim=True)
    return cs


@torch.no_grad()
def denoise_field(model, x, sigma, cond, window, stride, batch, guidance=1.0, negative=None):
    """One MultiDiffusion evaluation: average window predictions over the field.

    `cond` and `negative` provide conditioning windows (see TensorField); only a batch of
    windows is on the model's device at a time, so the field may stay in host memory.
    Guidance pushes away from `negative` or, without it, from the class-dropped condition."""
    _, _, h, w = x.shape
    device = next(model.parameters()).device
    out = torch.zeros_like(x)
    size = min(window, h, w)
    wt = _weights(size, x.device)
    ys, xs_ = _starts(h, window, stride), _starts(w, window, stride)
    # The window grid is regular and the weights separable, so their sum is an outer product.
    ramp = wt[0, 0, :, 0] / wt[0, 0, :, 0].max()
    rows, cols = torch.zeros(h, device=x.device), torch.zeros(w, device=x.device)
    for y in ys:
        rows[y : y + size] += ramp
    for xx in xs_:
        cols[xx : xx + size] += ramp
    boxes = [(y, xx) for y in ys for xx in xs_]
    for i in range(0, len(boxes), batch):
        part = boxes[i : i + batch]
        xs = torch.cat([x[:, :, y : y + size, xx : xx + size] for y, xx in part]).to(device)
        cs = _shape(cond.windows(part, size, device), model.conditioning)
        if negative is not None:
            ns = _shape(negative.windows(part, size, device), model.conditioning)
        sig = torch.full((len(part),), sigma, device=device)
        with torch.autocast(device.type, dtype=torch.bfloat16, enabled=device.type == "cuda"):
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
        d = d.to(x.device)
        for j, (y, xx) in enumerate(part):
            out[:, :, y : y + size, xx : xx + size] += d[j : j + 1] * wt
    out /= rows[:, None]
    out /= cols[None, :]
    return out


@torch.no_grad()
def sample_field(model, cond, steps=32, window=256, stride=128, batch=8, seed=0, guidance=1.0, negative=None):
    """Heun EDM sampling over a field of any size.

    cond: [1, C, H, W] tensor or a field with `shape`, `device` and `windows()`."""
    if isinstance(cond, torch.Tensor):
        cond = TensorField(cond)
    if isinstance(negative, torch.Tensor):
        negative = TensorField(negative)
    gen = torch.Generator(device="cpu").manual_seed(seed)
    h, w = cond.shape
    sigmas = sigma_schedule(steps)
    x = torch.randn((1, model.target_channels, h, w), generator=gen).to(cond.device) * sigmas[0]
    # In-place updates keep at most four field-sized tensors alive.
    for i in range(steps):
        s, s_next = sigmas[i], sigmas[i + 1]
        d = denoise_field(model, x, s, cond, window, stride, batch, guidance, negative)
        d.sub_(x).div_(-s)  # d = (x - D) / s
        if s_next > 0:
            x_next = x + (s_next - s) * d
            d2 = denoise_field(model, x_next, s_next, cond, window, stride, batch, guidance, negative)
            d2.sub_(x_next).div_(-s_next)
            del x_next
            d.add_(d2)
            del d2
            x.add_(d, alpha=(s_next - s) * 0.5)
        else:
            x.add_(d, alpha=s_next - s)
        del d
    return x


def parameter_count(model: nn.Module) -> int:
    return sum(p.numel() for p in model.parameters())
