# Numeric evaluation with surface cover v2

The v2 selection, release, worker request, metrics, worker result and completed
run use `schema_version: 2`. Other fields and bounds follow
[v1](numeric-evaluation-v1.md). The captured human approval remains version 1 and
pins the exact selection bytes. No new terrain vote or training admission results.

A v2 selection requires every prepared member to use version 2. A v1 selection
requires legacy version-1 members. Mixing versions is rejected. Prepared IDs pin
all mask artifacts and per-parent quality reports, including cover assessment,
evidence hashes, dates, absence and availability. Historical v1 releases remain
readable under their original policy; dataset status reports
`surface_cover: legacy_assessment_pending`. Version 2 reports `assessed` (optional
vegetation can still be unavailable).

The v2 numeric request retains all seven v1 layers and adds `snow-ice`,
`vegetation` and `surface-elevation`. Each is a hashed binary payload on the same
grid. Cover masks are Byte 0 unknown / 1 absent / 2 present; surface elevation is
little-endian Float32 metres with the separate provider-validity layer. Native
surface samples are retained independently of deterministic ground fill.

The worker rejects unknown snow/ice, invalid categories, mismatched validity,
nonfinite valid ground, changes to valid ground relative to measured surface,
and inconsistent masked fill. Valid ground excludes water and snow/ice, including
overlaps; vegetation presence or unknown coverage never changes ground validity.
Height and adjacent-slope metrics use only valid ground. All-ground-masked input
fails explicitly rather than producing empty reductions.

Version-2 region metrics add `snow_ice_cells` (including water overlaps).
`invalid_land_cells` counts non-water, non-snow/ice cells lacking valid ground.
Rust independently checks these counts and every payload hash before publishing.
No vegetation supervision or inferred negative vegetation labels are produced.

The bounded image-codec and scalar fitting experiments retain their v1 boundary
and reject v2 inputs until their own cover propagation is implemented. This does
not prevent v2 CPU numeric evaluation or imply production training support.
