# Implementation progress

Updated 2026-09-21. Twelve real terrain regions are qualified and prepared; one reviewed tile is excluded.
The [master plan](plans/terrain-design-master-plan.md) remains authoritative.

## Resume here

The owner delegated interpolation-quality decisions to the implementer and asked
for simplicity. We adopted [supplier_qa_pilot_v1](design/source-quality-pilot-v1.md):
use the released provider product's QA as the baseline, retain known limitations,
and exclude identified defects. Routine supplier interpolation beneath forest is
acceptable. Do not reopen the confidence question, invent a density cutoff, or
launch more LiDAR checks for the existing candidates.

The owner answered “all yes, except 6, that one is no” for batch 2. Tiles 1–5
are selected and prepared; tile 6 is excluded, with all its data preserved. The
three original Río Abajo selections remain current. No repeat vote is needed.

The owner answered **“all yes”** for batch 3 numbers 1–4. All four are selected,
qualified and prepared, with river water masked separately and native ground
preserved. The original batch page/JSON remain unchanged; exact response and
identities are in `data/review/pilot-batch-3-2026-09-21/decisions-v1/`.

The next actual input is the **numeric pilot dataset checkpoint**, not another
source-tile review. The [proposal report](reports/numeric-pilot-proposal-v1.md) and
local `data/review/pilot-dataset-proposal-v1/review.html` describe 12 selected tiles
from six areas, grouped into calibration and validation. The proposal is pinned
and **not yet approved**. It authorizes no training/model promotion; release
commands and the production Python loader are still unimplemented.

Continue in the existing checkout on `feat/east-ridges-qualification`; no worktree.
This continuation builds on `ea512aa` and uses existing production tools without
code changes. Merging remains with the owner.

## Implemented and verified

| Area | Current behavior |
| --- | --- |
| Contracts | Rust CLI validates world/runtime presets, nine candidate class IDs and source metadata. Vehicle settings and model tolerances remain unset. |
| Acquisition | Bounded HTTPS range downloads, strong ETags, resumable chunks, immutable raw assets, original metadata and local import. Live TNM probes previously returned 504; direct NOAA/USGS assets worked. |
| Native review | Float32/native-mask crops with metric browser previews, multiple hillshades, profiles and low-altitude views. Display products remain separate from elevation. |
| Curation | Immutable SQLite history, captured evidence, explicit whole-tile decisions, stale-write protection, integrity checks and portable exports. Technical judgments belong to the implementer. |
| Quality | `native_quality_v2` checks exact grids, categorical masks, at least 99.5% usable terrestrial coverage and optional annotated-feature support. `supplier_qa_pilot_v1` supplies the declared evidence basis; no validator threshold changed. |
| Preparation | Same-survey native regions preserve valid Float32 bits and separate masks; immutable publication, current-parent checks and catalog registration. Twelve real regions now prepared; water receives deterministic masked fill. |
| Ground sampling | Three early 64 m samples and three completed full-crop count checks. All native count bins verified against retained XY points. Counts remain diagnostics, not measured feature accuracy. |
| Numeric pilot proposal | Twelve selected native regions from six geographic areas; calibration/validation grouping and a small Float32/Float16 arithmetic probe are ready for the dataset checkpoint. |
| Future work | Geographic dataset releases, Python numeric/model evaluation, label editing, generation, world editing and terrain export remain unimplemented. |

## Current Río Abajo state

The catalog has **35 immutable curation records and 12 registered prepared regions**.
Its integrity check reports no unregistered records or unreferenced objects.
The first preparation migrated the database to schema 2; the check report itself
still has JSON schema version 1.

The three original Río Abajo records preserve the owner's existing human acceptance,
candidate class 1 and confirmed dataset permission. Codex's six technical findings
pass under the delegated supplier-QA rule. Each crop has 1,048,576 usable
terrestrial cells, zero water cells, zero project repairs and zero optional
feature annotations. Coverage describes usability under that rule, not an
independent per-cell accuracy measurement.

The three footprints are disjoint, so each was prepared separately at its native
0.5 m spacing. No mosaic across gaps, smoothing, repairs, resampling or vertical
transformation occurred. All 3,145,728 decoded Float32 values are bit-identical to
the accepted reviews; all output mask values were independently checked.

| Crop | Current curation record | Prepared manifest |
| --- | --- | --- |
| north-hollows | `d93a7e616fc399b108c8be2df50a42a39b4588c2367c3e8232a88a067f742a1d` | `0bea5dd7cc7fb507b9d32472ec32805bc542d1805b79eec943cda473871819c2` |
| east-ridges | `725793b75f16db1aeb276cc83c5ae84a3b84665d8014e49c321acb8462232890` | `e70cda3efae445259d83394c2cec2d969eae135c133287ade735a7a6727d6373` |
| south-saddles | `b413a168100ca35b625752eef7f9b108c9f39cb7107bc8e6cfaa5a614aa01f4f` | `f366ddbc9f7ee3c65ea3dee522c0c3ccd81843bd1f25a8d439b2ff869c7aebd1` |

