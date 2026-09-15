# Design overview

The [master plan](../plans/terrain-design-master-plan.md) defines the terrain
workshop. The [initial specification](initial-design-spec.md) captures the
original request; the master plan refines its model assumptions and defaults.

## Component boundaries

`crates/topography-cli` owns the command-line interface.
`crates/topography-core` implements strict preset, candidate-legend and source-metadata
validation. Its acquisition module supplies bounded USGS discovery, raw
acquisition/manual import, integrity verification and native-grid review crops.
SQLite catalog persistence, regional preparation, human review records, datasets,
generation, world composition/editing and export remain reserved boundaries.
GDAL/PROJ and PDAL will supply established geospatial operations.

`training/` is a separate Python package for future numeric evaluation and model
jobs. Rust will drive that work through versioned batch manifests. Bash in
`scripts/` handles small orchestration tasks.

## Inputs and artifacts

Small reviewed descriptions live in `catalog/`; draft settings live in
`configs/`. Initial contract documentation lives in `schemas/`, with a synthetic
source record in `examples/manifests/`. Shared batch schemas will follow their
consumers. Local databases and large artifacts live in `data/`,
separated by pipeline stage and excluded from Git.

Physical Float32 elevation and explicit masks remain authoritative. Dataset
releases, model runs, world revisions, and exports carry lineage and review
state. World composition retains the real-terrain blending baseline alongside
the planned diffusion experiments.

Raw import and review crops start the real-terrain path without settling pilot
decisions. The next milestone still requires curated real terrain and verified
numeric model/loader precision, following the master plan's implementation sequence.
