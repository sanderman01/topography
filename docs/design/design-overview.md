# Design overview

The [master plan](../plans/terrain-design-master-plan.md) defines the terrain
workshop. The [initial specification](initial-design-spec.md) captures the
original request; the master plan refines its model assumptions and defaults.

## Component boundaries

`crates/topography-cli` owns the command-line interface.
`crates/topography-core` reserves modules for configuration, catalog persistence,
provider access/manual import, preparation, review, datasets, generation,
world composition/editing, and export. These modules contain documentation only.
GDAL/PROJ and PDAL will supply established geospatial operations.

`training/` is a separate Python package for future numeric evaluation and model
jobs. Rust will drive that work through versioned batch manifests. Bash in
`scripts/` handles small orchestration tasks.

## Inputs and artifacts

Small reviewed descriptions live in `catalog/`; draft settings live in
`configs/`. Shared contracts will live in `schemas/`, with tiny examples in
`examples/manifests/`. Local databases and large artifacts live in `data/`,
separated by pipeline stage and excluded from Git.

Physical Float32 elevation and explicit masks remain authoritative. Dataset
releases, model runs, world revisions, and exports carry lineage and review
state. World composition retains the real-terrain blending baseline alongside
the planned diffusion experiments.

The scaffold does not implement processing or settle the remaining pilot
decisions. The next milestone remains inspectable real terrain and verified
numeric precision, following the master plan's implementation sequence.
