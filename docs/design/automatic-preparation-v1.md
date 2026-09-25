# Automatic library preparation

Approved workflow: technical checks run when a tile joins the library; a saved
Yes queues preparation; the gallery reports progress or a concrete technical
problem. A No excludes the tile. Processing never invents a vote, overwrites raw
terrain or admits training data. Existing accepted tiles need no repeat review.

## Source assessment and scope

Reuse explicit source assessments instead of reinterpreting supplier documents
for every crop. A version-1 JSON profile pins `source_sha256`, `asset_sha256`,
`policy: supplier_qa_pilot_v1`, captured evidence and technical findings. It is an
implementer-authored technical assessment, not a human terrain vote. Confirmed
permission must already be declared by the source and backed by captured profile
evidence. Unknown/restricted/prohibited permission cannot become confirmed by
successful processing. Evidence is reusable by path/hash across profiles.

Profile shape:

Each evidence path is relative and contained beneath the profile directory. A public
document includes its HTTPS `source_url`; local assessments, policy and delegation
evidence may omit it. Optional `blockers: ["description"]` prevents qualification
for known unresolved problems while preserving the tile's selection.

```json
{
  "schema_version": 1,
  "policy": "supplier_qa_pilot_v1",
  "source_sha256": "64 hex characters",
  "asset_sha256": "64 hex characters",
  "evidence": [{"id":"supplier", "path":"supplier.txt", "sha256":"64 hex characters", "description":"Exact source supplier QA and permission assessment", "source_url":"https://example.org/source"}],
  "permission_evidence_ids": ["supplier"],
  "findings": {
    "coordinates":{"note":"Documented native metric grid and datum", "evidence_ids":["supplier"]},
    "ground_support":{"note":"Supplier QA policy applies", "evidence_ids":["supplier"]},
    "artifacts_and_repairs":{"note":"No identified blocking defects; no project repairs", "evidence_ids":["supplier"]},
    "feature_fidelity":{"note":"Native source limitations retained", "evidence_ids":["supplier"]},
    "water_classification":{"note":"Inspected terrestrial footprint", "evidence_ids":["supplier"]},
    "unsupported_feature_gaps":{"note":"No optional feature annotations requested", "evidence_ids":["supplier"]}
  },
  "water": {"kind":"all_terrestrial", "windows":[[0,0,2048,2048]]}
}
```

All-terrestrial classification is allowed only inside an explicitly assessed
native window; no water is inferred from height or absent vectors. Alternatively
`water` is `{"kind":"mask","path":"water.tif","sha256":"..."}`: a complete
source-grid Byte raster with categories 0 unknown / 1 land / 2 water. Cropping
preserves its native grid; unknown cells block qualification. Source masks must
match the exact source grid and CRS and use contained relative paths beside the
profile. Both modes use the profile's captured water
finding. Known defects require a corrected scoped assessment; do not auto-pass
unresolved findings or erase existing failed findings.

## Version 2 surface-cover profile

Profile schema 2 uses policy `supplier_qa_cover_v1` and adds `snow_ice` and
`vegetation` descriptors. Each descriptor has a nonempty `note`, nullable
`observed_at` and captured `evidence_ids`. Its tagged shape is one of:

```json
{"kind":"all_absent","windows":[[0,0,2048,2048]],"note":"Assessed absent","observed_at":"2025-08-14","evidence_ids":["supplier"]}
{"kind":"mask","path":"snow-ice.tif","sha256":"64 hex characters","note":"Dated classification","observed_at":"2025-08-14","evidence_ids":["supplier"]}
{"kind":"unavailable","note":"No suitable public classification","observed_at":null,"evidence_ids":[]}
```

`all_absent` must cover the complete review window. `mask` is a complete
source-grid Byte raster with 0 unknown, 1 absent and 2 present. It follows the
same grid, CRS, categorical and contained-path checks as water. `unavailable` is
permitted only for vegetation and produces an all-unknown crop mask. Assessed
snow/ice with unknown crop cells blocks qualification; unknown vegetation remains
optional and does not remove valid ground.

