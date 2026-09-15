# Scripts

Run `bash scripts/check.sh` from the repository root to check the Rust scaffold.
It can also be invoked by path from another working directory.

Keep Bash scripts focused on small orchestration tasks around established tools.
Persistent catalog logic, provider adapters, validation, and build dependencies
belong in the Rust crates. Raster and coordinate operations should invoke
GDAL/PROJ; point-cloud processing should use PDAL.

Scripts must receive explicit input/output paths and propagate failures. Source
deletion or cache cleanup must be an explicit operation, separate from review
and ordinary build steps.
