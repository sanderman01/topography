# Scalar-codec fitting pilot result

Completed 2026-09-21. **Reject this fitted representation for terrain and proceed
with curated real patches and blending.** The one approved 200-step trial reduced
initial errors, but held-out interior height RMSE remained 7.66–12.17 m and the
12.5 cm synthetic ridge lost its local contrast completely. There is no useful
fine-terrain representation here to promote or connect to a diffusion model.

This is a technical decision about one small architecture and training budget,
not a claim that scalar autoencoders cannot work. No extra training, tuning,
GPU job or source review is scheduled. The twelve accepted sources and one
excluded tile retain their existing decisions. Gameplay tolerances remain unset.

## Scope and execution

The owner's exact response **“continue”** approved the
[bounded fitting proposal](../plans/scalar-codec-fit-pilot-v1.md) and readiness
for that run. A new `representation_fit_only` child release was published before
optimizer updates. Its evaluation parent remains unchanged and evaluation-only.
See the [workflow and contracts](../workflows/scalar-codec-fitting.md).

The trial fitted six regions (Río Abajo, central gorge, southern floodplain),
using 200 preselected native 512² windows, batch one and seed zero. Six regions
(Camuy, Guajataca and Arecibo) were validation only. Geographic assignment and the
512 m separation/context requirement preceded cropping; the minimum opposing
footprint gap remains approximately 8,043 m. Repeated windows are not independent
regions, and this collection has no untouched test split.

The scalar convolutional autoencoder uses 16/32 encoder channels, two stride-two
reductions, four latent channels and two fixed bilinear decoder upsamplings.
Float32 height/2,048 passes through a scalar input/output path with no skip around
the latent. Adam used learning rate 0.001, fixed precision and no scheduler,
validation-driven adaptation, clipping or per-window contrast adjustment.

Initial and final measurements use central 512² windows from all twelve regions
plus four synthetic cases. The real windows contain 3,057,805 valid ground cells
and 87,923 water cells. Both loss and ground metrics exclude invalid/water cells;
zero fill still affects neighboring convolution context. The interior metric
removes 32 cells from every outer edge, not internal mask boundaries.

## Measurements

Height RMSE ranges below are across the individual regions in each role.

| Measurement | Fitting regions | Held-out validation regions |
| --- | --- | --- |
| Exact direct-scalar Float32 baseline | 0 m; all ground bits preserved | 0 m; all ground bits preserved |
| Untrained full-window RMSE | 60.48–387.56 m | 76.37–155.03 m |
| Fitted full-window RMSE | 8.62–40.22 m | 8.63–26.09 m |
| Fitted interior RMSE | 7.24–33.35 m | 7.66–12.17 m |
| Fitted interior slope-component RMSE | 0.119–0.481 m/m | 0.133–0.325 m/m |
| Worst fitted full-window height error | 366.69 m | 200.55 m |

The large full-window errors include padding effects; substantial errors remain
in the interior. Improvement from random initialization does not establish
reconstruction fidelity. On identical central 256² crops, this model improves
height RMSE over the rejected image VAE in 8 of 12 regions, but its own RMSE is
still 7.10–34.05 m. The comparison uses retained arrays at exactly matching source
coordinates, not the differently sized original summary windows.

The sharp synthetic ridge's median local contrast falls from **10.125 m to
0.1758 m**; the narrow ridge falls from **0.125 m to 0 m**. Contrast uses the
specified ridge columns relative to the immediately flanking columns, measured
on interior rows. These ridges lie at 1,800 m, outside the fitting collection's
height distribution; they test representation behavior across the declared range
and are not claims about surveyed held-out ridges. The real held-out errors
already independently rule out using this fitted model as the terrain source.

## Resources and retained evidence

The worker took **74.90 seconds**, with **630,248 KiB peak self RSS** (about
615.5 MiB), two CPU compute threads and no GPU. It stayed within 1,800 seconds
and 6 GiB address space. The completed run occupies **59,013,048 bytes** (56.3 MiB),
well within the 512 MiB artifact budget. Existing environment and source data
were reused. RSS/time exclude supervisor work and Rust source verification.

Python 3.14.7, NumPy 2.3.5, CPU-only Torch 2.10.0 and Safetensors 0.7.0 were used.
No new package, model or source download was needed. The exact environment remains
in `training/requirements-codec-cpu.txt`; minimum Python 3.11 was not exercised.

Local evidence:

- `data/review/scalar-codec-fit-proposal-v1/`: original proposal, exact approval and
  readiness, pinned job, executed source snapshot, comparison and verification.
- `data/datasets/f6cf0af754c65840ea072d06898e92b376cb2e6591fb128ee0cac0d9220c641a/manifest.json`:
  separate fitting-only release.
- `data/models/scalar-codec-fit-v1/`: initial/final safetensors weights, 200-window
  schedule, loss history, 80 scalar/mask arrays, worker log, before/after current
  source checks and completion. Weights are for inference; resume is unsupported.
- Completion SHA-256:
  `528ec0e6cfa2014f3b9103bc764c8795ab8d90defc45ca49db836d49f63c0ba5`.
- Worker-result SHA-256:
  `f4cb0002b12d085eb69ccf4ad030ad0ffa954e5a7798f2bdbd62246e677e04f5`.

Verification recomputed every metric from retained arrays, checked original
values/masks, schedule roles, history and checkpoint shapes/hashes, and confirmed
identical current-source status before/after. The archived job hash matches the
worker, completion and proposal job. The archived source hashes match execution.

A fresh code review identified a job-file replacement race in the supervisor.
The actual run's unchanged identities were independently confirmed and its
outputs preserved. The runner fix pins execution and verification to archived,
prechecked job bytes; regression checks cover replacement of both the caller's
job and the archived job. No terrain fitting rerun is required to test that fix.

## Next milestone

Use the existing accepted terrain library to build the 4,096 × 4,096 m world
pilot through real patches and blending. Begin with the 16 m composition and
whole-world visual review, then refine under the master plan's physical contracts.
Retain connected authored water, a protected landmark, local edit boundaries and
reproducible patch transforms. Model experimentation must not hold up this work.
