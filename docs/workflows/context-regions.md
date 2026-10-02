# GLO-30 context regions

Context regions are the training data for the coarse levels of the
[terrain cascade](../plans/terrain-cascade-v1.md) (L1 at 128 m, L2 at 32 m). Each
region is a square of real terrain, 49,152 m across by default, at 32 m in the local
UTM zone, with game cover and a terrain class label.

## Plan

A plan TOML lists regions by id, class, centre and a reference place. See
[`catalog/plans/context-regions-v1.toml`](../../catalog/plans/context-regions-v1.toml).
`class_id` labels the whole region; optional `[[regions.areas]]` polygons of
`[lon, lat]` vertices override it (later areas win, class 0 means unlabelled).
`extent_m` is optional: a multiple of 1,024 m from 16,384 to 98,304.

## Build

```sh
target/release/topography context build configs/runtime.example.toml \
    catalog/plans/context-regions-v1.toml data/context/regions-v1
```

Each region takes one to three minutes, mostly downloads. The first region in a
country also downloads its Geofabrik OpenStreetMap extract once; only its water
features are kept in the runtime cache. Rerunning skips regions that are already
built and still match the plan. A region whose plan entry changed is reported as
stale: give it a new id, or delete the old directory on purpose. Failed regions are
listed and the command exits 1 after the others finish.

Each region directory holds:

| File | Content |
| --- | --- |
| `height.f32` | Float32 metres (EGM2008), GLO-30 averaged to 32 m; 0 m where GLO-30 has no tile (open sea) |
| `classes.u8` | Terrain class per cell (terrain-v2 legend) |
| `water.u8` | 255 water, 0 land (WorldCover majority class plus mapped OSM water) |
| `snow-ice.u8`, `vegetation.u8` | 2 present, 1 absent (game_cover_v7 rules) |
| `land-cover.u8` | WorldCover class, majority per cell |
| `overview.png` | Tinted hillshade for review |
| `region.json` | Grid, CRS, sources, licences, statistics and file digests |

About 23 MB per region. GLO-30 is read from its public COGs and not kept; the manifest
lists the tile URLs, so any region can be rebuilt.

GLO-30 is a surface model: forests add their canopy height. At 32 m and coarser this
is acceptable. It never supervises the 4 m or 1 m levels.

## Review

```sh
target/release/topography context serve data/context/regions-v1 8767
```

Open the printed loopback URL. Each region shows its class, size, height range, flat
share and a Google Earth link, with the hillshade inspector and cover overlays. **Y**
and **N** save a vote and move on; arrow keys browse. Votes are appended to
`decisions.jsonl` in the regions directory, keyed by the digest of `region.json`, so
a rebuilt region needs a new vote. Only Yes regions go into coarse-level releases.

## Licences

- Copernicus GLO-30: free worldwide licence allowing any use, including derived
  products; attribution "produced using Copernicus WorldDEM-30 © DLR e.V. 2010-2014
  and © Airbus Defence and Space GmbH 2014-2018 provided under COPERNICUS by the
  European Union and ESA; all rights reserved". Recorded in
  `configs/provider-licences-v1.toml`.
- GEBCO 2025 Grid: public domain, free use with the acknowledgement "GEBCO
  Compilation Group (2025) GEBCO 2025 Grid".
- ESA WorldCover (CC BY 4.0), GLIMS and OpenStreetMap (ODbL), as for the game cover.

## Training release

```sh
target/release/topography context release data/context/regions-v1 data/datasets/context-v2
```

The release hard-links the files of every region whose latest vote is Yes and pins
their digests and the votes file. Whole regions are held out for validation: one per
class with at least six accepted regions, picked by a stable hash of the id.

GLO-30 regions also get `seabed.f32`: the GEBCO 2025 grid (15 arc-second) read over
HTTPS from the public CEDA tiles and resampled to the region grid (cubic spline). The
regions themselves stay unchanged, so their votes stay valid. Training joins it to the
land (`cascade.join_seabed`): GLO-30 ground at 0 m that reaches GEBCO depths is sea;
there the land slope continues down from the coast until it meets the deepest GEBCO
within 1 km, and GEBCO takes over 1–3 km out. The water layer then keeps only water
above sea level. `context-v1` (no seabed) trained the retired L1 layout 4.

## Cascade levels L1 and L2

```sh
P=data/models/diffusion-env-v1/.venv/bin/python
$P -m topography_training.diffusion.train --release data/datasets/context-v2/release.json \
    --run-dir data/models/cascade-l2-run-2 --level l2 --channels 64,96,128,192,256 --hours 3
$P -m topography_training.diffusion.cascade_eval --run-dir data/models/cascade-l2-run-2 \
    --out data/models/cascade-l2-run-2/eval-1
```

Both levels generate a residual over the level above, on 256² windows:

- **L2 (32 m, 8 km windows)** refines the true surface block-averaged to 128 m on a
  randomly shifted grid, upsampled and smoothed (sigma 2 cells).
- **L1 (128 m, 32 km windows)** refines the mid-surface of an L0-like envelope: local
  low and high over 2 km, smoothed at 2 km. Sea is where that surface lies below
  0 m; there is no land/sea channel. Painted layouts give only this much, so the
  model has to invent ranges, valleys and coastlines itself instead of tracing brush
  strokes.

Conditioning (layouts 3 and 5): the coarse surface as a per-window unit-range shape
channel, its east and north slopes, absolute height / 4,096 m, then for L2 water
above sea level, snow/ice and vegetation, for L1 envelope relief, then the class
one-hot (dropped 10% of the time). Hero-feature lines are not implemented yet. The
seabed is part of the target, so coastlines come out where generated ground crosses
0 m, and the sea in generated terrain is the ground below 0 m connected to the field
edge (`cascade.open_sea`).

`cascade_eval` writes a blind review of held-out regions (`review.html`, with
`key.json` and the side-by-side `compare.html` kept closed until the votes are in),
terrain statistics and a nearest-training-window distance to detect memorization.
