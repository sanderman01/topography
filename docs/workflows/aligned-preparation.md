# Aligned native-region preparation

`prepare build` composes a bounded regional Float32 raster from source regions
that pass the [quality and eligibility workflow](regional-quality.md). Every input
must already have a current human acceptance with captured quality and permission
evidence. Processing produces an **unreviewed prepared region**, not human output
acceptance or a training release. The three accepted Río Abajo inputs have now
been qualified and prepared under [supplier_qa_pilot_v1](../design/source-quality-pilot-v1.md).
Their decoded Float32 samples match the reviewed native terrain exactly; the
existing source selections need no repeat visual review.

This first path preserves an existing aligned survey grid. It accepts 1–16
nonoverlapping review footprints sharing provider, collection, survey, CRS,
vertical datum and native spacing. Together they must cover a rectangular target
grid completely. This supports assembling adjacent selected areas within one
survey before training crops are extracted. It does not resolve cross-survey
offsets, overlaps, repairs or resampling. Those operations need their own evidence
and policies; incompatible inputs are rejected.

## Describe and build the region

Write a strict JSON request beside the input quality files, for example:

```json
{
  "schema_version": 1,
  "inputs": [
    {
      "quality_path": "candidate/quality.json",
      "curation_record_sha256": "COPY_THE_CURRENT_ACCEPTED_RECORD_SHA256"
    }
  ],
  "grid": {
    "origin": [700000, 2000010],
    "size": [20, 10],
    "spacing_m": [1, 1]
  },
  "masked_fill_m": 0.0
}
```

These coordinates are a synthetic example. Use the actual north-west **outer
corner**, column/row cell counts and positive east-west/north-south spacing from
the inspected source. Cell centres lie half a cell inside the boundaries; rows
run south. Every source origin must align exactly to whole target cells, and the
target spacing must equal the native spacing. The implementation derives an
integer offset and exactly reconstructs the source origin; decimal spacing such
as 0.51 m needs no added alignment tolerance. The request does not specify a
different CRS or invoke an implicit warp. The target is limited to 4,096 cells per
side, with 0.5–1 m spacing. Quality inputs retain the existing 2,048-cell side limit.

`masked_fill_m` is an explicit finite Float32 value, used everywhere water or
invalid/unsupported ground is masked out. It is not a water-surface elevation,
bed elevation or valid land. The example chooses zero. Authored water surfaces
remain a later world-composition input.

```sh
topography prepare build configs/runtime.example.toml data/review/region.json
```

The command prints the resulting manifest path under the configured `prepared`
root. It verifies current eligibility before reading samples, rechecks evidence
after loading, and verifies eligibility again before publication. The short
catalog registration transaction requires the same decision heads still to be
current. Changed or superseded input fails rather than reusing old permission.

## Inspect the result

The output directory is named by the SHA-256 of `manifest.json`. It contains:

| File | Meaning |
| --- | --- |
| `elevation.tif` | Float32 metres; supported valid land retains its exact bits, masked cells use the declared fill |
| `validity.tif` | 255 usable terrestrial elevation / 0 masked |
| `provider-validity.tif` | Original provider validity, kept distinct from terrain quality |
| `water.tif` | 255 water / 0 terrestrial; differs from the input classification's 1/2 encoding |
| `quality.tif` | Original 0 unknown / 1 supported / 2 unsupported classifications |
| `repairs.tif`, `features.tif` | Original binary masks; repaired input is currently excluded by eligibility |
| `quality-N.json` | Exact captured assessment for input N |
| `manifest.json` | Grid, survey/datum, masks, hashes, source decisions, class IDs, placements, fill and tool versions |

All masks are Byte rasters on the same grid. Validity is a separate raster, with
no reserved elevation code or hidden nodata mask. Zero can also be valid land;
consumers must consult validity. Class IDs label each input's destination window;
there is no per-cell class editor or training split yet.

GDAL's [VRT native pixel mapping](https://gdal.org/en/stable/drivers/raster/vrt.html)
and [GeoTIFF translation](https://gdal.org/en/stable/programs/gdal_translate.html)
perform composition without interpolation. Publication verifies all composed
sample bytes against the disjoint input bands and checks grid/type/unit metadata.
Compression is lossless DEFLATE. No averaging, smoothing, depression filling or
underwater reconstruction occurs. Human review still needs to judge adjacent
features and survey artifacts; no invented slope/seam tolerance is applied.

```sh
topography prepare verify configs/runtime.example.toml data/prepared/MANIFEST_SHA256/manifest.json
topography curate check configs/runtime.example.toml
```

`prepare verify` returns JSON with integrity, registration and current-parent
status. Exit 0 requires intact files, registration and unchanged source decision
heads. An intact orphan or superseded decision returns JSON and exit 1; corrupt
files or invalid contracts fail with a stderr diagnostic. Historical products
remain preserved even after source rejection. A later training-release consumer
must recheck current status and perform its separate geographic split/release
review; a prepared product never grants training admission by itself.

## Publication, recovery and backup

Files are staged beneath the prepared root, validated and synced before their
complete directory is published. The manifest publishes with the files. SQLite
registration follows, so a crash can leave an intact, unregistered directory.
Retrying the identical request regenerates/compares the deterministic result,
verifies existing files and registers it only if the source heads remain current.
Corrupt existing outputs are never replaced. Incomplete reserved/staging
directories need explicit inspection/housekeeping; existence is not completion.

The first successful prepared registration migrates the existing catalog from
schema 1 to 2 in one transaction, adding `prepared_regions`. Curation records and
their immutable hashes do not change. The new code reads both versions; older
binaries that only support schema 1 will reject a migrated database. Back up a
quiescent catalog and its referenced files before moving between tool versions.
`curate check` includes registered prepared-file integrity. Inspect an unregistered
manifest with `prepare verify`; the curation orphan lists concern curation files.

No restore, garbage collection or output-acceptance command is added. Preserve the
prepared directory, curation catalog/objects, raw data, reviews and original mask
evidence separately. Curation's small portable export does not include prepared
rasters, and original raw/review locator relationships remain necessary for
complete history verification.

## Capacity and verification

The implementation keeps bounded review samples in memory and decodes composed
layers one at a time. The worst allowed input list can occupy several hundred
MiB plus GDAL's 64 MiB cache, output buffers and temporary files. Limits constrain
dimensions/counts, not peak RSS. It is a regional pilot implementation, not the
full-world windowed preparation engine. Confirm workload readiness before large
real jobs under [AGENTS.md](../../AGENTS.md).

`python3 scripts/check_preparation.py target/debug/topography` uses tiny artificial
surveys and declarations to verify Float32 bits (including valid signed zero),
water/invalid fill, all masks, grid alignment, gaps/overlaps, repeated publication,
catalog migration, orphan registration, corrupt-file refusal and later rejection.
It creates no real acceptance and is included in `bash scripts/check.sh`.
See the [prepared contract](../../schemas/prepared-region-v1.md).
