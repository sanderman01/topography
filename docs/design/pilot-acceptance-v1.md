# Pilot technical checks and terrain review, version 1

These are technical requirements from the [master plan](../plans/terrain-design-master-plan.md),
sections 1, 2, 4–7 and 10. Configuration validation, raw import/integrity checks
and native-grid review crops are implemented. Curation records explicit human
decisions; regional mask checks and preparation eligibility are implemented.
Aligned native prepared-data publication, scoped evaluation/fitting releases and
bounded model evaluation are implemented. Output acceptance and production model
training remain pending.

## Exact contracts

- Prepared/generated elevations remain Float32 metres with explicit validity,
  quality and water masks and complete coordinate metadata.
- Water surfaces are authored separately. Underwater cells contain deterministic
  masked fill, excluded from terrain losses, controls and validation.
- Training, generation windows and export tiles are independent. Full export
  tiles span 1,024 intervals with 1,025 shared-edge vertices per side. Shared
  encoded border values must match exactly.
- Source identity, raw checksums, original metadata, transformations and review
  decisions stay traceable. Raw bytes and accepted outputs are preserved.
- Geographic splits precede overlapping crops. Reduced-fidelity 2 m sources may
  supervise only 4 m and 16 m levels, never measured 1 m targets.
- Values outside edit domains and inside locks remain unchanged exactly.
  Successful processing cannot create human acceptance.
- Peak GPU memory stays within the configured budget, at most 16 GB.

## Initial source gates

| Gate | Initial requirement | Evidence needed |
| --- | --- | --- |
| Training permission | Confirmed per dataset before prepared/training admission; prohibited, restricted, ambiguous or unknown permission excludes it | Specific clause or confirmation, license reference, attribution and reuse obligations |
| Source meaning | Numeric bare-earth DEM or supported classified ground returns | Surface type, survey provenance, horizontal CRS, vertical datum and units |
| Resolution | Prefer native 0.5–1 m; only explicitly reviewed 2 m class exceptions | Actual spacing, measurement support and preserved defining features; no upsampling claims |
| Terrestrial coverage | At least 99.5% valid terrestrial cells | Denominator excludes water; no unsupported gaps through defining landmarks or passages |
| Repairs/artifacts | Small reviewed repairs only; exclude unsupported large gaps and unresolved artifacts | Repair mask, before/after inspection, survey alignment and artifact findings |
| Feature fidelity | Features survive the heightfield and survey sampling | Cross-sections, multiple hillshades, point-cloud/reference comparison and oblique review |

The [preparation eligibility check](../workflows/regional-quality.md) enforces the
initial native-resolution gates against measured masks and current decisions.
Aligned preparation rechecks eligibility when building and registering. A source record
can be well-formed while excluded; `validate source` reports permission declarations
and unknown metadata without inspecting evidence or admitting data. Discovery
bounds do not prove exact coverage. Metadata strings do not prove accuracy.

## Human review

Present source tiles in batches and record a whole-tile Yes/No decision. Overhead,
oblique and fixed-altitude views are optional aids. The owner judges terrain
suitability without defining vehicle dimensions, movement limits, clearance
requirements or formal gameplay thresholds. These are outside the plan, rather
than unresolved pilot settings. Existing camera-height controls are viewing aids.

Keep separate records for processing completion, numeric checks and human
accept/reject decisions, each tied to an exact artifact/revision. A rejection
records exclusion; a reason is optional, and source files are preserved.

## Decisions from pilot evidence

| Decision | State and how to resolve it |
| --- | --- |
| Class precedence/minimum areas | Unset; inspect overlaps and layout, then version the legend and ownership policy |
| Final vocabulary/2 m exceptions | Nine candidates; review acquisitions, retain stable IDs and record retired IDs without reuse |
| Loader/codec height, slope and feature thresholds | Unset; measure ramps, flats, ridges and held-out real sources in metres before model work |
| Height encoding offset/range | Unset; choose one world-wide physical range, record clipping and keep all 65,536 codes usable |
| Inference overlap/large training crops | Draft settings; benchmark seams, memory and runtime |
| Model feasibility attempt budget | Unset; agree bounded experiments before expensive work, retaining real-patch blending as fallback |

No-clipping export error must stay within half the encoding step plus a declared
arithmetic tolerance. Numeric checks, source qualification and artifact integrity
remain technical responsibilities. Human terrain suitability is a Yes/No judgment,
with no separate threshold-setting task.
