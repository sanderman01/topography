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

The first raw store now uses `raw/objects/<sha256>` plus immutable receipts in
`raw/records/<sha256>.json`. The SQLite crop-curation catalog now has a sibling
`<catalog_db>.curation/` directory of immutable records and captured documents; see
[curation](../docs/workflows/source-curation.md). HTTPS staging
retains verified chunks under `staging/http/`, and small GDAL review runs contain
elevation, validity, hillshade and a review manifest. See the
[workflow](../docs/workflows/dem-acquisition.md) and
[raw receipt contract](../schemas/raw-receipt-v1.md). Temporary publication files
and unreferenced objects do not establish a completed import; the receipt is
published last. Retrying verifies/reuses complete objects without overwriting them.

Aligned native regions publish under `prepared/<manifest-sha256>/` with
`manifest.json`, seven raster layers and captured quality reports. Source decisions
must pass current eligibility; generated products remain unreviewed and excluded
from training. The catalog's schema-2 `prepared_regions` table registers completed
products after file publication. See [preparation and recovery](../docs/workflows/aligned-preparation.md).
Preserve intact unregistered output for retry and never treat partial directories
as completed products. Curation exports do not include these raster files.

Evaluation-only releases now publish `datasets/<manifest-sha256>/manifest.json`.
They pin selection, scoped approval and prepared identities without copying rasters
or creating model-training admission. Completed numeric runs in `models/<run>/`
retain the request, decoded binary inputs, metrics, worker result/log and completion
manifest. Verify the release before reuse: later source rejection preserves output
but revokes current use. See [numeric evaluation](../docs/workflows/numeric-evaluation.md).

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
