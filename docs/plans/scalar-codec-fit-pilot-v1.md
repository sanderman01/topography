# One bounded scalar-codec fitting trial

Proposed next step after the [image-codec rejection](../reports/image-codec-pilot-v1.md).
This is a small reconstruction experiment, not diffusion training or a commitment
to a production latent architecture. Technical choices remain with the implementer.
Obtain the owner's readiness and permission for this scoped fitting run before
starting it; previous readiness covered the completed pretrained reconstruction.

## Question and data

Can a small single-channel model trained directly on height values reconstruct
the pilot terrain substantially better than the rejected photographic VAE within
one short CPU budget? Compare with the exact direct-scalar baseline and report
height, slope and narrow-feature error. Do not invent gameplay acceptance limits.

Reuse the twelve accepted prepared regions and geographic grouping. Six regions
in Río Abajo, central gorge and southern floodplain may supply fitting windows;
the six Camuy/Guajataca/Arecibo regions are validation only. Keep the existing
512 m context/buffer and all masks. No new source vote or geographic regrouping
is needed. The current release is evaluation-only: create a separately scoped
`representation_fit_only` release, preserving the original, before any optimizer
step. Capture the owner response against this exact scope. Never silently turn
the current numeric release into training permission.

Use native 512² training windows, batch one, selected deterministically from the
fitting regions with a pinned seed. Roles are assigned before windowing. Evaluate
fixed central 512² windows of every region plus ramps, flats and sharp/narrow
synthetic ridges, separately by role. No final test-set or independent-survey
claim; this remains a small correlated pilot collection.

## Minimal model and run

Use Float32 height / 2,048, with no RGB route, clipping or per-window contrast fit.
Start one small convolutional single-channel autoencoder: 16/32 encoder channels,
two stride-two reductions, four latent channels, and a matching two-stage decoder
with fixed bilinear upsampling and scalar output. No source-to-decoder skip that
bypasses the latent representation. This specifies one diagnostic candidate;
changing latent semantics would later require a matching denoiser, not a decoder
swap into a photographic model.

Use masked squared scalar reconstruction error, Adam at an initial 0.001 learning
rate, fixed seed, batch one and at most 200 optimizer steps. Keep normalization,
precision and model shape fixed for this trial. Exclude water/invalid cells from
loss and ground metrics; document their influence on neighboring convolution
context. Measure initial and final held-out reconstruction without adapting to
validation. Retain checkpoints, loss history, scalar outputs and provenance.

Enforce two CPU compute threads, no GPU, a 30-minute processing deadline and a
6 GiB address-space ceiling before model imports. Actual peak RAM and runtime
are unknown until measured. Use the existing isolated CPU environment and allow
at most 512 MiB additional disk for this small model's artifacts. Preflight all
inputs, model shapes and failure behavior with tiny synthetic fixtures before
the bounded run; do not enlarge the limits on failure.

## Decision after the trial

Report the exact direct-scalar baseline, untrained and fitted reconstructions,
physical errors, feature survival and measured resource use. A promising result
permits planning the next feasibility check, not model promotion, production
training or generated-terrain acceptance. If this one trial gives no useful
result, preserve it and move the world pilot forward with curated patches and
blending instead of starting an open-ended model-tuning loop. Neither CPU fit nor
reconstruction alone establishes diffusion quality or GPU feasibility.
