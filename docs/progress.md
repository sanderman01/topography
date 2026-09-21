# Implementation progress

Updated 2026-09-21, after both approved sequential LiDAR checks completed. This
summarizes implemented behavior and observed evidence; the [master plan](plans/terrain-design-master-plan.md)
remains the implementation plan.

Continuation first resumed from `18745bc` on `main`. Regional quality was committed
as `faa58c7`, with checkpoint `23ad4c9`. The earlier continuation used
`feat/aligned-regional-preparation`, based on that checkpoint, in the same checkout
without a worktree. Native preparation is committed as `2bc1302`; bounded
whole-crop probe tooling is `1ccff40`, with checkpoint `dcdf15e`. The approved
real-data continuation was committed as `027cf36` on `feat/rio-abajo-full-crop-review`.
The visual/qualification continuation uses `feat/east-ridges-qualification` from
that commit, still in the same checkout. Merging remains with the user. Resume from the
checkpoint below rather than repeating acquisition or completed trials.

## Completed and verified

| Area | Current result |
| --- | --- |
| Contracts | Rust CLI validates draft world/runtime presets, nine stable terrain-class IDs and source metadata. Vehicle dimensions and measured pilot tolerances remain unset. |
| Raw acquisition | Bounded HTTPS acquisition with strong ETags, checked byte ranges, resumable completed chunks, immutable SHA-256 objects and provenance receipts. Manual Float32 GeoTIFF import and raw integrity verification use the same store. |
| Native review | Small GDAL crops retain numeric values, coordinates and provider masks. Browser previews add three light directions, row/column profiles, low-altitude/fixed-height views and saved camera settings. No preview creates acceptance. |
| Ground support | Three [64 m samples](reports/rio-abajo-ground-support.md) remain verified. All three [whole-crop runs](reports/rio-abajo-source-qualification.md) verify every native count bin over 512 × 512 m. North/east/south have 742,843 / 700,690 / 601,406 ground returns. Counts do not establish terrain quality. |
| Curation | SQLite journal captures evidence, crop-wide labels and explicit actor decisions against pinned review/legend hashes. History, stale-write rejection, recovery, integrity checks and portable metadata exports pass the synthetic CLI workflow. No data admission. |
| Regional quality | Whole-crop masks are checked for grid, categories and terrestrial coverage. Policy `native_quality_v2` permits an all-zero feature layer; optional annotated features still require complete support. Preparation preflight recomputes evidence and verifies the pinned current decision. |
| Regional preparation | [Native preparation](workflows/aligned-preparation.md) composes 1–16 eligible adjacent reviews from one survey, with exact Float32 land values and separate masks. Immutable publication, registration, recovery and revocation pass synthetic workflows. Outputs remain unreviewed and excluded from training. No real source is eligible yet. |
| Whole-crop probe | [Bounded full-review tooling](workflows/whole-crop-ground-support.md) completed all three readiness-approved live runs. North/south ran sequentially, each taking about 5.4 minutes for acquisition/counts plus unreported verification time, with about 72/73 MiB maximum child RSS. No GPU. |
| Permission evidence | The human approved the [dataset-specific permission basis](reports/rio-abajo-permission-review.md). Exact report/response evidence is captured; all three current crop records confirm permission. Technical admission remains unresolved. |
| Human tile selection | All three tiles are selected. East-ridges retains its earlier yes; the owner's “both yes” accepts north-hollows and south-saddles from the presented batch. No individual feature selection or mask authoring is required from the owner. |
| Real terrain | A USGS coastal tile and a native 0.5 m USGS/NOAA tile near Río Abajo are stored locally. The [karst trial](reports/rio-abajo-karst-trial.md) provides three 512 × 512 m crops and optional display contours. |
| Verification | Single-job Rust build, Clippy with warnings denied, contract workflows, synthetic GDAL workflows and acquisition recovery checks passed. All 3,145,728 karst crop samples and mask cells match their source windows and browser payloads. Headless Firefox rendered repeatable camera views. |

