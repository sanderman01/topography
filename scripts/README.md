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

These are small CLI workflow checks, not raster processing or model evaluation.
They add no Python or geospatial dependency.

Keep Bash scripts focused on small orchestration tasks around established tools.
Persistent catalog logic, provider adapters, validation, and build dependencies
belong in the Rust crates. Raster and coordinate operations should invoke
GDAL/PROJ; point-cloud processing should use PDAL.

Scripts must receive explicit input/output paths and propagate failures. Source
deletion or cache cleanup must be an explicit operation, separate from review
and ordinary build steps.
