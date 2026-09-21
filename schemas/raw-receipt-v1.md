# Raw receipt and review manifest v1

The first producer/consumer is the
[acquisition workflow](../docs/workflows/dem-acquisition.md). Receipts are JSON
with a strict Rust decoder in
[`acquisition/store.rs`](../crates/topography-core/src/acquisition/store.rs).
They are not prepared-raster, dataset-release or acceptance manifests.

## Storage and identity

```text
raw/
  objects/<sha256>            Original DEM or small metadata bytes
  records/<sha256>.json       Immutable raw receipt, published last
staging/
  http/<identity-sha256>/     Strong HTTP identity and verified range chunks
```

Object and receipt hashes are lowercase SHA-256 hex. Receipts reference the
object directory relative to their parent `records/` directory. Move `objects/`
and `records/` together for portability. No machine-local paths enter the receipt.
GDAL's run-local `description`/`files` paths are replaced by `asset` in the saved
inspection; original source and provider metadata are captured without changes.
The inspection's `driverShortName` records `GTiff` or the read-only `LIBERTIFF`
fallback. Both represent self-contained GeoTIFFs under the same import gates.

| Receipt field | Meaning |
| --- | --- |
| `schema_version` | Exactly 1 |
| `record_id` | Source-description identity; provider IDs remain in that source object |
| `asset_sha256`, `asset_bytes` | Actual retained DEM checksum and byte length |
| `source_sha256` | Exact original source TOML bytes |
| `original_metadata_sha256` | Exact referenced provider metadata bytes |
| `inspection_sha256` | Normalized GDAL JSON inspection |
| `method` | Producer writes `manual_import` or `https_ranges` |
| `http_etag` | Strong server ETag for acquisition, otherwise null |
| `gdal_version`, `topography_version` | Tools that produced this receipt |
| `training_permission_recorded` | Whether the source declares confirmation; not verification or admission |
| `synthetic` | Preserves the source's synthetic/example declaration |
| `review_state` | Exactly `unreviewed`; this producer cannot create acceptance |
| `unresolved_metadata` | Unresolved source declarations, including any missing upstream expected checksum |

The receipt filename hashes its exact JSON bytes. Verification checks this name
and rehashes all four referenced objects, including the recorded asset size.
Do not pretty-print or edit a receipt in place. Changing provenance creates a new
record; it does not update or overwrite a previous one.

Raw storage is intentionally available for excluded/unknown-permission sources.
There is no prepared/training admission path in these commands. A successful
receipt verification establishes stored-byte integrity, not truth of the source
declarations, legal evidence, measured quality or human review.

## Review output

`review.json` identifies `kind = native_grid_review`, schema version 1,
`review_state = unreviewed`, the raw receipt/asset checksums, an x/y/width/height
window in native cells, `resampling = none`, tool versions and checksums/lengths
of all review artifacts. It explicitly describes the provider validity mask and
the lack of prepared/training admission. This is a completion/provenance record,
not a human acceptance record or a general raster processing schema.

New reviews also include [native inspection metrics and a browser preview](native-review-v1.md).
Consumers should enumerate the artifact list; older v1 reviews without those
additional artifacts retain their original interpretation.