Implementation commits: `88242d4` (contracts), `b28de88` (acquisition),
`a80482b` (karst trial and previews), `3619c87` (curation and ground-support
evidence), `faa58c7` (regional quality and preparation eligibility), `2bc1302`
(native preparation), `1ccff40` (bounded whole-crop probe), and `dda43e0`
(optional feature annotations and whole-tile review). The initial
progress summary was committed as `66500a6`.
Rust execution used 1.98.1; the declared
minimum 1.85 was not directly tested. GDAL 3.13.3 and user-installed PDAL 2.10.1
are available. The locked SQLite dependency is bundled; no ML stack was selected.
The user chose to keep using the installed Rust toolchain. No additional
installation is needed for the completed workflow; minimum-compiler compatibility
remains unverified.

Raw objects and review artifacts live under ignored `data/`; source descriptions,
crop envelopes, reproducible commands and reports are versioned. Local task notes
in `TASKS.md` and `PLAN.md` remain outside commits.

## Checkpoint for the next session

The 2026-09-21 verification passed
`CARGO_BUILD_JOBS=1 bash scripts/check.sh`, including formatting, build, Clippy,
contract inputs, local GDAL acquisition/review, SQLite curation, regional mask
and eligibility checks, native preparation and tiny PDAL checks. New preparation
cases cover exact land bits (including signed zero), water/nonfinite fill, masks,
grid alignment, holes/overlaps, corrupt output, orphan registration and later
source rejection. Independent review found an alignment failure at 0.51 m native
spacing. A failing CLI reproduction now passes by reconstructing the exact source
origin from integer offsets; no alignment tolerance was introduced. Tiny probe
checks cover rectangular aggregation, streamed limits, timeout termination,
pipeline reuse and a CLI dry plan. Eligible preparation fixtures use artificial
metadata and human declarations in isolated temporary catalogs; no real source
was admitted. The earlier declared-CRS regression remains covered.
All three real point/count reports were rechecked before these two tooling slices.
The full loopback HTTPS acquisition checks and Firefox camera checks passed in
earlier implementation slices; they were not rerun for this slice.

After the whole-tile workflow correction,
`CARGO_BUILD_JOBS=1 bash scripts/check.sh` passed again. The quality workflow now
exercises an unannotated tile through assessment and eligibility, and still
rejects insufficient coverage and unsupported annotated features. The changed
case failed on the old mandatory-feature gate before the fix. Independent review
found no actionable issues. That earlier catalog check reported eight records, zero
prepared regions and no unregistered records or unreferenced objects. Its east-ridges eligibility report had 16 technical/evidence blockers; human
acceptance is no longer a blocker. Batch-page links and artifact hashes were checked.

After the earlier east-ridges live run, the full repository check passed again without
production code changes. All new measurement and diagnostic artifact hashes were
verified, the spatial map was rendered and inspected, and current catalog status,
portable exports and blocked eligibility checks were verified for all three crops.
The measurement independently counted retained XY points and compared every
native raster bin. The diagnostic only decodes that small completed count raster;
it does not perform another point-cloud run.

The real catalog contains **thirteen immutable records and three current human
whole-tile acceptances**, transcribed by Codex with the exact responses captured.
The owner's “both yes” accepts north-hollows and south-saddles from the batch;
east-ridges retains its earlier acceptance.
All three retain candidate class ID 1, confirmed permission and unset terrestrial
coverage. Codex has recorded a documented all-terrestrial water classification;
the other five technical findings remain unknown. The east-ridges yes is selection, not a
claim that technical checks passed. Its current record is
`4079d784b9801f1ffb0baab4661542c248a187aa7b119365796f1a1e37ed468d`.
North-hollows is `d01727f32157fe6c44c527aa0a1aaad4f6fe18fdcca7d938cfc251311a3a443a`;
south-saddles is `6be376955c685d08a50996ec0167171ea199755cf897e65f2057d657334933a4`.
No prepared/training admission was recorded. The real catalog remains database schema 1;
only a successful prepared registration would migrate it to schema 2.

