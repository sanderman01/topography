# Design overview

The [master plan](../plans/terrain-design-master-plan.md) defines the terrain
workshop. The [initial specification](initial-design-spec.md) captures the
original request; the master plan refines its model assumptions and defaults.

## Component boundaries

`crates/topography-cli` owns the command-line interface.
`crates/topography-core` implements strict preset, candidate-legend and source-metadata
validation. Its acquisition module supplies bounded USGS discovery, raw
acquisition/manual import, integrity verification and native-grid review crops.
SQLite curation, technical quality assessment, native preparation and evaluation
releases preserve decisions and provenance. Generation, world composition/editing
and export remain reserved boundaries. GDAL/PROJ and PDAL supply established
geospatial operations.

The [batch source workflow](batch-source-workflow-v1.md) adds list-driven acquisition
and one local review gallery. Rust owns queue resume and durable curation decisions;
the browser supplies quick whole-tile Yes/No review. The
[implemented workflow](../workflows/batch-source-review.md) replaces per-file
download orchestration and separate source-review links for library expansion.

`training/` is a separate Python package with a CPU scalar numeric worker and
future model jobs. Rust drives evaluation through versioned JSON and hashed
scalar payloads. Bash in
`scripts/` handles small orchestration tasks.

## Inputs and artifacts

Small reviewed descriptions live in `catalog/`; draft settings live in
`configs/`. Initial contract documentation lives in `schemas/`, with a synthetic
source record in `examples/manifests/`. The numeric job has its own versioned
producer/consumer contract. Local databases and large artifacts live in `data/`,
separated by pipeline stage and excluded from Git.

Physical Float32 elevation and explicit masks remain authoritative. Dataset
releases, model runs, world revisions, and exports carry lineage and review
state. World composition retains the real-terrain blending baseline alongside
the planned diffusion experiments.

Raw import and review crops start the real-terrain path without settling pilot
decisions. The next milestone still requires curated real terrain and verified
numeric model/loader precision, following the master plan's implementation sequence.
