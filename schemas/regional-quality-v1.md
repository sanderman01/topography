# Regional quality v1

Producers/consumers: `quality draft`, `quality assess`, `prepare check`.
See the [workflow](../docs/workflows/regional-quality.md). This Rust-only contract
assesses an existing native review; it is not a prepared-raster contract.

## Input

Strict JSON fields, with unknown fields rejected:

- `schema_version`: exactly `1`.
- `raw_receipt`, `review`: `{path, sha256}` for the exact existing inputs.
- `masks`: exactly `water`, `quality`, `repairs`, `features`. Each is `null`
  (unresolved) or `{path, sha256}`.

The [template](../examples/manifests/regional-quality-v1.json) uses placeholder
paths and zero hashes; it cannot be assessed unchanged. Prefer `quality draft`
to generate actual pins. Leaving masks null is an explicit incomplete assessment.

Relative paths resolve beside the input document. SHA-256 strings are 64 lowercase
hexadecimal characters. Documents are limited to 1 MiB; mask files to 128 MiB.
Native reviews enforce 1–2,048 cells per axis. Masks must be self-contained Byte
GeoTIFFs with one band and exactly matching dimensions, geotransform, CRS and axis
mapping. There is no coordinate tolerance or implicit resampling. Nontrivial
scale/offset, nodata and hidden validity masks are rejected.

Water is 0 unknown / 1 terrestrial / 2 water. Quality is 0 unknown / 1 supported /
2 unsupported. Repairs and features are binary 0 absent / 1 present. The review's
distinct provider-validity mask is 0 invalid / 255 valid. Invalid categories fail
before a report is published. Zero elevation is not a water or invalidity marker.

## Assessment

`schema_version: 1`, `kind: regional_quality_assessment`,
`policy: native_quality_v2`; pretty JSON without a trailing newline.
The report pins raw receipt, review and asset SHA-256, `window_cells`, `grid`
(size, geotransform, coordinate system), mask hashes/byte lengths, GDAL and
Topography versions, `blockers` and `admission: not admitted`. It has no local
paths or generated timestamps.

`counts` is null until all four masks exist. Otherwise it contains:

| Field | Count |
| --- | --- |
| `terrestrial_cells` | Water category 1 |
| `valid_terrestrial_cells` | Land, quality 1, provider 255 and finite elevation |
| `water_cells`, `unclassified_water_cells` | Water categories 2 and 0 |
| `unsupported_terrestrial_cells`, `unknown_terrestrial_quality_cells` | Land with quality 2 and 0 |
| `provider_invalid_terrestrial_cells` | Land with provider validity 0 |
| `nonfinite_terrestrial_cells` | Provider-valid land with NaN or infinity |
| `repaired_cells`, `defining_feature_cells` | Respective category 1 across the whole footprint |
| `unsupported_feature_cells` | Marked feature cells that are not valid terrestrial cells |

Deficit counts overlap; do not sum them to derive validity. Coverage uses integer
arithmetic: `valid_terrestrial_cells * 1000 >= terrestrial_cells * 995`, with a
positive denominator. Unknown water, unsupported feature
cells, nonfinite provider-valid land or any repairs block eligibility independently.
Unknown ground quality counts as invalid ground, never as implicit support.

The feature layer may be entirely zero: whole-tile review does not require
individual feature annotations. Marked features still require supported terrain
at every marked cell. Policy v2 removes only the requirement to mark at least one
feature; the four-layer contract and all other quality gates remain unchanged.
Historical `native_quality_v1` reports retain their meaning. Reassessment emits
v2 and a new hash, so existing captured v1 evidence must be refreshed before
preparation; this does not require a new visual vote on unchanged terrain.

Assessment completion is independent of eligibility. Invalid file contracts fail
without a completion report; incomplete or failing classifications produce a
report with blockers. Reports are written to new files only.

## Preparation eligibility

`prepare check` recomputes the assessment and verifies the pinned current curation
head and its full history. The exact assessment hash must be captured and
referenced by coverage and four findings: ground support, artifacts and repairs,
water classification, unsupported feature gaps. Both coverage counts must match.
All six findings must pass; human acceptance, an active class, complete source
metadata and confirmed permission with captured documents are required. Native
spacing must match the raster. The declared horizontal `EPSG:code` must match
the raster's horizontal PROJJSON EPSG identity (including within a compound CRS).
Unsupported declarations and absent identities block eligibility.

Coordinate and feature-fidelity findings still need supporting evidence. The tool
checks identities and declarations, not the truth of masks, reviewer identity or
legal interpretation. A non-synthetic declaration alone does not prove provenance.

Output fields: `schema_version: 1`, `kind: preparation_eligibility`,
`policy: native_quality_v2`, `curation_record_sha256`, `quality_report_sha256`,
`review_sha256`, `eligible`, `blockers`, `admission: not admitted`.
Exit status is 0 for eligible, 1 for blocked/erroneous input, 2 for unsupported
arguments. Blocked results are JSON; errors go to stderr.

This is a point-in-time observation. It neither reserves a decision head nor
authorizes later processing without revalidation. No database schema changes,
old-record reinterpretation, prepared-pool writes or dataset admission occur.
Changed inputs or tool versions require new evidence and review. Preserve
previous reports and accepted decisions.