Retained local inputs and outputs (ignored by Git; preserve and back up separately):

| Artifact | Location |
| --- | --- |
| Karst raw receipt | `data/raw/records/68ffca798986f0a68154b1bd7bd1a66ecbe1e089d3abeb63140d5cc95ea8795d.json` |
| Final native crop previews and comparison | `data/review/rio-abajo-pilot/` — `north-hollows`, `east-ridges`, `south-saddles`, and `comparison.json` |
| Completed point samples | `data/review/rio-abajo-ground-support/` — `north-hollows-v3`, `east-ridges`, `south-saddles`; each has `report.json` |
| Completed whole-crop measurements | `data/review/rio-abajo-ground-support/{north-hollows,east-ridges,south-saddles}-full-v1/` — points, counts, pipelines and verified `report.json` |
| Whole-crop spatial diagnostic and review map | `data/review/rio-abajo-ground-support/east-ridges-full-review-v1/` — `review.html`, `spatial-review.json`, SVG and local reproduction script |
| Curation drafts, record IDs, original OPR XML and portable exports | `data/review/rio-abajo-curation/`; record IDs are in `records.json` |
| Earlier permission annotations, IDs, exports, blocked eligibility results and pre-change SQLite backup | `data/review/rio-abajo-curation-2026-09-21/`; all three heads are superseded by whole-tile decisions |
| Earlier east-ridges visual annotation, source evidence and sampling-distance diagnostic | `data/review/rio-abajo-east-ridges-qualification-v1/`; its current-head snapshot is superseded |
| East-ridges acceptance and the completed two-tile review batch | `data/review/rio-abajo-tile-decisions-2026-09-21/`; `record.json` holds the east-ridges ID; the reviewed `review-batch.html` and JSON remain unchanged |
| North/south acceptances, captured batch response, backup, exports, eligibility and planned measurements | `data/review/rio-abajo-tile-decisions-2026-09-21/accepted-batch-v1/`; `records.json` holds both new IDs, `workload-batch.json` holds sequential run arguments |
| Measurement completion, current decisions, backup, exports and eligibility | `data/review/rio-abajo-full-crop-completion-2026-09-21/`; `records.json` holds current IDs and `measurements.json` pins the approved runs |
| Latest mask drafts, assessments and context evidence | `data/review/rio-abajo-quality-v2-2026-09-21/`; each crop's `water-classified-v1/` holds its latest quality input and assessment |
| Catalog and immutable curation objects | `data/catalog/topography.sqlite3` and `data/catalog/topography.sqlite3.curation/` |
| Incomplete regional quality drafts, assessments and eligibility reports | `data/review/rio-abajo-quality-preflight-2026-09-21/{north-hollows,east-ridges,south-saddles}/` |
| Whole-crop workload plan and source HEAD identity | `data/review/rio-abajo-ground-support-plans-2026-09-21/` |
| Original tile/NOAA permission metadata, reviewed report and human decision evidence | `data/review/rio-abajo-permission-2026-09-21/` |

Earlier incomplete point-sample directories remain preserved; use the completed
directories listed above. No full 932 MB source-cloud file was stored or hashed;
east-ridges retains a 1,175,307,805-byte point CSV, and the new north/south CSVs
total 2,378,351,624 bytes (about 2.22 GiB).
Transferred network bytes are unknown.

On resuming, inspect Git status and read the
[whole-crop findings](reports/rio-abajo-whole-crop-review.md),
[whole-crop workflow](workflows/whole-crop-ground-support.md) and
[permission evidence](reports/rio-abajo-permission-review.md). The existing local catalog can
be checked without creating new decisions:

```sh
target/debug/topography curate check configs/runtime.example.toml
```

The [regional quality workflow](workflows/regional-quality.md) now provides
`quality draft`, `quality assess` and `prepare check`. The three real crops have
latest assessments with all-terrestrial water masks, zero optional feature
annotations and zero project repairs. Exact grids, categories and decoded mask
values were checked. Ground quality alone is unset; assessment counts stay null.
Fresh eligibility checks against all three current heads have nine remaining
technical/evidence blockers and no human-acceptance blocker. Earlier records,
reports and exports were preserved. No quality masks or prepared rasters were
inferred from the ground-return measurements.

Continuation scope and next work:

The user answered “1. Ready. 2. Looks good.” to the specific workload and permission
questions. Both are handled. The user subsequently opened the review and said
“I've opened it. It looks fine. Now what?” Overall visual suitability is positive;
do not ask for the same approval again or require grid-square selection just to
continue. East-ridges remains the first candidate. Other substantial point-cloud
runs need their own readiness confirmation; none is currently launched.

The owner subsequently reported no apparent water bodies in the reviewed crop.
The red boxes mark empty ground-return bins, not water. Use no apparent water as
the basis, together with the inspected native terrain and public USGS context
images, for the all-terrestrial masks now published. The implementer recorded
the classification and canopy limitations; it excludes no land from the quality
denominator and is not proof of absolute absence of concealed water. This observation is recorded in the
[qualification report](reports/rio-abajo-source-qualification.md), without changing
the earlier immutable catalog records. The initial whole-tile decision captured
that observation without a technical classification; the latest evidence update
records the implementer's classification separately.

**Use only publicly downloadable source data. Never send emails or prepare email
drafts, and do not ask the owner to contact people or institutions for data.**
The earlier contact-request draft was removed; no email was sent. The retained
`source-evidence.json` contact proposal is historical and superseded. Original
survey QA/water vectors are optional supporting evidence, not mandatory inputs.
No owner action to obtain them is needed.

**Human review is whole-tile yes/no, in batches.** The owner explicitly rejected
selecting individual ridges, hollows or other features. All three tiles now have
their yes; do not ask again. The proposed detailed feature review was cancelled
before any masks or detailed feature-review page were created. The completed batch is
`data/review/rio-abajo-tile-decisions-2026-09-21/review-batch.html`, linking the existing
`data/review/rio-abajo-pilot/north-hollows/preview.html` and
`data/review/rio-abajo-pilot/south-saddles/preview.html`. The response “both yes”
is captured against the batch, page and crop hashes in `accepted-batch-v1`.
These crops share a survey and do not establish independent regional diversity.

Both approved whole-crop support measurements are complete. The owner confirmed
“Ready—run both sequentially” against the exact plans in `accepted-batch-v1`;
its `readiness.json` captures the response and batch hash. The full embedded plans
match the approved plans. North/south acquisition took 290.62/290.82 seconds,
counts 32.67/29.19 seconds, plus unreported verification time. Maximum child RSS
was 71.99/73.40 MiB; parent/cache and total RAM were not measured. Each stayed
within the 20-million-point, 2-GiB-CSV and 30-minute processing limits. No GPU.
Both reports verify all 1,048,576 native bins against retained XY points.
Artifact sizes and non-CSV hashes were rechecked; the probe already hashed the
CSVs. Current catalog integrity, status, portable exports and blocked eligibility
were checked after attaching the evidence. No production code changed in this
continuation, so the previously passing full development suite was not rerun.
Do not repeat any completed point-cloud job.

