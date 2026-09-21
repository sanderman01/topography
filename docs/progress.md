# Implementation progress

Updated 2026-09-21. The first three real terrain regions are qualified and prepared.
The [master plan](plans/terrain-design-master-plan.md) remains authoritative.

## Resume here

The owner delegated interpolation-quality decisions to the implementer and asked
for simplicity. We adopted [supplier_qa_pilot_v1](design/source-quality-pilot-v1.md):
use the released provider product's QA as the baseline, retain known limitations,
and exclude identified defects. Routine supplier interpolation beneath forest is
acceptable. Do not reopen the confidence question, invent a density cutoff, or
launch more LiDAR checks for the existing candidates.

The three original Río Abajo crops keep their whole-tile yes and now have verified
prepared outputs. The next actual input is **yes/no for the six new tiles** in
[batch 2](reports/pilot-tile-batch-2.md), available locally at
`data/review/pilot-batch-2-2026-09-21/review-batch.html`.
No decisions have been recorded for that batch. Preserve the exact presented
page and its pinned JSON when recording the owner's response.

Continue in the existing checkout on `feat/east-ridges-qualification`; no worktree.
This continuation builds on `6aa6f4f` and uses existing production tools without
code changes. Merging remains with the owner.

## Implemented and verified

| Area | Current behavior |
| --- | --- |
| Contracts | Rust CLI validates world/runtime presets, nine candidate class IDs and source metadata. Vehicle settings and model tolerances remain unset. |
| Acquisition | Bounded HTTPS range downloads, strong ETags, resumable chunks, immutable raw assets, original metadata and local import. Live TNM probes previously returned 504; direct NOAA/USGS assets worked. |
| Native review | Float32/native-mask crops with metric browser previews, multiple hillshades, profiles and low-altitude views. Display products remain separate from elevation. |
| Curation | Immutable SQLite history, captured evidence, explicit whole-tile decisions, stale-write protection, integrity checks and portable exports. Technical judgments belong to the implementer. |
| Quality | `native_quality_v2` checks exact grids, categorical masks, at least 99.5% usable terrestrial coverage and optional annotated-feature support. `supplier_qa_pilot_v1` supplies the declared evidence basis; no validator threshold changed. |
| Preparation | Same-survey native regions preserve valid Float32 bits and separate masks; immutable publication, current-parent checks and catalog registration. Three real regions now prepared. |
| Ground sampling | Three early 64 m samples and three completed full-crop count checks. All native count bins verified against retained XY points. Counts remain diagnostics, not measured feature accuracy. |
| Next review batch | Six native 512 m candidates from three further areas: Camuy, central gorge, southern river/floodplain. Three public DEM assets acquired; decisions unset. |
| Future work | Geographic dataset releases, Python numeric/model evaluation, label editing, generation, world editing and terrain export remain unimplemented. |

## Current Río Abajo state

The catalog has **16 immutable curation records and 3 registered prepared regions**.
Its integrity check reports no unregistered records or unreferenced objects.
The first preparation migrated the database to schema 2; the check report itself
still has JSON schema version 1.

All three current records preserve the owner's existing human acceptance,
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
| New six-tile batch and comparisons | `data/review/pilot-batch-2-2026-09-21/` |
| New batch discovery/source inputs and display overviews | `data/staging/pilot-batch-2-2026-09-21/` |
| Catalog and captured evidence | `data/catalog/topography.sqlite3` and sibling `.curation/` |

Earlier incomplete assessments, source evidence and catalog snapshots remain
historical. Do not use their blocker lists as current status. Current IDs are in
the table above. Original reviewed pages, raw assets and completed evidence were
not overwritten.

The [source qualification report](reports/rio-abajo-source-qualification.md)
retains the measurement results and adopted rule. North/south full-crop jobs ran
sequentially under the owner's explicit readiness: acquisition/counts took about
5.4 minutes each plus verification, maximum child RSS about 72/73 MiB, GPU unused.
Do not repeat them. Full-source COPC bytes were not stored; retained CSVs are
measurement evidence, not ground-quality masks.

## Verification for this continuation

- Existing `quality assess` and `prepare check` pass for all three real inputs.
- `prepare build`, `prepare verify`, curation status/export and `curate check`
  completed; all three outputs are registered, intact and current.
- Independent GDAL decoding compared every prepared elevation bit with its source
  review and checked all seven raster layers, including the original provider mask.
  Captured policy/evidence hashes and input identities were checked.
- `scripts/check_review.py` verifies all 6,291,456 samples in batch 2 against raw
  windows, provider masks and embedded browser payloads, including grid/CRS and
  metrics. Each new crop has 1,048,576 finite provider-valid samples.
- Documentation links, batch links, hashes, candidate metadata and footprint
  consistency are checked before committing. No production code changed, so the
  previously passing `CARGO_BUILD_JOBS=1 bash scripts/check.sh` was not rerun.

Earlier full checks covered rustfmt, locked offline build, Clippy, valid/invalid
contracts, local GDAL workflows, SQLite recovery, mask eligibility and native
preparation failures. Development uses Rust 1.98.1, GDAL 3.13.3 and PDAL 2.10.1;
Rust 1.85 itself was not tested. No Python ML environment or additional system tool
was installed. The model-loader/autoencoder precision milestone is still open.

## Continue after batch selection

1. Record yes/no for numbers 1–6 against the pinned batch identities. Do not ask
   for ridges, hollows, mask drawings or detailed reasons. A no records exclusion,
   without deleting raw data.
2. Qualify selected crops with supplier QA, known metadata, documented water
   classification and dataset-specific permission evidence. Use the existing
   masks/curation/preparation workflow. Further point measurements are optional
   only when they answer a concrete observed problem.
3. Add independent areas as needed. Keep the two nearby crops from each batch-2
   source in one future geographic split group; geographic separation must precede
   overlapping training crops. No split assignment or class-diversity gate is
   complete yet.
4. Implement the pilot dataset release boundary and numeric loader/representation
   checks before choosing or training a model. Keep the 16 GB VRAM ceiling and
   curated-patch/blending fallback.
5. Continue toward the 4,096 m world pilot, then regional editing and export as
   ordered in the master plan. Vehicle dimensions remain explicitly unset.

**Standing constraints:** public downloads only; never send or draft email, or
ask the owner to contact institutions. Whole-tile yes/no review in batches.
Preserve accepted source decisions while technical work continues. Ask readiness
only for genuinely substantial new workloads under AGENTS.md; Rust builds and
file downloads are exempt. Keep `PLAN.md` and `TASKS.md` outside commits.
