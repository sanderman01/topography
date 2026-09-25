# Shared data contracts

The initial contracts use strict typed TOML decoders in Rust, with semantic
checks, instead of a second schema engine. Their first consumer is
`topography validate <world|runtime|legend|source> <path>`.

- [World/runtime v1](../configs/README.md): dimensions, paths and review settings.
- [Candidate legend v1](legend-v1.md): stable IDs, colors and optional precedence.
- [Source metadata v1](source-v1.md): declarations and explicit unknowns.
- [Raw receipt/review v1](raw-receipt-v1.md): byte integrity and native review provenance.
- [Native crop inspection v1](native-review-v1.md): sample counts and browser previews.
- [Crop curation v1](curation-v1.md): evidence, explicit decisions, labels and portable history.
- [Batch acquisition and gallery v1](batch-gallery-v1.md): target lists, verified resume and direct whole-tile decisions.
- [Regional quality v1](regional-quality-v1.md): whole-crop mask measurements and current preparation eligibility.
- [Regional quality v2](regional-quality-v2.md): snow/ice, optional vegetation and evidenced cover assessment.
- [Prepared native region v1](prepared-region-v1.md): aligned rasters, pinned decisions, immutable publication and current status.
- [Prepared native region v2](prepared-region-v2.md): independent measured surface, ground fill and cover masks.
- [Numeric evaluation v1](numeric-evaluation-v1.md): scoped releases, geographic separation and the Rust/Python scalar job boundary.
- [Numeric evaluation v2](numeric-evaluation-v2.md): surface-cover propagation and ground-only metrics, with explicit legacy compatibility.

The `validate` commands read metadata only. Acquisition/import additionally inspect
GeoTIFF metadata and preserve raw bytes; `verify` rehashes stored receipts/objects.
Those commands do not verify license evidence or create acceptance records.
Curation captures evidence bytes and explicit actor declarations; it does not
establish the truth of claims or admit prepared/training data. The numeric
consumer uses versioned JSON with strict decoders in both languages and shared
workflow fixtures; do not infer its contract from the discovery/source record.

The broader contract coverage still planned includes:

- Source identity, original metadata, checksums, CRS, vertical datum, units,
  measured resolution, surface type, license evidence, and review decisions.
- Prepared rasters, masks, transformation records, and accepted areas.
- Dataset releases, class-legend versions, geographic splits, normalization,
  physical spacing, and source lineage.
- Model jobs and results, controls, versions, seeds, checkpoints, measured GPU
  memory, runtime, and numeric/structural evaluations.
- World revisions, source transforms, water surfaces, editable layers, locks,
  dependencies, and approvals.
- Export coordinates, orientation, texture layout and dimensions, shared borders,
  global height offset/scale, validity masks, and attribution.

Keep tiny examples in `examples/manifests/`. Version contracts explicitly;
dataset, model, and world manifests must pin compatible versions. Changing the
biome vocabulary is a migration, and retired class IDs must not be reused.
