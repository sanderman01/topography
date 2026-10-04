# Progress

Updated 2026-10-03. Follow the [master plan](plans/terrain-design-master-plan.md).

## Done

- **Foundation:** Rust CLI validates presets, legend and source metadata;
  `scripts/check.sh` runs format, build, Clippy and CLI workflows.
- **Acquisition:** USGS and LINZ discovery, batch planning, resumable HTTPS
  download and manual import of raw Float32 GeoTIFFs with checksums.
- **Curation and review:** SQLite catalog and a persistent
  [review gallery](workflows/batch-source-review.md) with whole-tile Yes/No
  votes. Gallery v21: 205 tiles, 167 Yes, 29 No, all accepted tiles Ready.
- **Preparation:** automatic after a Yes, through the gallery worker, including
  technical checks, artifact checks (facets, stripes, seams) and game cover.
- **Game cover v7:** snow/ice, vegetation, water and WorldCover land cover per
  tile, from WorldCover, GLIMS and OpenStreetMap. Snow/ice is ground; water is
  smoothed at 10 m.
- **1 m diffusion pilot:** release `diffusion-v8` (132 tiles, all twelve
  classes); runs 1–4 on a 32 m coarse surface, run 4 passed the blind review.
  It ignored the local shape (same-seed outputs of different windows correlated
  0.99) and became cascade level L4. See the [diffusion plan](plans/terrain-diffusion-v1.md).
- **Caldera island sketch (2026-09-29):** a 32 km world on a procedural 32 m
  base was rejected: large landforms looked hand-drawn. This led to the cascade.
- **[Terrain cascade](plans/terrain-cascade-v1.md)** ([workflow](workflows/terrain-cascade.md)):
  - Context regions: 64 of 74 GLO-30 regions accepted, with a GEBCO seabed
    (release `context-v2`); 36 lidar regions averaged to 4 m accepted (`lidar-v1`).
  - Models, all passing blind review: L1 128 m (run 2, 7/7), L2 32 m (run 2,
    32/32), L3 4 m (run 1, 32/32), L4 1 m (run 5, 32/32; same-seed correlation
    0.29).
  - Terrain continues under water (direction change 2026-10-02): sea is ground
    below 0 m connected to open water.
  - `cascade_chain`: L1 → L2 → L3 from a layout derived from a real region, with
    drainage, lakes, tarns and streams by terrain class.
  - `cascade_l4`: banded, resumable 1 m generation with memory-mapped output.
- **Export and viewing:** 16-bit heightmap tiles (257² `shared_edge`), quadtree
  LOD levels, mask tiles and COG masters, streamed row by row (32 km at 1 m in
  13 min, under 200 MB), and a loopback 3D viewer.
- **First cascade world (in progress):** Howe Sound layout, 32 km, L1–L3 at 4 m
  in about an hour → `data/worlds/cascade-world-1-howe-sound`, 4 m export
  `data/exports/cascade-world-1-howe-sound-4m`.

## Next

0. **Class conditioning** comes first:
   [class conditioning, step 4](plans/class-conditioning-v1.md#steps).
   1. Done (2026-10-04): gates v2, the guidance experiment (not enough alone), the
      fully voted `lidar-v3`, `--scale-shift` and `--coarse-blur`.
   2. L3 run 2 on `lidar-v3` (started 2026-10-04, 5 h), then its gates and an owner
      review.
   3. If it works, retrain L2 and L1 with the same changes, then L4; regenerate
      Fujion.

   Background: [expert review](reports/expert-review-2026-10-04.md).
1. Owner review of the first world at 4 m; then L4 at 1 m (about 12 h of GPU, or
   a 16 km square in about 3 h first) and export.
2. [World layout spec](plans/world-layout-spec-v1.md): authored layouts rendered to
   L0 rasters (class map, envelope, feature lines) instead of layouts derived from
   real regions.
3. Open cascade issues ([hydrology and data issues from the expert review](plans/terrain-cascade-v1.md#gates-and-fallbacks)): rivers entering from outside the world need a layout
   input; generated mesas and dunes are softer than real at L1/L2; some generated
   islands have rounded outlines.

Details: [source coverage](reports/pilot-source-coverage-v1.md),
[automatic preparation](reports/automatic-preparation-rollout-v1.md),
[surface-cover checkpoint](reports/surface-cover-workflow-v2.md).
