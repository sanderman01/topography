# Terrain diffusion workflow

Implements the [terrain diffusion pilot](../plans/terrain-diffusion-v1.md). Set up
the GPU environment as described in [training/README.md](../../training/README.md#terrain-diffusion).
`PY` below is `data/models/diffusion-env-v1/.venv/bin/python`.

## 1. Training release

```sh
topography dataset diffusion configs/runtime.example.toml \
  data/review/library-gallery-v15/gallery.json data/datasets/diffusion-v1
```

Copies raw layers of every accepted Ready tile's current prepared product into a
new directory with a hashed manifest. Overlapping products keep the larger one.
Rebuild into a new directory after accepting more tiles. Loading the gallery
verifies every raw asset, which takes a few minutes.

Each tile also gets automatic artifact checks: an `artifacts.u8` layer
(1 facet, 2 stripe, 4 seam) and a per-tile summary in the manifest. The training
elevation has stripes filtered out and facets smoothed; seam cells are left out.
Open `report.html` in the release to see the shares per tile and the worst
close-up before training.

## 2. Training (GPU; warn before running)

```sh
$PY -m topography_training.diffusion.train \
  --release data/datasets/diffusion-v1/manifest.json \
  --run-dir data/models/diffusion-run-1 --hours 8
```

Prints and logs (`log.jsonl`) loss, validation loss, steps/s and peak VRAM.
Rerunning the same command resumes from `checkpoint.pt`. EMA weights are saved as
`ema-<step>.safetensors`. For a smoke run use `--steps 300 --val-every 100 --save-every 300`.

## 3. Evaluation and blind review

```sh
$PY -m topography_training.diffusion.evaluate \
  --run-dir data/models/diffusion-run-1 --out data/models/diffusion-run-1/eval-1
```

Generates from held-out validation windows. Writes:

- `metrics.json`: per-patch slope, curvature and relief, the nearest
  training-crop distance for real and generated patches, and the seam slope ratio.
- `seam.png`: a MultiDiffusion strip.
- `review.html`: shuffled real and generated hillshades.

The owner answers Yes/No per patch in `review.html` and pastes the JSON result
back. `key.json` maps ids to real/generated.

## 4. World generation

```sh
$PY -m topography_training.diffusion.generate \
  --run-dir data/models/diffusion-run-1 --world <world-input-dir> --out <new-dir>
```

The input format is documented in `generate.py`: a 32 m coarse surface, a 32 m
class map and an optional 1 m water mask. The output is Float32 elevation plus
masks. The 16-bit export is a later step.
