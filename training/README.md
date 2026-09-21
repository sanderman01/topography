# Training and evaluation

The Python package now implements a bounded CPU numeric evaluation worker. It
loads scalar Float32 metre elevations and separate masks from a versioned Rust
request, then measures Float32/Float16 normalization error on real terrain and
synthetic features. No trainer, autoencoder, diffusion or GPU dependency exists.

```sh
python3 -m venv training/.venv
training/.venv/bin/python -m pip install --only-binary=:all: -e ./training
```

Use Python 3.11–3.14 with the pinned NumPy 2.3.5. Keep the interpreter path inside
the virtual environment; resolving its executable symlink bypasses that environment.
`src/topography_training/payload.py` validates/loads scalar payloads, and
`numeric.py` computes metrics and publishes a complete worker result. Imports have
no job, download or GPU side effects.

Use the Rust `evaluate numeric` command for normal operation. It validates the
approved evaluation release, decodes GDAL rasters, invokes the worker and checks
current sources/results before publishing. The direct module invocation is the
internal batch boundary, not a way to bypass catalog/source-admission checks.

See the [workflow](../docs/workflows/numeric-evaluation.md),
[v1 contracts](../schemas/numeric-evaluation-v1.md) and
[implementation plan](../docs/plans/numeric-evaluation-v1.md).
Future model jobs retain the 16 GB VRAM ceiling and require numeric representation
validation before expensive training. The current metric results do not approve
a model, vehicle clearance or generated terrain.