Prepared manifests are `data/prepared/<sha256>/manifest.json`.
`prepare verify` reports verified integrity, registration and current parents for
all three. Their schema retains `review_state: unreviewed` and
`training_admission: not admitted`. This does not require another visual review
of unchanged source tiles; no dataset release or new human output decision has
been invented.

## Batch 2 completed

Batch 2 contributed five acceptances and one rejection; the current catalog now
has twelve human acceptances and one rejection. The exact
batch response and all six decisions are captured in
`data/review/pilot-batch-2-2026-09-21/decisions-v1/`; the original page/JSON remain
unchanged. Tile 6 (`river-south-terrace`) is rejected. Its current record is
`310b3d541c8b4d7a8c6e69450bf1847034ba11611a82916aae859e3f8a163bc5`.
Do not delete its data, prepare it, or infer a rejection reason.

Five accepted crops passed the supplier-QA, coordinate, permission and complete
mask gates. Codex reviewed the exact USGS tile XML and 3DEP permission basis for
all three sources; no new human legal approval is claimed. Public imagery context
identified no open water surface, with visibility limits retained in evidence.
The crops are all-terrestrial, with zero project repairs/optional annotations.
Their broad candidate labels do not assert narrow slots or braided channels.

All five native regions are prepared separately. Independent decoding verified
5,242,880 elevation values bit for bit and every output mask; prepared verification
reports intact, registered and current. Current curation/prepared IDs are in
`data/review/pilot-batch-2-qualification-v1/records.json` and the
[batch report](reports/pilot-tile-batch-2.md). Technical evidence updates preserve
the original owner votes. No training dataset is released.

## Batch 3 completed and numeric proposal

[Batch 3](reports/pilot-tile-batch-3.md) is complete. Four current qualified
records preserve the exact owner “all yes”; technical findings belong to Codex.
Both source tiles have retained original USGS XML, NOAA collection lineage and
source-specific published permission review. Codex interpreted river polygons
from public imagery and native DEMs. Their display-resolution, date and canopy
limits remain documented; no surveyed per-cell water-boundary accuracy is claimed.

All four quality assessments and native preparations passed. Independent decoding
checked 3,834,334 valid land values bit for bit, 359,970 water cells as exact
positive-zero fill and all seven raster layers. The catalog check has no orphans.
Current IDs are `data/review/pilot-batch-3-qualification-v1/records.json`.
The original eight prepared regions and batch-2 rejection remain unchanged.

The proposed numeric pilot contains 12 regions / 3.146 km² of source footprint,
with six geographic groups across three broad families. Keep each area's crops
in one role: Río Abajo, central gorge and southern floodplain for calibration;
Camuy, Guajataca and Arecibo for validation. Minimum opposing edge separation is
8,042.645 m, exceeding the declared 512 m context/buffer. This grouping is for
small native numeric evaluation only. There is no untouched test set, independent
provider-project sample or full coarse-scale training context. Larger context
requires a fresh geographic check; do not silently reuse this split for 8,192 m
windows, whose buffer it would fail.

A temporary CPU arithmetic probe ran on all 12 prepared regions plus ramps,
flats and sharp/narrow synthetic ridges. Fixed experimental normalization is
height / 2,048 with inverse × 2,048, in Float32. All 12,222,942 valid real samples
round-trip bit-identically; water is excluded from height/slope/terracing metrics.
Float16 storage introduces up to 0.25 m error on real ground and removes the
0.125 m synthetic ridge at 1,800 m. The 9.1-second run used about 146 MiB peak
Python-process RSS, no GPU; this is not a total process-tree memory measurement.
This is preliminary evidence, not a production loader, autoencoder result or
accepted model/gameplay tolerance. No new production code or ML dependencies.

## Retained local artifacts

These are ignored by Git; preserve and back them up separately.

