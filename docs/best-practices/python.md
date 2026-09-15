# Python

Applies to `training/`. Only the package scaffold exists: Python 3.11+, setuptools,
and a `src/topography_training/` layout. No trainer, inference command, ML
dependencies, Python test runner, formatter, or linter is configured yet.

## Packaging and code

- Keep reusable Python under `training/src/topography_training/`. Install the
  package into an isolated environment using the instructions in
  [training/README.md](../../training/README.md). Keep imports independent of the
  caller's working directory; avoid application-level `sys.path` edits. See
  [PyPA's explanation of the src layout](https://packaging.python.org/en/latest/discussions/src-layout-vs-flat-layout/).
- Declare dependencies in `training/pyproject.toml`. Select and pin a reproducible
  ML/GPU environment during the feasibility pilot, recording compatible package,
  accelerator-runtime, and driver versions. Do not claim that a Python lock file
  captures the whole GPU environment.
- Support Python 3.11 when introducing syntax or standard-library APIs. Keep
  imports free of training, downloads, GPU allocation, and filesystem writes.
- Separate loading, transformations, training, inference, and evaluation into
  callable units. Annotate public interfaces; document array shape, dtype, units,
  channel meaning, and mask conventions. Do not hide physical transformations
  behind generic image preprocessing.

## Batch jobs and failures

- Implement the versioned manifest boundary described in
  [architecture and contracts](architecture-and-contracts.md). Validate input
  versions, paths, paired raster shapes, numeric ranges, and available resources
  before a costly run.
- Make the entry point accept explicit input/output locations and return a
  nonzero status on failure. Preserve diagnostic context; do not catch broad
  exceptions and report success or silently skip unreadable examples.
- When invoking tools, pass argument lists, check return codes, and manage
  timeouts/cancellation. Stream long-running logs instead of buffering them all.
  See Python's [subprocess reference](https://docs.python.org/3/library/subprocess.html).
- Write result manifests, metrics, logs, and checkpoints under the chosen run
  directory in `data/models/`. Publish a completion record only when the declared
  outputs are usable. Keep human promotion separate from process completion.
- Make checkpoints sufficient for the advertised resume behavior: include
  optimizer, scheduler, random state, and progress as applicable. Record when
  resuming changes the experiment's semantics.

## Numeric data and resources

- Read elevation as numeric scalar data with physical spacing and explicit
  masks. Keep normalization fixed per model level and invertible within its
  declared tolerance. Never route DEMs through RGB conversion or photographic
  augmentation defaults. Follow [machine learning](machine-learning.md).
- Load crop windows on demand and bound worker counts, prefetch buffers, and
  batch sizes. Account for CPU copies and pinned memory as well as GPU tensors.
  Keep the pilot inside 16 GB VRAM.
- Use inference/evaluation behavior deliberately when those jobs are added;
  disable unnecessary gradient tracking and preserve training/evaluation mode
  semantics. Record precision choices and their measured terrain errors.

## Verify

For packaging changes, verify installation and import from outside the source
tree in the selected environment. For implemented jobs, run a tiny manifest
through the real entry point and inspect outputs and a failure case. The Rust
check script does not verify Python jobs. Add documented tooling when it becomes
necessary; do not prescribe commands for tools the project has not configured.
Requested formal tests belong in `training/tests/`.
