# Crop curation v1

Producer/consumer: `topography curate draft`, `record`, `status`, `check`, `export`.
Inputs and records use strict typed JSON decoding with unknown-field rejection.
This is a local Rust contract; it is not a prepared-raster or Python batch interface.
See the [workflow](../docs/workflows/source-curation.md) for commands and recovery.

## Draft

| Field | Meaning |
| --- | --- |
| `schema_version` | Exactly `1` |
| `raw_receipt_path`, `review_path`, `legend_path` | Local inputs; relative paths resolve beside the draft |
| `target` | Pinned `raw_receipt_sha256`, `review_sha256`, `legend_sha256` |
| `previous_record_sha256` | Current crop decision head, or `null` for first record |
| `claims` | Actor, decision, reason, label, quality findings, coverage and permission review |
| `evidence` | Up to 16 entries `{path, evidence: {id, description, source_url, sha256}}` |

Digests are 64 lowercase hexadecimal characters. Files must match their pinned
digests; the entire raw parent and declared review artifacts are reverified before
recording. A review must be a v1 unscaled native-grid crop with matching source,
window, dimensions, coordinates and artifact hashes. Evidence IDs are unique,
1–64 ASCII letters/digits/underscores/hyphens. Each document and serialized record
is limited to 1 MiB. Draft generation creates no database and makes no decision.

## Claims

- `actor`: `{kind, name}`; kind is `unassigned`, `human` or `assistant`.
  When an assistant transcribes an actual human decision, identify the human
  decision actor and capture the exact response and assistant recorder in evidence.
- `decision`: `annotate`, `accept` or `reject`. Accept/reject requires a named
  human declaration; identity is not authenticated.
  Whole-tile yes/no selection can be recorded with unresolved technical findings.
- `reason`: nonempty explanation required for every recorded decision.
- `primary_class_id`: an active ID in the exact captured legend, or `null`.
  Required for acceptance. This labels the entire crop, without assigning per-cell
  ownership or resolving the candidate legend's unset precedence.
- `findings`: exactly `coordinates`, `ground_support`, `artifacts_and_repairs`,
  `feature_fidelity`, `water_classification`, `unsupported_feature_gaps`.
  Each contains `{state, note, evidence_ids}`. State is `unknown`, `pass` or `fail`;
  resolved findings require a nonempty note and existing evidence IDs.
- `coverage`: `null` or `{terrestrial_cells, valid_terrestrial_cells, note,
  evidence_ids}`. Counts must fit within crop dimensions and valid cells must not
  exceed a positive terrestrial denominator. The 99.5% precheck uses integer
  arithmetic. Evidence still needs to establish terrestrial classification and quality.
- `license`: the [source v1 license fields](source-v1.md). A confirmed review
  additionally requires all attribution/reuse/redistribution descriptions and
  nonempty `permission_evidence_ids` referencing captured documents.

## Immutable record and export

A portable record retains `schema_version`, `kind: crop_curation`, `target`,
`previous_record_sha256`, `asset_sha256`, `window_cells`, `legend_version`,
`claims`, evidence descriptors without local paths, `preparation_blockers`,
`admission: not admitted`, and `topography_version`. Its ID is the SHA-256 of the
exact JSON bytes. No timestamp is generated during publication, so identical
retries retain identity. A new decision's previous-record link distinguishes it.

Automatic preparation may append a technical successor carrying the exact
existing human actor, Yes decision, reason and class. Its optional `automation`
object has `kind: supplier_qa_qualification_v1`,
`inherited_human_record_sha256`, `profile_sha256` and `quality_report_sha256`.
This attributes new technical findings to automation; it is not a new human vote.
Original record bytes remain unchanged. Existing failed findings and permission
restrictions cannot be cleared automatically. Old records omit this field.
Earlier binaries with strict decoding cannot read records containing this new
field; use the updated CLI after automatic qualification has run.

SQLite stores the record bytes and machine-local input locators, plus the current
head per review digest. Application ID is `0x544f504f`; curation-only catalogs use
`user_version=1`. [Prepared-region registration](prepared-region-v1.md) migrates to
database schema 2 without rewriting these v1 decision records. Current tools read
both database versions; earlier schema-1-only tools reject version 2.
History is append-only through the CLI. Rejection preserves raw bytes and old
records. Human acceptance is distinct from preparation admission; blockers record
the policy result at publication and are not silently recomputed on later reads.

Portable export contains `records/`, `objects/` and `export.json` with
`schema_version: 1`, `kind: curation_export`, `review_sha256`,
`records_oldest_first`, `includes_rasters: false`, and `admission: not admitted`.
The final manifest lists immutable record IDs; those records pin the captured
objects and external raw/review lineage. There is no terrain or dataset export yet.
