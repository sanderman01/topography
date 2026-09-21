# Crop curation and evidence

The CLI now records crop-wide labels, annotations and explicit human accept/reject
decisions in SQLite. Each decision pins the exact raw receipt, native review and
legend, and captures small supporting documents by SHA-256. It does not prepare
rasters, paint per-cell labels, verify the truth of evidence, or admit training data.

## Whole-tile yes/no review

Present candidate previews in batches. The owner's task is to say yes or no for
each whole tile. No ridge selection, passage marking, mask authoring or detailed
justification is required. A yes selects that tile; technical checks and training
admission remain separate. Do not ask for another visual approval of unchanged
terrain when technical evidence is updated.

Use the [persistent gallery](batch-source-review.md) for normal batch review.
Its Yes/No buttons save decisions against the exact review identity in the catalog,
including when technical findings are still unknown. The implementer reads those
saved votes and handles qualification with the commands below. Conversation-based
recording remains available when explicitly needed; do not duplicate gallery votes.

## Create and record a decision

Start with a verified [native review](dem-acquisition.md). Paths below are examples;
substitute the receipt and crop you actually inspected.

```sh
topography curate draft configs/runtime.example.toml data/raw/records/RECEIPT_SHA256.json data/review/candidate/review.json catalog/labels/terrain-v1.toml data/review/candidate/decision.json
```

Edit the generated JSON. The default actor is `unassigned`, the decision is
`annotate`, and the label, coverage and quality findings are unresolved. Publication
requires an actor kind/name and a reason. Only an explicit `human` actor may record
`accept` or `reject`; acceptance also requires an active primary class ID. This is
a local declaration of authorship, not login authentication or a digital signature.
Use `assistant` for the assistant's own judgments. When transcribing an actual
human yes/no, identify the human as the decision actor and capture the exact
response, reviewed artifact and assistant recorder in supporting evidence.
Never invent a human decision or attribute technical pass findings to a visual yes.

Each evidence entry has a local `path` and an `evidence` object containing `id`,
`description`, optional HTTPS `source_url`, and the SHA-256 of the reviewed bytes.
Use `sha256sum path/to/report.json` to obtain the hash. Evidence documents are
limited to 1 MiB each, at most 16 per record. Reference large rasters and point
clouds through reports containing their checksums; retain those artifacts separately.

```sh
topography curate record data/review/candidate/decision.json configs/runtime.example.toml
topography curate status configs/runtime.example.toml REVIEW_SHA256
topography curate check configs/runtime.example.toml
topography curate export configs/runtime.example.toml REVIEW_SHA256 data/exports/candidate-curation
```

`record` prints the immutable decision SHA-256. `status` returns the current
portable JSON after checking the crop's complete decision history. Use the review
file's SHA-256 for `REVIEW_SHA256`, also present in the draft's `target` object.
`check` verifies all indexed records, objects, raw parents and review artifacts.
It reports published but unregistered records and unreferenced evidence objects.
It never accepts, repairs or deletes data.

## Findings and limits

The six evidence categories are coordinates, classified ground support,
artifacts/repairs, feature fidelity, water classification and unsupported gaps
through defining features. `pass` and `fail` require a note and captured evidence
IDs. `unknown` can carry observations without claiming that the gate is satisfied.
Terrestrial coverage needs explicit counts and evidence establishing its denominator;
provider-valid cells and bins containing LiDAR returns are different measurements.
These are technical assessment fields, not a questionnaire for the owner. Leave
unresolved findings unknown without blocking the recording of a whole-tile yes/no.

