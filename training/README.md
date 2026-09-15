# Training and evaluation

This directory reserves a separate Python package for the model experiments in
the master plan. Only the importable package scaffold exists; no trainer,
inference command, or model dependencies are implemented yet.

```text
src/topography_training/  Future loaders, numeric evaluation, training, inference
tests/                    Future Python tests
```

Use Python 3.11 or newer. An editable development environment can be prepared
when Python implementation begins:

```sh
python3 -m venv training/.venv
training/.venv/bin/python -m pip install -e ./training
```

PyTorch, Diffusers, and GPU-specific dependencies will be selected and pinned
during the numeric precision and memory pilot, within 16 GB VRAM. Keep source
elevation in physical units; an image loader must not silently convert numeric
DEMs to photographic RGB data.

## Planned batch boundary

Rust will invoke Python jobs with a versioned input manifest and an explicit
output directory. Jobs will read an approved dataset release and configuration,
then write a result manifest, metrics, logs, and checkpoints. A successful process
exit reports job completion; human acceptance remains a separate catalog decision.
The exact command syntax and schemas will be defined with the first numeric
evaluation job.

The contract must include dataset/model versions, normalization and physical
spacing, controls, seed, precision settings, and software versions. Jobs must
report peak GPU memory and runtime and propagate failures through nonzero exit
codes. Persist run artifacts in `data/models/`, not inside this source directory.
