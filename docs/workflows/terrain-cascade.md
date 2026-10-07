# Terrain cascade workflow

Implements the [terrain cascade](../plans/terrain-cascade-v1.md): L1 (128 m) →
L2 (32 m) → L3 (4 m) → L4 (1 m), each a residual diffusion model over the level
above. `P` below is `data/models/diffusion-env-v1/.venv/bin/python` (set up as in
[training/README.md](../../training/README.md#terrain-diffusion)). Training and
generation are GPU workloads: warn before running them.

Current models (2026-10-03):

| Level | Run | Training data | Blind review |
| --- | --- | --- | --- |
| L1 128 m | `cascade-l1-run-2` | `context-v2` (GLO-30 + GEBCO) | 7/7 Yes |
| L2 32 m | `cascade-l2-run-2` | `context-v2` | 32/32 Yes |
| L3 4 m | `cascade-l3-run-1` | `lidar-v1` (lidar averaged to 4 m) | 32/32 Yes |
| L4 1 m | `cascade-l4-run-5` | `diffusion-v8` (1 m library tiles) | 32/32 Yes |

## Training data

- L1 and L2: [GLO-30 context regions](context-regions.md), release `context-v2`.
- L3: lidar regions from `context build` with `source = "lidar"` plans
  (`catalog/plans/lidar-regions-v1.toml`): USGS 3DEP and LINZ tiles averaged to
  4 m with a validity mask, voted and released like the GLO-30 regions.
- L4: the 1 m library release from `topography dataset diffusion` (see the
  [diffusion workflow](terrain-diffusion.md#1-training-release)). Its tiles are
  split by geographic bands inside each tile.

## Training and blind review

```sh
$P -m topography_training.diffusion.train --release data/datasets/lidar-v1/release.json \
    --run-dir data/models/cascade-l3-run-1 --level l3 --channels 64,96,128,192,256 --hours 3
$P -m topography_training.diffusion.train --release data/datasets/diffusion-v8/manifest.json \
    --run-dir data/models/cascade-l4-run-5 --level l4 --hours 8
$P -m topography_training.diffusion.cascade_eval --run-dir data/models/cascade-l4-run-5 \
    --out data/models/cascade-l4-run-5/eval-1
```

Refining levels (L2–L4) all use conditioning layout 3: the level above as a
per-window unit-range shape channel, its east and north slopes, absolute height /
4,096 m, water, snow/ice, vegetation and the class one-hot. Each level sees the
level above as block means on a randomly shifted grid, upsampled and smoothed
(L2 4× blocks, L3 8×, L4 4×). L4 leaves water out of the loss, since lidar
records water surfaces, not ground. Training drops the class (10%) and, for
layout 3, the shape and slopes (10%) and snow/ice and vegetation (10%), so
guidance can push against them at generation time. Rerunning `train` with the
same run directory resumes.

`cascade_eval` writes a blind review (`review.html`; keep `key.json` and
`compare.html` closed until the votes are in), terrain statistics, a
nearest-training-window distance and a same-seed check: different windows
generated with one seed should not correlate (run 4 of the old 1 m model: 0.99;
L4 run 5: 0.29).

## Generating a world

`cascade_chain` builds L1 → L2 → L3 from a layout derived from a real region (a
stand-in for an authored layout until the [layout spec](../plans/world-layout-spec-v1.md)
renders L0 rasters):

```sh
$P -m topography_training.diffusion.cascade_chain \
    --l1-run data/models/cascade-l1-run-2 --l2-run data/models/cascade-l2-run-2 \
    --l3-run data/models/cascade-l3-run-1 --release data/datasets/context-v2/release.json \
    --region howe-sound --l3-km 32 --out data/worlds/cascade-world-1-howe-sound
```

L1 and L2 cover the whole 48 km region; L3 covers a central square of `--l3-km`
× 1,024 m. Without `--l3-km`, L3 covers the whole field: the default for authored
layouts, which specify the world size. After L1 the land is made to drain (largest depressions kept as lakes,
others carved); after L2 compact deep hollows become tarns and streams are traced.
Water amounts follow the region class (`hydrology.WATER`) unless overridden with
`--lake-share`, `--tarn-share` and `--stream-km2`. The output is a 4 m world
(`manifest.json`, layers, hillshades of each level and the real region) that
exports directly. A 32 km L3 takes about an hour.

`cascade_l4` refines that 4 m world to 1 m in bands of rows:

```sh
$P -m topography_training.diffusion.cascade_l4 --run data/models/cascade-l4-run-5 \
    --world data/worlds/cascade-world-1-howe-sound --out data/worlds/cascade-world-1-howe-sound-1m
```

Bands are 2,048 m with a 256 m halo (`--band-m`, `--halo-m`); overlapping bands
share their starting noise and are cross-faded. Output layers are memory-mapped,
so host memory stays near 3 GB at any size. Rerunning the same command resumes
after the last finished band. Water keeps the smooth 4 m surface. `--km N` limits
the run to a central N × 1,024 m square for quick checks; older chain worlds need
`--class-id`. Speed is about 30,000 cells/s at 12 steps and stride 192: roughly
3 h for 16 km, 12 h for 32 km.

## Export

```sh
target/release/topography export heightmap data/worlds/cascade-world-1-howe-sound-1m \
    data/exports/cascade-world-1-howe-sound-1m
target/release/topography export view data/exports/cascade-world-1-howe-sound-1m
```

Tiles are written as `{layer}/L{level}/{prefix}_L{level}_{tx}_{ty}.png`, for example
`height/L0/height_L0_0_0.png` and `vegetation/L0/veg_L0_0_0.png` (prefixes `height`,
`water`, `snow`, `veg`, `landcover`). For authored layouts the chain reads the layout's forest share and snowline
bilinearly, and its class map by nearest cell, at positions warped by about 150 m, so
cover has no 128 m grid steps. After L4, redraw the cover at 1 m from the layout:

```sh
$PY -m topography_training.diffusion.cover --world data/worlds/<world>-l4 --layout data/worlds/<L0 dir>
```

Generated worlds get land cover before export:

```sh
$PY -m topography_training.diffusion.landcover --world data/worlds/<world> --treeline-m 1000
```

This adds `land-cover.u8` with ESA WorldCover codes derived by rule (a recorded
judgment call, not observed cover):
- water 80, snow/ice 70 and forest 10, taken from the world's own masks;
- bare 60 above 35° or more than 400 m above the treeline, grassland 30 just above it;
- dunes, badlands, lava and glacier forelands bare, canyons and badlands shrub 20,
  other open land grassland;
- class borders warped by about 150 m.

The export holds the tiles and `manifest.json`. Add `--masters` to also write
full-world COGs (Float32 `elevation.tif` and the mask layers). They duplicate the
world output, so they are only needed when the export travels without it.

Export streams the world layers row by row: a 32 km world at 1 m exports in
about 13 minutes with a peak under 200 MB. The 4 m chain output exports the same
way for a quick 3D look before L4.
