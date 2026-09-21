# East-ridges source qualification

Updated 2026-09-21. The project owner opened the
[whole-crop review](rio-abajo-whole-crop-review.md) and replied:
“I've opened it. It looks fine. Now what?” This is recorded as positive overall
visual suitability. East-ridges remains the first candidate for qualification.
Individual grid-square selection is not required to continue. No protected
landmarks, flight envelope, quality radius or water classification were supplied.

The new assistant annotation captures that exact response against the viewed
map's hash and the original crop identity. It preserves the confirmed permission
decision and unresolved technical findings. The catalog now has seven historical
records, three current annotations and zero prepared regions.

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

## Source evidence and next action

The official [survey report](https://noaa-nos-coastal-lidar-pds.s3.amazonaws.com/laz/geoid18/9102/supplemental/PR_USVI_Lidar_2018_Project_Report.pdf)
lists the exact tile on page 90. It describes delivered hydro breaklines,
ground-void polygons and low-confidence polygons on pages 5, 34 and 36–37.
These are useful leads for explicit water and quality masks.

The inspected NOAA collection/supplemental listings and USGS OPR/LPC project
roots provide no corresponding vector download. This is a scoped search, not
proof that the files are unavailable elsewhere. The LPC delivery lives beneath
`PR_PuertoRico_VirginIslands_2018_D18/PR_PRVI_D_2018/`, unlike the shorter OPR path.

A [specific request](rio-abajo-source-files-request.md) is ready for the published
dataset contact. No message was sent. Obtain those files or another documented
source of equivalent classifications, inspect the actual crop intersections,
then construct water/quality/repair/feature masks with explicit provenance.
Technical source acceptance and preparation remain gated on that evidence.
Do not request the same visual approval again or substitute occupancy for quality.

## Retained artifacts and verification

Ignored local directory: `data/review/rio-abajo-east-ridges-qualification-v1/`.
It contains the visual-response evidence, annotation and record ID, pre-change
SQLite backup, distance raster/report, source listings, the 4,790,079-byte project
PDF and `source-evidence.json` with URLs and checksums. Preserve it separately
from Git. Existing source data, reviewed maps and prior records remain unchanged.

Verification checked the distance raster's hash, CRS, dimensions, inherited grid,
zero-bin count and maximum; catalog integrity and the current annotation were
checked through the CLI. Production code was unchanged.
