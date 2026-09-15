# Source metadata contract v1

The first consumer is `topography validate source <path>`. Input is UTF-8 TOML,
limited to 1 MiB, with unknown-field rejection at every table level. The executable
schema and semantic checks live in
[`catalog/source.rs`](../crates/topography-core/src/catalog/source.rs).
The [synthetic example](../examples/manifests/source-v1.toml) leaves permission and
physical metadata unknown. No external schema validator is needed.

This is a discovery/source-description contract, not a completed-import manifest.
It cannot represent preparation completion or acceptance. Future provider and
manual-import consumers must verify raw bytes, preserve original metadata and
record exact footprints, masks, coordinate conventions and decisions when their
artifact contracts are implemented. Never use this validation exit code to admit data.

## Top-level fields

Fields are required unless marked optional. Strings must be nonempty when
present. Omitted optional values explicitly mean unknown; do not substitute zero,
an empty string or a guessed CRS/datum. Unresolved fields appear in the report.

| Field | Type / meaning |
| --- | --- |
| `schema_version` | Integer, exactly 1 |
| `record_id` | Local record identifier, separate from provider IDs |
| `synthetic` | Boolean, true for examples without real sources; never admitted as survey data |
| `provider`, `collection_id`, `asset_id` | Original identifiers; no portal-wide permission assumption |
| `survey_id` | Optional original survey identifier |
| `acquisition_dates` | String list preserving original provider date/range notation; empty means unknown |
| `source_urls` | Nonempty list of original non-secret URLs |
| `original_metadata_path` | Preserved metadata reference, relative to this manifest's parent or absolute |
| `asset_sha256` | Optional 64 hexadecimal digits; declared, not computed/verified |
| `processing_history` | Ordered list of known processing descriptions; empty means unknown |
| `spatial`, `license` | Required tables below |

The validator checks string presence, not URL reachability, date syntax, CRS
definitions or referenced files. It does not expand/open `original_metadata_path`.
Original metadata retains provider fields not mapped here, including exact
coverage polygons and per-survey detail.

## Spatial table

| Field | Type / meaning |
| --- | --- |
| `surface_type` | `bare_earth_dem`, `classified_point_cloud`, `surface_model`, `imagery`, `bathymetry`, or `unknown` |
| `bounds_wgs84` | Optional finite `[west, south, east, north]` longitude/latitude degrees, ordered within ±180/±90; no antimeridian crossing in v1 |
| `horizontal_crs` | Optional authority identifier or original CRS definition |
| `vertical_datum` | Optional original height reference; never inferred from horizontal CRS |
| `horizontal_units`, `vertical_units` | Optional explicit original unit names |
| `native_spacing_m` | Optional two finite positive native spacings in x/y metres; omit if geographic spacing has not been established in metres |
| `ground_points_per_m2` | Optional finite nonnegative ground-return density |

Bounds support discovery, not exact footprint or valid-ground claims. Spacing is
not effective feature resolution. Surface models, imagery and bathymetry remain
identifiable records; this does not qualify them for ground-elevation training.

## License table

| Field | Type / meaning |
| --- | --- |
| `training_permission` | `confirmed`, `prohibited`, `restricted`, `ambiguous`, or `unknown` |
| `license_name`, `license_url` | Optional dataset-specific license identity and reference |
| `evidence` | Optional clause or confirmation relied on for training permission |
| `attribution` | Optional required text, or an explicit documented absence of requirements |
| `edited_map_reuse`, `redistribution` | Optional documented obligations/restrictions for each use |

`confirmed` requires nonempty `license_name`, `license_url` and `evidence`.
These are record assertions: the CLI checks presence, not legal validity. Every
other permission state excludes prepared/training admission under project policy;
it is not a deferred admission state. Confirmed permission alone does not satisfy
the [quality and human gates](../docs/design/pilot-acceptance-v1.md).

Credentials and expiring signed URLs must not appear in portable records. Keep
attribution and obligations after exclusion. Rejection and deletion are separate
operations; this consumer performs neither.
