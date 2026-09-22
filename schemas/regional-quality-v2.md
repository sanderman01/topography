# Regional quality v2

Producers/consumers: `quality assess`, `prepare check`, and automatic preparation.
Version 2 extends [v1](regional-quality-v1.md) with explicit surface-cover
classification. Version 1 bytes and meanings remain unchanged.

## Input

`schema_version` is `2`. `raw_receipt` and `review` retain the v1 pinned-file
shape. `masks` contains exactly `water`, `quality`, `repairs`, `features`,
`snow-ice`, and `vegetation`. Cover masks are exact-grid Byte GeoTIFFs with
0 unknown, 1 absent, and 2 present. Snow/ice is required. Vegetation may be
null and is then synthesized as all unknown; unknown vegetation never means
absent and never blocks ground eligibility.

`surface_cover` contains exactly `snow-ice` and `vegetation`. Each entry has
`status` (`assessed` or `unavailable`), a nonempty `note`, `observed_at` as a
nonempty string or null, and `evidence`, an array of pinned `{path, sha256}`
files. Assessed cover requires a mask and at least one evidence file.
Snow/ice cannot be unavailable. Only vegetation may be unavailable, which
requires a null mask and may have no evidence. Reports retain evidence hashes
and byte lengths but omit local paths.

All v1 grid, raster, size, digest, and mutation checks apply. Unknown snow/ice
blocks even where water is present. Vegetation categories do not alter ground
eligibility.

## Assessment

Reports use `schema_version: 2`, `kind: regional_quality_assessment`, and
`policy: native_quality_cover_v1`. They retain all v1 fields and add the
portable `surface_cover` map. Counts add `ground_cells`, `snow_ice_cells`,
`unknown_snow_ice_cells`, `vegetation_cells`, and
`unknown_vegetation_cells`.

Ground is a cell classified as nonwater and snow/ice absent. The 99.5 percent
coverage denominator is `ground_cells`; zero ground blocks safely.
The retained `terrestrial_cells` and `valid_terrestrial_cells` fields use this
ground domain in v2; v1 retains its original nonwater domain. Known
snow/ice is excluded from ground counts and prepared ground elevation. Unknown
snow/ice always blocks. Provider validity remains independent and does not
establish ground beneath cover.

Eligibility snapshots also use schema version 2 and the quality policy above.
Human votes, findings, permission, and all other v1 preparation gates remain
required and are not fabricated or weakened.
