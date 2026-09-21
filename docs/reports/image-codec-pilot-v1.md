# Image-codec pilot result

Completed 2026-09-21. **Reject the tested image-VAE representation for terrain.**
On the twelve fixed real-terrain windows, full-window height RMSE ranged from
5.47 to 31.24 m; removing the outer 32 cells still left 4.34 to 28.53 m RMSE.
The worst valid height error was 100.76 m. These distortions are large compared
with the exact scalar Float32 baseline. Do not spend LoRA training on this
representation.

This is Codex's technical decision about one pinned VAE with fixed absolute-height
normalization. It does not reject all image models or terrain diffusion, alter
source acceptance, or establish a vehicle/gameplay tolerance. The twelve accepted
tiles remain accepted, and the earlier rejected tile stays excluded.

## Experiment

The owner answered “Continue” to the bounded CPU readiness request. We ran
`stabilityai/sd-vae-ft-mse` revision `31f26fdeee1355a5c34592e401dd41e45d25a493`, using
the public safetensors file with its pinned provider checksum. See the
[plan](../plans/image-codec-pilot-v1.md) and [reproducible workflow](../workflows/image-codec-evaluation.md).
Model weights, configuration and model card are retained locally.

Each source supplied its fixed central 256² window, 128 m square at native 0.5 m
spacing. Geographic calibration/validation roles were inherited before cropping.
Four 256² synthetic cases added a ramp, flat, sharp ridge and narrow ridge.
Float32 scalar height / 2,048 was repeated into three channels, encoded using the
posterior mode and decoded in Float32. The channel mean was inverted to metres.
No gradient updates, random latent sampling, image/gamma conversion or clipping
occurred. All three output channels and original/decoded scalar arrays are saved.

The real windows contain 752,040 valid ground cells and 34,392 water cells.
Ground metrics exclude water and invalid cells; slopes require two valid adjacent
samples. Masked fill can still influence model context. Interior metrics remove
only the outer window boundary, not internal water boundaries.

## Why it fails this representation check

| Measurement | Observed result |
| --- | --- |
| Real full-window height RMSE | 5.47–31.24 m |
| Real interior height RMSE | 4.34–28.53 m |
| Worst real valid height error | 100.76 m |
| Interior error after removing each window's mean bias, diagnostic only | 0.69–7.66 m RMS |
| Synthetic flat interior RMSE | 1.75 m; it also acquired nonzero slopes |
| Sharp ridge local contrast, original → reconstructed median | 10.125 m → 9.588 m |
| Narrow ridge local contrast, original → reconstructed median | +0.125 m → −0.0062 m |

The contrast diagnostic measures each interior row's designated ridge columns
relative to the immediately flanking columns, then takes the median. For the
narrow ridge, the reconstructed contrasts ranged from −0.0081 to +0.0110 m:
the positive 12.5 cm feature was not preserved. Generic exact-flatness counts
alone would miss this because reconstruction adds small oscillations.

Mean-bias removal is an after-the-fact diagnostic, not fitted normalization or a
fix applied to results. Substantial spatial errors remain. Averaging channels also
does not establish scalar fidelity: per-channel errors and channel spread are
retained in the worker report. Neither border effects nor a constant elevation
offset explains away the failure.

## Resources and verification

The worker completed in **154.14 seconds**, with **1,125,432 KiB** (about 1.07 GiB)
peak self RSS, two CPU compute threads and no GPU. It stayed below the 30-minute
deadline and 6 GiB address-space ceiling. The supervisor and separate Rust source
checks are outside that RSS/time observation. Model, isolated environment and
outputs use about 1.4 GiB additional disk; the completed run itself is 23,113,425 bytes.

Environment: Python 3.14.7, NumPy 2.3.5, CPU-only PyTorch 2.10.0, Diffusers 0.36.0
and Safetensors 0.7.0 on Linux x86_64. The complete dependency freeze is retained
and mirrored in `training/requirements-codec-cpu.txt`. Imports emitted upstream
warnings about disabling CUDA autocast; the CPU-only build has no CUDA/ROCm
backend and every tensor/model operation explicitly used CPU.

The supervisor verified all sixteen cases, rehashed every output, compared source
arrays to pinned inputs and recomputed all reported metrics from saved decoded
channels. Rust verified current release eligibility before and after. Small
workflow checks cover masks, analytical metre/slope errors, corrupt completion,
missing cases/provenance, failed workers, timeout, excessive logs, SIGTERM and
failed completion writes. The normal full repository check passed.

A fresh code review found a SIGTERM cleanup gap and non-atomic completion writes.
Both were reproduced and fixed after the successful model run, without rerunning
inference. The executed source files were archived under the proposal directory
before those supervisor fixes. They match the worker's recorded code hashes;
original results and measurements remain unchanged. The fixed paths were verified
with cancellation and filesystem-failure fixtures. The worker now also has an
independent deadline if its supervisor is forcibly lost.

## Retained artifacts and next step

- `data/models/image-codec-pilot-v1/`: final result, worker report/log and eighty
  original/reconstructed/channel/mask binary arrays.
- `data/review/codec-pilot-proposal-v1/`: pinned job, exact readiness response,
  environment freeze, current-source checks, executed code and technical assessment.
- `data/models/sd-vae-ft-mse-31f26fde/`: verified public model/configuration/card.
- `data/models/codec-environment-v1/.venv/`: isolated CPU environment.

The [next proposed model trial](../plans/scalar-codec-fit-pilot-v1.md) handles scalar
heights directly with a bounded training/evaluation budget. The current dataset release permits numeric evaluation
only; model fitting requires a separately scoped release and workload readiness.
Keep the real-terrain patch/blending baseline available so model work cannot
indefinitely postpone the world pilot. No model or generated terrain is promoted
by this experiment, and no additional source-tile review is needed.
