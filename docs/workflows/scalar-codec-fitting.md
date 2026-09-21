# Bounded scalar-codec fitting

This is one diagnostic reconstruction trial under the
[approved scope](../plans/scalar-codec-fit-pilot-v1.md). It reuses the CPU environment
from the [image-codec workflow](image-codec-evaluation.md); no new dependencies,
downloads or GPU runtime are required. Production training, diffusion, promotion
and resume are outside this consumer's scope.

## Separate release

The original numeric release remains evaluation-only. Publish a child release with
`dataset fit-build <runtime> <evaluation-manifest> <approval>` and verify it with
`dataset fit-verify <runtime> <fitting-manifest>`. The child preserves the original
geographic roles and source gates, using calibration regions for fitting and
validation regions only for measurements. Its hash-addressed manifest embeds the
exact approval document. Current-parent checks run again at use; rejection revokes
reuse without deleting historical releases.

The strict approval contains `schema_version: 1`,
`kind: representation_fit_approval`, `scope: representation_fit_only`,
`dataset_sha256` of the evaluation parent, `actor_kind: human`, `actor_name`,
`recorder`, `decision: approve`, `exact_response`, `plan_sha256`, `proposal_sha256`,
and `trial: scalar_codec_fit_v1`. Actor/response records are local declarations,
not authenticated signatures. The trial name selects the fixed limits below;
this is not permission for an open-ended tuning loop.

The child manifest contains `schema_version: 1`, `kind: representation_fit_release`,
`purpose: representation_fit_only`, `dataset_sha256`, `approval_document` and
`topography_version`. The containing directory is the manifest's SHA-256. The
parent resolves through configured dataset storage. Neither release creates
human acceptance of a prepared output or fitted model.

## Fixed experiment

The JSON job contains `schema_version: 1`, `kind: scalar_codec_fit_job`, and four
`{path, sha256}` references: `numeric_run` (completed numeric manifest),
`fitting_release`, `runtime` (runtime TOML) and `cli` (built executable). Paths are
absolute, hashes cover exact file bytes. All references are pinned and all scalar
payloads are validated before use. The supervisor invokes Rust before and after
fitting, comparing current prepared identities, grids, roles and groups with the
numeric payload. Execution and completion verification use an archived copy of
the exact prechecked job bytes; completion identities must still match the initial
admission checks. The worker interface is internal; invoke the supervisor normally.

```sh
data/models/codec-environment-v1/.venv/bin/python -m topography_training.scalar_fit data/review/scalar-codec-fit-proposal-v1/job.json data/models/scalar-codec-fit-v1
```

This command documents the actual run location; it refuses an existing output.
Obtain readiness for a new substantial run. Do not rerun a completed experiment
just because its command is listed here.

The fixed candidate uses Float32 height/2,048, one input/output channel, 16/32
encoder channels, two stride-two convolutions and four quarter-grid latent
channels. The decoder uses two fixed bilinear upsampling stages, with no bypass
around the latent. Adam uses learning rate 0.001, betas (0.9, 0.999), epsilon 1e-8,
zero weight decay and no scheduler. Exactly 200 scheduled steps use batch one,
seed zero and native 512² windows from the six calibration regions, in round-robin
region order with seeded offsets. There is no validation-based model selection.

Water and invalid cells are excluded from loss and ground metrics. Their zero
fill still influences neighboring convolution context. Padding also affects
boundaries. Metrics retain both the full window and a fixed 32-cell interior;
this does not remove every mask-boundary effect. Values are not clipped,
converted to RGB or independently contrast-normalized.

Initial and final evaluation use central 512² windows of all twelve regions and
four fixed synthetic cases. Physical height/slope metrics share the direct-scalar
implementation. Synthetic high-altitude ridges deliberately probe beyond the
pilot's elevation distribution; do not interpret them as held-out survey terrain.
This collection has no untouched test split and is not independent survey evidence.

## Limits and retained output

The worker receives two CPU compute threads, one interop thread, a 1,800-second
alarm and supervisor deadline, 6 GiB virtual address space before NumPy/Torch
imports, and a 64 MiB per-file ceiling. CUDA/ROCm Torch builds are rejected.
Supervision bounds logs to 1 MiB and checks the 512 MiB artifact budget, including
when the worker has already exited. Overshoot between polls is possible; fixed
artifact shapes keep the ordinary result well below the budget. Actual RAM/time
are observations, not the configured limits.

Each case saves original, initial and final Float32 grids and Byte validity/water
masks. The run also retains initial/final safetensors weights, the preselected
200-window schedule, loss history, source-code hashes, versions, worker RSS/time
and before/after Rust source status. These are inference checkpoints only; no
optimizer state or resume is advertised. See upstream
[PyTorch determinism](https://docs.pytorch.org/docs/2.10/notes/randomness.html) and
[safetensors serialization](https://huggingface.co/docs/safetensors/api/torch).

Completion verification checks every artifact hash, checkpoint shape/type,
case identity, mask/original value, schedule role and loss-history entry. It
recomputes all metrics from retained scalar arrays before atomic `result.json`
publication. This validates provenance and measurements, not a proof that an
arbitrary untrusted worker executed its claimed optimizer. Failure leaves logs,
partial evidence and `failure.json`; `worker-result.json` alone is not completion.
No completion record promotes terrain or admits production training.

## Checks

```sh
data/models/codec-environment-v1/.venv/bin/python scripts/check_scalar_fit.py
data/models/codec-environment-v1/.venv/bin/python scripts/check_scalar_fit.py --job data/review/scalar-codec-fit-proposal-v1/job.json
bash scripts/check.sh
```

The default checks use tiny arrays, one tiny optimizer update and checkpoint
reload, role exclusion and subprocess limits. The optional local-input check
creates temporary identity-output fixtures for completion validation; it does
not perform the 200-step terrain fit. Dataset checks exercise separate scope,
stale/corrupt sources and preservation of the evaluation release.
