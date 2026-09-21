# Implementation progress

Updated 2026-09-21. Twenty real terrain regions are qualified and prepared;
the original twelve are released for initial numeric evaluation. Scalar Float32 loading passes. Both bounded
reconstruction candidates were rejected. The owner rejected the first coarse
world layout. Batch acquisition and a persistent terrain gallery now support
source expansion before another world-quality review. Two rejected tiles remain
excluded; a selected glacier-covered crater crop is separately blocked from
ground preparation.
The [master plan](plans/terrain-design-master-plan.md) remains authoritative.

## Resume here

**All five new tiles are Yes; no owner input is pending.** Gallery v7 now has
28 tiles: 26 accepted, two rejected and none pending. The two Zion clicks saved
normally. Three Grand Canyon clicks failed because a first visual choice inherited
confirmed permission metadata without captured curation evidence. The
[gallery fix and recovery](reports/gallery-vote-recovery-v1.md) preserve the two
saved votes and transcribe only the three missing Yes choices from the owner's
explicit message. No repeat review is needed.

Continue technical qualification of these five accepted crops, including river
masks and captured permission/QA evidence, then the queued Milford Sound sources.
The existing twenty prepared regions and original numeric release are unchanged.

Current snapshot: `data/review/library-gallery-v7/gallery.json`, SHA-256
`5f52f7528e2ee89583741df98efc7e4a0caa1b2ad62be0df3c79ad177e37fe7d`.
Integration and preservation evidence: `data/review/landmark-continuation-v1/`;
latest vote recovery: `data/review/gallery-vote-recovery-v1/`.
Restart with:

```sh
target/debug/topography gallery serve configs/runtime.example.toml data/review/library-gallery-v7/gallery.json 8765
```

**The owner liked Grand Canyon and requested more.** Three varied crops from the
adjacent source now join the [gallery batch](reports/grand-canyon-expansion-v1.md).
Keep that preference for future expansion. Zion's download also finished and
[two canyon contexts](reports/zion-review-v1.md) are selected. Together the new five
preserve all 17,301,504 native Float32 samples and provider masks exactly. Read
actual gallery decisions from the catalog before qualification. The owner has
explicitly accepted all five; visual selection does not establish qualification.

The four previous gallery choices were all Yes. The original Grand Canyon crop
and two St. Helens flanks are qualified and prepared. The glacier-covered crater
rim retains its Yes but is excluded from ground preparation. Missing source
metadata was completed through immutable replacements; independent checks prove
that the three selected successor crops have identical native terrain, validity
and physical coordinates. Original vote bytes, receipts, reviews and histories
remain preserved. See the [continuation report](reports/landmark-continuation-v1.md)
and its pinned equivalence evidence. No repeated owner vote or new technical
certification was inferred. At that checkpoint, the catalog had 55 records,
26 review heads and 20 prepared regions. After the five new owner votes it has
60 records, 31 review heads and the same 20 prepared regions.
Extra historical heads retain the metadata-only predecessors;
they are not extra terrain patches. All 50 earlier records and 17 prepared entries
are byte-preserved.

**Delegate suitable independent tasks to Sol (`gpt-5.6-sol`).** The owner requested
this default and [AGENTS.md](../AGENTS.md) now records it. Give distinct file scopes,
verify returned work and serialize shared catalog writes/integrity scans. Keep the
existing checkout; do not use worktrees.

**The three Banks Yes votes and coastal No are saved.** The three inland crops
are [qualified and prepared](reports/banks-peninsula-qualification-v1.md), with
all 7,077,888 Float32 samples and prepared masks verified independently. Supplier
interpolation beneath forest remains accepted under the delegated rule. At that checkpoint, the
catalog had 46 records, 19 current heads and 17 prepared regions. All 43 earlier
records and the other 16 heads are unchanged; no chat votes were duplicated.
Evidence and requests: `data/review/banks-peninsula-qualification-v1/`.

The two previously accepted wider Puerto Rico contexts are also prepared; their
[qualification](reports/connected-context-qualification-v1.md) and source records
remain unchanged. The current [source inventory](reports/pilot-source-coverage-v1.md)
now spans nine named areas and 22.232003 km² of unique prepared footprint, adding
Grand Canyon and Mount St. Helens to Puerto Rico and Banks Peninsula. The original twelve-region numeric
release and its 512 m context split remain unchanged. No model job or new training
release was started.

