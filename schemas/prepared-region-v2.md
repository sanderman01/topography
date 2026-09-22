# Prepared native ground and cover v2

Version 2 extends [v1](prepared-region-v1.md) without changing legacy products.
Build requests use `schema_version: 2`; every quality parent must also be v2.
Mixed v1/v2 parents are rejected.

Completion manifests use `schema_version: 2`,
`kind: prepared_native_region`, and `policy: native_ground_cover_v1`. They
retain all v1 metadata and parent identities. The fixed artifact set adds:

- `surface-elevation.tif`: the exact source Float32 surface samples, including
  snow/ice-covered samples, paired with unchanged `provider-validity.tif`.
- `snow-ice.tif`: Byte 0 unknown / 1 absent / 2 present.
- `vegetation.tif`: Byte 0 unknown / 1 absent / 2 present. Unavailable
  vegetation evidence produces all-zero unknown values.

`elevation.tif` remains Float32 metres but represents supported ground. Water,
known snow/ice, unsupported ground, provider-invalid cells, and nonfinite cells
receive the request's deterministic `masked_fill_m`; `validity.tif` is 255 only
for supported, finite, provider-valid ground. Other v1 layers retain their
encodings, including binary prepared water and categorical quality.

Both Float32 elevation rasters and all Byte masks share the exact output grid.
Artifact integrity, immutable quality reports, registration, current-parent
checks, review state, and non-admission behavior are unchanged.
