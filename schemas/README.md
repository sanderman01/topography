# Shared data contracts

The initial contracts use strict typed TOML decoders in Rust, with semantic
checks, instead of a second schema engine. Their first consumer is
`topography validate <world|runtime|legend|source> <path>`.

- [World/runtime v1](../configs/README.md): dimensions, paths and review settings.
- [Candidate legend v1](legend-v1.md): stable IDs, colors and optional precedence.
- [Source metadata v1](source-v1.md): declarations and explicit unknowns.

These validators read metadata only. They do not inspect raster files, verify
license evidence, create acceptance records or implement batch jobs. Share a
machine-readable schema with Python when the first cross-language consumer exists;
do not infer a processing contract from the discovery/source record.

Future Rust tools and Python jobs will share artifact contracts covering:

- Source identity, original metadata, checksums, CRS, vertical datum, units,
  measured resolution, surface type, license evidence, and review decisions.
- Prepared rasters, masks, transformation records, and accepted areas.
- Dataset releases, class-legend versions, geographic splits, normalization,
  physical spacing, and source lineage.
- Model jobs and results, controls, versions, seeds, checkpoints, measured GPU
  memory, runtime, and numeric/structural evaluations.
- World revisions, source transforms, water surfaces, editable layers, locks,
  dependencies, and approvals.
- Export coordinates, orientation, cell-to-vertex conversion, shared borders,
  global height offset/scale, validity masks, and attribution.

Keep tiny examples in `examples/manifests/`. Version contracts explicitly;
dataset, model, and world manifests must pin compatible versions. Changing the
biome vocabulary is a migration, and retired class IDs must not be reused.
