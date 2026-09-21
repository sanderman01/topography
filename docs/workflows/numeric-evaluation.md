# Numeric evaluation workflow

This implements the first reusable CPU loader/representation check. It publishes
an evaluation-only collection of current prepared regions and measures scalar
precision. Source selection, technical configuration and successful processing
remain distinct; the owner judges terrain, while the implementer handles this
workflow. Model training, autoencoder reconstruction and promotion remain future work.

## Install the worker

From the repository root:

```sh
python3 -m venv training/.venv
training/.venv/bin/python -m pip install --only-binary=:all: -e ./training
```

Python 3.11+ is required. NumPy 2.3.5 is pinned and supports Python 3.11–3.14;
see [upstream release notes](https://numpy.org/doc/stable/release/2.3.5-notes.html).
No Torch, diffusion library or GPU runtime is installed by this package.
GDAL is used by Rust for native raster decoding; the worker reads explicit scalar
payloads with [NumPy frombuffer](https://numpy.org/doc/stable/reference/generated/numpy.frombuffer.html).
Use an isolated environment and preserve its interpreter path rather than resolving
the executable symlink to the system interpreter.

## Publish a collection

Use the [strict v1 contracts](../../schemas/numeric-evaluation-v1.md) to write a
selection and a captured approval. Pin the exact prepared IDs and keep nearby
regions in one role. The initial pilot uses six geographic areas and a 512 m
maximum context/buffer; it has no independent final test split. Confirm actual
scope from the conversation; hesitant assent must not become technical certification.

```sh
target/debug/topography dataset build configs/runtime.example.toml data/review/numeric-evaluation-v1/selection.json data/review/numeric-evaluation-v1/approval.json
target/debug/topography dataset verify configs/runtime.example.toml data/datasets/RELEASE_SHA256/manifest.json
```

Replace `RELEASE_SHA256` with the printed directory identity. Build verifies
all current prepared parents and publishes one immutable manifest. It does not
rewrite rasters, create a new human terrain decision or admit model training.
Verification fails on corrupt, missing or superseded parents. Preserve historical
outputs rather than removing them when they become stale. Dataset manifests are
portable metadata; raw/prepared data and the curation catalog still need backup.

## Evaluate

```sh
target/debug/topography evaluate numeric configs/runtime.example.toml data/datasets/RELEASE_SHA256/manifest.json training/.venv/bin/python data/models/numeric-pilot-v1
```

The output directory must not exist. Rust verifies the release, decodes seven
native layers sequentially, pins a JSON request and little-endian binary samples,
then invokes `python -m topography_training.numeric <request> <new-output-dir>`.
The worker loads one region at a time. Its maximum runtime is 600 seconds, log
limit 1 MiB and thread count one. Bounds are 32 regions and 2,048 cells per side;
confirm readiness before using those limits for a substantial workload.

The initial normalization is height ÷ 2,048 with inverse × 2,048, in Float32.
The alternative casts normalized heights to Float16 before decoding. Valid signed
zero is preserved in the Float32 path. Neither input loading nor evaluation uses
RGB, gamma, per-crop contrast normalization, resampling or a reserved height code.
Water/invalid fill is excluded from height and adjacent slope metrics. Range and
all-masked errors stop the job. Fixed small synthetic cases expose narrow-feature
loss in addition to the real regions.

Success prints the completed run manifest. The directory retains `request.json`,
all binary inputs, `metrics.json`, `worker-result.json`, `worker.log` and
`manifest.json`. Rust checks worker completion, unchanged payloads and current
source status before publication. A failed worker does not produce a completed
run. An interrupted publication can leave a reserved empty directory: inspect it
and use a new output location; no automatic deletion or partial-run promotion.

There is no resume or run-catalog registration in v1. Source changes can make a
completed run historical; verify the release before reusing its results. A
standalone Python result reports numeric completion only and carries no catalog
or source-admission authority.

## Verify implementation

```sh
training/.venv/bin/python scripts/check_numeric.py
python3 scripts/check_datasets.py target/debug/topography --python training/.venv/bin/python
bash scripts/check.sh
```

The checks use tiny artificial sources in temporary catalogs, preserve real data,
and cover geographic separation, duplicate/overlapping source membership, scoped
approval, corrupted/stale inputs, signed zero, masks, precision errors, worker
failure and output collisions. `check.sh` runs worker checks when the local
training environment is installed; otherwise it reports the skip explicitly and
still checks the Rust release path. No GPU runs.
