# Scripts

Run `bash scripts/check.sh` from the repository root to check the Rust workspace.
It can also be invoked by path from another working directory.

Fetch dependencies once with `cargo fetch --locked`; checks keep locked offline
resolution. Use `CARGO_BUILD_JOBS=1 bash scripts/check.sh` for one build job. The
script checks formatting, builds, runs Clippy, and exercises help/version and the
four contract validators. Fixtures cover physical dimensions, fractional/rectangular
grids, partial exports, invalid fields/numbers, duplicate IDs, missing permission
evidence, the VRAM ceiling, paths with spaces and runtime paths from another
working directory. Expected failures check exit codes and diagnostics.

When Python 3.11+ and GDAL are installed, the main check also runs local synthetic
DEM import/review, SQLite curation, regional quality, native preparation and
evaluation-release checks. With the isolated [training environment](../training/README.md)
installed, it also checks the CPU numeric worker and Rust/Python completion path.
Otherwise it explicitly reports that stage as skipped.
It does not download real terrain or run model evaluation.

`training/.venv/bin/python scripts/check_numeric.py` checks scalar loading, masks,
signed zero, physical slopes, precision loss and malformed jobs.
`python3 scripts/check_datasets.py target/debug/topography --python training/.venv/bin/python`
checks scoped approval, geography, corruption, later source rejection and worker
success/failure. Omit `--python` for release checks without NumPy. These checks
use small artificial terrain, temporary catalogs and no GPU.

`training/.venv/bin/python scripts/check_codec.py` checks the shared reconstruction
metrics, masks, process failure, timeouts and log limits without model weights.
With `--job <pinned-codec-job.json>` it also checks completed/corrupt output using
temporary identity reconstructions. See [the codec workflow](../docs/workflows/image-codec-evaluation.md).
The real autoencoder experiment is never part of routine `check.sh`.

For the complete acquisition workflow check:

```sh
python3 scripts/check_acquisition.py target/debug/topography
```

Use the actual binary path if `CARGO_TARGET_DIR` is customized. Add `--local-only`
to omit HTTPS cases. The full check needs GDAL, curl, OpenSSL and permission to
bind a loopback socket. It uses a temporary self-signed localhost certificate as
a private trust root, never disables TLS verification, and contacts no external
service. Temporary synthetic files are removed after the run.

It checks exact Float32 crop values and masks, pixel coordinates, raw checksums,
repeat import, corrupt objects, recovery before receipt publication,
non-Float32 rejection, unknown-permission isolation, archived discovery mapping,
interrupted transfers, corrupt cached chunks, changed ETags, bad ranges and limits.
Local checks also verify browser Float32/mask bytes, extrema excluding nodata,
nonfinite/all-invalid crops, one-cell ENVI output, unknown-unit view gating,
and safe embedding of retained metadata in HTML.
These are CLI workflow checks; live provider availability and source fidelity
require the separate [real-data workflow](../docs/workflows/dem-acquisition.md).

`python3 scripts/check_cover_acquisition.py target/debug/topography` (run by
`check.sh`) drafts cover masks against a loopback HTTPS server with synthetic
WorldCover, GLIMS and Sentinel-2 fixtures. It checks all-absent, present and
boundary bands, stale and reconstructed outlines, cloud gaps, missing dates,
source failures, exact source-grid output and v2 preparation of a merged profile.
It needs GDAL, curl and OpenSSL and contacts no external service.

For an acquired source and one or more newly generated review crops:

```sh
python3 scripts/check_review.py target/debug/topography data/raw/records/RECEIPT_SHA256.json data/review/candidate-1/review.json --output data/review/candidate-1-comparison.json
```

