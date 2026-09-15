# Terrain export

Applies to the future `export` module and delivery manifests. Export is a
conversion from authoritative physical terrain to a documented consumer format.
The [master plan](../plans/terrain-design-master-plan.md), sections 5 and 10,
defines these requirements; Bevy and QuadTree implementation remains outside scope.

## Distinguish cells from vertices

- Keep the master elevation a cell-center Float32 raster in metres. Derive
  physical extent from cell count times spacing; record origin, axes, row order,
  dimensions, and coordinate transforms.
- Convert explicitly to shared-edge vertex grids at export. A tile spanning
  1,024 cell intervals needs 1,025 vertices on each full side. Training crops and
  inference windows do not determine delivery tile geometry.
- Define cell-center-to-vertex interpolation, mask propagation, halo access, and
  outer-world boundary behavior with the first exporter. The plan has not chosen
  those algorithms. Read shared samples from the same authoritative surface in
  global coordinates, with identical arithmetic and encoding on both tiles.
- Require bit-identical shared encoded vertices and consistent border masks,
  including four-tile corners. Do not interpolate or normalize each tile in
  isolation. GDAL's [raster data model](https://gdal.org/en/stable/user/raster_data_model.html)
  explains pixel-corner geotransforms and pixel-center positions; metadata alone
  does not perform this project's cell-to-vertex conversion.
- Handle partial tiles explicitly. At 1 m spacing, the exact 32,000 m preset has
  31 full 1,024-interval tiles and a 256-interval remainder per axis; a final
  partial side therefore needs 257 vertices. Store actual tile dimensions and
  extents rather than silently padding or changing world scale.

## Use one documented height encoding

- Retain the Float32 master and export UInt16 with one linear offset/scale for
  the world. Keep all 65,536 codes available for heights; nodata uses a separate
  validity mask. Do not reserve zero or another height code for missing terrain.
- Define the chosen valid elevation range, rounding, clipping, storage order,
  and inverse transform. For an in-range height `h` and range `[h_min, h_max]`:

  ```text
  step_m = (h_max - h_min) / 65535
  code = round((h - h_min) / step_m)
  decoded_m = h_min + code * step_m
  ```

- Require a finite range with `h_max > h_min`. Validate bounds before conversion;
  report out-of-range counts and apply an explicit policy instead of silent
  clipping. A 4,096 m range gives roughly 0.0625 m steps; the no-clipping rounding
  error is at most half a step, apart from declared arithmetic tolerance. This
  is encoding precision, not survey or model accuracy.
- Measure quantization error against the authoritative surface sampled at the
  exported vertex positions, not an unrelated cell index. Separately measure
  any error introduced by the selected sampling conversion.
- Exclude invalid and underwater fill from terrain elevation checks while still
  validating mask coverage, dimensions, consistency, and the water-surface
  contract. Never let deterministic fill bias valid-land interpolation or the
  chosen height range.

## Package and verify

- Deliver the Float32 master or indexed equivalent, height tiles, biome/water/
  validity masks, encoding and coordinate metadata, attribution, overviews, and
  a manifest tying the export to an accepted world revision and validation report.
- Use windowed writes and deliberate compression settings. Prefer lossless
  numeric storage unless an explicit measured error budget permits loss. For
  GeoTIFF/COG output, consult the
  [GDAL COG driver](https://gdal.org/en/stable/drivers/raster/cog.html) for creation
  options and overview behavior; a file extension is not proof of correct layout.
- Reopen delivered files through the intended reader. Verify orientation,
  extent, decoded heights, masks, partial tiles, all shared borders/corners, and
  checksums. Use small ramps and distinct axis markers to reveal flips and
  half-cell mistakes; add real-terrain review before promotion.