The historical v1 preparation precheck reports unresolved findings, missing acceptance,
unconfirmed/excluded ML permission, missing source metadata, synthetic sources,
unsupported metric grids and failure to establish the initial 99.5% terrestrial
coverage threshold. This consumer supports native 0.5–1 m grids; it does not
implement the plan's possible lower-fidelity exceptions. A permanent blocker also
states that regional preparation and independent admission checks are pending.
Even a human-accepted record therefore remains `not admitted`. Those immutable
prechecks retain their original interpretation. The separate
[regional quality and eligibility check](regional-quality.md) now recomputes
whole-crop mask measurements against the current decision and captured evidence.
It reports eligibility without admitting or preparing data.
The [aligned preparation consumer](aligned-preparation.md) separately repeats
these checks before publishing eligible native regions. A prepared output remains
unreviewed and excluded from training; later source decisions can make it stale.

Permission reviews are separate declarations, leaving the original source metadata
unchanged. `confirmed` requires dataset-specific permission evidence, captured
documents and explicit attribution, edited-map reuse and redistribution obligations.
The CLI verifies document identity and required fields; it cannot establish legal
permission from the wording. Follow the [source-admission policy](../best-practices/data-management.md).

## Revisions, recovery and backup

Create a fresh draft for each change. It pins the current head in
`previous_record_sha256`. New drafts deliberately reset review claims to unresolved;
carry forward findings only after reviewing their evidence. Recording a new decision
appends history; it does not overwrite previous acceptance or delete raw assets.
The latest decision controls current status, so a later annotation or rejection
supersedes acceptance. Identical draft replay is idempotent, including older records.
A different submission against a stale head fails.

When updating technical evidence for an unchanged accepted tile, retain the
captured original human decision and its scope in the new record. Do not replace
it with a fresh visual-review request or silently turn an annotation into a human
decision. Version 1 has one current record containing both selection and technical
findings; preserving the decision during these updates is the implementer's task.

Missing source declarations can require a new immutable import before preparation.
For a metadata-only replacement, retain the original receipt, review, decision and
gallery. Carry its selection into the replacement's curation record only after
checking the same raw asset checksum, native window, actual grid/CRS/axis mapping,
vertical interpretation, decoded Float32 values and provider validity. Capture
the original decision bytes and the equivalence check as evidence, and state that
the implementer carried the existing selection; do not describe it as a new vote
or human technical certification. The new review has a separate curation chain.
Keep one tile entry in the next gallery while preserving the earlier snapshot.
This is a manual use of existing commands, not automatic acceptance transfer. A
changed surface, mask, window or physical interpretation does not qualify for it.

The database is the configured `catalog_db`; its sibling directory with `.curation`
appended contains `records/<sha256>.json` and `objects/<sha256>`. Unrelated SQLite
databases and unknown schema versions are rejected. Files publish before a short
SQLite transaction registers the record and updates the head. After a crash, retry
the original draft to register intact published files if its previous head still
matches. Orphaned files remain available for explicit review/housekeeping. Missing
or corrupt registered objects fail verification and require restoring intact bytes
from backup; there is no silent repair or promotion. Abandoned temporary files or
an incomplete export directory remain identifiable and are not completion records.

The first prepared-region registration transactionally migrates database schema
1 to 2, adding the prepared registry while preserving curation record bytes.
Current code reads both versions. `curate check` also checks registered prepared
files; its orphan lists concern curation files only.

Version 1 limits a catalog to 4,096 decisions. Export writes a new directory with
the selected crop's full history, captured documents and an `export.json` completion
manifest. Local raw/review locator paths stay in SQLite. The export does **not**
include raw, crop or prepared rasters, nor provide an automated restore command. Back up those
files and their original directory relationships separately. Back up a quiescent
database or use SQLite's backup API; copying an active database alone is insufficient.

The locked `rusqlite` dependency builds bundled SQLite using a C compiler; a system
SQLite development package is not required. Development ran on Rust 1.98.1. The
declared minimum Rust 1.85 was not directly tested, and rusqlite does not promise
that minimum. Use the installed toolchain for this pilot; minimum-version assurance
remains a compatibility check to complete before release.

See the [curation contract](../../schemas/curation-v1.md) and
[workflow checks](../../scripts/README.md).
