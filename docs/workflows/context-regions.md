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
- ESA WorldCover (CC BY 4.0), GLIMS and OpenStreetMap (ODbL), as for the game cover.
