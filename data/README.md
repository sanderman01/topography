# Local artifact storage

Only this README and directory placeholders are tracked. Runtime contents are
ignored by Git. Keep small reviewed inputs in `catalog/`, configuration in
`configs/`, and synthetic test data in `tests/fixtures/`.

| Directory | Contents |
| --- | --- |
| `catalog/` | Local SQLite catalog, indexes, and journal files |
| `raw/` | Immutable original assets and metadata, identified by checksums |
| `prepared/` | Regional Float32 elevation in metres, validity/quality masks, transformation records |
| `review/` | Hillshades, contours, oblique previews, metrics, and working annotations |
| `datasets/` | Approved releases, paired tiles and controls, geographic splits, normalization, lineage |
| `models/` | Run manifests, configurations, checkpoints, evaluations, and software versions |
| `worlds/` | Revision manifests, base surfaces, placements, ownership, generated regions, edits, locks |
| `exports/` | Height tiles, masks, encoding/coordinate metadata, attribution, and validation reports |
| `cache/` | Rebuildable derived caches |
| `staging/` | Partial downloads and temporary processing output |

Each release, run, world revision, and export should have its own identified
subdirectory and manifest when implemented. Derived artifacts must identify
their inputs, checksums, configuration, tool/model versions, and approval state.
Keep large rasters in files, not in database blobs.

Rejection records an exclusion; it does not delete raw assets. Cache housekeeping
is separate from curation. Preserve accepted world output and source provenance;
ignored files are not backed up by Git, and accepted artifacts need separate
storage/backup arrangements.

Retain terrestrial elevation as floating-point metres with explicit validity.
Water has a separate mask and authored surface; the terrain beneath it uses the
plan's deterministic masked fill, not inferred bathymetry. Export encoding must
not redefine the authoritative internal elevation surface.

The example storage paths are configurable so large data can live on a separate
volume. Do not assume a full-world job fits in RAM or GPU memory just because
one raster does.
