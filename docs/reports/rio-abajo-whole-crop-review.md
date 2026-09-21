# Río Abajo east-ridges whole-crop review

Measured 2026-09-21 after the project owner confirmed readiness for the bounded
run. The existing [whole-crop workflow](../workflows/whole-crop-ground-support.md)
completed without raising its limits. All 1,048,576 native count bins match an
independent count of retained XY points. This is ground-return evidence;
**terrain quality and human crop acceptance remain unresolved**.

The owner also approved the [dataset permission basis](rio-abajo-permission-review.md).
That separate decision is captured for the exact source tile and all three
existing crops. It does not accept their terrain or admit them to training.

The subsequent [qualification follow-up](rio-abajo-source-qualification.md)
records the owner's positive overall visual review and the next technical work.
The measured results and catalog snapshot below describe this earlier run.

## Whole crop compared with its centre sample

The full footprint is 512 × 512 m in EPSG:6566, with a 2 m acquisition buffer.
Counts select class 2, exclude withheld/synthetic points and include overlap
returns. The earlier centre sample covered 64 × 64 m, or 1.5625% of the crop.

| Measurement | Whole east-ridges crop | Earlier centre sample |
| --- | ---: | ---: |
| Area | 262,144 m² | 4,096 m² |
| Selected ground returns | 700,690 | 16,433 |
| Ground returns/m² | 2.673 | 4.012 |
| Occupied 0.5 m bins | 41.882% | 55.51% |
| Occupied 1 m bins | 73.319% | 83.74% |
| Occupied 2 m bins | 94.618% | 97.75% |
| Occupied 4 m bins | 99.768% | 100.00% |

The centre sample overstates average direct support for the whole footprint.
Occupancy means at least one selected return in a bin. None of these percentages
is valid terrestrial coverage, and the 99.768% figure does not satisfy the plan's
separate 99.5% terrain-quality gate. Aggregating counts creates no new detail.

The buffered CSV holds 15,464,573 points: 14,641,123 class 1, 710,548 class 2,
112,703 class 7 and 199 class 18. No class 9 or 20 appears in this retained
buffered selection; that absence does not prove that the crop contains no water.

## Spatial evidence for the next human review

There are 38 empty 4 × 4 m bins, forming 28 groups under four-neighbour adjacency.
The largest group comprises four bins in an 8 × 8 m square, 64 m², near easting
169700–169708 and northing 257752–257760 m. These are empty count bins, not a
measured invalid-terrain area or an interpolation-radius estimate.

A small local diagnostic overlays all empty 4 m bins on the existing hillshade:

`data/review/rio-abajo-ground-support/east-ridges-full-review-v1/review.html`

It links to the existing multi-light/profile/oblique preview. Grid columns A–H
run west to east; rows 1–8 run north to south. Each reference square is 64 × 64 m.
Red squares show only the 38 empty count bins; unmarked terrain has no quality
approval. The largest empty group is in **E6**, on the central ridge's southern
flank in this hillshade. Review D6–E6 alongside any intended ridge or passage.
Single-light shading does not establish feature geometry or acceptance.

The diagnostic derives its counts from the verified raster, checks the total
against the full report, and checks its 38-bin count against the independently
verified aggregation. It retains exact bin bounds, component extents, parent
hashes and the local `compose-review.py` reproduction script in
`spatial-review.json`. Reproducing it requires a new sibling output directory;
the script refuses to replace its completed diagnostic. It uses Python's standard
library and GDAL on the small count/hillshade rasters, without reacquiring points.

The subsequent owner clarification supersedes the request to identify individual
ridges, hollows or passages. Human source review is whole-tile yes/no, and
east-ridges has its yes. The reference grid remains a diagnostic aid. Technical
water/quality/repair classifications are implementation work; optional feature
annotations are not required. Do not turn occupied bins into a quality mask or
infer a safe flight envelope.

The retained NOAA collection metadata describes hydro-flattening with ground
points and hydro breaklines, and lists TIN interpolation. Those source methods
motivate examining interpolation and water handling; they do not classify this
crop for us. No point-height/DEM-height comparison was made: the recorded
GEOID18/GEOID12B processing difference remains unresolved.

## Resource observations and retained identity

Acquisition took 268.05 seconds and count rasterization took 30.29 seconds.
Independent verification and checksumming added time that was not separately
reported. Maximum Linux child RSS was 81,232 KiB (79.33 MiB); this does not include
Python or kernel page cache. CSV output was 1,175,307,805 bytes. The run used no
GPU and completed within the 30-minute, 20-million-point and 2-GiB CSV bounds.
These observations are for this crop and source; they do not guarantee the cost
of another run. Transferred network bytes and a full-cloud checksum remain unknown.

The completed measurement is retained under
`data/review/rio-abajo-ground-support/east-ridges-full-v1/`. Its `report.json`
SHA-256 is `b264ed336fbb6060a37edd48f94ac783131f51000ae15c49a59a540e7c1cafb9`.
The selected cloud retains ETag `"53ae7b22474d6d0ac160c4ca02a7fe85-112"`;
the ETag is not a SHA-256. Range reads used `If-Match` and the horizontal CRS
captured from the COPC header. No full source-cloud file was stored.

Spatial diagnostic SHA-256:
`c8138588b7acb54f31e3492e175f56c90cb572fc8056de31796c3bd75ed4c5fa`.
All generated maps, rasters, points, local scripts, catalogs and evidence copies
are ignored by Git and need separate backup.

## Catalog continuation

The real catalog now contains six immutable records and three current assistant
annotations. Each current record confirms the captured human permission decision;
none records terrain acceptance. East-ridges also captures the complete ground
report and spatial diagnostic. North-hollows and south-saddles retain their earlier
centre-sample evidence. All six quality findings and terrestrial coverage remain
unresolved for every crop.

New drafts, record IDs, portable exports, blocked eligibility results and a
pre-change SQLite backup are under `data/review/rio-abajo-curation-2026-09-21/`.
The earlier records and exports remain intact. `curate check` verifies six indexed
records, zero prepared regions and no unregistered records/unreferenced objects.
All three `prepare check` results remain ineligible with 17 blockers each; the
permission blocker has been removed. The raw source metadata remains immutable.
