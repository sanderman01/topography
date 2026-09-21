# Data management

Applies to provider adapters, manual import, the catalog, dataset releases, and
artifact storage. Raw acquisition/import and integrity checks exist; SQLite
crop-curation persistence and gated native preparation exist; training releases
remain planned. Use the
[master plan](../plans/terrain-design-master-plan.md), sections 2–4 and 9, for
the project's source-admission and provenance requirements.

## Acquire identifiable source data

- Start with one useful numeric bare-earth DEM provider and manual import.
  Prefer documented APIs, STAC catalogs, indexes, and direct assets. Query the
  actual area and collection; a candidate location or portal listing does not
  prove released coverage at the required resolution.
- Preserve provider, collection, survey and asset IDs; acquisition dates;
  footprint; original metadata; stable source URLs; checksums; horizontal CRS;
  vertical datum; units; surface type; measured spacing; and processing history.
  Record unknown values explicitly and block admission when they are essential.
- Keep provider identifiers distinct from local artifact identities. Preserve
  original STAC metadata when mapping it into local fields; consult
  [STAC best practices](https://github.com/radiantearth/stac-spec/blob/master/best-practices.md)
  when implementing a STAC adapter. Do not assume all providers expose STAC.
- Estimate transfer/storage size and use bounded downloads, retry/backoff, and
  resumable staging. Detect a changed upstream asset before appending to a
  partial download. Verify integrity before registering a completed raw asset.
- Apply the master plan's dataset-specific licensing gate before admission to
  prepared or training pools: record the clause or confirmation permitting ML
  training, plus attribution and reuse requirements. Prohibited or ambiguous
  training permission excludes the dataset; portal-level assumptions are
  insufficient. This is the project's admission policy, not a blanket legal
  conclusion about public elevation data.

## Separate source, state, and derived artifacts

- Keep small reviewed provider descriptions, areas, and legends in `catalog/`.
  Keep large files and the local catalog in the stage directories under `data/`,
  following [data/README.md](../../data/README.md). Store raster files outside
  database blobs and allow artifact roots on another volume.
- Identify immutable raw assets by checksums. Record rejection as an exclusion
  with a reason; do not delete or alter the original file during curation.
- Identify every prepared product, dataset release, model run, world revision,
  and export. Record parent checksums, resolved configuration, transformations,
  tool/model versions, masks, and review state.
- Key caches on all inputs that affect the output, including controls, masks,
  normalization, model version, and seed where relevant. Resume verified completed
  stages; a file's existence or modification time is not proof of validity.
- Invalidate dependent artifacts when inputs change. Preserve accepted artifacts
  as explicit revisions rather than silently replacing them on recomputation.

## Publish and preserve results

- Use catalog transactions for related metadata and review updates. Keep them
  short; perform long downloads and processing outside a write transaction.
  SQLite's guarantees cover its database, not separately stored raster files;
  see [SQLite atomic commit](https://www.sqlite.org/atomiccommit.html).
- Stage and validate files before marking artifacts complete. Define how startup
  recovers abandoned staging files, published but unregistered files, and catalog
  entries whose files are missing. Keep checksum checks available for reconciliation.
- Record acceptance, rejection, annotation, and locks against the exact artifact
  or revision reviewed. A successful job cannot create a human acceptance record.
- Export portable manifests with lineage and decisions. Keep credentials and
  expiring authentication parameters out of versioned metadata and logs.
- Preserve accepted world output as well as the recipe that produced it. Git
  ignores `data/`; plan separate backup and restore for valuable artifacts and
  use a consistent database backup method. Rebuildable caches have a different
  retention policy from raw sources and accepted revisions.

## Verify

Trace a small imported source through a derived artifact and its manifest.
Exercise interrupted acquisition, checksum mismatch, repeated import, and
rejected admission when implementing those paths. Confirm that rejection leaves
raw bytes intact and that recovery never promotes partial output.
