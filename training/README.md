# Training and evaluation

The Python package implements a bounded CPU numeric evaluation worker and the
GPU [terrain diffusion pilot](#terrain-diffusion). It
loads scalar Float32 metre elevations and separate masks from a versioned Rust
request, then measures Float32/Float16 normalization error on real terrain and
synthetic features. A separate bounded CPU experiment evaluates a pinned image
autoencoder. One narrowly scoped scalar-autoencoder fitting trial is implemented;
production training and diffusion remain unimplemented.

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
[v1 contracts](../schemas/numeric-evaluation-v1.md),
[v2 surface-cover contracts](../schemas/numeric-evaluation-v2.md), and the
[implementation plan](../docs/plans/numeric-evaluation-v1.md).
Future model jobs retain the 16 GB VRAM ceiling and require numeric representation
validation before expensive training. The current metric results do not approve
a model, vehicle clearance or generated terrain.

The optional [image-codec workflow](../docs/workflows/image-codec-evaluation.md)
uses its own CPU-only environment in `data/models/codec-environment-v1/.venv`.
Its measured dependency versions are in `requirements-codec-cpu.txt`. Install
Torch from the official CPU index first, then that requirements file and the
`codec` extra. Do not add these dependencies to the simple numeric environment.
`codec.py` supervises the bounded process; `reconstruction.py` validates pinned
inputs, runs the VAE and verifies retained output arrays. `metrics.py` supplies
shared physical errors. Imports do not load model weights or start jobs.

The [scalar fitting workflow](../docs/workflows/scalar-codec-fitting.md) reuses that
CPU environment under a separate `representation_fit_only` release. It retains
initial/final weights, native scalar outputs and a fixed 200-step loss history.
Normal checks include tiny fitting checks when the optional environment exists.

## Terrain diffusion

`topography_training.diffusion` trains a pixel-space residual denoiser on a
training release and generates terrain for world inputs; see the
[plan](../docs/plans/terrain-diffusion-v1.md) and
[workflow](../docs/workflows/terrain-diffusion.md). It needs a GPU PyTorch in its
own environment. For AMD RDNA4 (RX 9060 XT, gfx1200) use the ROCm 7.2 wheels:

```sh
python3 -m venv data/models/diffusion-env-v1/.venv
data/models/diffusion-env-v1/.venv/bin/python -m pip install torch --index-url https://download.pytorch.org/whl/rocm7.2
data/models/diffusion-env-v1/.venv/bin/python -m pip install -e './training[diffusion]'
```

The ROCm wheel is about 6 GB. If `/tmp` is too small, set `TMPDIR` and
`--cache-dir` to a directory on disk. If the download keeps breaking, download
the wheel with a browser or `curl -C -`, then install the file with
`--extra-index-url https://download.pytorch.org/whl/rocm7.2`; its `triton-rocm`
dependency exists only on that index.

The wheel bundles its ROCm runtime; only the `amdgpu` kernel driver and access to
`/dev/kfd` and `/dev/dri/renderD*` are required.