**Mount St. Helens resumed successfully.** All 315,921,778 bytes were downloaded
and imported. Receipt:
`10d39ba242193053284ffc87b89d80f758c1f6b0fce51c6622d96568eb4eda0f`.
The requested retry reached 98.2% but failed repeatedly on the final block at the
old two-minute limit. A bounded fix allows five minutes per range attempt while
preserving 8 MiB chunks, exact URL/ETag/size checks and three attempts. HEAD and
metadata limits are unchanged. The final block then completed. A scaled real
HTTPS regression failed before the fix and passes afterward, including verified
reuse; full `bash scripts/check.sh` passes. One fresh code review found no
remaining findings. Download run: `data/review/source-expansion-batch-v1/`.
The inspected crop queue `data/staging/source-expansion-v1/targets-review-v2.json`
reuses that receipt in `data/review/st-helens-review-v2/`.

**Use `docs/locations.md` explicitly for further discovery.** The preceding Banks
batch was opportunistic and did not directly rank that list. The new shortlist
retains all twelve examples and marks unqueried candidates honestly:
`data/staging/location-shortlist-v1/shortlist.json`. Actual USGS 1 m results exist
for Grand Canyon and Zion Narrows. The 600 MB bounded queue selects one source
from each. Grand Canyon is imported; its partially populated footprint required
choosing a complete northeastern side-canyon crop. Both downloads are complete; the adjacent Grand Canyon source is also imported.
Use `shortlist-v2.json` in that directory for the updated shortlist. Two LINZ
Southland assets near Milford Sound are now queued in
`data/staging/fjord-shortlist-v1/download-targets.json` (400 MB aggregate cap),
with collection/footprint/item metadata hash-verified. They are not yet downloaded;
inspect actual raster coverage before claiming connected fjord cliffs. Fjords and
island/cliff terrain remain gaps.

**Use the tools for source expansion.** The owner requested target lists instead
of repeated agent-driven downloads and one gallery for rapid review. The
[batch/gallery design](design/batch-source-workflow-v1.md),
[implementation plan](plans/batch-source-workflow-v1.md) and
[workflow](workflows/batch-source-review.md) are implemented. Use `batch download`
and `batch review`, then `gallery build` and `gallery serve`. Public downloads
only; no correspondence. Existing LINZ raw assets, historical galleries and all
rejections remain preserved. Waioweka is an unused discovery lead.

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

The owner responded to the numeric proposal, **“I have no idea what I am looking
at. I suppose its fine.”** Treat this as assent to initial evaluation with
technical responsibility left to Codex. Do not ask for another technical signoff
or treat the response as technical certification. The original proposal/page
remain unchanged, with the exact response captured separately.

The [numeric evaluation result](reports/numeric-evaluation-v1.md) completes the
reusable scalar loader step on all twelve regions. Every valid ground sample
round-trips exactly in Float32; Float16 absolute-height storage loses detail.
The evaluation-only release is published and no model training is admitted.
The owner then answered **“Continue”** to readiness for the bounded CPU image-codec
experiment. It completed all twelve real windows and four synthetic cases in
154.14 seconds, about 1.07 GiB worker RSS and no GPU. Codex rejected this fixed
image-VAE representation: real-window height RMSE was 5.47–31.24 m, and the
12.5 cm narrow ridge became a shallow dip. See the
[result](reports/image-codec-pilot-v1.md). Do not repeat it or train LoRA on it.

The owner then answered **“continue”** to the [single-channel fitting trial](plans/scalar-codec-fit-pilot-v1.md).
The separately scoped fitting release was published, and the one 200-step CPU
trial completed in 74.90 s at about 615.5 MiB worker RSS, no GPU. Codex rejected
this candidate: held-out interior height RMSE remains 7.66–12.17 m and the narrow
ridge's local contrast disappears. See the [result](reports/scalar-codec-fit-v1.md).
Preserve both model runs; do not launch further tuning or request another source vote.
The original numeric release remains evaluation-only, and production training
is not admitted.

**Next: expand the source library before another world-quality review.**
The owner rejected `data/worlds/pilot-layout-proposal-v2/review.html`: repetition,
dull rectangular areas, visible patch joins and a boring river route. Exact
feedback is pinned to the original manifest/page in
`data/review/world-layout-decisions-v1/rejection.json`. Preserve the rejected
proposal and existing source acceptances. The
[proposal report](reports/world-layout-proposal-v1.md) records the outcome.

