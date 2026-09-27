"""Train the residual terrain denoiser.

python -m topography_training.diffusion.train --release <manifest.json> --run-dir <dir> [options]

The run directory holds config.json, log.jsonl, checkpoint.pt (resumable) and
ema-<step>.safetensors. Rerunning with the same run directory resumes.
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import sys
import time
from pathlib import Path

import numpy as np


def _parse(argv):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--release", type=Path, required=True)
    p.add_argument("--run-dir", type=Path, required=True)
    p.add_argument("--steps", type=int, default=100_000)
    p.add_argument("--hours", type=float, default=8.0, help="wall-clock limit for this invocation")
    p.add_argument("--batch", type=int, default=12)
    p.add_argument("--crop", type=int, default=256)
    p.add_argument("--lr", type=float, default=2e-4)
    p.add_argument("--warmup", type=int, default=1000)
    p.add_argument("--ema", type=float, default=0.999)
    p.add_argument("--class-dropout", type=float, default=0.1)
    p.add_argument("--channels", type=str, default="64,128,256,384,512")
    p.add_argument("--val-every", type=int, default=1000)
    p.add_argument("--save-every", type=int, default=2000)
    p.add_argument("--log-every", type=int, default=50)
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--no-checkpointing", action="store_true", help="faster, ~2x VRAM")
    return p.parse_args(argv)


def _atomic_save(obj, path: Path, save):
    tmp = path.with_suffix(path.suffix + ".tmp")
    save(obj, tmp)
    os.replace(tmp, path)


def main(argv=None) -> int:
    args = _parse(argv)
    import torch
    from safetensors.torch import save_file

    from . import data
    from .model import Denoiser, edm_loss, parameter_count

    if not torch.cuda.is_available():
        print("error: no GPU visible to PyTorch", file=sys.stderr)
        return 2
    device = torch.device("cuda")
    run = args.run_dir
    run.mkdir(parents=True, exist_ok=True)
    ckpt_path = run / "checkpoint.pt"

    tiles = data.load_release(args.release)
    config_path = run / "config.json"
    if config_path.exists():
        config = json.loads(config_path.read_text())
    else:
        config = {
            "schema_version": 1,
            "kind": "terrain_diffusion_run",
            "release": str(args.release.resolve()),
            "residual_scale_m": data.residual_scale(tiles),
            "coarse_m": data.COARSE_M,
            "coarse_smooth_m": data.SMOOTH_M,
            "coarse_offset": "random per crop",
            "water_smooth_m": data.release_water_smooth_m(json.loads(args.release.read_text())),
            "crop": args.crop,
            "arch": {"block_out_channels": [int(c) for c in args.channels.split(",")]},
            "class_dropout": args.class_dropout,
            "seed": args.seed,
            "torch": torch.__version__,
            "hip": torch.version.hip,
            "device": torch.cuda.get_device_name(0),
        }
        config_path.write_text(json.dumps(config, indent=2))
    scale = config["residual_scale_m"]

    smooth_m = config.get("coarse_smooth_m", 0.0)
    train = data.CropSampler(tiles, data.TRAIN, config["crop"], smooth_m=smooth_m)
    val = data.CropSampler(tiles, data.VALIDATION, config["crop"], smooth_m=smooth_m)
    print(f"tiles={len(tiles)} train_positions={len(train.positions)} val_positions={len(val.positions)} scale={scale} m", flush=True)

    model = Denoiser(config["arch"]).to(device)
    if not args.no_checkpointing:
        model.unet.enable_gradient_checkpointing()
    ema = copy.deepcopy(model).eval().requires_grad_(False)
    opt = torch.optim.AdamW(model.parameters(), lr=args.lr, weight_decay=0.0)
    step = 0
    if ckpt_path.exists():
        state = torch.load(ckpt_path, map_location=device, weights_only=False)
        model.load_state_dict(state["model"])
        ema.load_state_dict(state["ema"])
        opt.load_state_dict(state["opt"])
        step = state["step"]
        rng = np.random.default_rng()
        rng.bit_generator.state = state["rng"]
        print(f"resumed at step {step}", flush=True)
    else:
        rng = np.random.default_rng(args.seed)
    print(f"parameters={parameter_count(model) / 1e6:.1f}M", flush=True)

    # Fixed validation crops, noise levels and noise so losses are comparable.
    vrng = np.random.default_rng(1234)
    vt, vc, vm = (torch.from_numpy(a).to(device) for a in val.batch(48, scale, vrng, augment=False))
    vsig = torch.tensor([0.05, 0.2, 0.5, 1.0, 2.0, 5.0], device=device).repeat(8)
    vnoise = torch.from_numpy(vrng.standard_normal(vt.shape).astype(np.float32)).to(device)

    def val_loss():
        from .model import SIGMA_DATA

        total = 0.0
        with torch.no_grad(), torch.autocast("cuda", dtype=torch.bfloat16):
            for i in range(0, len(vt), 12):
                s = vsig[i : i + 12]
                x = vt[i : i + 12] + vnoise[i : i + 12] * s.reshape(-1, 1, 1, 1)
                d = ema(x, s, vc[i : i + 12]).float()
                w = ((s**2 + SIGMA_DATA**2) / (s * SIGMA_DATA) ** 2).reshape(-1, 1, 1, 1)
                m = vm[i : i + 12]
                total += float((w * (d - vt[i : i + 12]) ** 2 * m).sum() / m.sum().clamp(min=1))
        return total / (len(vt) / 12)

    log = (run / "log.jsonl").open("a")
    start, last, losses = time.time(), time.time(), []
    torch.cuda.reset_peak_memory_stats()
    model.train()
    while step < args.steps and time.time() - start < args.hours * 3600:
        target, cond, mask = train.batch(args.batch, scale, rng)
        drop = rng.random(args.batch) < config["class_dropout"]
        cond[drop, 3:] = 0
        cond[drop, 3] = 1
        target, cond, mask = (torch.from_numpy(a).to(device, non_blocking=True) for a in (target, cond, mask))
        for g in opt.param_groups:
            g["lr"] = args.lr * min(1.0, (step + 1) / args.warmup)
        with torch.autocast("cuda", dtype=torch.bfloat16):
            loss = edm_loss(model, target, cond, mask)
        opt.zero_grad(set_to_none=True)
        loss.backward()
        torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
        opt.step()
        with torch.no_grad():
            for pe, pm in zip(ema.parameters(), model.parameters()):
                pe.lerp_(pm, 1 - args.ema)
        step += 1
        losses.append(float(loss))
        if not np.isfinite(losses[-1]):
            print(f"error: non-finite loss at step {step}", file=sys.stderr)
            return 1
        record = None
        if step % args.log_every == 0:
            now = time.time()
            record = {
                "step": step,
                "loss": float(np.mean(losses)),
                "steps_per_s": args.log_every / (now - last),
                "peak_vram_gb": torch.cuda.max_memory_allocated() / 2**30,
            }
            last, losses = now, []
        if step % args.val_every == 0:
            record = (record or {"step": step}) | {"val_loss": val_loss()}
        if record:
            log.write(json.dumps(record) + "\n")
            log.flush()
            print(json.dumps(record), flush=True)
        if step % args.save_every == 0 or step == args.steps:
            state = {"model": model.state_dict(), "ema": ema.state_dict(), "opt": opt.state_dict(),
                     "step": step, "rng": rng.bit_generator.state}
            _atomic_save(state, ckpt_path, torch.save)
            weights = {k: v.contiguous() for k, v in ema.state_dict().items()}
            _atomic_save(weights, run / f"ema-{step:07d}.safetensors", save_file)
    state = {"model": model.state_dict(), "ema": ema.state_dict(), "opt": opt.state_dict(),
             "step": step, "rng": rng.bit_generator.state}
    _atomic_save(state, ckpt_path, torch.save)
    _atomic_save({k: v.contiguous() for k, v in ema.state_dict().items()},
                 run / f"ema-{step:07d}.safetensors", save_file)
    print(f"stopped at step {step}; peak VRAM {torch.cuda.max_memory_allocated() / 2**30:.2f} GiB", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
