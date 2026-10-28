# Terrain export

Applies to the `export` module and delivery manifests
([heightmap export v2](../../schemas/heightmap-export-v2.md)). Export is a
conversion from authoritative physical terrain to a documented consumer format.
The [master plan](../plans/terrain-design-master-plan.md), sections 5 and 10,
defines these requirements; the game's Bevy quadtree renderer remains outside scope.

## Deliver heightmap textures

- Keep the master elevation a cell-center Float32 raster in metres. Derive
  physical extent from cell count times spacing; record origin, axes, row order,
  dimensions, and coordinate transforms.
- Export single-channel 16-bit heightmap textures and matching layer textures,
  not meshes or vertex buffers. Each level-0 texel is one master sample read
  directly from the authoritative surface in global coordinates; do not resample.
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
  spacing is 2^*k* times level 0, and texel *i* keeps level-0 sample *i*·2^*k*'s
  position; continue until one tile covers the world.
- Build every level from one global pyramid before cutting tiles, so tiles of a
  level agree across seams. Heights and continuous layers take the area mean
  (weights ¼, ½, ¼ per axis over the coarse texel's footprint), class layers the
  weighted mode, and ids are sampled, never averaged. Do not decimate: a
  decimated level aliases thin ridges and rivers.
- Give every non-height layer a small apron (4 texels) repeated from its
  neighbours, so the game can filter and blend across tile borders; aprons past
  the world edge repeat the edge texel. Height tiles keep the plain layout.
- Handle world edges and partial tiles explicitly (with `shared_edge`, the last
  tile has no neighbour to share and is one texel short). Store actual tile dimensions
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

## Export water as layers

- Ground continues under water. Export the water surface (in the height
  encoding), an 8-bit coverage fraction and a 16-bit body id as separate layers;
  the game gets depth as surface minus ground and the body's type from the
  description. Do not export a binary water mask.
- Lakes and the sea keep one surface level at every level; river surfaces
  average only samples of the same body. Carry the surface a few texels past the
  shore (4, the apron) so the game can blend the shoreline.
- Coverage is the area mean, so streams narrower than a texel keep their share.
  Body ids are dense export ids (sea 1) mapped to the world's bodies in
  `description.json`.
- Write water tiles only where a tile or its apron holds water, and state in the
  manifest that a missing file means no water.
- Check before publishing: surface at or above ground where wet at the finest
  level, one level per lake at every level, coverage matching surface over
  ground, identical shared edges. Mixed shore texels on coarse levels can sit
  below the mean ground; report them, do not fail.

## Package and verify

- Deliver the Float32 master or indexed equivalent, heightmap texture tiles,
  water, biome, land-cover and density layer tiles with legends, encoding and
  coordinate metadata, attribution, overviews, and a manifest tying the export to
  an accepted world revision and validation report.
- Use windowed writes and deliberate compression settings. Prefer lossless
  numeric storage unless an explicit measured error budget permits loss. For
  GeoTIFF/COG output, consult the
  [GDAL COG driver](https://gdal.org/en/stable/drivers/raster/cog.html) for creation
  options and overview behavior; a file extension is not proof of correct layout.
- Reopen delivered files through the intended reader. Verify orientation,
  extent, decoded heights, masks, partial tiles, duplicated borders/corners, and
  checksums. Use small ramps and distinct axis markers to reveal flips and
  half-cell mistakes; add real-terrain review before promotion.
