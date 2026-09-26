# Regional quality v3

Version 3 extends [v2](regional-quality-v2.md) with one mask. Input
`schema_version` is `3` and `masks` contains the v2 masks plus `land-cover`, an
exact-grid Byte GeoTIFF holding only WorldCover v200 class codes (0, 10, 20,
30, 40, 50, 60, 70, 80, 90, 95, 100). A missing `land-cover` mask blocks.
`surface_cover` is unchanged; land cover shares the snow/ice and vegetation
evidence. It never affects ground, coverage or blockers.

Reports use `schema_version: 3` and `policy: native_quality_cover_v2`, with v2
fields and counts.
