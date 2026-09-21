# Río Abajo source qualification

Updated 2026-09-21. The project owner opened the
[whole-crop review](rio-abajo-whole-crop-review.md) and replied:
“I've opened it. It looks fine. Now what?” This is recorded as positive overall
visual suitability. East-ridges remains the first candidate for qualification.
Individual grid-square selection is not required to continue. No protected
landmarks, flight envelope or quality radius were supplied.

The owner has now explicitly clarified that source review is whole-tile yes/no,
without selecting individual ridges, hollows or other features. The earlier
“looks fine” is recorded as east-ridges acceptance for candidate selection in
`data/review/rio-abajo-tile-decisions-2026-09-21/`, with the exact responses and
assistant recorder identified. No technical findings were promoted to pass.
The later batch response “both yes” also selects north-hollows and south-saddles.
The catalog now has sixteen historical records, three current human acceptances
and three verified prepared regions. No training dataset has been released. The batch response, pinned
identities, exports and new record IDs are under
`data/review/rio-abajo-tile-decisions-2026-09-21/accepted-batch-v1/`.
The earlier measurement/mask checkpoint is preserved in
`data/review/rio-abajo-full-crop-completion-2026-09-21/`. Current qualified decisions,
complete masks, preparation requests and verification are in
`data/review/rio-abajo-supplier-qa-pilot-v1/`.

The earlier assistant annotation captured that exact response against the viewed
map's hash and the original crop identity. It preserves the confirmed permission
decision and unresolved technical findings in its historical seven-record snapshot.

## Water observation and acquisition scope correction

The owner subsequently wrote: “I do not think there is any water bodies present
in the terrain segment” and identified
`data/review/rio-abajo-ground-support/east-ridges-full-review-v1/review.html`.
Record this as no apparent water in the reviewed crop. The red boxes in that
map mark empty ground-return bins, not detected water. There is no identified
water body in the evidence inspected so far. A water mask may legitimately be
entirely terrestrial; requiring the mask does not require finding a water body.

The owner also explicitly limited acquisition to publicly downloadable data and
prohibited sending or drafting emails. The earlier email-request draft has been
removed. No email was sent. Treating unpublished survey vectors as a required
next input was an unnecessary dependency and is superseded by this correction.
The visual water observation is captured with the latest whole-tile decision;
previous catalog records and immutable evidence remain unchanged. It does not by itself establish a
verified per-cell classification or terrain-quality acceptance.

