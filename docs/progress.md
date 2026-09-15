# Implementation progress

Updated 2026-09-15, after the source-curation and ground-support slice. This
summarizes implemented behavior and observed evidence; the [master plan](plans/terrain-design-master-plan.md)
remains the implementation plan.

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
`a80482b` (karst trial and previews). Rust execution used 1.98.1; the declared
minimum 1.85 was not directly tested. GDAL 3.13.3 and user-installed PDAL 2.10.1
are available. The locked SQLite dependency is bundled; no ML stack was selected. Current-toolchain builds and
Clippy pass; minimum-compiler compatibility remains unverified.

Raw objects and review artifacts live under ignored `data/`; source descriptions,
crop envelopes, reproducible commands and reports are versioned. Local task notes
in `TASKS.md` and `PLAN.md` remain outside commits.

## What the evidence does not establish

- The TNM discovery API returned HTTP 504 in live probes. Its mapping was checked
  with an archived synthetic response; official direct asset delivery worked.
- The karst tile exposes broad hills, hollows and saddles. Nominal 0.5 m spacing
  and provider-valid samples do not prove small-feature fidelity, ground-return
  support or terrestrial quality. Water and repairs are not classified.
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