**Next input is one project policy decision**, described concretely in the
[qualification report](reports/rio-abajo-source-qualification.md#decision-needed-before-source-admission).
The recommendation is to use published supplier QA as the pilot's evidence basis
for usable native ground, with known defects excluded and interpolation uncertainty
explicitly retained. This has not been approved or implemented. The current
99.5% threshold does not define how to certify supplier-interpolated ground;
more count diagnostics cannot settle that criterion. The alternative needs an
explicit feature scale/error target before a bounded validation experiment.
This is not a request to identify terrain features or approve the same tiles again.

The implementer handles technical evidence and masks separately. Do not turn
unresolved technical findings into a questionnaire or automatic pass. Optional
feature annotations may be absent (an all-zero feature layer); the updated
policy retains the 99.5% terrestrial-coverage and other technical gates. Refresh
old assessment evidence under v2 before preparation, preserving the existing
human decision for unchanged terrain. Additional public downloads must address
a concrete technical question; unavailable files and correspondence are not prerequisites.

The new small-raster diagnostic measures crop-limited distance to occupied
ground-bin centres: median 0.5 m, p95 1.118 m, p99 1.803 m, maximum 5.5 m in E6.
It does not set an acceptance radius or measure terrain height error. It uses
existing counts, without another point-cloud job. The earlier east-ridges annotation
and historical eligibility report are under `rio-abajo-east-ridges-qualification-v1`.
Current acceptance and eligibility are under `rio-abajo-full-crop-completion-2026-09-21`.

Construct masks from justified evidence as implementation work, using an empty
feature layer unless the owner requests annotations. Occupancy is not a quality
mask. Human tile selection may precede these checks. Then assess, capture the
exact reports and use the implemented native preparation path. Eligibility is
recomputed at publication; a saved check is not a permit for a later run. Current
processing supports unrepaired native 0.5–1 m crops and matching declared horizontal
EPSG identities. Cross-survey alignment, repair policy, other CRS declarations and
secondary-resolution exceptions remain unsupported.

Resolve full-crop terrain support, any optional annotated-feature gaps and
water/repair evidence before admitting selected sources. Keep the vehicle
envelope explicitly unset. Use the LiDAR header's
horizontal CRS; do not compare point-cloud and DEM heights until their differing
GEOID18/GEOID12B processing is resolved.

## What the evidence does not establish

- The TNM discovery API returned HTTP 504 in live probes. Its mapping was checked
  with an archived synthetic response; official direct asset delivery worked.
- The karst tile exposes broad hills, hollows and saddles. Nominal 0.5 m spacing
  and provider-valid samples do not prove small-feature fidelity or terrestrial
  quality. All three now have whole-crop return counts. Counts include overlap
  returns and do not establish terrestrial quality, feature fidelity or water/repair
  masks. The separate water classification is a documented context judgment.
- Dataset-specific permission is confirmed under the owner's recorded decision.
  All three crops have human whole-tile acceptance. These sources have not
  entered prepared or training pools. No new agency correspondence is claimed.
- The byte-preserving extraction/browser checks are not the planned numeric
  model-loader/autoencoder round-trip evaluation.

## Remaining implementation order

1. Resolve the proposed source-support evidence policy, then continue collecting
   whole-tile yes/no decisions in batches. Handle technical support and
   masks separately using the assessment/eligibility workflow; do not require
   feature selection or repeat approval of unchanged terrain.
2. Apply the verified native preparation path to eligible real regions and review
   the resulting artifacts. Extend alignment/repair policies only when concrete
   sources require them; preserve accepted revisions and raw sources.
3. Curate several independent karst, gorge and smoother river/coastal regions;
   establish geographic splits before overlapping crops and release a pilot dataset.
4. Measure numeric loader/representation errors before selecting or training a
   model. Keep the 16 GB VRAM ceiling and real-patch blending fallback.
5. Build the 4,096 m pilot, including biome boundaries, connected water, a protected
   landmark and a demonstrably local rerun. Then expand and complete export/recovery.

Prepared-output acceptance, dataset releases, model jobs, world composition,
per-cell label editing and terrain export are not yet implemented at this
snapshot. Human decisions, vehicle settings and measured tolerances must remain
explicit inputs rather than values invented by successful processing.
