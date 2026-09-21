# Numeric evaluation contracts v1

Consumers: `dataset build`, `dataset verify`, `evaluate numeric` and the installed
`topography_training.numeric` worker. See the [workflow](../docs/workflows/numeric-evaluation.md).
These are evaluation-only contracts, with no model-training or output-promotion state.
Rust/Python reject unsupported versions and unknown fields at their input boundary.

## Selection and approval

A selection contains `schema_version: 1`, `purpose: numeric_evaluation_only`,
`maximum_context_m` in (0, 8192], finite `separation_buffer_m` at least that context,
`normalization: {offset_m: 0, scale_m: 2048}`, and `groups`.
Each group has an ASCII alphanumeric/underscore/hyphen `id` (1–80 characters),
`role: calibration|validation`, and nonempty `prepared` SHA-256 list. Both roles
are required; group IDs and prepared membership are unique, with 2–32 groups and
at most 32 regions total. Locators derive from configured prepared storage.

Prepared grids are limited to 2,048 cells per side and must fit the declared
physical context. Current registered parents, common CRS/datum and nonoverlapping
footprints are required. Opposing roles must have footprint-edge separation at
least the declared buffer and cannot share a raw source asset. These checks do
not prove statistical independence of surveys or infer geographic grouping from
landform semantics; the declared grouping remains an implementer judgment.

An approval has `schema_version: 1`, `kind: numeric_evaluation_approval`,
`selection_sha256` of the exact selection file bytes, `actor_kind: human`,
`actor_name`, `recorder`, `decision: approve`, `exact_response`,
`scope: numeric_evaluation_only`, and `reviewed_proposal_sha256` /
`reviewed_page_sha256`. Actor, recorder and response are nonempty and at most
4,096 bytes. All hashes are lowercase 64-character SHA-256 strings. These are
local declarations, not authenticated signatures or evidence of technical expertise.

## Published release

The single immutable `datasets/<sha256>/manifest.json` contains
`schema_version: 1`, `kind: numeric_evaluation_release`, the evaluation-only
`purpose`, exact UTF-8 `selection_document` and `approval_document` strings,
and `topography_version`. Embedding exact document strings preserves the approval's
byte-level target without inventing a cross-language JSON canonicalization rule.
The manifest is limited to 1 MiB. Its directory name hashes its exact bytes.

There is no new SQLite table. Verification derives prepared locators from the
runtime roots and checks current registration/history. A later source edit or
rejection makes use fail; historical release bytes remain unchanged. Publication
is idempotent for identical documents. A concurrent change can leave a preserved
release that fails current verification; existence is never sufficient for use.

## Rust-to-Python numeric request

`schema_version: 1`, `kind: numeric_evaluation_job`, `dataset_sha256`, fixed
`normalization`, and 1–32 `regions`. Each region contains `prepared_sha256`,
`group`, `role`, native `grid: {origin, size, spacing_m}`, `masked_fill_m`, and
`layers`. Grid size is [columns, rows], spacing is [east-west, north-south] metres,
and origin is the north-west outer corner. Rows run south.

Seven exact layer names are required: `elevation`, `validity`, `provider-validity`,
`water`, `quality`, `repairs`, `features`. Each value is
`{path, bytes, sha256}`. Paths are flat binary filenames beside the request;
symlinks and traversal are rejected. Elevation is little-endian Float32 metres,
all masks unsigned Byte. Payload length must equal width × height × item size.

Validity/provider-validity/water use 0/255; quality uses 0/1/2; repairs must be
zero in this consumer; features use 0/1. Validity must equal provider-valid,
supported, nonwater ground; marked features must be valid. All masked elevations
must match the declared Float32 fill bits. All-masked inputs fail. Valid ground
must be finite and within −2,048 to +2,048 m; no clipping occurs.

## Results

The Python worker writes `metrics.json` and `result.json` in a new directory.
Its result pins request, dataset and metric hashes and declares `gpu_used: false`;
`environment` records Python/NumPy/package/platform versions, source-code hashes,
elapsed time and worker-process RSS scope. It does not authorize source use by
itself: the Rust command performs release verification before and after the job.

Metrics report per-region identity/role, land/water/invalid counts, Float32 and
Float16 height max/RMS error in metres, slope-component max/RMS error in m/m,
valid/nonflat neighbor counts, newly flat pairs and bit agreement. Slopes require
both neighboring samples valid and use the actual axis spacing. No valid neighbor
pairs yields null slope errors. Four fixed synthetic cases supplement real data.
No pass threshold for a future model or gameplay clearance is invented.

Rust checks completion identities, declared region counts and metric shape,
rehashes all payloads, rechecks the release, and publishes the new run directory.
The final `manifest.json` is `kind: verified_numeric_evaluation`, pins request,
worker result and metrics, records current sources at its final check and retains
`training_admission: not admitted`. It contains no human output acceptance.
Request, binary payloads, log and worker result remain with the run for diagnosis
and reproduction. A run's existence does not imply its sources are still current.
