# Numeric evaluation v1

Continue the selected twelve-region pilot from the master plan and the owner's
assent to initial evaluation. The owner judges terrain appearance; technical
configuration and validation belong to the implementer. Preserve the exact
hesitant response without presenting it as a technical certification.

Implement a small evaluation-only release and CPU numeric job. Rust owns release
validation, current-parent checks, GDAL decoding and publication. Python owns
scalar loading and numeric metrics. No training, GPU framework, resampling,
window extraction, codec or model selection is introduced in this step.

## Release

`dataset build <runtime> <selection.json> <approval.json>` publishes a portable,
content-addressed manifest below the configured datasets root. Selection is a
strict v1 contract: evaluation-only purpose, maximum physical context, separation
buffer, fixed normalization, and named calibration/validation groups containing
prepared manifest hashes. Approval pins the exact selection bytes, names the
human decision actor and assistant recorder, and captures the original response
and reviewed proposal/page identities. This is a local authorship declaration,
not authentication or a model-training permission token.

Require both roles, unique group/member identities, all prepared parents current
and registered, a common metric CRS/datum, disjoint footprints, and cross-role
separation at least the maximum context. A repeated source asset cannot cross
roles. Keep nearby members in one role even if given different group names.
Bound releases to 32 regions and evaluated rasters to 2,048 cells per side.
Require each raster's physical extent to fit the declared maximum context.

The immutable release embeds the selection and approval, with exact hashes and
tool version. Source evidence remains reachable through prepared/curation
lineage. No SQLite migration is needed: the content-addressed manifest is the
release record, and `dataset verify` rechecks current registered source parents.
Do not copy or rewrite source rasters. A stale release remains preserved and
fails use verification. Identical publication is idempotent; corruption fails.

## CPU job boundary

`evaluate numeric <runtime> <release-manifest> <python> <new-output-dir>` verifies
the release, decodes all seven native raster layers sequentially through GDAL,
and writes bounded little-endian Float32/Byte payloads with pinned hashes into a
private staging directory beside the output. The job request includes the exact
release, grid, physical spacing, masks and fixed normalization. Invoke the
installed `topography_training.numeric` module with explicit request/output paths.

Python validates strict version/fields, identities, payload sizes, shapes and mask
categories; loads one region at a time; checks finite valid land and deterministic
masked fill; rejects all-masked inputs and out-of-range valid elevations. Ground
metrics exclude water/invalid cells. Slope metrics use only adjacent valid pairs
and actual per-axis spacing. Numeric payload loading remains scalar Float32.

Measure Float32 and Float16 normalized absolute-height storage using a fixed
zero offset and 2,048 m scale. Preserve valid signed zero in the Float32 route.
Report absolute/RMS metre error, slope-component error, nonflat neighbor pairs
turned flat and exact-bit agreement, separately per region/role. Add tiny fixed
ramps, flats, sharp and narrow ridges. These are representation measurements,
not accepted terrain tolerances or rejection of all mixed-precision model use.

The worker writes metrics and a result manifest only after all cases pass input
validation. Record source-code hashes, Python/NumPy versions, elapsed time,
process RSS scope and zero GPU use. Rust checks result identities, completion,
bounded metrics and artifacts, rechecks the source release, then publishes the
completed run without overwriting existing output. Failed runs never acquire a
completion manifest. Preserve no half-published result as an accepted run.

## Verification and scope

Use the existing small CLI workflow-check style, with synthetic accepted sources
in isolated temporary catalogs. Exercise corrupted/stale parents, duplicate and
overlapping/cross-role sources, malformed approval and unsupported versions,
unknown fields, water/invalid exclusion, signed zero, endian decoding, rectangular
spacing, range errors, all-masked inputs, failed workers and existing outputs.
Verify installation/import outside the checkout and run `scripts/check.sh`.

Then publish the approved twelve-region evaluation release and run its small CPU
job. Report actual precision findings plainly and continue toward the next model
representation experiment. A substantial GPU/CPU workload still requires the
AGENTS.md readiness pause; downloads and Rust builds remain exempt.
