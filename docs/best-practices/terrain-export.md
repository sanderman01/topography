# Terrain export

Applies to the future `export` module and delivery manifests. Export is a
conversion from authoritative physical terrain to a documented consumer format.
The [master plan](../plans/terrain-design-master-plan.md), sections 5 and 10,
defines these requirements; the game's Bevy quadtree renderer remains outside scope.

## Deliver heightmap textures

- Keep the master elevation a cell-center Float32 raster in metres. Derive
  physical extent from cell count times spacing; record origin, axes, row order,
  dimensions, and coordinate transforms.
- Export single-channel 16-bit heightmap textures and matching mask textures,
  not meshes or vertex buffers. Each texel is one master sample read directly
  from the authoritative surface in global coordinates; do not resample.
- Tiles span a configurable power-of-two interval count. The default copies
  MSFS 2020 DEM tiles: 256 intervals, `shared_edge`, 257² texels at 16 bits.
  Training crops and inference windows do not determine tile geometry.
- Support two layouts over the same global sample lattice:
  - `shared_edge`: 2ⁿ + 1 texels per full side. Neighbours repeat their border
    row/column, so each tile closes its own seams. Require bit-identical
    duplicated texels and masks, including four-tile corners.
  - `power_of_two`: 2ⁿ texels per full side, no duplication. The renderer reads
    the closing row/column from the neighbouring tile.
- Export a quadtree of LOD levels with the same tile texel size. Level *k*
  spacing is 2^*k* times level 0; continue until one tile covers the world.
  Default to nested decimation (coarse texel = level-0 sample at index × 2^*k*)
  so coarse heights equal fine heights exactly; treat a filtered pyramid as an
  optional setting. Sample masks the same way.
- Handle world edges and partial tiles explicitly. Store actual tile dimensions
  and extents rather than silently padding or changing world scale. GDAL's
  [raster data model](https://gdal.org/en/stable/user/raster_data_model.html)
  explains pixel-corner geotransforms and pixel-center positions.
- GPU block compression is optional and derived. BC4 keeps roughly 8-bit
  precision per block and BC6H is half-float, both far coarser than 16-bit
  heights over a kilometre range. Keep lossless 16-bit textures as the delivered
  truth; add a compressed variant only with a measured error budget.

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
- Measure quantization error against the authoritative sample behind each
  texel, not an unrelated cell index.
- Exclude invalid and underwater fill from terrain elevation checks while still
  validating mask coverage, dimensions, consistency, and the water-surface
  contract. Never let deterministic fill bias valid-land interpolation or the
  chosen height range.

## Package and verify

- Deliver the Float32 master or indexed equivalent, heightmap texture tiles,
  biome/water/validity mask textures, encoding and coordinate metadata, attribution, overviews, and
  a manifest tying the export to an accepted world revision and validation report.
- Use windowed writes and deliberate compression settings. Prefer lossless
  numeric storage unless an explicit measured error budget permits loss. For
  GeoTIFF/COG output, consult the
  [GDAL COG driver](https://gdal.org/en/stable/drivers/raster/cog.html) for creation
  options and overview behavior; a file extension is not proof of correct layout.
- Reopen delivered files through the intended reader. Verify orientation,
  extent, decoded heights, masks, partial tiles, duplicated borders/corners, and
  checksums. Use small ramps and distinct axis markers to reveal flips and
  half-cell mistakes; add real-terrain review before promotion.
