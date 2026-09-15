# Native crop inspection v1

`topography review` creates the [raw-linked review manifest](raw-receipt-v1.md)
and the following additional review artifacts. Existing v1 manifests without
these additional files remain valid; consumers enumerate the manifest's
`artifacts` list. Nothing here records source admission or human acceptance.

## Metrics

`metrics.json` has `schema_version = 1`, `kind = "native_crop_inspection"` and:

- `cells`, `provider_valid_cells`, `provider_invalid_cells`: full native-crop
  counts using the provider's binary validity mask.
- `finite_valid_cells`, `nonfinite_valid_cells`: valid samples split by finiteness.
- `minimum_native`, `maximum_native`: extrema over finite valid samples;
  null when there are none. No invalid/nodata value enters these extrema.
- `vertical_units_declared`, `vertical_datum_declared`: retained source assertions,
  possibly null. These are not independently verified coordinate transformations.
- `metric_north_up_preview`: whether the browser can offer a metre-based view.
- `vehicle_envelope`, `terrestrial_coverage`: explicitly null.
- `review_state = "unreviewed"`, `admission = "not evaluated"`.

Provider validity does not establish measured ground support or a terrestrial
denominator. These counts cannot substitute for the 99.5% terrestrial admission gate.

## Browser preview

`preview.html` is self-contained and reads no remote resources. It embeds native
Float32 samples as base64-encoded little-endian bytes, the separate provider mask,
and three GDAL display hillshades at azimuths 315°, 45° and 225°. Embedded heights
are never encoded as RGB, normalized or quantized. The page uses numeric samples
for row/column profiles; it skips nonfinite and invalid cells.

The perspective view requires declared metre horizontal/vertical units, an
inspected projected east/north metre CRS, matching raster axis order, a north-up
unrotated grid and no conflicting band unit. Missing PROJJSON disables that view.
Other crops still provide native overhead views, profiles and metrics.

The CPU renderer has a 60° field of view and 1:1 vertical scale. It renders two
screen columns at a time with at most 4,096 nearest-cell distance samples per ray.
It stops at invalid/nonfinite cells and crop boundaries. It is an inspection aid,
not a precision renderer, collision test, mesh export or evidence of ground support.
The overhead image uses raster-cell aspect and a bounded display size; physical
distances belong in profiles/coordinates, particularly for rectangular cells.

Observer clearances are 32, 64 and 128 m above the selected cell; fixed datum
height is a separate mode. These are camera clearances, not a populated vehicle
envelope. The address fragment preserves observer cell, heading, mode, height,
clearance and light choice through **Keep view in address**. Browser interaction
changes no source or review file and creates no acceptance record.

GDAL decodes temporary ENVI files within the owned review staging directory.
Preview input is limited to 2,048 cells per side; temporary binary reads are
bounded to the declared dimensions. Raw inspection retains its stricter sidecar
isolation. A maximum-size HTML preview is roughly 45 MB before browser overhead;
the 1,024² pilot previews are roughly 11 MB each. Keep large rasters in GIS tools.

The parent review manifest hashes the metrics and complete HTML alongside the
GeoTIFFs. [The verification script](../scripts/check_review.py) checks the raw
receipt, crop artifact hashes, source-window Float32 bits/masks, coordinates,
and the embedded browser samples. This is a native-extraction check, separate
from the planned model/loader numeric precision evaluation.