The owner then questioned whether twelve patches could support a representative
pilot or final world. They suffice for initial loading/precision checks, not
world-quality or broad model-generalization conclusions. The
[coverage report](reports/pilot-source-coverage-v1.md) distinguishes twelve
512 m crops from six geographic areas, all in one Puerto Rico survey collection,
from the much broader library still needed. The rejected layout used only six
patches across 121 placements. More sources alone will not fix poor composition.

Resume public-source discovery and acquisition with larger connected landscapes,
several independent areas per pilot family and genuine transitions, then broaden
to the remaining candidate families. Preserve geographic separation before
overlapping crops; the numeric pilot's existing split is only for 512 m context.
Use batch whole-tile yes/no reviews, with technical qualification delegated.
Measure distinct source areas and coverage rather than target a crop count alone.

A subsequent small authored experiment is retained in
`data/worlds/pilot-layout-proposal-v3/` as **internal only**, with no human
acceptance. Most terrain is authored and six sources supply localized residuals;
it does not solve source diversity and is not the next owner review. No reusable
world composer or 4 m/1 m pilot is complete. Obtain readiness before genuinely
substantial new workloads; prior scalar-trial readiness has no broader scope.

Continue in the existing checkout on `feat/east-ridges-qualification`; no worktree.
This checkpoint builds on `942dcf2`; `817877c` adds the scalar fitting workflow.
Merging remains with the owner.

## Implemented and verified

| Area | Current behavior |
| --- | --- |
| Contracts | Rust CLI validates world/runtime presets, nine candidate class IDs and source metadata. Vehicle settings and model tolerances remain unset. |
| Acquisition | Bounded HTTPS range downloads, strong ETags, resumable chunks, immutable raw assets, original metadata and local import. List-driven queues pin inputs, resume verified successes and report independent failures. |
| Review gallery | One loopback browser gallery with thumbnails, native terrain views, filters, Y/N and arrow keys, direct journal votes and stale-write protection. Existing votes/evidence survive. |
| Native review | Float32/native-mask crops with metric browser previews, multiple hillshades, profiles and low-altitude views. Display products remain separate from elevation. |
| Curation | Immutable SQLite history, captured evidence, explicit whole-tile decisions, stale-write protection, integrity checks and portable exports. Technical judgments belong to the implementer. |
| Quality | `native_quality_v2` checks exact grids, categorical masks, at least 99.5% usable terrestrial coverage and optional annotated-feature support. `supplier_qa_pilot_v1` supplies the declared evidence basis; no validator threshold changed. |
| Preparation | Same-survey native regions preserve valid Float32 bits and separate masks; immutable publication, current-parent checks and catalog registration. Twelve real regions now prepared; water receives deterministic masked fill. |
| Ground sampling | Three early 64 m samples and three completed full-crop count checks. All native count bins verified against retained XY points. Counts remain diagnostics, not measured feature accuracy. |
| Numeric evaluation | Scoped immutable release, six geographic groups, current-parent verification, scalar Rust/Python job and full twelve-region Float32/Float16 measurements. |
| Image-codec evaluation | Pinned public VAE, deterministic CPU reconstruction, retained scalar/channel outputs, physical metrics and bounded supervision. First representation technically rejected. |
| Scalar fitting | Separate fitting-only child release, fixed CPU trial, initial/final weights and native outputs, checked scope and completion. First candidate technically rejected. |
| Coarse world proposal | Owner rejected the first reviewed 4,096 m layout at 16 m. Later authored experiment is internal only; source expansion precedes another world-quality review. |
| Future work | Production model training, reusable world composition/editing, label editing, refinement and terrain export remain unimplemented. |

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

The released numeric pilot contains 12 regions / 3.146 km² of source footprint,
with six geographic groups across three broad families. Keep each area's crops
in one role: Río Abajo, central gorge and southern floodplain for calibration;
Camuy, Guajataca and Arecibo for validation. Minimum opposing edge separation is
8,042.645 m, exceeding the declared 512 m context/buffer. This grouping is for
small native numeric evaluation only. There is no untouched test set, independent
provider-project sample or full coarse-scale training context. Larger context
requires a fresh geographic check; do not silently reuse this split for 8,192 m
windows, whose buffer it would fail.