The implementer subsequently inspected small public
[USGS context images](https://basemap.nationalmap.gov/arcgis/rest/services/USGSImageryOnly/MapServer)
covering all three crops. No open water body was identified. The 200 × 200 images
sample each 512 m crop at 2.56 m; canopy and shadows limit visibility. Based on
this context and the native terrain review, all three now have documented
all-terrestrial masks. This is a classification judgment, not proof that no
concealed water exists. It excludes no land from the quality denominator.

The masks under `data/review/rio-abajo-quality-v2-2026-09-21/` also contain zero
optional feature annotations and zero project repairs. The latter does not deny
supplier interpolation. That intermediate checkpoint left quality unset in each crop's
`water-classified-v1/` directory. Those incomplete assessments remain preserved.
The current supplier-QA qualification below completes the quality layer. Curation
identifies Codex's technical judgments separately from the owner's whole-tile yes.

## Completed sequential measurements

The owner explicitly approved running north-hollows and south-saddles
sequentially. Both completed under the saved limits, with no GPU and no terrain
changes. Each covers 512 × 512 m with a 2 m acquisition buffer, and every native
count bin was independently verified against the retained XY points.

| Measurement | North-hollows | South-saddles |
| --- | ---: | ---: |
| Buffered points | 16,621,724 | 14,671,955 |
| Selected ground returns inside crop | 742,843 | 601,406 |
| Ground returns per m² | 2.834 | 2.294 |
| Occupied 0.5 m bins / 1,048,576 | 467,002 | 374,712 |
| Empty 4 m bins / 16,384 | 59 | 249 |
| Acquisition time | 290.62 s | 290.82 s |
| Count-raster time | 32.67 s | 29.19 s |
| Maximum child RSS | 71.99 MiB | 73.40 MiB |
| Retained point CSV bytes | 1,263,251,274 | 1,115,100,350 |

Verification adds unreported time. Child RSS excludes the Python parent and
cache; total RAM and transferred network bytes were not measured. CSVs total
about 2.22 GiB, plus small count rasters and metadata. No full source COPC file
was stored. Counts include overlap returns; empty bins are neither demonstrated
terrain defects nor water. These measurements do not establish fine-feature
accuracy or terrestrial quality coverage.

Completed reports are under
`data/review/rio-abajo-ground-support/{north-hollows,south-saddles}-full-v1/`.
The completion directory pins both reports, approved plans and readiness
evidence, and retains the catalog backup, updated decisions and portable exports.
All artifact sizes and non-CSV hashes were rechecked. The probe already hashed
each CSV and verified its count bins; those operations were not redundantly rerun.

## Additional sampling evidence

A small GDAL calculation on the existing count raster measures distance from
each cell centre to the nearest occupied ground-bin centre within the crop:

| Statistic | Distance |
| --- | ---: |
| Median | 0.500 m |
| 95th percentile | 1.118 m |
| 99th percentile | 1.803 m |
| Maximum | 5.500 m |

These are nearest-rank percentiles including occupied bins at zero distance.
The maximum occurs at column 610, row 688 (zero-based), in reference square E6.
It was independently checked against neighbouring occupied bins. The zero-distance
count agrees with the 439,165 occupied native bins in the verified measurement.

The [GDAL proximity operation](https://gdal.org/en/stable/programs/gdal_raster_proximity.html)
uses explicit count values 1–10 as targets; empty bins encoded -1 are excluded.
It preserves the count raster grid. That grid inherits the already documented
PDAL origin serialization difference of less than 1e-8 m from the review grid;
this diagnostic is not a preparation mask.

Distances are to bin centres, not exact point locations. A point can lie up to
0.354 m from its half-metre bin centre. Returns outside the crop are omitted,
so edge distances can be overstated; 3,283 cells are potentially affected by that
boundary. No quality cutoff, repair radius or elevation accuracy follows from
these statistics. No new point-cloud run was needed.

## Public source evidence and next action

The official [survey report](https://noaa-nos-coastal-lidar-pds.s3.amazonaws.com/laz/geoid18/9102/supplemental/PR_USVI_Lidar_2018_Project_Report.pdf)
lists the exact tile on page 90. It describes delivered hydro breaklines,
ground-void polygons and low-confidence polygons on pages 5, 34 and 36–37.
These are optional evidence sources if publicly downloadable and relevant to
this crop, not mandatory inputs.

The inspected NOAA collection/supplemental listings and USGS OPR/LPC project
roots provide no corresponding vector download. This is a scoped search, not
proof that the files are unavailable elsewhere. The LPC delivery lives beneath
`PR_PuertoRico_VirginIslands_2018_D18/PR_PRVI_D_2018/`, unlike the shorter OPR path.

All three existing tiles have their whole-tile yes and completed full-crop
sampling measurements. No further point-count run or feature-marking exercise
is needed. Additional public downloads should answer a concrete technical
question; correspondence and unavailable supplementary files are not prerequisites.

## Adopted pilot policy and prepared regions

The owner delegated the decision: “Do whatever you judge appropriate. Try to
keep it simple.” The implementer adopted
[supplier_qa_pilot_v1](../design/source-quality-pilot-v1.md). Published supplier
product QA is the evidence basis for finite provider-valid native ground, with
concrete local defects excluded. Routine supplier interpolation beneath forest
is acceptable. No further confidence threshold or LiDAR measurement is required.
The existing 99.5% mask-coverage gate and all source/permission checks remain.

All three crops have 1,048,576 usable terrestrial cells under that rule, with no
identified local defect exclusions, zero project repairs and no optional feature
annotations. All six technical findings now pass under Codex's documented
judgment. This does not claim independently measured fine-feature accuracy.
Original human selections and permission evidence are retained unchanged.

Each crop is prepared as its own 512 × 512 m region because the selected
footprints are disjoint. The native 0.5 m grid and PRVD02 metre values are retained;
no resampling, smoothing, repair, mosaic across gaps or vertical transformation ran.

| Crop | Prepared manifest SHA-256 |
| --- | --- |
| North-hollows | `0bea5dd7cc7fb507b9d32472ec32805bc542d1805b79eec943cda473871819c2` |
| East-ridges | `e70cda3efae445259d83394c2cec2d969eae135c133287ade735a7a6727d6373` |
| South-saddles | `f366ddbc9f7ee3c65ea3dee522c0c3ccd81843bd1f25a8d439b2ff869c7aebd1` |

Each manifest is `data/prepared/<sha256>/manifest.json`. `prepare verify` reports
integrity verified, registered and current for all three. Independent decoded
comparison checked all 3,145,728 Float32 samples bit for bit against the accepted
native crops, plus every output mask. The policy's exact bytes, owner delegation,
source-report identity and per-crop qualification are captured in the pinned
curation records and portable exports. `native-comparison.json` retains the result.

The prepared schema still records `review_state: unreviewed` and
`training_admission: not admitted`; preparation is not a dataset release.
Do not request another visual approval of these unchanged source tiles. The next
selection is the [new six-tile batch](pilot-tile-batch-2.md).

## Retained artifacts and verification

Ignored local directory: `data/review/rio-abajo-east-ridges-qualification-v1/`.
It contains the visual-response evidence, annotation and record ID, pre-change
SQLite backup, distance raster/report, source listings, the 4,790,079-byte project
PDF and `source-evidence.json` with URLs and checksums. Preserve it separately
from Git. Existing source data, reviewed maps and prior records remain unchanged.
The `next_action` in the retained `source-evidence.json` describes the earlier
contact proposal and is superseded by the public-only continuation above.

Verification checked the distance raster's hash, CRS, dimensions, inherited grid,
zero-bin count and maximum; catalog integrity and the current annotation were
checked through the CLI. Production code was unchanged.
