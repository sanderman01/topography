# Architecture and contracts

Use this guide when adding components, configuration, schemas, or Rust/Python
interfaces. Preset, candidate-legend and source-description validators exist;
processing and cross-language batch contracts remain to be implemented.

## Keep responsibilities explicit

- Keep argument parsing, terminal output, and exit codes in `topography-cli`.
  Put reusable operations and validation in the corresponding `topography-core`
  module. Library code returns results rather than terminating the process.
- Keep persistent catalog and pipeline orchestration in Rust. Delegate raster
  and coordinate operations to GDAL/PROJ, point-cloud operations to PDAL, and
  model computation to the Python package. Bash only connects small tool steps.
- Introduce a trait, crate, service, or generalized workflow layer when a concrete
  implementation needs it. The first acquisition milestone needs one numeric DEM
  provider and manual import; both should produce the same validated source
  contract. See the [design overview](../design/design-overview.md).

## Validate configuration at the boundary

- Treat `configs/` as draft examples until a loader and validator exist. Define
  defaults, override precedence, path resolution, and unknown-field handling
  together with the first consumer. Reject misspelled settings instead of
  silently falling back to a default.
- Derive each axis's physical extent from cell count times spacing. Reject
  non-finite values, non-positive sizes, contradictory dimensions, incompatible
  masks, and invalid window/stride combinations before allocating large buffers.
- Keep world size, training crop size, inference overlap, and delivery tile size
  independent. Validate partial edge tiles and rectangular grids.
- Use explicit units and coordinate conventions in fields and documentation.
  Keep vehicle dimensions, tolerances, and encoding ranges configurable; the
  pilot must establish values still absent from the plan.
- Record the resolved, non-secret configuration in each run. Resolve machine
  paths without embedding credentials in manifests. Follow the
  [configuration notes](../../configs/README.md).

## Version the batch boundary

- Add schemas in `schemas/` and tiny examples in `examples/manifests/` with the
  first producer and consumer. Select the format and schema tooling then; JSON
  Schema is a reference option, not an installed project dependency.
- Include schema version, artifact identity, input checksums, physical grid and
  normalization, dataset/model/control versions, tool versions, and output paths
  where relevant. Pin compatible versions across both languages.
- Reject unsupported versions and missing required fields with actionable
  errors. Specify extension behavior explicitly. Schema shape checks do not
  replace semantic checks such as matching raster dimensions. For object-field
  validation concepts, see the
  [JSON Schema object reference](https://json-schema.org/understanding-json-schema/reference/object).
- Invoke Python with an input manifest and explicit output directory. Require
  an identifiable result manifest plus metrics, logs, and declared outputs.
  Validate completion and outputs before catalog registration; process success
  does not imply human acceptance.
- Stage partial outputs separately and publish completion only after validation.
  A catalog transaction cannot atomically commit external raster files; define
  recovery for interruptions between file publication and catalog updates.
- Preserve old artifacts and their interpretation during schema migrations.
  Label-vocabulary changes must version dataset, model, and world contracts;
  never reuse retired class IDs. See [shared contracts](../../schemas/README.md)
  and [data management](data-management.md).

## Before accepting a change

Exercise valid and invalid inputs through the actual consumer. Check that Rust
and Python agree on units, masks, versions, paths, and completion behavior. Keep
the [master plan](../plans/terrain-design-master-plan.md), configuration examples,
schemas, and user documentation consistent with the implemented interface.