This compares every decoded Float32 bit and provider-mask cell with the original
window, checks coordinate metadata and artifact hashes, and verifies the embedded
browser samples and extrema. It runs sequential small GDAL conversions, uses
temporary files, and refuses to replace an existing comparison report. It uses
Python 3.11+ and the already-required GDAL tools; no Python numeric dependencies.
It is a developer verification script (run without Python's `-O` assertion bypass).
See the [karst trial](../docs/reports/rio-abajo-karst-trial.md) for a real example.

`python3 scripts/check_curation.py target/debug/topography` exercises explicit
actor decisions, pinned inputs, evidence copies, labels, invalid records, stale
heads, idempotent replay, preserved acceptance/rejection history, publication
recovery, corrupt/missing objects and portable metadata exports. It uses synthetic
terrain and human declarations clearly marked as fixtures; no real human acceptance.

`python3 scripts/check_quality.py target/debug/topography` checks whole-crop
categorical masks, exact grid/type/hash requirements, unknown classifications,
the 99.5% coverage boundary, defining-feature gaps, repairs, water exclusion,
nonfinite water, CRS declaration mismatches, pinned assessment evidence,
coverage disagreements and stale/superseded decisions. It exercises both eligible
and excluded outcomes in temporary synthetic catalogs. The eligibility fixture
declares a non-synthetic source to reach that branch; its terrain, permission and
human declarations remain artificial. No actual prepared data is published.
Run without Python's `-O` assertion bypass.

`python3 scripts/check_surface_cover.py target/debug/topography` exercises the
versioned snow/ice and vegetation contracts with tiny native-grid rasters. It
checks legacy serialization, explicit unavailable versus assessed cover,
snow/ice ground exclusion, exact surface preservation, malformed cover masks,
captured evidence, automatic preparation and restart behavior. All inputs and
catalog records are isolated synthetic fixtures.

`python3 scripts/check_preparation.py target/debug/topography` builds tiny adjacent
native regions from artificial eligible reviews, including 0.51 m spacing. It
checks exact Float32 land bits, signed zero, deterministic masked fill, distinct
masks, coordinates, holes/overlaps, immutable retries, catalog migration, orphan
registration, corrupt-file refusal and later source rejection. All declarations
and prepared products stay in temporary fixture directories. See the
[preparation workflow](../docs/workflows/aligned-preparation.md).

With PDAL installed, the main check also runs
`python3 scripts/check_ground_support.py --self-check`: a tiny local point fixture
checks classified-ground selection, withheld/synthetic flag exclusion and every
count bin. It also checks rectangular footprints and partial aggregation bins,
streamed point/byte limits, timeout termination, retained-pipeline reuse and a
public CLI dry plan. It includes overlap returns. There is no network or model work.

The optional `check_ground_support.py` developer probe reads one 64 × 64 m sample
at the centre of a native 0.5 m review crop, with a 2 m buffer, one COPC reader
worker and two active chunks. It requires a matching source URL, strong ETag,
captured PDAL COPC-reader metadata and the exact review CRS. See
`python3 scripts/check_ground_support.py --help` and the
[ground-support report](../docs/reports/rio-abajo-ground-support.md) for the trial.
The [whole-crop workflow](../docs/workflows/whole-crop-ground-support.md) documents
`--full-review`, its local-only `--plan` command and the concrete readiness bounds
for east-ridges. That mode accepts rectangular native reviews up to 2,048 cells
per side. The [east-ridges full-crop trial](../docs/reports/rio-abajo-whole-crop-review.md)
now records a completed live run and its observed resource use.
Full-detail range reads use `If-Match`; selected chunks may extend outside the
requested area. The probe does not implement acquisition's chunk cache or a hard
network-byte cap. No full source-cloud SHA-256 is claimed.

Retained XY/classification CSV, pipelines, PDAL metadata, a 0.5 m count GeoTIFF
and a final `report.json` allow numeric inspection. GDAL/PDAL perform coordinate
transforms and count rasterization; each bin is independently checked against the
CSV. Counts include overlapping returns and are not independent-pulse density,
terrain validity or effective feature resolution. No elevation/Z comparison occurs.
Default samples retain a 15-minute subprocess timeout and final verification caps
of one million points and 128 MiB CSV. Full-review runs cap total processing at
30 minutes, 20 million points and 2 GiB CSV, with streamed limits during production.
Full-review Linux limits
also bound each child to 2 GiB virtual address space and Python to 512 MiB;
these are not measured peak RAM estimates. Check workload readiness before a
substantial point-cloud run, using the detailed limits in the whole-crop workflow.
Partial outputs remain identifiable; a missing final report means incomplete.
`--reuse-sample` rechecks retained outputs only when both saved pipelines match;
it records unknown original runtime/RAM and never replaces a report. Run without
Python's `-O` assertion bypass, as with the other developer verification scripts.

Keep Bash scripts focused on small orchestration tasks around established tools.
Persistent catalog logic, provider adapters, validation, and build dependencies
belong in the Rust crates. Raster and coordinate operations should invoke
GDAL/PROJ; point-cloud processing should use PDAL.

Scripts must receive explicit input/output paths and propagate failures. Source
deletion or cache cleanup must be an explicit operation, separate from review
and ordinary build steps.

The optional CPU environment enables `scripts/check_scalar_fit.py` in the full
check. It checks small scalar-model updates, mask gradients, calibration-only
window selection, safetensors reload and disk limits; it never launches a full
terrain fit. See [scalar fitting](../docs/workflows/scalar-codec-fitting.md) for
the separately authorized job and optional completion-contract fixtures.
# Batch/gallery checks

`python3 scripts/check_batch_gallery.py target/debug/topography --local-only`
checks local queue validation/resume, partial failure, gallery publication and
source integrity without opening a port. The regular `check.sh` includes it.
Omit `--local-only` to exercise direct gallery votes, stale heads, preserved
technical evidence, first votes on confirmed/prohibited source declarations,
HTTP restrictions and damaged-preview refusal through a
temporary loopback service. Full `check_acquisition.py` also exercises queued
HTTPS downloads, metadata retrieval, independent failures and verified resume.

For actual browser interaction, add `--browser /path/to/chromium` to
`check_batch_gallery.py`. It uses Node and Chromium's DevTools pipe, with a
temporary browser profile and synthetic catalog only. It checks buttons, filters,
reload, preview-ready voting, sandboxed-frame shortcuts and escaped labels.
The optional browser is not installed by this workflow.

`python3 scripts/check_automatic_preparation.py target/debug/topography` exercises
profile pinning, preflight without voting, Yes-to-Ready preparation, exact native
samples/masks, restart/reaccept, stale decisions and technical exceptions using
tiny local rasters and a loopback gallery. `check.sh` includes it when Python and
GDAL are available. It never uses the real catalog or downloads source terrain.

Acquisition checks also exercise the alternate TIFF reader when the installed
GDAL exposes `LIBERTIFF`: a wrapper makes the primary reader unavailable while
real GDAL verifies native samples, nodata validity, coordinates and Float32
rejection through import/review. Both-reader failure must leave no new receipt.
