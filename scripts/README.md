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
DEM import/review checks. Otherwise it explicitly reports that stage as skipped.
It does not download real terrain or run model evaluation.

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

Keep Bash scripts focused on small orchestration tasks around established tools.
Persistent catalog logic, provider adapters, validation, and build dependencies
belong in the Rust crates. Raster and coordinate operations should invoke
GDAL/PROJ; point-cloud processing should use PDAL.

Scripts must receive explicit input/output paths and propagate failures. Source
deletion or cache cleanup must be an explicit operation, separate from review
and ordinary build steps.
