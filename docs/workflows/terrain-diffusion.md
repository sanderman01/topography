# Terrain diffusion workflow

Implements the [terrain diffusion pilot](../plans/terrain-diffusion-v1.md). Set up
the GPU environment as described in [training/README.md](../../training/README.md#terrain-diffusion).
`PY` below is `data/models/diffusion-env-v1/.venv/bin/python`.

The training release (step 1) is current: it trains cascade level L4. Steps 2 onward
describe the original 1 m pilot on a 32 m coarse surface, superseded by the
[terrain cascade workflow](terrain-cascade.md) (L4 run 5, banded generation).

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
class map and 1 m water, snow/ice, vegetation and land-cover layers
(`topography world input` writes them from WorldCover, GLIMS and OpenStreetMap).
Runs from run 4 onward generate terrain for that cover; rebuild older world
inputs that lack `snow-ice.u8`. The output is Float32 elevation plus masks.
`--guidance 2 --negative ice` sharpens glacier surfaces (see the diffusion plan's
known issues).

### Invented worlds

```sh
topography world sketch configs/sketch/caldera-island.toml <new-world-input-dir>
topography world preview <world-input-dir> <new-preview-dir> [8]
```

`world sketch` builds the same world input from a procedural recipe instead of a
real place: an island outline plus ordered landform features (caldera, range,
glacier, fjord, archipelago, cliffs, dunes, mesas, gorge, karst, braided river,
cones), each painting its terrain class. Land takes the class of the nearest
feature; carved paths, glaciers and the crater override. Snow/ice follows a
windward and lee snowline, and land cover follows biome, elevation and wind-side
moisture. It uses a local metric CRS and no source data. A 32 km sketch takes
under a minute on CPU and writes about 4 GB, mostly 1 m masks.

Use a sketch for layout experiments only. As a base surface its landforms read
as brush strokes (rejected 2026-09-29); the [terrain cascade](../plans/terrain-cascade-v1.md)
plans real-data levels above 1 m instead.

`world preview` interpolates any world input's 32 m surface to a few metres and
samples its masks, so `export heightmap` and `export view` can show the layout
before GPU generation.

## 5. Export and viewing

```sh
topography export heightmap <world-output-dir> <new-export-dir> [256] [shared_edge]
topography export view <export-dir> [port]
```

`export view` serves a loopback WebGL2 viewer (default port 8766) that reads only
the exported tiles: an overview from the finest LOD level of at most 1,025
samples per side, with double-click loading 1 or 2 km of level 0 around the
picked point. Cover colours come from the land-cover, water and snow/ice masks.
