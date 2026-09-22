# Surface-cover workflow checkpoint

Updated 2026-09-22. The version-2 quality, automatic preparation, regional
preparation and CPU numeric contracts now implement the master plan's snow/ice
and optional vegetation rules. See [automatic preparation](../design/automatic-preparation-v1.md),
[quality v2](../../schemas/regional-quality-v2.md),
[prepared v2](../../schemas/prepared-region-v2.md) and
[numeric v2](../../schemas/numeric-evaluation-v2.md).

- Snow/ice unknown blocks revised ground qualification; confirmed absence and
  present cover require scoped evidence. Vegetation unknown remains non-blocking.
- Measured Float32 surface elevation is retained separately from ground. Water
  and snow/ice receive deterministic ground fill and are excluded from ground
  height/slope metrics. Vegetation does not exclude valid ground.
- Existing products remain immutable. The gallery labels legacy products
  **Surface assessment pending**, retaining their links and saved votes.
- Automatic v2 profiles can append technical revisions to an existing Yes;
  failed findings, restricted permissions and the original human choice remain.
- Numeric v2 releases pin v2 products and carry all cover layers. Legacy
  releases retain their original contract. Existing bounded model experiments
  reject v2 inputs until their separate cover handling is implemented.

`bash scripts/check.sh` passed, including Rust formatting/build/Clippy, local
CLI/gallery workflows, v1 compatibility, v2 preparation, numeric datasets and
the installed optional model checks. The optimized CLI build also passed.
Synthetic cover cases exercise mixed/absent/partial/all-ice/all-vegetated tiles,
unknown vegetation, evidence/raster corruption, exact surface preservation,
ground fill, restart and technical backfill without another vote. A source-mask
mutation regression verifies checksum checks after GDAL reads. Independent Sol
reviews found no remaining material issue.

The implementation checkpoint preserved the live catalog at 77 records,
38 review identities and 30 prepared products, with 33 Yes and 2 No gallery
choices. Real source backfill follows this checkpoint. Source-layer acquisition,
useful cover overlays and future model/world/export propagation remain work in
the master plan; successful synthetic verification does not assess a real tile.

Local restart details, verification logs and a catalog snapshot are retained in
`data/review/surface-cover-implementation-v1/`. Scoped Río Abajo profile drafts
are under `data/review/surface-cover-backfill-v1/`. A separate unexecuted bounded
St. Helens classification job is under `data/review/st-helens-surface-cover-v1/`;
it requires resource readiness before launch.
