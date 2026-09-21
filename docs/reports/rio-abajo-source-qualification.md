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
The catalog has thirteen historical records and three current human acceptances.
No prepared region or training admission exists. The batch response, pinned
identities, exports and new record IDs are under
`data/review/rio-abajo-tile-decisions-2026-09-21/accepted-batch-v1/`.
The subsequent measurement and mask updates preserve those selections in
`data/review/rio-abajo-full-crop-completion-2026-09-21/`.

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
supplier interpolation. Latest drafts and assessments are in each crop's
`water-classified-v1/` directory; earlier incomplete drafts remain preserved.
The quality mask is still unset. Each assessment has one missing-mask blocker
and null coverage counts. Current curation records identify Codex's water
classification judgment separately from the owner's existing whole-tile yes.

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

## Decision needed before source admission

The remaining issue is the evidence rule for **supported ground**. The current
99.5% gate says how much land must meet that rule, but does not define how to
qualify supplier-interpolated cells beneath forest. The survey report describes
linear interpolation and ground voids; regional accuracy checkpoints and its
1:5000 artifact review do not measure fine-feature fidelity in these crops.
More occupancy diagnostics cannot supply the missing criterion.

Proposed pilot policy, **not yet approved or implemented**:

- Use the released native bare-earth DEM and its documented supplier QA as the
  evidence basis for usable ground, retaining original units, datum, source
  identity and processing history. Apply the existing 99.5% threshold to finite,
  provider-valid terrestrial cells after excluding documented local defects.
- Preserve whole-crop sampling diagnostics and all known uncertainty. Exclude
  demonstrated large voids or artifacts; do not assume empty count bins are
  defects, or claim that a 0.5 m grid establishes measured 0.5 m feature accuracy.
  Missing supplier QA polygons remain an explicit limitation.
- Identify this evidence basis in a versioned policy and carry it into prepared
  and eventual dataset provenance. Keep source selection, preparation and later
  training release distinct. Existing human yes decisions remain valid.

This is the recommended practical route for the pilot, accepting uncertainty
in supplier interpolation. It changes the admission evidence basis and therefore
requires the owner's agreement before changing the plan or policy. The alternative
is to declare a required feature scale/error and design one bounded validation
experiment against it; an unplanned height comparison alone would not settle
acceptability. No numeric tolerance or all-supported quality mask has been invented.

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