| Artifact | Location |
| --- | --- |
| Current policy, exact delegation, supplier report references, catalog backup | `data/review/rio-abajo-supplier-qa-pilot-v1/` |
| Complete quality inputs/assessments, technical qualification, curation records/exports, build requests and verification | The same directory, under each of `north-hollows`, `east-ridges`, `south-saddles` |
| Current IDs and native-bit comparison | `rio-abajo-supplier-qa-pilot-v1/records.json` and `native-comparison.json` |
| Original native crops, previews and source comparison | `data/review/rio-abajo-pilot/` |
| Immutable raw sources and receipts | `data/raw/`; original karst receipt `68ffca798986f0a68154b1bd7bd1a66ecbe1e089d3abeb63140d5cc95ea8795d` |
| Original dataset permission evidence and human approval | `data/review/rio-abajo-permission-2026-09-21/` |
| Earlier tile votes and exact batch response “both yes” | `data/review/rio-abajo-tile-decisions-2026-09-21/` and its `accepted-batch-v1/` |
| Completed whole-crop LiDAR evidence | `data/review/rio-abajo-ground-support/{north-hollows,east-ridges,south-saddles}-full-v1/` |
| Earlier blocked measurement checkpoint and mask evidence | `data/review/rio-abajo-full-crop-completion-2026-09-21/`, `data/review/rio-abajo-quality-v2-2026-09-21/` |
| Completed batch 2, captured decisions and comparisons | `data/review/pilot-batch-2-2026-09-21/` |
| Batch 2 qualification, backup, masks, exports, prepared results and native comparison | `data/review/pilot-batch-2-qualification-v1/` |
| Completed batch 3, captured votes, previews and comparisons | `data/review/pilot-batch-3-2026-09-21/` |
| Batch 3 source inputs and display overviews | `data/staging/pilot-batch-3-2026-09-21/` |
| New batch discovery/source inputs and display overviews | `data/staging/pilot-batch-2-2026-09-21/` |
| Catalog and captured evidence | `data/catalog/topography.sqlite3` and sibling `.curation/` |
| Batch 3 source qualification, masks, exports, prepared results and native comparison | `data/review/pilot-batch-3-qualification-v1/` |
| Pinned numeric pilot proposal, review page, separation calculations and reproducible arithmetic probe | `data/review/pilot-dataset-proposal-v1/` |

Earlier incomplete assessments, source evidence and catalog snapshots remain
historical. Do not use their blocker lists as current status. Current original-region IDs are in
the table above; batch-2 IDs are in its qualification directory. Original reviewed pages, raw assets and completed evidence were
not overwritten.

The [source qualification report](reports/rio-abajo-source-qualification.md)
retains the measurement results and adopted rule. North/south full-crop jobs ran
sequentially under the owner's explicit readiness: acquisition/counts took about
5.4 minutes each plus verification, maximum child RSS about 72/73 MiB, GPU unused.
Do not repeat them. Full-source COPC bytes were not stored; retained CSVs are
measurement evidence, not ground-quality masks.

## Verification for this continuation

- Four new votes match exact “all yes”, original batch/page/review identities and
  portable exports. Source selections are separate from Codex technical findings.
- Existing quality/eligibility/preparation/verification workflows passed for all
  four new regions. Catalog integrity reports 35 records and 12 prepared regions;
  13 current heads contain 12 accepts and one preserved rejection, with no orphans.
- Independent GDAL decoding checked all 4,194,304 new prepared samples and masks.
  Existing raw/native/browser comparison reports remain pinned in the records.
- OGR reports all five interpreted water polygons valid. A nodata marker emitted
  by rasterization was caught by quality assessment and explicitly removed from
  draft masks; the same workflow then passed without code changes.
- Twelve-region arithmetic metrics, geographic separation, proposal/page hashes,
  source selection/evidence identities and local documentation links are checked.
  No production code changed; the full `scripts/check.sh` suite was not repeated.

Earlier full checks covered rustfmt, locked offline build, Clippy, valid/invalid
contracts, local GDAL workflows, SQLite recovery, mask eligibility and native
preparation failures. Development uses Rust 1.98.1, GDAL 3.13.3 and PDAL 2.10.1;
Rust 1.85 itself was not tested. No Python ML environment or additional system tool
was installed. The production model-loader/autoencoder milestone remains open despite the successful
preliminary Float32 arithmetic probe.

## Continue after the numeric dataset checkpoint

1. Capture the owner's decision against `review-identity.json` and the exact
   proposal/page hashes in `data/review/pilot-dataset-proposal-v1/`. The pending
   approval is for this small numeric evaluation collection and grouping, not
   source re-selection, model training or acceptance of an accuracy tolerance.
2. Implement the versioned dataset release boundary and reusable Python numeric
   evaluation job. Validate current prepared parents, geographic grouping, all
   paired layers, fixed normalization and masks before publication/use. Reuse the
   documented local probe as measured reference, not as production code.
3. Keep numeric-pilot and future model-training releases distinct. This collection
   has no untouched test split or large coarse-context coverage; expand/repartition
   only when the next concrete experiment requires it. Do not restart broad tile
   review before implementing the numeric consumer.
4. Evaluate candidate loaders/codecs in metres before choosing or training a
   model. Establish actual tolerances and a bounded experiment budget from the
   pilot; obtain readiness for substantial new workloads. Keep 16 GB VRAM ceiling
   and curated-patch/blending fallback.
5. Continue toward the 4,096 m world pilot, regional editing and export as ordered
   in the master plan. Vehicle dimensions remain explicitly unset.

**Standing constraints:** public downloads only; never send or draft email, or
ask the owner to contact institutions. Whole-tile yes/no review in batches.
Preserve accepted source decisions while technical work continues. Ask readiness
only for genuinely substantial new workloads under AGENTS.md; Rust builds and
file downloads are exempt. Keep `PLAN.md` and `TASKS.md` outside commits.
