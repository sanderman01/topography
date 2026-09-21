# First image-codec reconstruction experiment

Next step after the [numeric result](../reports/numeric-evaluation-v1.md), under
master-plan §7. Technical choices belong to the implementer. The owner is asked
only for workload readiness, not to approve this configuration or judge metrics.

## Concrete experiment

Evaluate the public `stabilityai/sd-vae-ft-mse` autoencoder at revision
`31f26fdeee1355a5c34592e401dd41e45d25a493`. This is a candidate for the image-model
route, not a selected terrain architecture. The provider documents this VAE as
compatible with Stable Diffusion and lists Float32 weights. See the
[model card](https://huggingface.co/stabilityai/sd-vae-ft-mse) and
[AutoencoderKL API](https://huggingface.co/docs/diffusers/api/models/autoencoderkl).
Save the model card/configuration and verify the safetensors download against
the provider's pinned file identity before loading; use no pickle checkpoint.

Captured public API metadata and the experiment request are in
`data/review/codec-pilot-proposal-v1/`. The weights are 334,643,276 bytes with SHA-256
`a1d993488569e928462932c8c38a0760b874d166399b14414135bd9c42df5815`.
Metadata reports no account gate. Recheck access at download; no correspondence
or accounts should be needed. Weights and ML dependencies are not installed yet.

Use the completed numeric release, preserving its twelve native regions and
geographic roles. Select one fixed central 256 × 256 cell window from each region
(128 m at the current 0.5 m spacing), plus fixed ramp, flat, sharp-ridge and narrow-
ridge arrays. Do not choose windows based on reconstruction results. Split roles
remain inherited before windowing. This is evaluation only and no weights change.

Repeat normalized scalar height / 2,048 across three channels in Float32. Encode
using the posterior mode and decode in Float32, on CPU, in evaluation/inference
mode. No image conversion, gamma, clipping, per-crop normalization or stochastic
sampling. Invert the channel mean × 2,048; retain per-channel differences so that
averaging cannot hide color-channel disagreement. Record original/reconstructed
scalar arrays, metre errors, physical slope changes and narrow-feature losses.
Exclude masked cells/pairs. Report full-window and fixed interior metrics to
separate edge effects; masked fill can influence neighboring reconstructions and
must remain a documented limitation. Reject nonfinite or malformed output.

## Workload and implementation bounds

- CPU only, at most two compute threads, one window at a time; no GPU or training.
- Stop after 30 minutes of model processing. Implement timeout/cancellation and
  bounded logs before launching. Partial runs remain explicitly incomplete.
- Set a 6 GiB virtual-address-space ceiling before model imports/loading. This is
  a safety bound, not measured peak RAM; several GiB of RAM may be needed and the
  exact peak/runtime are unknown. Stop on the limit rather than increasing it.
- Allow up to 4 GiB additional disk for an isolated CPU environment, public model
  and outputs; check space before downloading. Preserve the current numeric env.
- Record exact dependency versions, model/config hashes, input release/request,
  code hashes, mode/precision, elapsed time and resource observations. Validate
  the release before and after. No output promotion or model-training admission.

Implementation belongs in the Python package with a small explicit batch entry
point. Reuse scalar loading and shared error calculations. First verify tiny
identity/perturbation cases and failure paths without running the full model;
then run the bounded experiment under the owner's readiness confirmation.
Dependency installation and downloads are exempt from the readiness pause, but
heavy model construction/forward passes are not. Do not infer readiness from the
earlier LiDAR or numeric-dataset responses.

Review the result technically and present terrain examples only when a useful
visual choice exists. No per-feature questionnaire, another grouping approval,
new source votes or model training follows automatically. If this representation
loses terrain detail, evaluate the smallest defensible scalar alternative within
a separately bounded attempt, keeping curated terrain/blending as the fallback.
