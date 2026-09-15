# Numeric DEM acquisition and review

Implemented: bounded USGS project-based 1 m GeoTIFF discovery, selected HTTPS
acquisition, local Float32 GeoTIFF import, raw integrity verification and small
native-grid review crops. These commands do not admit data to prepared/training
pools or record human acceptance. Use the separate [curation workflow](source-curation.md)
for evidence, crop labels and explicit decisions. Regional mosaicking, repairs, water
classification and numeric model evaluation remain future work.

## Tools and setup

Build the Rust workspace after fetching locked dependencies:

```sh
cargo fetch --locked
CARGO_BUILD_JOBS=1 cargo build --workspace --locked --offline
```

Install the GDAL command-line tools and curl through your system package manager.
This workflow was verified with GDAL 3.13.3 and curl 8.22.0. It invokes
`gdalinfo`, `gdal_translate` and `gdaldem`; no Rust GDAL bindings, Python GDAL
bindings or ML environment are required. Python 3.11+ and OpenSSL are used only
by the optional verification script. See [development checks](../../scripts/README.md).

Use a local copy of [the runtime preset](../../configs/runtime.example.toml) to
select storage roots. Set `paths.base_dir` relative to that copy's parent. The
examples below run from the repository root. Close other applications before
substantial raster or point-cloud work, following [AGENTS.md](../../AGENTS.md).

## Discover candidates

```sh
target/debug/topography discover usgs '-66.84,18.32,-66.83,18.33' 5 data/staging/discovery-pr
```

The bounding rectangle is west, south, east, north in longitude/latitude degrees.
The adapter fixes the collection to `Digital Elevation Model (DEM) 1 meter` and
the format to GeoTIFF. It requests at most 1–100 items and retains:

- `response.json`: exact original TNM API response, including unmapped fields.
- `discovery.json`: query bounds, mode, counts, known byte estimates and unknown sizes.
- One v1 source TOML file per candidate, with a safe local ID derived from the
  original product ID. Original IDs remain in the record and response.

The output directory must be new. More matches than returned are explicitly
reported as truncated; narrow the area instead of treating one page as complete
coverage. Bounds are not exact valid-ground footprints. Publication dates do not
become survey acquisition dates. CRS, datum, actual native spacing and training
permission remain unknown until supported by metadata and inspection.

An archived TNM response can be mapped without a network request:

```sh
target/debug/topography discover usgs '-66.84,18.32,-66.83,18.33' 5 data/staging/replayed-pr saved-response.json
```

The summary records `saved_response` mode. The caller must supply the matching
query bounds and an export from the stated collection; the file itself does not
prove the query that produced it. This also makes schema mapping reproducible
when the API is unavailable. Our live API probes returned HTTP 504; the adapter's
mapping was exercised with a synthetic archived response. Real download/import
was verified separately against the official USGS S3 delivery.

