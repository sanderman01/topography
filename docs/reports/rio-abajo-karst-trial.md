# Río Abajo: early karst acquisition trial

Date: 2026-09-15. **Acquisition and native-crop verification completed; human
acceptance is unset.** Three 512 × 512 m crops expose repeated hills, enclosed
hollows and intervening saddles near the Río Abajo candidate point in Puerto Rico.
These are promising broad karst forms. They do not establish fine pinnacle
fidelity, measured ground support or usable airship passages.

This completes the early acquisition trial in the [master plan](../plans/terrain-design-master-plan.md).
The source remains raw/unreviewed and excluded from prepared/training admission
pending quality evidence and human crop acceptance. The subsequent
[whole-crop review](rio-abajo-whole-crop-review.md) records additional ground
measurements and the owner's dataset-permission approval on 2026-09-21.
The vehicle envelope remains explicitly unset.

## Inspect the results

The following files are local ignored artifacts, created by the commands below.
They are available in the working checkout; a fresh checkout must reproduce them.

| Crop / browser preview | Native source window, x/y/width/height | Elevation range (m, declared PRVD02) | Initial morphology observation |
| --- | --- | --- | --- |
| [North hollows](../../data/review/rio-abajo-pilot/north-hollows/preview.html) | `80,80,1024,1024` | 128.552–301.958 | Broad low floor beside a steep, rounded hill; enclosing slopes hide terrain beyond |
| [East ridges](../../data/review/rio-abajo-pilot/east-ridges/preview.html) | `1800,800,1024,1024` | 174.715–328.883 | Repeated rounded hills with intervening hollows and changes in skyline |
| [South saddles](../../data/review/rio-abajo-pilot/south-saddles/preview.html) | `900,1850,1024,1024` | 174.241–313.942 | Lower connections between hills and broader recovery spaces to inspect |

Each crop has 1,048,576 finite provider-valid cells, zero provider-invalid cells
and zero nonfinite valid cells. This is **not** 100% verified terrestrial quality:
the provider mask does not identify interpolation support, water or ground density.
Small rough features visible on the slopes require comparison with classified
ground returns or independent reference data before treating them as real detail.

The [GeoJSON crop envelopes](../../catalog/areas/rio-abajo-crops.geojson) locate
these windows in longitude/latitude and retain the parent asset hash. They are
candidate annotations, not accepted labels or a training split.

The local `data/review/rio-abajo-pilot/contours/` directory contains 10 m display
contours for each crop and a manifest with their hashes, parent review hashes,
tool version and interval. These GeoJSON files retain native EPSG:6566 coordinates
through GDAL's CRS extension; the catalog envelopes above use longitude/latitude.
The 10 m contour interval is a display choice, not a quality or flight tolerance.

### Repeatable morphology checks

The preview's row and column profiles retain native samples. For example, row
819 in North hollows has a low sample at column 93 (131.480 m) and a high sample
at column 770 (299.867 m), separated horizontally by 338.5 m. This gives a concrete
landform scale; that separation is not a measured passage width.