Before the reusable worker, a temporary CPU arithmetic probe ran on all 12 prepared regions plus ramps,
flats and sharp/narrow synthetic ridges. Fixed experimental normalization is
height / 2,048 with inverse × 2,048, in Float32. All 12,222,942 valid real samples
round-trip bit-identically; water is excluded from height/slope/terracing metrics.
Float16 storage introduces up to 0.25 m error on real ground and removes the
0.125 m synthetic ridge at 1,800 m. The 9.1-second run used about 146 MiB peak
Python-process RSS, no GPU; this is not a total process-tree memory measurement.
This is preliminary evidence, not a production loader, autoencoder result or
accepted model/gameplay tolerance. That earlier probe changed no production code.
The reusable implementation and measured result now supersede it for execution;
the original probe remains preserved.

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
| Exact selection and scoped assent for the reusable consumer | `data/review/numeric-evaluation-v1/` |
| Immutable evaluation-only release | `data/datasets/6b7e2049f45d94bd87216ac07d96c38ece2885e8f22a00fb9d5e0686d6f07411/manifest.json` |
| Completed numeric run, all binary inputs, metrics and software provenance | `data/models/numeric-pilot-v1/` |
| Completed image-codec run and all original/reconstructed/channel/mask arrays | `data/models/image-codec-pilot-v1/` |
| Pinned codec job, readiness, environment freeze, current-source checks, executed code and rejection assessment | `data/review/codec-pilot-proposal-v1/` |
| Public pinned model artifacts / isolated CPU environment | `data/models/sd-vae-ft-mse-31f26fde/` / `data/models/codec-environment-v1/.venv/` |

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

## Earlier source verification

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
  No production code changed in that earlier source-only checkpoint.

Earlier full checks covered rustfmt, locked offline build, Clippy, valid/invalid
contracts, local GDAL workflows, SQLite recovery, mask eligibility and native
preparation failures. Development uses Rust 1.98.1, GDAL 3.13.3 and PDAL 2.10.1;
Rust 1.85 itself was not tested.

## Numeric consumer verification

- The installed Python worker ran outside the checkout and through the Rust CLI.
  Python 3.14.7 and pinned NumPy 2.3.5 are installed in `training/.venv`. That
  environment remains unchanged; the later CPU codec uses a separate environment.
  Python 3.11 remains untested.
- Full `bash scripts/check.sh` covers formatting, locked offline build, Clippy,
  existing workflows, release geography/currentness and the CPU numeric boundary.
  Faulty successful workers exercise incomplete metrics, invalid counts,
  missing provenance/synthetic cases and oversized logs.
- One fresh review identified incomplete completion validation and an exited-worker
  log-limit bypass. Both were reproduced and fixed; workflow checks preserve the
  failures. No further source review or technical approval was requested.
- All 12,222,942 valid samples preserved their Float32 bits. All 359,970 water
  cells were excluded. The worker took 2.81 s, about 131 MiB peak self RSS, no GPU;
  total Rust/GDAL process-tree time and memory were not instrumented.
- Independent verification checked the release/completion hash chain, all 84
  binary payload hashes, worker source hashes and complete real-region metrics.
  Previous source outputs and decisions remain unchanged.

## Image-codec verification

- Public safetensors checksum matches the pinned provider identity. Configuration,
  model card and exact CPU dependency versions are preserved; no private source,
  email, credential or GPU setup was needed.
- Small analytical metrics, mask exclusion, child failure, timeout and log-limit
  checks passed. A tiny CPU AutoencoderKL API smoke ran under the same address
  limit. Temporary identity fixtures exercised complete/corrupt result handling.
- All sixteen actual cases completed. Verification rehashed eighty binary output
  arrays, compared originals/masks against inputs and recomputed metrics. The
  existing Rust release check passed before and after. Source acceptances and
  the earlier rejection remain unchanged.
- One fresh reviewer reproduced SIGTERM leaving a worker alive and partial
  completion publication on fsync failure. Both are fixed with cleanup on
  termination and atomic completion publication; new workflow fixtures pass.
  A worker-local alarm also bounds execution if its supervisor is forcibly lost.
- Full `bash scripts/check.sh` passed after the fixes, including existing Rust,
  data workflows, numeric jobs and small codec checks. The shared metric extraction
  preserves prior numeric behavior; Rust accepts historical two-file and current
  three-file worker source-hash provenance.
- The model ran before the supervisor fixes. Its exact executed code is archived
  against reported hashes, and retained results are unchanged. No model rerun
  was needed to verify cancellation/publication fixes. Nothing was promoted.

