# Progress

Updated 2026-10-11. Follow the [master plan](plans/terrain-design-master-plan.md).

## Done

- **Foundation:** Rust CLI validates presets, legend and source metadata;
  `scripts/check.sh` runs format, build, Clippy and CLI workflows.
- **Acquisition:** USGS and LINZ discovery, batch planning, resumable HTTPS
  download and manual import of raw Float32 GeoTIFFs with checksums.
- **Curation and review:** SQLite catalog and a persistent
  [review gallery](workflows/batch-source-review.md) with whole-tile Yes/No
  votes. Gallery v22 adds the expansion-5 tiles for thin classes.
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
  - First models, all passing blind review: L1 128 m (run 2, 7/7), L2 32 m (run 2,
    32/32), L3 4 m (run 1, 32/32), L4 1 m (run 5, 32/32; same-seed correlation
    0.29). They ignored the class (see below).
  - Terrain continues under water (direction change 2026-10-02): sea is ground
    below 0 m connected to open water.
  - `cascade_chain`: L1 → L2 → L3 from an authored layout or a real region, with
    drainage, lakes, tarns and streams by terrain class.
  - `cascade_l4`: banded, resumable 1 m generation with memory-mapped output.
- **[Class conditioning](plans/class-conditioning-v1.md)** (2026-10-03 to 10-05): all
  four models had ignored the terrain class. Class gates (recognition and per-class
  roughness with the coarse input hidden), class embedding, balanced and mixed-class
  windows, coarse dropout and blur fixed that. Releases `context-v5` (all 127 GLO-30
  regions voted), `lidar-v3` (79 accepted lidar regions) and `diffusion-v9` (160 1 m tiles).
- **Authored layouts:** the Fujion [brief and spec](../configs/layouts/fujion/brief.md)
  render to L0 with `topography world layout`, with an overview, reachability check
  and reference sheet (`layout.py`). The layout is then painted in GIMP (`paint.py`:
  `export`, `import`, `relabel`, `transplant`, `roughen`); the map border is sea.
- **[Biome forms](plans/biome-forms-v1.md)** (2026-10-07 to 10-11), from the
  [2026-10-07 expert review](reports/expert-review-2026-10-07.md):
  - legend v3: 18 classes with subclasses (layered canyons, cone karst, drowned karst,
    crest-train dunes, cinder cones, plain), per-cell glacier and plain labels;
  - real exemplars held at L1 for craters, calderas, cinder cones and drowned karst;
  - streams reach the sea, follow smoothed paths and have descending beds;
  - snow and tree lines follow aspect, slope, gullies and noise;
  - form channels (roughness, flats, peaks, hollows, direction) at every level and
    local 128-cell L1 windows.

  Current models, all on legend v3 with form channels and passing their gates:

  | Level | Run | Data | Roughness rank, coarse shown |
  | --- | --- | --- | --- |
  | L1 128 m | `cascade-l1-run-6` | `context-v5` | 0.97 |
  | L2 32 m | `cascade-l2-run-8` | `context-v5` | 0.82 |
  | L3 4 m | `cascade-l3-run-4` | `lidar-v3` | 0.93 |
  | L4 1 m | `cascade-l4-run-7` | `diffusion-v9` | 0.90 |
- **Export and viewing:** 16-bit heightmap tiles (257² `shared_edge`) named
  `{layer}_L{level}_{tx}_{ty}`, quadtree LOD levels, mask, land-cover and biome tiles,
  `checksums.txt`, and COG masters with `--masters`. Export streams row by row (32 km
  at 1 m in about 13 min, under 200 MB). The loopback 3D viewer shows biome colours.
- **Fujion at 32 km and 1 m (2026-10-11):** `data/worlds/fujion-v3-chain-14-l4`, L3 in
  66 min and 1 m bands in 1,010 min; export `data/exports/fujion-v3-chain-14-l4`
  (1.1 GB without masters).

## Next

1. Owner review of Fujion chain 14 at 1 m.
2. Generated canyons, badlands and dunes still have too few
   flats (L1 surfaces stay sloped).
3. Rivers entering from outside the world need a layout input.
4. A 64 km world after the biomes are accepted; then regional reruns and editing
   (master plan §9).

Details: [source coverage](reports/pilot-source-coverage-v1.md),
[automatic preparation](reports/automatic-preparation-rollout-v1.md),
[surface-cover checkpoint](reports/surface-cover-workflow-v2.md).
