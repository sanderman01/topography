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

## Planned surface-layer follow-up

The implemented v1 profile, package and prepared-product contracts have no
snow/ice or vegetation fields. A later schema version will add an explicit
per-layer assessment of unknown, assessed absent or present, with assessed
coverage kept separate from provider validity. Missing or unavailable data never
means absent. Confirmed absence may use a compact all-clear declaration and need
not be rasterized until a consumer requires it. Preserve existing v1 products,
their original qualification and human votes; they are not automatically certified
under the new mask contract.

Snow/ice present will require a native-aligned mask plus source, observation date
and provenance. An unresolved footprint will hold revised ground qualification;
the workflow will not fabricate ground beneath ice. A wholly ice-covered tile
may retain its accepted review but cannot serve as a bare-earth training sample.

Vegetation will be a best-effort independent layer that may remain unavailable
or partial without blocking valid bare-earth preparation or invoking
canopy-confidence checks. It may logically overlap snow/ice and water and cannot
be inferred from a biome label or bare-earth DEM. Tiles assessed with neither
vegetation nor snow/ice are a valid empty case. Vegetation classification is in
scope; rendering, placement and growth remain outside scope.

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
  awaiting_acceptance, preparing, ready, needs_attention and not_selected.
  The page refreshes statuses without changing the selected preview or retrying
  votes. A technical successor alone must not turn a deliberate vote into a stale
  user-decision conflict; actual competing human votes retain conflict protection.

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
