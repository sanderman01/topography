# Shared data contracts

Place versioned machine-readable schemas here as their first consumers are
implemented. No manifest schema or validator is implemented by this scaffold.

The Rust tools and Python jobs will share contracts covering:

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
