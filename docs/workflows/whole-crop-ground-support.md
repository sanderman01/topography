# Whole-crop classified-ground support probe

The optional developer probe in
[`scripts/check_ground_support.py`](../../scripts/check_ground_support.py) can
measure all cells of an existing native 0.5 m review crop. It accepts rectangular
grids from 1 to 2,048 cells per side, with a 2 m acquisition buffer. Its default
remains the existing 64 × 64 m centre sample. This is supporting evidence for
the [regional quality workflow](regional-quality.md), not source admission or
human acceptance.

Ground counts select classification 2, exclude withheld and synthetic points,
and include overlap returns. The retained evidence includes XY and flags, with
no Z comparison, terrain reconstruction, water classification or vertical datum
transformation. The probe takes the source horizontal CRS from the captured
COPC header and verifies it against the reader metadata after acquisition. The
target must match the review's projected metre CRS. HTTP range requests include
the supplied `If-Match` ETag; a changed source requires a new identity review.

## Plan without acquiring points

Run from the repository root with Python 3.11+ and GDAL installed:

```sh
python3 scripts/check_ground_support.py \
  https://noaa-nos-coastal-lidar-pds.s3.amazonaws.com/laz/geoid18/9102/prvi_d/20180506_PR_PRVI_D_2018_19QGA68505700.copc.laz \
  data/review/rio-abajo-pilot/east-ridges/review.json \
  data/review/rio-abajo-ground-support/east-ridges-full-v1 \
  --etag '"53ae7b22474d6d0ac160c4ca02a7fe85-112"' \
  --source-metadata data/review/rio-abajo-ground-support/north-hollows-v2/sample-metadata.json \
  --target-crs EPSG:6566 --full-review \
  --max-points 20000000 --max-csv-mib 2048 --timeout-seconds 1800 --plan
```

`--plan` hashes the local review artifacts, checks local CRS metadata and prints
JSON. It does not contact the COPC service, run PDAL, create the output directory
or establish that the saved ETag is still current. The listed local files are
the retained inputs of the [Río Abajo sample trial](../reports/rio-abajo-ground-support.md).
Keep assertions enabled; `python -O` is rejected.

For east-ridges this plans 1,024 × 1,024 cells covering 512 × 512 m, or
262,144 m². Bounds in EPSG:6566 are approximately west 169400, south 257588,
east 169912, north 258100 m; the JSON retains the original transform precision.
The buffer covers 516 × 516 m, or 266,256 m². No density assumption determines
which points are read; there is no resolution thinning or successful truncated
result at the point cap.

The dry plan was verified on 2026-09-21 and retained under
`data/review/rio-abajo-ground-support-plans-2026-09-21/east-ridges.json`.
A separate HTTP HEAD check at 09:06 UTC returned the same strong ETag, byte-range
support and a 932,466,055-byte source length. Its small identity record is beside
the plan. No source point bytes were acquired by either check.

## Resource readiness and execution

Before removing `--plan`, warn the user and obtain readiness confirmation under
[AGENTS.md](../../AGENTS.md). This run requires Linux and PDAL. Use a new output
directory. The example's ETag is historical; verify source identity before a
fresh run. Run the exact command above with only `--plan` removed after readiness
and identity review.

The whole-crop execution has these maximum limits; the three numeric CLI flags
can lower their respective limits but cannot raise them:

| Resource | Enforcement |
| --- | --- |
| Processing duration | 1,800 seconds total after local validation, including point acquisition, counts and verification; timeout kills the active child process group |
| Buffered output points | 20,000,000; streamed CSV rows checked while PDAL runs, then independently checked during numeric verification |
| Retained point CSV | 2,048 MiB; streamed bytes checked before writing each chunk |
| Each child address space | 2,048 MiB through Linux `RLIMIT_AS`; this limits virtual address space, not measured peak RSS |
| Python address space | 512 MiB through Linux `RLIMIT_AS` during processing |
| Each child-created file | 64 MiB through `RLIMIT_FSIZE`; core dumps disabled |
| Child diagnostic streams | 1 MiB per stream; acquisition log at most 1 MiB and combined count log at most 2 MiB |
| Concurrency | One PDAL/GDAL child at a time, one COPC reader worker, two live chunks; GDAL, OpenMP and OpenBLAS configured for one thread |

Children also have CPU-time limits no greater than the configured timeout.
Existing stricter hard process limits are retained. These limits can abort a
valid but expensive crop. They do not guarantee completion, cap network bytes
or bound kernel page-cache use. No GPU is used. Allow about 2.25 GiB of local
disk for the CSV and other artifacts; the CSV cap and child-file limits apply
during production, rather than relying on a final size check to limit memory.

The east-ridges centre sample retained 259,974 buffered points in 19,758,092 CSV
bytes. Scaling only by the buffered-area ratio, 266256 / 4624, suggests roughly
15 million points and 1.14 GB of CSV. This is a planning extrapolation, not a
measured crop density. Whole-crop runtime and peak RSS are unknown; the sample's
28-second acquisition and approximately 75 MiB child RSS do not establish them.
A readiness prompt can therefore describe up to 30 minutes, 2 GiB child virtual
address space plus 512 MiB Python, roughly 2.25 GiB disk, and uncertain actual
RSS/runtime. The run may hit a cap and leave incomplete evidence.

## Outputs and interpretation

Successful runs retain the point CSV, pipelines, reader metadata, logs, native
ground-count GeoTIFF and checksummed `report.json`. Partial output is preserved
after failure; only a fully written report is published as `report.json`.
Whole-crop reports use kind `ground_support_full_review` and include the workload
plan. The existing sample report kind and metrics remain unchanged. For backward
compatibility, `ground_returns_in_sample` and `sample_area_m2` also name the
whole measured footprint in a full-review report.

Each count bin is compared against an independent count from the streamed CSV.
The verifier reads GDAL's XYZ output in small chunks, with a byte cap, and stores
only one compact UInt32 grid for expected counts. It aggregates occupancy at
0.5, 1, 2 and 4 m without interpolation. Coarser bins anchor at the north-west
corner; partial eastern and southern edge bins are included for rectangular
sizes not divisible by the aggregation factor. Empty count cells mean no direct
selected return, not invalid terrestrial coverage or water. Occupied cells do
not establish independent pulses, numeric terrain accuracy or feature resolution.

`--reuse-sample` still rechecks retained sample outputs with identical pipelines
and source metadata, without reacquisition. It also accepts retained full-review
outputs when `--full-review` is supplied and both pipeline metadata files exist.
As before, it requires an output directory without a completion report and
records the original acquisition time and RSS as unknown. Keep completed evidence
immutable; do not delete its report to invoke reuse.

## Verification

```sh
python3 scripts/check_ground_support.py --self-check
```

This uses tiny local point fixtures and checks the original square sample,
rectangular occupancy and partial edge bins, classification/flag selection,
streamed point and byte rejection, timeout termination, retained-pipeline reuse,
maximum footprint bounds and a public-CLI dry plan against an unreachable source
URL. It does not acquire or decode real COPC points. Whole-crop live execution
remains unverified until a readiness-approved run completes.

The implementation was checked with PDAL 2.10.1 and GDAL 3.13.3. Relevant upstream
interfaces are the [COPC reader](https://pdal.io/en/stable/stages/readers.copc.html),
[text writer](https://pdal.io/en/stable/stages/writers.text.html),
[count writer](https://pdal.io/en/stable/stages/writers.gdal.html) and
[GDAL XYZ driver](https://gdal.org/en/stable/drivers/raster/xyz.html).
