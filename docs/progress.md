# Progress

Updated 2026-09-25. Follow the [master plan](plans/terrain-design-master-plan.md).

## Done

- **Foundation:** Rust CLI validates presets, legend and source metadata;
  `scripts/check.sh` runs format, build, Clippy and CLI workflows.
- **Acquisition:** USGS discovery, resumable HTTPS download and manual import of
  raw Float32 GeoTIFFs with checksums; raw assets are immutable.
- **Curation and review:** SQLite catalog, batch download queue and a persistent
  [review gallery](workflows/batch-source-review.md) with whole-tile Yes/No
  votes (33 Yes, 2 No).
- **Preparation:** native same-survey preparation without resampling; 30
  prepared regions across 11 areas and six families (Río Abajo karst, Banks
  Peninsula, Grand Canyon, Zion, St. Helens, Milford Sound, Badlands and others).
- **Automatic preparation:** a Yes queues technical checks and preparation in
  the gallery worker; exceptions are reported without a repeat vote.
- **Numeric work:** geographically split evaluation release and CPU numeric
  check. Both model experiments (image codec, scalar fit) failed; continue with
  curated real terrain and blending.
- **World layout:** the first 16 m layout was rejected for repetition, visible
  joins and dull geography. Expand the library before another world review.
- **Snow/ice and vegetation masks (v2 contracts):** implemented and verified on
  synthetic cases. Unknown snow/ice blocks qualification; vegetation is optional.
  Legacy products show "Surface assessment pending".
- **Cover backfill (2026-09-25):** [report](reports/surface-cover-backfill-v1.md).
  `cover draft/merge` fetches WorldCover, GLIMS and Sentinel-2 evidence; the
  gallery shows cover overlays. 15 tiles are v2 Ready (Río Abajo, dunes,
  Grand Canyon, Badlands, Banks, one gorge); dunes striping qualified as a
  sub-accuracy supplier artifact. Gallery v13: 33 Yes, 2 No; 45 prepared products.

## Next

1. Decide how to fill Sentinel-2 seasonal-snow gaps (15 tiles: Puerto Rico,
   Milford, Zion) and whether tiny unknown snow/ice fractions may pass
   (St. Helens flanks). The crater stays blocked by unresolved glacier extent.
2. Broaden terrain families (six of about nine) with reusable source profiles.
3. Later: label editing, cross-survey alignment, a 4 km pilot world, full extent,
   editing/rebuild and export.

Details: [source coverage](reports/pilot-source-coverage-v1.md),
[automatic preparation](reports/automatic-preparation-rollout-v1.md),
[surface-cover checkpoint](reports/surface-cover-workflow-v2.md).
