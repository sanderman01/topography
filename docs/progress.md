# Progress

Updated 2026-09-29. Follow the [master plan](plans/terrain-design-master-plan.md).

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
- **Terrain diffusion pilot:** release `diffusion-v8` (132 tiles, all twelve
  classes), four 8 h GPU training runs. Run 4 conditions on snow/ice and
  vegetation and passed the blind review (32/32 Yes). A 4 km Milford Sound pilot
  world took 96 min of GPU time. See the [diffusion plan](plans/terrain-diffusion-v1.md).
- **Export and viewing:** 16-bit heightmap tiles (257² `shared_edge`), quadtree
  LOD levels, mask tiles, and a loopback 3D viewer.
- **World sketch test (2026-09-29):** `world sketch` builds a world input from a
  procedural recipe, `world preview` shows any world input before generation,
  and `generate --scale` makes quick 4 m or 2 m versions. A 32 km caldera island
  was generated at 4 m (44 min) and 2 m (2 h 40 min). The owner rejected it:
  large landforms looked hand-drawn, and the canyon country and karst came out
  flat. The 2 m export stays in `data/exports/caldera-island-2m` for comparison.
- **Housekeeping (2026-09-29):** pruned superseded releases (v5–v7), weights of
  runs 1–3 and the smoke run, run 4's intermediate snapshots, download chunks,
  caches, unreferenced review output and the rejected sketch worlds. `data/`
  went from 146 GB to 80 GB.

## Next

The [terrain cascade](plans/terrain-cascade-v1.md) replaces the single 32 m →
1 m step with levels at 128 m, 32 m, 4 m and 1 m, each trained on real terrain at
its scale.

1. [World layout spec](plans/world-layout-spec-v1.md): brief, spec, fit check
   and reference sheet before any generation.
2. GLO-30 context regions and the 128 m / 32 m models: done as pilots. 64 of 74
   regions accepted; L2 run 1 (3 h) passed the blind review 30/32 (both No were
   open-sea windows), L1 run 1 (2 h) 7/7. L1 output gets its sea resolved to 0 m.
3. Reduced 4 m lidar regions and the 4 m model.
4. Run 5 of the 1 m model on the 4 m surface (fixes for local shape, glaciers
   and flats).
5. Banded 1 m generation, then the first cascade world.

Open reviews: the run 4 twin-free blind review (`data/models/diffusion-run-4/eval-2`)
and the Milford pilot world's mid scale.

Details: [source coverage](reports/pilot-source-coverage-v1.md),
[automatic preparation](reports/automatic-preparation-rollout-v1.md),
[surface-cover checkpoint](reports/surface-cover-workflow-v2.md).
