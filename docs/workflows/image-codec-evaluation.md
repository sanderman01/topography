# Bounded image-codec evaluation

This experiment tests reconstruction through one public pretrained image VAE,
before any LoRA training. It consumes a completed Rust numeric run, reuses scalar
Float32 loading and physical error metrics, and never promotes a model or terrain.
See the [experiment plan](../plans/image-codec-pilot-v1.md).

## Isolated CPU environment

From the repository root, use a separate environment so the small numeric worker
keeps its NumPy-only installation:

```sh
python3 -m venv data/models/codec-environment-v1/.venv
data/models/codec-environment-v1/.venv/bin/python -m pip install --no-cache-dir --only-binary=:all: torch==2.10.0 --index-url https://download.pytorch.org/whl/cpu
data/models/codec-environment-v1/.venv/bin/python -m pip install --no-cache-dir --only-binary=:all: -r training/requirements-codec-cpu.txt -e './training[codec]'
```

The requirements record the environment actually exercised on Python 3.14.7 /
Linux x86_64. Python 3.11 is the package target but was not tested here. Torch's
CPU wheel is installed first from its [official index](https://pytorch.org/get-started/previous-versions/);
the remaining requirements use PyPI. The experiment rejects CUDA/ROCm Torch builds,
uses CPU explicitly, and disables gradient tracking. No accelerator setup is needed.

## Pinned inputs

Use a current evaluation-only release and its completed numeric run. Save public
VAE weights, configuration and model card at the revision in the plan, then write
a strict model manifest with `schema_version: 1`, `kind: local_image_codec`,
`repository`, `revision`, and `artifacts`. The exact artifact names are `config.json`,
`diffusion_pytorch_model.safetensors`, and `README.md`; each has `bytes` and
`sha256`. The runner accepts only the specified repository/revision/weight hash.
It loads local safetensors only, with network access disabled in the worker.

The job is versioned JSON with `schema_version: 1`,
`kind: image_codec_evaluation_job`, `dataset_sha256`, `numeric_run`, and `model`.
Both references are `{path, sha256}`, with an absolute path to the corresponding
manifest and the hash of its exact bytes. These machine-local experiment requests
stay out of Git. The numerical run's request, metrics, worker result and every
scalar payload are rechecked before model allocation and again at completion.

The real job is `data/review/codec-pilot-proposal-v1/job.json`; model artifacts are
under `data/models/sd-vae-ft-mse-31f26fde/`. Its environment freeze, captured owner
readiness and before/after current-source checks are retained beside the job.

## Run

Verify the exact release through Rust before the model job and after it succeeds:

```sh
target/debug/topography dataset verify configs/runtime.example.toml data/datasets/6b7e2049f45d94bd87216ac07d96c38ece2885e8f22a00fb9d5e0686d6f07411/manifest.json
data/models/codec-environment-v1/.venv/bin/python -m topography_training.codec data/review/codec-pilot-proposal-v1/job.json data/models/image-codec-pilot-v1
target/debug/topography dataset verify configs/runtime.example.toml data/datasets/6b7e2049f45d94bd87216ac07d96c38ece2885e8f22a00fb9d5e0686d6f07411/manifest.json
```

Choose a new output directory for a new run. Inputs and previous outputs are never
overwritten. This module owns only the bounded numeric experiment; it does not
query the curation catalog. Its `result.json` explicitly requires the separate
Rust current-source check before reuse. A later rejection makes retained output
historical. Neither process success nor file existence authorizes source use.

The runner takes the fixed central 256² native window of each released region,
followed by four 256² synthetic cases. It uses two CPU compute threads, one window
at a time, a 30-minute worker deadline and a 6 GiB address-space limit set before
NumPy/Torch imports. The supervisor stops oversized logs and reads bounded failure
diagnostics. Ask readiness before this substantial workload unless it has already
been granted for the run. Limits are not measured peak usage.

Scalar height / 2,048 is repeated across three input channels. Encoding uses the
posterior mode; decoding stays Float32. There is no random latent sampling, image
conversion, clipping, contrast fitting or diffusion denoiser. The channel mean
is converted back to metres, and all three channels are retained to measure
disagreement. This direct encoder/decoder round trip does not apply the latent
scaling used between a VAE and denoiser; see the
[versioned AutoencoderKL documentation](https://huggingface.co/docs/diffusers/v0.36.0/api/models/autoencoderkl).

Height and adjacent-slope metrics exclude invalid/water cells. Both full-window
and interior results are recorded; the latter removes a fixed 32-cell border.
Masked fill still influences the convolution/attention context, so interior
cropping does not eliminate water-boundary effects. Zero-valued valid land is
measured normally. Reconstructed nonfinite values fail the run.

Each case retains original/decoded Float32 grids, all three decoded channels,
validity and water masks, window coordinates and geographic role. Before writing
the final result, the supervisor rehashes artifacts, compares original arrays
against pinned inputs and recomputes every reported metric from the saved output.
Software versions, source-code hashes, mode, limits and worker-process RSS/time
are recorded. `worker-result.json` alone is not completion. Failures retain logs,
partial outputs and `failure.json`, without a final result. No resume is supported.

## Checks

```sh
training/.venv/bin/python scripts/check_codec.py
training/.venv/bin/python scripts/check_codec.py --job data/review/codec-pilot-proposal-v1/job.json
bash scripts/check.sh
```

The first check uses small analytical arrays and subprocess fixtures, without a
model or downloads; it is included in the normal checks when NumPy is installed.
The optional job check also validates the completion boundary against pinned local
inputs using temporary synthetic identity outputs. It never runs the VAE or
records real model results. The actual model run is separate and requires readiness.
