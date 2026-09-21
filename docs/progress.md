# Implementation progress

Updated 2026-09-21, after the east-ridges water observation and public-download-only correction. This
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
| Ground support | Three [64 m samples](reports/rio-abajo-ground-support.md) remain verified. The new [east-ridges whole-crop run](reports/rio-abajo-whole-crop-review.md) verifies all 1,048,576 count bins over 512 × 512 m: 700,690 ground returns, 2.673/m², 38 empty 4 m bins. Full-crop terrain quality remains unresolved. |
| Curation | SQLite journal captures evidence, crop-wide labels and explicit actor decisions against pinned review/legend hashes. History, stale-write rejection, recovery, integrity checks and portable metadata exports pass the synthetic CLI workflow. No data admission. |
| Regional quality | Whole-crop water/quality/repair/feature masks are checked for grid, categories, terrestrial coverage and feature gaps. Preparation preflight recomputes evidence and verifies the pinned current decision. Eligibility does not publish or admit prepared data. |
| Regional preparation | [Native preparation](workflows/aligned-preparation.md) composes 1–16 eligible adjacent reviews from one survey, with exact Float32 land values and separate masks. Immutable publication, registration, recovery and revocation pass synthetic workflows. Outputs remain unreviewed and excluded from training. No real source is eligible yet. |
| Whole-crop probe | [Bounded full-review tooling](workflows/whole-crop-ground-support.md) now passes a readiness-approved live run. East-ridges acquisition took 268.05 s, counts 30.29 s, with 79.33 MiB maximum child RSS; verification adds unreported time. No GPU. |
| Permission evidence | The human approved the [dataset-specific permission basis](reports/rio-abajo-permission-review.md). Exact report/response evidence is captured; all three current crop annotations now confirm permission. Terrain acceptance is still absent. |
| Visual suitability | The owner opened the map and said it looks fine, then reported no apparent water bodies. [Qualification follow-up](reports/rio-abajo-source-qualification.md) records these observations and the sampling-distance diagnostic. Only publicly downloadable source data may be used; no emails or email drafts. |
| Real terrain | A USGS coastal tile and a native 0.5 m USGS/NOAA tile near Río Abajo are stored locally. The [karst trial](reports/rio-abajo-karst-trial.md) provides three 512 × 512 m crops and optional display contours. |
| Verification | Single-job Rust build, Clippy with warnings denied, contract workflows, synthetic GDAL workflows and acquisition recovery checks passed. All 3,145,728 karst crop samples and mask cells match their source windows and browser payloads. Headless Firefox rendered repeatable camera views. |

Implementation commits: `88242d4` (contracts), `b28de88` (acquisition),
`a80482b` (karst trial and previews), `3619c87` (curation and ground-support
evidence), `faa58c7` (regional quality and preparation eligibility), `2bc1302`
(native preparation), and `1ccff40` (bounded whole-crop probe). The initial
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

After the approved live run, the full repository check passed again without
production code changes. All new measurement and diagnostic artifact hashes were
verified, the spatial map was rendered and inspected, and current catalog status,
portable exports and blocked eligibility checks were verified for all three crops.
The measurement independently counted retained XY points and compared every
native raster bin. The diagnostic only decodes that small completed count raster;
it does not perform another point-cloud run.

The real catalog contains **seven immutable records and three current assistant
annotations**, each with candidate class ID 1, confirmed permission, unknown
whole-crop quality findings and unset terrestrial coverage. New records capture
the user's actual permission approval without declaring human terrain acceptance.
East-ridges also captures the full ground-support report, spatial diagnostic,
the owner's positive visual response and the later ground-bin distance report.
No prepared/training admission was recorded. The last `curate check` found seven
indexed records, zero prepared regions, no unregistered records and no unreferenced
objects. The real catalog remains database schema 1;
only a successful prepared registration would migrate it to schema 2.

Retained local inputs and outputs (ignored by Git; preserve and back up separately):

| Artifact | Location |
| --- | --- |
| Karst raw receipt | `data/raw/records/68ffca798986f0a68154b1bd7bd1a66ecbe1e089d3abeb63140d5cc95ea8795d.json` |
| Final native crop previews and comparison | `data/review/rio-abajo-pilot/` — `north-hollows`, `east-ridges`, `south-saddles`, and `comparison.json` |
| Completed point samples | `data/review/rio-abajo-ground-support/` — `north-hollows-v3`, `east-ridges`, `south-saddles`; each has `report.json` |
| Completed whole-crop measurement | `data/review/rio-abajo-ground-support/east-ridges-full-v1/` — points, counts, pipelines and verified `report.json` |
| Whole-crop spatial diagnostic and review map | `data/review/rio-abajo-ground-support/east-ridges-full-review-v1/` — `review.html`, `spatial-review.json`, SVG and local reproduction script |
| Curation drafts, record IDs, original OPR XML and portable exports | `data/review/rio-abajo-curation/`; record IDs are in `records.json` |
| Permission annotations, IDs, exports, blocked eligibility results and pre-change SQLite backup | `data/review/rio-abajo-curation-2026-09-21/`; north/south heads remain current, east-ridges is superseded below |
| Current east-ridges visual annotation, source evidence and sampling-distance diagnostic | `data/review/rio-abajo-east-ridges-qualification-v1/`; `record.json` holds the latest ID |
| Catalog and immutable curation objects | `data/catalog/topography.sqlite3` and `data/catalog/topography.sqlite3.curation/` |
| Incomplete regional quality drafts, assessments and eligibility reports | `data/review/rio-abajo-quality-preflight-2026-09-21/{north-hollows,east-ridges,south-saddles}/` |
| Whole-crop workload plan and source HEAD identity | `data/review/rio-abajo-ground-support-plans-2026-09-21/` |
| Original tile/NOAA permission metadata, reviewed report and human decision evidence | `data/review/rio-abajo-permission-2026-09-21/` |

