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

## Next

1. Backfill surface-cover evidence for existing tiles without repeat votes:
   three Río Abajo drafts and the St. Helens crater (NOAA 9211 class-21 snow).
2. Verify present, absent, partial, unknown and fully covered cases on real tiles,
   including date/boundary uncertainty.
3. Automate source-layer acquisition and add optional cover overlays in the gallery.
4. Resolve the three technical exceptions: two dunes tiles with striping and the
   St. Helens glacier tile.
5. Broaden terrain families (six of about nine) with reusable source profiles.
6. Later: label editing, cross-survey alignment, a 4 km pilot world, full extent,
   editing/rebuild and export.

Details: [source coverage](reports/pilot-source-coverage-v1.md),
[automatic preparation](reports/automatic-preparation-rollout-v1.md),
[surface-cover checkpoint](reports/surface-cover-workflow-v2.md).
