# Grand Canyon preference and landmark continuation

Follow-up: all five new tiles are accepted. See the
[gallery fix and vote recovery](gallery-vote-recovery-v1.md). Counts below retain
the original publication checkpoint.

2026-09-21. The owner liked the Grand Canyon side-canyon tile and asked for more.
Three additional Grand Canyon crops and two Zion crops were prepared for the
next whole-tile gallery. The preference is also retained in the master plan for
later batches. All four earlier gallery choices were saved Yes decisions; none
was inferred or duplicated from the new chat message.

Gallery v7 has 28 tiles: 21 selected Yes, two No and five pending. Its manifest
is `data/review/library-gallery-v7/gallery.json`, SHA-256
`5f52f7528e2ee89583741df98efc7e4a0caa1b2ad62be0df3c79ad177e37fe7d`.
The catalog has 55 immutable records, 26 review heads and 20 prepared regions;
historical metadata identities explain the additional heads. Comparison against
the consistent pre-work backup preserves all 50 earlier records and all 17 earlier
prepared entries. The prepared footprint union is 22.232003 km² across nine named
areas, with overlap counted once. Full inventory and preservation results are in
`data/review/landmark-continuation-v1/`.

The [Grand Canyon qualification](grand-canyon-qualification-v1.md) and
[St. Helens qualification](st-helens-qualification-v1.md) retain supplier evidence
and original selections. The glacier-covered crater rim remains selected but
excluded from preparation; glacier elevation is not treated as measured ground
beneath the ice or relabeled as water.

The original source declarations lacked survey, acquisition-range, CRS and
processing fields required by preparation. New immutable imports complete those
declarations from public evidence. Three unchanged selected crops have
metadata-only successors under the [curation workflow](../workflows/source-curation.md).
Their curation records preserve the original Yes bytes and identify the
implementer's technical work separately. Original receipts, reviews, decisions
and galleries remain available.

The primary agent independently checked every native elevation and provider-mask
TIFF byte, raw asset checksum, native window, actual grid, CRS, axis mapping and
NAVD88-metre interpretation for all three successors. The complete comparison is
`data/review/landmark-continuation-v1/metadata-equivalence.json`, SHA-256
`ace43282f4a51b5bfde05f320cc1111ed883c91c1b558800243812f9f13c043c`.
This supplements the equivalence evidence already captured with each curation
record. It establishes unchanged terrain and coordinates; it does not extend
selection to another footprint or certify new human technical findings.

The five new candidates are documented in the
[Grand Canyon expansion](grand-canyon-expansion-v1.md) and
[Zion review](zion-review-v1.md). Their 17,301,504 native Float32 samples and
provider masks were compared exactly with the raw sources. Metadata was completed
before publishing this batch; independent comparisons also preserve the initial
unvoted native outputs. Local water/quality qualification remains subsequent work
for any newly selected crops. No model job or dataset release was started.

Further discovery used the Milford Sound example from [locations](../locations.md).
The public LINZ Southland 1 m collection, simplified capture footprint and twelve
nearby STAC item documents were archived and checked against published hashes.
Two assets near the head of Milford Sound are listed in
`data/staging/fjord-shortlist-v1/download-targets.json`, with a 400 MB aggregate
asset cap. They are queued, not downloaded or admitted. Item rectangles do not
establish complete fjord or cliff coverage; actual raster inspection comes next.

Sol subagents handled Grand Canyon expansion and the two independent source
qualification tasks. [AGENTS.md](../../AGENTS.md) now records this default and
requires shared catalog writes and integrity scans to run sequentially. Separate
metadata and source work can still run in parallel. The checkout and original
branch were retained; no worktree was used.

Final verification: `topography curate check` validates all 55 records and 20
prepared regions, with no unregistered records or unreferenced evidence objects.
Actual Chromium inspection passed without voting; the screenshot was inspected
and every catalog row remained equal to the pre-browser snapshot. An independent
Sol review found no remaining actionable issues after the complete equivalence
proof was linked from both qualification reports. No production code changed;
verification exercised the real data, curation, preparation and gallery workflows.