See [TNM Access](https://apps.nationalmap.gov/tnmaccess/) and
[USGS dataset definitions](https://apps.nationalmap.gov/datasets/) for the API and
the distinction between project-based and seamless 1 m products.

## Acquire or import a selected DEM

Inspect a generated source record and retain its original metadata. The
[source contract](../../schemas/source-v1.md) also supports a manually authored
record for an official direct download, with original XML/JSON metadata saved
alongside it. Do not fill unknowns with guessed dates, units or permission.

```sh
target/debug/topography acquire path/to/source.toml configs/runtime.example.toml 67108864
target/debug/topography import path/to/source.toml path/to/local-dem.tif configs/runtime.example.toml 67108864
```

The final argument is a hard asset-size limit in bytes, here 64 MiB. Each command
prints the raw receipt path on stdout. Diagnostics go to stderr; operational or
input errors return status 1 and invalid command syntax returns 2.

Both paths require an explicit `bare_earth_dem` declaration and a nonempty,
self-contained GeoTIFF with one Float32 band, internal CRS and a nonsingular
geotransform. The file stays in its original numeric representation, units and
orientation. There is no conversion, resampling or unit/datum inference at import.
Integer encodings, RGB/multiband imagery, external sidecar georeferencing, archives
and point clouds are deliberately unsupported by this first importer.

The original source description and its referenced metadata must fit within
1 MiB each. They are captured byte for byte. GDAL records the file's dimensions,
geotransform, CRS/axis mapping, raster footprint, band type and nodata/mask metadata
when available. This inspects the raster envelope, not exact valid-ground coverage
or every compressed block. Metadata declarations and actual raster inspection
remain separate; a conflicting or incomplete source still needs review.

### HTTPS identity and resume

Acquisition uses the first `source_urls` entry. It must be a stable HTTPS URL
without credentials, query parameters or fragments. Curl configuration files are
disabled; TLS verification stays enabled and redirects are limited to HTTPS.
The server must provide a positive Content-Length within the selected limit,
a strong ETag and exact byte-range responses. Use manual import for other servers.

Completed 8 MiB chunks are retained under the configured staging root, keyed by
URL, strong ETag and total size. Every range request sends `If-Match`. Only HTTP
206 responses with the exact requested Content-Range, matching ETag and expected
length become completed chunks. Transient transfers retry at most three times;
permission failures and identity/range violations stop. A failed chunk restarts
from its beginning. Completed chunks are rehashed before reuse.

A changed HEAD identity creates a separate staging set, preserving the old one.
Corrupt cached chunks stop the run and remain available for investigation; choose
a new staging root to retry without discarding evidence. There is no automatic
cache deletion. Allow approximately twice the asset size for retained chunks and
the assembled raw file, plus metadata and review outputs. Imports stream through
small buffers; they do not allocate the uncompressed full raster.

If `asset_sha256` is declared, the assembled/copied bytes must match it. Otherwise
the computed SHA-256 identifies the retained snapshot; it is not an independently
published upstream checksum. Manual imports also compare the copied bytes against
the source again to detect changes while copying.

## Verify raw storage

```sh
target/debug/topography verify data/raw/records/RECEIPT_SHA256.json
```

See the [raw receipt contract](../../schemas/raw-receipt-v1.md). Raw objects and
receipts use checksums as names and are published without replacing existing
files. A repeated identical import revalidates existing content and returns the
same receipt. Changes to source metadata or inspection/tool versions produce a
new record while retaining shared unchanged objects.

Publication stages files on the raw destination filesystem, syncs them, publishes
objects, then publishes the receipt last. An interrupted run can leave temporary
files or unreferenced objects; neither is a completed import. Retry verifies and
reuses complete objects, then publishes the missing receipt. Acquisition has no SQLite
registration yet. Verification rehashes the receipt and every referenced object;
missing or corrupt bytes fail without deletion or repair.

## Create a small review crop

```sh
target/debug/topography review data/raw/records/RECEIPT_SHA256.json data/review/candidate-1 '9000,300,512,512'
```

The window is x offset, y offset, width, height in native raster cells, from the
upper-left storage corner. Each crop side must be 1–2,048 cells and fit inside the
source. The output directory must be new. The command verifies raw provenance,
then creates:

- `elevation.tif`: an unscaled Float32 crop, with native spacing and mask semantics.
- `validity.tif`: the provider validity mask, 0 invalid / 255 valid.
- `hillshade.tif`: a display product, never elevation ground truth.
- `inspection.json` and `review.json`: coordinates, parent receipt checksum,
  crop window, tool versions and artifact checksums.
- `metrics.json` and `preview.html`: finite/masked sample counts, native extrema,
  three light directions, row/column profiles and an interactive perspective view
  when metric metadata supports it. See the [inspection contract](../../schemas/native-review-v1.md).

GDAL uses a 64 MiB cache setting and one worker. Sidecar/PAM writes are disabled
so inspection cannot alter raw assets. Crop extraction does not interpolate or
quantize heights. Source nodata is retained; zero can still be a valid elevation.
Provider validity is not a terrestrial quality or water mask. Underwater fill,
water surface authoring and coverage admission await explicit preparation rules.

Open `preview.html` directly in a browser for overhead, cross-section and
low-altitude inspection. **Keep view in address** preserves a repeatable camera
view in the URL fragment. Open the GeoTIFFs in QGIS for detailed numeric review.
This does not establish vehicle fit, numerical model precision, fine-feature
fidelity or source acceptance. The [pilot criteria](../design/pilot-acceptance-v1.md)
and dataset-specific training-permission gate still apply; the vehicle envelope
remains unset.

The [Río Abajo trial](../reports/rio-abajo-karst-trial.md) traces a native 0.5 m
survey tile to three inspectable crops using this same acquisition command.
