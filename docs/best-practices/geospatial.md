# Geospatial processing

Applies to numeric digital elevation models (DEMs), classified LiDAR, prepared
rasters, and terrain quality metrics. Raw import/review now invokes GDAL tools;
a bounded PDAL ground-support probe and aligned native regional preparation are
available. Cross-survey joins and resampling remain planned. Geospatial tools are not required
for Rust builds or metadata-only validation.

## Preserve physical meaning

- Use bare-earth terrain data for ground elevation. Keep surface models,
  hillshades, imagery, and bathymetry identified separately. Prefer native
  0.5–1 m ground data; resampling a coarse source onto a fine grid does not create
  measured detail. Evaluate the features that survive the source's sampling.
- Store authoritative prepared and generated elevations as Float32 metres with
  explicit validity/quality masks. Document horizontal CRS, vertical datum,
  units, origin, axis order, row orientation, spacing, and cell-center convention.
  Use the [GDAL raster data model](https://gdal.org/en/stable/user/raster_data_model.html)
  when interpreting geotransforms and masks.
- Process each Earth survey in an appropriate projected metric CRS. Do not treat
  degrees as metres or assume every CRS uses longitude/latitude axis order. Keep
  the source-to-fictional-world transform explicit. See the
  [PROJ FAQ](https://proj.org/en/stable/faq.html) for axis-order conventions.
- Resolve vertical units and datum before joining surveys. Use verified vertical
  transformations where available, recording grids and tool versions. Isolate
  incompatible surveys instead of silently mixing ellipsoidal and sea-level
  heights or applying an unexplained offset.

## Prepare regions before cropping

- Inspect source footprints, alignment, masks, and overlap differences first.
  Mosaic an aligned region before extracting training crops. Declare overlap
  precedence; do not average misaligned ridges or smooth away survey steps.
- Resample to an explicit working grid once when practical. Choose elevation
  interpolation and downsampling filters deliberately; inspect overshoot and
  lost cliff detail. Resample class labels with nearest neighbor and choose a
  documented categorical/coverage rule for masks, never continuous interpolation.
  Record nodata handling and target alignment with the operation. Consult
  [gdalwarp](https://gdal.org/en/stable/programs/gdalwarp.html) for the selected
  tool version rather than relying on defaults.
- When rasterizing LiDAR, select classified ground returns, inspect supporting
  point density, and include a buffer around the desired crop. Choose and record
  interpolation radius, statistic, and unsupported-cell policy. See
  [PDAL's GDAL writer](https://pdal.io/en/stable/stages/writers.gdal.html).
- Preserve repair masks and before/after data. Limit void filling to reviewed
  repairs. Sinkholes and calderas are desired terrain, so blanket depression
  filling and broad smoothing are inappropriate defaults.
- Never apply gamma, histogram equalization, RGB conversion, or photographic
  enhancement to elevation. Keep display hillshades and color ramps separate
  from numerical source rasters.

## Masks and capacity

- Keep biome IDs, water, validity, repairs, and quality distinguishable. Zero
  elevation can be valid land; do not infer missingness or water from height.
- Water has an authored surface and a separate mask. Store a fixed deterministic
  fill under it, excluding that fill from terrain losses, structural controls,
  seam metrics, and elevation validation. Do not interpolate it into nearby
  valid land or invent a submerged bed.
- Use windowed reads/writes, tiled storage, and overviews. Account for input,
  output, masks, halos, temporary buffers, and compression memory. One full
  32,768² Float32 layer is 4 GiB before any of those additions; do not assume the
  complete processing job fits in RAM because a single layer does.

## Verify

Inspect known coordinates, terrestrial values, and mask boundaries after each
transform. Use ramps and sharp features to expose axis flips, half-cell shifts,
unit mistakes, interpolation artifacts, and invalid-cell leakage. Review real
terrain with several hillshade directions, contours, cross-sections, and oblique
views. The [master plan](../plans/terrain-design-master-plan.md), sections 4–5,
sets initial quality gates; [validation and review](validation-and-review.md)
separates exact invariants from pilot tolerances and human judgments.