Earlier incomplete point-sample directories remain preserved; use the completed
directories listed above. No full 932 MB source-cloud file was stored or hashed;
the completed range-read measurement retains a 1,175,307,805-byte point CSV.
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
incomplete assessments with all masks explicitly null. New eligibility checks
against current records are retained in the continuation directory: each remains
blocked with 17 findings, and the permission blocker is gone. Earlier records,
reports and exports were preserved. No quality masks or prepared rasters were
created from the ground-return measurements.

Continuation scope and next work:

The user answered “1. Ready. 2. Looks good.” to the specific workload and permission
questions. Both are handled. The user subsequently opened the review and said
“I've opened it. It looks fine. Now what?” Overall visual suitability is positive;
do not ask for the same approval again or require grid-square selection just to
continue. East-ridges remains the first candidate. Other substantial point-cloud
runs need their own readiness confirmation; none is currently launched.

The owner subsequently reported no apparent water bodies in the reviewed crop.
The red boxes mark empty ground-return bins, not water. Use no apparent water as
the working interpretation for a proposed all-terrestrial water mask, recording
its visual basis and limitations; no verified water classification has yet been
published. This observation is recorded in the
[qualification report](reports/rio-abajo-source-qualification.md), without changing
the seven existing catalog records.

**Use only publicly downloadable source data. Never send emails or prepare email
drafts, and do not ask the owner to contact people or institutions for data.**
The earlier contact-request draft was removed; no email was sent. The retained
`source-evidence.json` contact proposal is historical and superseded. Original
survey QA/water vectors are optional supporting evidence, not mandatory inputs.
No owner action to obtain them is needed.

The next technical work is to finish a concrete water/quality/repair/feature
classification proposal from the existing public DEM, survey report, measured
ground returns and visual review. Additional public downloads should address a
specific remaining uncertainty. Document unsupported conclusions instead of
inventing thresholds or making unavailable survey files a prerequisite.
A source-wide QA report alone does not establish each cell's quality.

The new small-raster diagnostic measures crop-limited distance to occupied
ground-bin centres: median 0.5 m, p95 1.118 m, p99 1.803 m, maximum 5.5 m in E6.
It does not set an acceptance radius or measure terrain height error. It uses
existing counts, without another point-cloud job. The new east-ridges annotation
and eligibility report are under `rio-abajo-east-ridges-qualification-v1`; the
crop still has 17 technical/admission blockers.

Construct explicit water/quality/repair/feature masks from justified evidence,
then record actual human
source/crop decisions. Occupancy is not a quality mask. Then assess, capture the
exact reports and use the implemented native preparation path. Eligibility is
recomputed at publication; a saved check is not a permit for a later run. Current
processing supports unrepaired native 0.5–1 m crops and matching declared horizontal
EPSG identities. Cross-survey alignment, repair policy, other CRS declarations and
secondary-resolution exceptions remain unsupported.

Resolve full-crop terrain support, defining-feature gaps, water/repair evidence
and actual human crop decisions before admitting real sources. Keep the vehicle
envelope explicitly unset. Use the LiDAR header's
horizontal CRS; do not compare point-cloud and DEM heights until their differing
GEOID18/GEOID12B processing is resolved.

## What the evidence does not establish

- The TNM discovery API returned HTTP 504 in live probes. Its mapping was checked
  with an archived synthetic response; official direct asset delivery worked.
- The karst tile exposes broad hills, hollows and saddles. Nominal 0.5 m spacing
  and provider-valid samples do not prove small-feature fidelity or terrestrial
  quality. East-ridges now has whole-crop return counts; north-hollows and
  south-saddles still have only 1.5625% samples. Counts include overlap returns
  and do not establish terrestrial quality, feature fidelity or water/repair masks.
- Dataset-specific permission is confirmed under the owner's recorded decision.
  Human source/crop acceptance remains unresolved, and these sources have not
  entered prepared or training pools. No new agency correspondence is claimed.
- The byte-preserving extraction/browser checks are not the planned numeric
  model-loader/autoencoder round-trip evaluation.

## Remaining implementation order

1. Resolve whole-crop support, defining features, mask classifications and human
   decisions for the pilot using the implemented assessment/eligibility workflow.
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