## Scalar fitting and coarse-layout verification

- Separate fitting release `f6cf0af754c65840ea072d06898e92b376cb2e6591fb128ee0cac0d9220c641a`
  preserves the parent evaluation-only release and the six/six geographic split.
  Scope/currentness/corruption/rejection workflow checks pass.
- `data/models/scalar-codec-fit-v1/result.json` has SHA-256
  `528ec0e6cfa2014f3b9103bc764c8795ab8d90defc45ca49db836d49f63c0ba5`.
  Initial/final weights, all 200 scheduled loss entries and 80 scalar/mask arrays
  are retained. Direct scalar Float32 remains exact on every measured land cell.
- All scalar fitting artifact hashes, shapes, original/mask values and physical
  metrics were verified. Before/after Rust source status matches. Peak/time cover
  the worker only; the completed run uses 59,013,048 bytes. No GPU or new download.
- One fresh reviewer found a caller-job replacement race. It was reproduced and
  fixed: execution/verification use archived prechecked bytes and completion IDs
  must agree with initial admission. Caller/archive replacement fixtures pass.
  Actual-run job/source identities were independently confirmed unchanged; exact
  executed code is archived, and no model rerun was needed.
- Full `bash scripts/check.sh` passes after the fix, plus the optional real-input
  completion fixtures. Tiny checks cover loss masking, calibration-only schedule,
  actual optimizer/checkpoint reload and resource/cancellation failures. Existing
  image-codec completion fixtures pass after shared scalar-loader extraction.
- Coarse world v2 contains 64,183 land cells and 1,353 connected water cells.
  Ground below water is deterministic zero; surface levels descend southward.
  Its local edit changes 1,965 land cells, preserving outside/locked bits exactly.
  All 32 artifacts were independently rehashed; all six source regions remain
  current. Overview inspected; four viewpoints exercised with a Node canvas
  fixture. No full browser automation was run.
- World proposal manifest is
  `087193857ae1f3d6f492e2329488dc761db097c5f57faa2d90a0a36fb4635ad9`;
  review HTML is
  `d35253aaf69f84493b5316b363f48c0c70f2e4dc22593e4e280d097b99da858d`.
  The first draft is preserved separately; neither has human acceptance.

## Layout rejection and source-coverage checkpoint

- The exact v2 rejection matches the preserved manifest and review-page hashes.
  Its recipe contains six sources and 121 placements; no source vote changed.
- Prepared manifests confirm twelve 512 m-square patches, six geographic groups,
  three broad classes and one survey collection, totaling 3.145728 km².
- The internal v3 manifest and all 64 retained artifact hashes match. It has no
  human acceptance and is not submitted for another world-quality review.
- All 55 local Markdown links in the changed documents and `git diff --check`
  passed. No production code changed; the full repository suite was not rerun
  for this documentation and local-artifact checkpoint.

## Batch and gallery verification

- Full `bash scripts/check.sh` passed after core fixes, including the new local
  batch/gallery fixtures. The separate HTTPS workflow checks queued metadata and
  asset downloads, independent failure and verified resume without retransfers.
- Actual Chromium checks cover buttons, filters, reload, escaped metadata,
  preview-ready voting and keyboard shortcuts inside the sandboxed preview.
  The real thirteen-tile gallery was inspected without voting. All thirteen
  catalog heads match the captured pre-gallery state.
- One independent review found mutable metadata on failed-source resume and
  repeated raw-raster hashing during votes. Both were reproduced and fixed.
  Metadata pins precede acquisition attempts; verified startup snapshots support
  quick visual decisions while journal integrity, evidence and head checks remain.
  Raw numeric verification still requires intact source bytes.
- A thin-thumbnail case was also fixed: previews fit within 192 × 192 pixels
  without upscaling. A real-page browser check caught clipped thumbnail captions;
  fixed grid rows now retain names and decision labels. Final browser checks,
  build, formatting and Clippy passed after the presentation adjustment.
- No new dependencies, model job, GPU work, source decision or training admission.
  No worktree, merge, push or email. Local rasters and TASKS.md stay outside Git.

**Standing constraints:** public downloads only; never send or draft email, or
ask the owner to contact institutions. Whole-tile yes/no review in batches.
Preserve accepted source decisions while technical work continues. Ask readiness
only for genuinely substantial new workloads under AGENTS.md; Rust builds and
file downloads are exempt. Keep `PLAN.md` and `TASKS.md` outside commits.
