# Implementation progress

Updated 2026-09-15, after the source-curation and ground-support slice. This
summarizes implemented behavior and observed evidence; the [master plan](plans/terrain-design-master-plan.md)
remains the implementation plan.

**Paused at the user's request.** The latest implementation is committed as
`3619c87` on `feat/source-curation`; merging remains with the user. Resume from
the checkpoint below rather than repeating acquisition or the completed trials.

## Completed and verified

| Area | Current result |
| --- | --- |
| Contracts | Rust CLI validates draft world/runtime presets, nine stable terrain-class IDs and source metadata. Vehicle dimensions and measured pilot tolerances remain unset. |
| Raw acquisition | Bounded HTTPS acquisition with strong ETags, checked byte ranges, resumable completed chunks, immutable SHA-256 objects and provenance receipts. Manual Float32 GeoTIFF import and raw integrity verification use the same store. |
| Native review | Small GDAL crops retain numeric values, coordinates and provider masks. Browser previews add three light directions, row/column profiles, low-altitude/fixed-height views and saved camera settings. No preview creates acceptance. |
| Ground support | [PDAL samples](reports/rio-abajo-ground-support.md) cover one 64 m centre square per karst crop. All 49,152 count bins match retained XY points. Measured ground-return density is 2.66–4.01/m²; full-crop quality remains unresolved. |
| Curation | SQLite journal captures evidence, crop-wide labels and explicit actor decisions against pinned review/legend hashes. History, stale-write rejection, recovery, integrity checks and portable metadata exports pass the synthetic CLI workflow. No data admission. |
| Real terrain | A USGS coastal tile and a native 0.5 m USGS/NOAA tile near Río Abajo are stored locally. The [karst trial](reports/rio-abajo-karst-trial.md) provides three 512 × 512 m crops and optional display contours. |
| Verification | Single-job Rust build, Clippy with warnings denied, contract workflows, synthetic GDAL workflows and acquisition recovery checks passed. All 3,145,728 karst crop samples and mask cells match their source windows and browser payloads. Headless Firefox rendered repeatable camera views. |

Implementation commits: `88242d4` (contracts), `b28de88` (acquisition),
`a80482b` (karst trial and previews), `3619c87` (curation and ground-support
evidence). The initial progress summary was committed as `66500a6`.
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

The last verification of `3619c87` passed
`CARGO_BUILD_JOBS=1 bash scripts/check.sh`, including formatting, build, Clippy,
contract inputs, local GDAL acquisition/review, SQLite curation and tiny PDAL
count/flag checks. All three real point/count reports were also rechecked.
The full loopback HTTPS acquisition checks and Firefox camera checks passed in
earlier implementation slices; they were not rerun for curation.

The real catalog contains **three assistant annotations**, each with candidate
class ID 1, unknown whole-crop quality findings and unset terrestrial coverage.
No human acceptance or prepared/training admission was recorded. The last
`curate check` found no unregistered records or unreferenced objects.

Retained local inputs and outputs (ignored by Git; preserve and back up separately):

| Artifact | Location |
| --- | --- |
| Karst raw receipt | `data/raw/records/68ffca798986f0a68154b1bd7bd1a66ecbe1e089d3abeb63140d5cc95ea8795d.json` |
| Final native crop previews and comparison | `data/review/rio-abajo-pilot/` — `north-hollows`, `east-ridges`, `south-saddles`, and `comparison.json` |
| Completed point samples | `data/review/rio-abajo-ground-support/` — `north-hollows-v3`, `east-ridges`, `south-saddles`; each has `report.json` |
| Curation drafts, record IDs, original OPR XML and portable exports | `data/review/rio-abajo-curation/`; record IDs are in `records.json` |
| Catalog and immutable curation objects | `data/catalog/topography.sqlite3` and `data/catalog/topography.sqlite3.curation/` |

Earlier incomplete point-sample directories remain preserved; use the completed
directories listed above. The full 932 MB point-cloud tile was not downloaded.

On resuming, inspect Git status and read the
[ground-support findings](reports/rio-abajo-ground-support.md) and
[curation workflow](workflows/source-curation.md). The existing local catalog can
be checked without creating new decisions:

```sh
target/debug/topography curate check configs/runtime.example.toml
```

The next implementation slice is regional quality checks and the admission
boundary for aligned preparation. Use the existing evidence/decision journal.
Before expanding point-cloud processing, define the whole-crop footprint and
resource bounds, then obtain readiness for that new substantial workload. Prior
PDAL readiness covered the three small samples and their rechecks. Rust builds
and file downloads remain exempt under [AGENTS.md](../AGENTS.md).

Resolve full-crop ground support, defining-feature gaps, water/repair evidence,
dataset-specific training permission and actual human decisions before admitting
real sources. Keep the vehicle envelope explicitly unset. Use the LiDAR header's
horizontal CRS; do not compare point-cloud and DEM heights until their differing
GEOID18/GEOID12B processing is resolved. No further implementation is underway
during this pause.

## What the evidence does not establish

- The TNM discovery API returned HTTP 504 in live probes. Its mapping was checked
  with an archived synthetic response; official direct asset delivery worked.
- The karst tile exposes broad hills, hollows and saddles. Nominal 0.5 m spacing
  and provider-valid samples do not prove small-feature fidelity or terrestrial
  quality. Ground-return measurements cover only 1.5625% of each crop and include
  overlap returns; they do not establish whole-crop support. Water and repairs
  are not classified.
- Dataset-specific training permission and human source/crop acceptance remain
  unresolved. These raw sources have not entered prepared or training pools.
- The byte-preserving extraction/browser checks are not the planned numeric
  model-loader/autoencoder round-trip evaluation.

## Remaining implementation order

1. Complete regional source-quality checks and preparation admission; durable
   evidence, crop-wide labels and explicit decisions now have a CLI consumer. Resolve
   the real pilot's permission and ground-support evidence alongside that work.
2. Prepare aligned, traceable regional rasters with distinct validity, quality,
   water and repair information. Preserve accepted revisions and raw sources.
3. Curate several independent karst, gorge and smoother river/coastal regions;
   establish geographic splits before overlapping crops and release a pilot dataset.
4. Measure numeric loader/representation errors before selecting or training a
   model. Keep the 16 GB VRAM ceiling and real-patch blending fallback.
5. Build the 4,096 m pilot, including biome boundaries, connected water, a protected
   landmark and a demonstrably local rerun. Then expand and complete export/recovery.

Regional preparation, dataset releases, model jobs, world composition, per-cell
label editing and terrain export are not yet implemented at this
snapshot. Human decisions, vehicle settings and measured tolerances must remain
explicit inputs rather than values invented by successful processing.
