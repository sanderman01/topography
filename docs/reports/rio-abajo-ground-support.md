# Río Abajo classified-ground support trial

Run on 2026-09-15 with PDAL 2.10.1 and GDAL 3.13.3. This extends the
[karst crop trial](rio-abajo-karst-trial.md) with one 64 × 64 m centre sample per
512 × 512 m crop. Each sample covers 1.5625% of its crop. All results are
assistant observations; human acceptance and training admission remain absent.

The subsequent [whole-crop east-ridges review](rio-abajo-whole-crop-review.md)
measured the full footprint on 2026-09-21. It also records the later permission
decision and current catalog state. The sample observations below remain historical.

## Measured support

Counts select classification 2, exclude withheld and synthetic points, and retain
overlap returns. They measure return density in plan view, not independent pulses,
terrain accuracy or effective feature resolution. The table's percentages mean
cells containing at least one selected return, **not** valid terrestrial coverage.

| Sample | Ground returns | Returns/m² | 0.5 m cells occupied | 1 m | 2 m | 4 m |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| north-hollows | 10,906 | 2.663 | 42.76% | 74.63% | 95.90% | 99.61% |
| east-ridges | 16,433 | 4.012 | 55.51% | 83.74% | 97.75% | 100.00% |
| south-saddles | 12,177 | 2.973 | 44.26% | 73.27% | 90.23% | 97.66% |

Every one of the 49,152 native 0.5 m count bins matched an independent count of
the retained XY points. Larger-bin occupancy aggregates the same counts without
interpolation. The DEM's provider mask was fully valid, while many half-metre
bins have no direct classified-ground return. This supports investigating
interpolation and gaps before claiming sub-metre terrain detail. It does not
establish that empty bins are invalid terrain or that larger occupied bins resolve
features at those sizes. No acceptance threshold was invented from these results.

## Source and coordinates

The [NOAA collection index](https://noaa-nos-coastal-lidar-pds.s3.amazonaws.com/laz/geoid18/9102/index.html)
links the matching PR_PRVI_D survey. The selected
[COPC tile](https://noaa-nos-coastal-lidar-pds.s3.amazonaws.com/laz/geoid18/9102/prvi_d/20180506_PR_PRVI_D_2018_19QGA68505700.copc.laz)
shares the DEM's `19QGA68505700` tile identifier. PDAL's inspected header reports
132,542,708 points in a 932,466,055-byte object. HTTP identity during the trial was
ETag `"53ae7b22474d6d0ac160c4ca02a7fe85-112"`; this multipart ETag is not a SHA-256.
The complete cloud was not downloaded or hashed.

The header's horizontal WKT is NAD83(2011) / UTM Zone 19N; its projection matches
EPSG:6348. The final probe uses that captured WKT directly. PDAL converts XY to
the DEM's EPSG:6566 metric grid, with network grid downloads disabled. The header
declares PRVD02 height, while the embedded NOAA processing pipeline records a
GEOID18 conversion. The original DEM collection records GEOID12B. No elevation/Z
comparison or vertical transformation was performed in this trial.

Displayed sample bounds in EPSG:6566 metres, rounded from the native transform:

| Sample | West | South | East | North |
| --- | ---: | ---: | ---: | ---: |
| north-hollows | 168764 | 258172 | 168828 | 258236 |
| east-ridges | 169624 | 257812 | 169688 | 257876 |
| south-saddles | 169174 | 257287 | 169238 | 257351 |

The reader uses a 2 m buffer, full detail, one worker, two active chunks and
`If-Match` on range requests. Retained CSV keeps XY to eight decimal places and
classification/flags; it is measurement evidence, not authoritative elevation.
PDAL writes count bins without distance interpolation or fallback fill. The
verification allows 1e-8 m for geotransform serialization, separate from any
terrain-quality tolerance. See the upstream
[COPC reader](https://pdal.io/en/stable/stages/readers.copc.html),
[HTTP file specification](https://pdal.io/en/stable/stages/filespec.html) and
[GDAL count writer](https://pdal.io/en/stable/stages/writers.gdal.html).

## Reproduction and retained artifacts

After reviewing the source metadata and workload, run from the repository root:

```sh
python3 scripts/check_ground_support.py https://noaa-nos-coastal-lidar-pds.s3.amazonaws.com/laz/geoid18/9102/prvi_d/20180506_PR_PRVI_D_2018_19QGA68505700.copc.laz data/review/rio-abajo-pilot/east-ridges/review.json data/review/new-east-ground-sample --etag '"53ae7b22474d6d0ac160c4ca02a7fe85-112"' --source-metadata data/review/rio-abajo-ground-support/north-hollows-v2/sample-metadata.json --target-crs EPSG:6566
```

Use a new output directory. Current source identity must be checked before a new
trial; an ETag change requires reviewing the replacement source. The captured
metadata argument is PDAL pipeline metadata containing `stages.readers.copc`;
its horizontal CRS must match the header read during sampling. Target CRS must
match the review raster. Developer scripts require assertions enabled.

Final local outputs are under `data/review/rio-abajo-ground-support/`:
`north-hollows-v3/`, `east-ridges/`, `south-saddles/`. Each contains retained points,
source metadata, pipelines, logs, count GeoTIFF and a checksummed `report.json`.
The original north-hollows sandbox attempt stalled. A subsequent configuration
used the wrong UTM-zone code and produced an empty sample; it has no completion
report. The fixed version derives the CRS from the header and rejects empty
samples. The successful north sample's retained outputs were rechecked after a
CSV numeric-decoding fix; its original runtime/RAM remain explicitly unknown.
Earlier incomplete directories are preserved.

East/south sampling took approximately 28/48 seconds, with 0.58/0.67 seconds for
local counting. Linux child peak RSS was approximately 75/74 MiB. These are
observations for these samples, not resource promises for other COPC files.
No full-cloud hash, transferred-byte total, full-crop support map, terrestrial
quality mask, water classification or landmark-gap assessment was produced.

## Curation and remaining gates

The [curation workflow](../workflows/source-curation.md) captures each sample
report, the original crop comparison and the exact original
[USGS OPR tile XML](https://prd-tnm.s3.amazonaws.com/StagedProducts/Elevation/OPR/Projects/PR_PRVI_D_2018/metadata/USGS_OPR_PR_PRVI_D_2018_19QGA68505700.xml).
That XML's SHA-256 is
`9c16846d2b516d04bd213a8cc235998437ddcea775cb72867477c6003249c7d9`.
It ties the source to a specific OPR tile; it is retained for permission review,
without upgrading training permission from unknown.

All three crop records use actor `assistant`, decision `annotate`, candidate class
ID 1, unknown whole-crop findings and unset terrestrial coverage. Their raw receipts
remain immutable and unreviewed. Portable curation exports and record IDs are
under `data/review/rio-abajo-curation/`. `topography curate check` verifies them.
These ignored files need separate backup; Git contains this report and tooling.

At this sample checkpoint, whole-crop evidence and permission were unresolved.
The later whole-crop report records progress on both. Native preparation and
independent admission checks are now implemented, while actual crop acceptance,
quality masks, vehicle dimensions and tolerances remain unresolved.
