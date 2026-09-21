# Prepared native region v1

Consumers: `prepare build <runtime.toml> <region.json>` and
`prepare verify <runtime.toml> <manifest.json>`. See the
[workflow](../docs/workflows/aligned-preparation.md) for examples and recovery.
These strict typed JSON contracts reject unknown fields and unsupported versions.

## Build request

The [request template](../examples/manifests/prepared-region-request-v1.json)
uses synthetic coordinates, a placeholder path and a zero hash. Replace them
with the exact inspected grid, quality request and current accepted record.

- `schema_version`: `1`.
- `inputs`: 1–16 entries `{quality_path, curation_record_sha256}`. Relative quality
  paths resolve beside the request; the hash pins a current accepted decision.
- `grid`: `{origin: [west, north], size: [columns, rows], spacing_m: [x, y]}`.
  Origin is the finite outer north-west corner. Each size is 1–4,096; positive
  spacing is 0.5–1 m and must equal the native inputs. The inherited metric CRS
  must match across inputs, as must their provider, collection, survey and datum.
- `masked_fill_m`: finite Float32 value, explicitly excluded wherever validity
  is zero. It represents neither authored water level nor bathymetry.

Each source review is bounded by the quality contract (2,048 cells per side).
Inputs must map to integer pixel offsets and cover the target exactly without
overlap or holes. No resampling, cross-survey blending, repair or offset tolerance
is implicit. A request is not an admission token: all quality/decision checks run
again when building and decision heads are checked during catalog registration.

## Completion manifest

`schema_version: 1`, `kind: prepared_native_region`,
`policy: native_preparation_v1`. The directory name is the SHA-256 of the exact
pretty-JSON `manifest.json` bytes. Fields include:

- `grid`, `coordinate_system` (GDAL WKT/axis mapping), `vertical_datum`,
  `vertical_units: m`, `survey` (provider/collection/survey IDs).
- `masked_fill_m`, `resampling: none`, GDAL and Topography versions.
- `parents`: raw-receipt, review, asset, curation-record and quality-report hashes,
  accepted `primary_class_id`, and `destination_window: [x, y, width, height]`.
- `artifacts`: exact `{path, sha256, bytes}` entries for seven named GeoTIFFs and
  one `quality-N.json` per input. Names are fixed, flat and unique. Each artifact
  is limited to 256 MiB when verified; metadata retains the 1 MiB limit.
- `review_state: unreviewed`, `training_admission: not admitted`.

The seven rasters are elevation (Float32 metres) and Byte validity,
provider-validity, water, quality, repairs and features. Their exact names and
encodings are in the workflow. All are north-up cell-centre rasters sharing the
grid. No nodata height code is reserved. Valid land preserves its Float32 bits;
masked cells carry the fill. Defining features and categorical quality retain
their reviewed encodings; water is converted to binary 0 land / 255 water.

Immutable source assessments are included byte-for-byte. Raw assets, review
rasters and source mask files are referenced through hashes and the curation
journal, not copied into the product. Retain their original bytes separately.

## Catalog and status

Successful publication adds a schema-2 `prepared_regions` row with manifest ID,
exact body and local manifest locator. Schema-1 migration is transactional and
does not rewrite curation history. Files publish first; a failed transaction can
leave a complete but unregistered region. Rebuilding compares existing artifacts
without replacement and retries registration against current heads.

`prepare verify` returns `kind: prepared_region_status`, `schema_version: 1`,
`manifest_sha256`, `integrity: verified`, `registered`, `current`,
`review_state: unreviewed`, `training_admission: not admitted`.
Exit 0 requires registered/current status; intact orphaned or superseded outputs
produce JSON and exit 1. Contract/integrity errors return stderr and exit 1.
Source-history integrity is checked, including raw and review parents. A later
annotation/rejection changes current status; it never deletes existing artifacts.

There is no prepared-output acceptance, dataset release, or automatic orphan
promotion. Consumers must check current status when using products and apply
their own later-stage review gates.
