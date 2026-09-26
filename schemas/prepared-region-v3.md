# Prepared native ground and cover v3

Version 3 extends [v2](prepared-region-v2.md) with one layer and changes nothing
else. Build requests use `schema_version: 3`; every quality parent must be
[regional quality v3](regional-quality-v3.md).

Completion manifests use `schema_version: 3`, `kind: prepared_native_region`
and `policy: native_ground_cover_v2`. The fixed artifact set adds:

- `land-cover.tif`: Byte ESA WorldCover v200 class of each cell (10 tree,
  20 shrub, 30 grass, 40 crop, 50 built-up, 60 bare/sparse, 70 snow/ice,
  80 water, 90 wetland, 95 mangrove, 100 moss/lichen), 0 outside coverage.
  Raw classes: game water, snow/ice and vegetation masks do not override them.

Numeric dataset selections at v2 accept v3 products and ignore `land-cover`.