At East ridges, row 819 / column 370 is 204.573 m. Inspect its
[32 m view](../../data/review/rio-abajo-pilot/east-ridges/preview.html#x=370&y=819&heading=30&clearance=32&light=1)
and [128 m view](../../data/review/rio-abajo-pilot/east-ridges/preview.html#x=370&y=819&heading=30&clearance=128&light=1)
with heading 30° and northeast light. The nearby hillside blocks most of the
lower view, exposing a climb requirement. Also inspect the
[fixed 300 m datum-height view](../../data/review/rio-abajo-pilot/east-ridges/preview.html#x=370&y=819&heading=30&mode=fixed&height=300&light=1).
Move the observer while holding that fixed height to avoid hiding climb demands
with a terrain-following camera. The page also offers 64 m above-terrain views.

These previews are human review aids. They do not calculate collision clearance,
turning room, climb rates or route feasibility. The large rounded hills shown
here do not prove that Tsingy-like needles, narrow slots, caves or overhangs can
be reproduced with this source and a heightfield.

## Source and geographic evidence

The regional lead comes from USGS's description of northern Puerto Rico's cone
and doline karst in the [Ground Water Atlas](https://pubs.usgs.gov/ha/ha730/ch_n/N-PR_VItext1.html)
and the Forest Service's [Puerto Rican Karst—A Vital Resource](https://research.fs.usda.gov/treesearch/2864).
The specific candidate point is longitude −66.73°, latitude 18.36°; this trial
does not claim that the crop envelopes follow an official forest boundary.

The [NOAA dataset record](https://www.fisheries.noaa.gov/inport/item/60105/full-list)
identifies USGS/Dewberry ground-classified, hydro-flattened 0.5 m DEMs, with
NAD83(2011) Puerto Rico/USVI coordinates and PRVD02/GEOID12B metre elevations.
NOAA converted the delivered rasters to cloud-optimized GeoTIFFs. The collection
date range is 2018-04-17 through 2019-04-18; a tile-specific date is unresolved.
This is the DEM product, not NOAA's separately processed point-cloud delivery.

The official [project VRT index](https://noaa-nos-coastal-lidar-pds.s3.amazonaws.com/dem/USGS_PostMaria_PuertoRico_DEM_2018_9101/USGS_PostMaria_PuertoRico_DEM_2018_m9101_EPSG-6566.vrt)
located `prvi_d/USGS_OPR_PR_PRVI_D_2018_19QGA68505700.tif` at that point.
The downloaded file is 26,869,376 bytes, Float32, 3,000 × 3,000 cells, with
0.5 m spacing, projected CRS EPSG:6566 and a metre band unit. Its upper-left
pixel corner is approximately (168500, 258500) m and rows run south. The vertical
datum comes from the retained original metadata; no vertical transformation ran.

The live TNM discovery query again returned HTTP 504 after retrying outside the
sandbox. The trial therefore used the official NOAA index and direct asset
through the **existing generic acquisition command**, without adding a provider
adapter or claiming successful live TNM mapping.

### Retained identities

| Artifact | SHA-256 |
| --- | --- |
| Raw GeoTIFF | `7c4c85bd143c970d45330e2904f20d96ae0b8bd3b7065b43db43797054b77211` |
| Original NOAA metadata XML | `6313f0cb304ffc26e5be51c660cbc3c032faf7adbc546bab79f41a62133f0c81` |
| Retained source description | `42e6bb735ec1b23acd516d85d41584a91448ac6312ee2cf5541df9d68e714869` |
| Raw inspection JSON | `3762acb1bab2fe0fc494f2b729f572a2df3c82fe7675915b83b7326edbd998b8` |
| Raw receipt JSON | `68ffca798986f0a68154b1bd7bd1a66ecbe1e089d3abeb63140d5cc95ea8795d` |
| Discovery VRT index | `8360bb1d993b9cd61bb18c89f354cb110242701de8187b12eaa53c3a76b7da95` |

The receipt and its objects are under `data/raw/`. The index and original metadata
are also retained under `data/staging/karst-trial/`. Each crop's `review.json`
hashes its GeoTIFFs, inspection, metrics and complete browser preview, linking
them to the exact raw receipt. ETag `"4b32d87e1e400a013bde896070ae9b2f-4"` was the
HTTP range identity; it is not presented as an independently published checksum.

## Reproduce from the repository root

Use the [acquisition setup](../workflows/dem-acquisition.md) and a built CLI.
These commands assume new output directories. Existing artifacts must be kept;
choose another review directory for a new revision.

```sh
mkdir -p data/staging/karst-trial
curl --disable --fail --silent --show-error --proto '=https' --max-time 30 --max-filesize 1048576 'https://noaa-nos-coastal-lidar-pds.s3.amazonaws.com/dem/USGS_PostMaria_PuertoRico_DEM_2018_9101/USGS_PostMaria_PuertoRico_DEM_m9101_met.xml' -o data/staging/karst-trial/original.xml
cp catalog/candidates/rio-abajo-source.toml data/staging/karst-trial/source.toml
target/debug/topography acquire data/staging/karst-trial/source.toml configs/runtime.example.toml 33554432
```

The asset cap is 32 MiB. Compare the fetched metadata and asset hashes with the
table above; stop and record a new source revision if they changed. Acquisition
prints the receipt path. With the retained versions it is:

```sh
raw_receipt=data/raw/records/68ffca798986f0a68154b1bd7bd1a66ecbe1e089d3abeb63140d5cc95ea8795d.json
target/debug/topography verify "$raw_receipt"
target/debug/topography review "$raw_receipt" data/review/rio-abajo-pilot/north-hollows '80,80,1024,1024'
target/debug/topography review "$raw_receipt" data/review/rio-abajo-pilot/east-ridges '1800,800,1024,1024'
target/debug/topography review "$raw_receipt" data/review/rio-abajo-pilot/south-saddles '900,1850,1024,1024'
python3 scripts/check_review.py target/debug/topography "$raw_receipt" data/review/rio-abajo-pilot/north-hollows/review.json data/review/rio-abajo-pilot/east-ridges/review.json data/review/rio-abajo-pilot/south-saddles/review.json --output data/review/rio-abajo-pilot/comparison.json
```

With the installed `gdal_contour` tool, generate the optional display contours:

```sh
mkdir -p data/review/rio-abajo-pilot/contours
for crop in north-hollows east-ridges south-saddles; do
  gdal_contour --config GDAL_CACHEMAX 64 --config GDAL_NUM_THREADS 1 --config GDAL_PAM_ENABLED NO -i 10 -a elevation_m -f GeoJSON "data/review/rio-abajo-pilot/$crop/elevation.tif" "data/review/rio-abajo-pilot/contours/$crop.geojson"
done
```

Contours interpolate lines for display; they never modify the elevation raster.

Tool versions can change inspection and receipt hashes; use the actual printed
receipt and record the difference. No source joining, resampling, normalization,
water classification or repair occurs in these native crops.

## Verification and remaining gates

`CARGO_BUILD_JOBS=1 bash scripts/check.sh` passed with Rust 1.98.1 and GDAL 3.13.3.
The enhanced local workflow covers exact browser sample bytes, mask counts,
unknown-unit view gating, nonfinite/all-invalid inputs, tiny crops and HTML-safe
metadata. `scripts/check_review.py` compared all 3,145,728 real crop samples and
mask cells with their raw source windows, checked coordinate metadata and output
hashes, and verified the embedded browser data. All comparisons passed; see the
local [comparison report](../../data/review/rio-abajo-pilot/comparison.json).

Headless Firefox rendered the browser preview and restored saved camera settings.
This checks the review interface, not a full flight model. Rust 1.85 itself was
not installed or directly tested; no dependencies were added. No training ran.

Next, resolve dataset-specific training permission, inspect classified ground
support and small-scale artifacts, distinguish water/terrestrial quality, and
record the human crop decision. Then implement explicit regional preparation
and admission. These geographically adjacent review windows are not a dataset
split. The numeric model/loader round-trip milestone remains a separate task.

## Ground-support follow-up

The [classified-ground trial](rio-abajo-ground-support.md) now measures one 64 m
centre sample in each crop and captures the reports in the SQLite curation journal.
These observations do not establish whole-crop quality or human acceptance.