The v2 package and quality input retain both masks plus per-layer status, note,
date and pinned evidence. Snow/ice is excluded from the ground denominator and
ground output, while the measured provider surface remains separate. Existing v1
profiles, package bytes, products, qualifications and human votes remain valid
legacy artifacts and are not cover-certified. Backfill appends an attributed
technical successor to the existing human Yes and never clears a failed finding.

## Existing interfaces and integration

- Acquisition source and gallery input accept optional `preparation_profile`.
  Batch pins its identity and passes it to gallery input. Published gallery tiles
  retain a `{path,sha256}` profile reference; old galleries remain readable.
- `preparation::automatic::stage(raw, review, profile, output)` publishes an
  immutable preflight package with masks and quality assessment. It verifies
  source identity, profile evidence, numeric coordinates and coverage, and never
  writes curation. Unknown/missing profiles remain reviewable with attention status.
- `preparation::automatic::prepare(runtime, package, legend, expected_head)`
  carries the exact existing Yes into an evidenced technical successor and
  invokes existing preparation. No/replaced decisions block publication. The
  successor clearly attributes new findings to automation, retaining the original
  human vote bytes. Repeating an unchanged job creates no extra records/products.
- `preparation::automatic::existing(runtime, review)` returns an eligible existing
  prepared manifest when one is registered with current parents. Integrity is
  checked before reporting Ready. Existing products can be recognized without a
  profile. This does not admit them to a dataset.
- One background worker per gallery handles bounded native crops sequentially.
  No new dependencies, network access, point clouds, GPU or large mosaics. Votes
  are saved through the existing handler immediately; raster work is off-thread.
- Job inputs and outputs persist beneath the runtime review root. On restart,
  verified preflight/prepared artifacts are reused; interrupted work resumes.
  Failed jobs are attempted once per service start or changed decision. Restart
  retries exceptions and verifies all outputs afresh; persisted status is advisory
  and is not trusted as proof of completion. Failed jobs do not loop endlessly.
  Changed inputs create new identities; corrupt
  output is reported, never silently replaced. Current catalog heads are checked
  before technical publication and prepared registration.
- `/api/tiles` adds `preparation: {state,message,manifest}`. States are checking,
  awaiting_acceptance, preparing, ready, assessment_pending, needs_attention and
  not_selected. Legacy products retain their manifest link with
  `assessment_pending`; configuring a v2 profile queues a cover backfill without
  asking for another Yes. Only v2 products report Ready under the cover contract.
  The page refreshes statuses without changing the selected preview or retrying
  votes. A technical successor alone must not turn a deliberate vote into a stale
  user-decision conflict; actual competing human votes retain conflict protection.
- `/api/overlay/N` returns packed 2-bit water, snow/ice and vegetation
  categories (0 unknown / 1 absent / 2 present) for a v2 Ready tile's native
  window, after checking the manifest digest and mask hashes. Other tiles get 404.
  It is display-only and writes nothing.

## Verification

Exercise real CLI/HTTP workflows in isolated tiny fixtures: staged checks before
acceptance create no vote; Yes reaches Ready with exact Float32/masks; No prevents
publication; restart reuses output; interrupted qualification resumes; unknown
profile/water, mismatched source and changed/corrupt evidence show attention;
failed technical findings are retained; stale human votes still fail; prepared
outputs become unusable after rejection. Existing batch/gallery/curation checks,
Chromium inspection and `bash scripts/check.sh` remain required.

Real rollout preserves a catalog snapshot and all existing records/outputs.
Configure assessed profiles for the four accepted dunes/Badlands candidates,
start the updated gallery and verify the real job results. Tile problems remain
visible; automatic processing is not permission to smooth artifacts or change votes.
