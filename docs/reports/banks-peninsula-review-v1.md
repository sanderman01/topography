# Banks Peninsula source expansion

2026-09-21. Two public LINZ Banks Peninsula 2023 DEM assets are imported, with
four native 1,536 × 1,536 m review crops. These are new geographic coverage beyond
the accepted Puerto Rico collection. The owner subsequently accepted the three
inland crops and rejected the coastal crop. The selected crops are now
[qualified and prepared](banks-peninsula-qualification-v1.md); training admission
remains separate.

The [LINZ collection](https://nz-elevation.s3.ap-southeast-2.amazonaws.com/canterbury/banks-peninsula_2023/dem_1m/2193/collection.json)
identifies the 1 m DEM product, capture dates, providers and CC BY 4.0 declaration.
Its original STAC records and checksums are retained. The provider-linked
[survey record](https://portal.opentopography.org/datasetMetadata?otCollectionID=OT.022024.2193.2)
documents NZTM2000 horizontal metres, NZVD2016 vertical metres and source credits.
Dataset-specific permission and quality evidence is retained with qualification.

| Source tile | Bytes | Published and verified asset SHA-256 |
| --- | ---: | --- |
| `BX24_10000_0504` | 64,196,499 | `691e1d5fb0869ec7bb5af229089981a6f8660616d82e66adfb12de4a39ffc5f1` |
| `BX25_10000_0502` | 55,274,229 | `b3781517b9817019581370efb9d60d27c7c4f88449c03624d3074296c62f55e9` |

Raw receipt identities are
`8c848f5f932d0fc975f203a818eb51049725911159f30a88d2c898e8f9575510` and
`a35234db765490f53976964e7dd67c088c46a34ec1c0426608f1d6ee91614be9`.
The 4,800 × 7,200-cell sources have native 1 m spacing. Their provider LERC encoding
declares a 0.001 m maximum storage error at full resolution and 0.1 m for overviews.
Crops read full resolution and retain its decoded Float32 values exactly; neither
encoding setting establishes measured survey accuracy.

Three crops have 2,359,296 finite provider-valid cells each. The southern crop
from the eastern source has 2,158,953 finite provider-valid cells and 200,343
provider-invalid cells near its coastal edge. That edge remains explicit in the
preview mask. No missing-cell fill or terrestrial-quality claim was invented.
The gallery labels the three inland crops provisionally as volcanic terrain and
the coastal crop as dissected coast; these broad selection labels are not
fine-feature accuracy claims.

Two compatibility fixes were necessary in the existing tools. The system GTiff
reader cannot open this LERC encoding, while GDAL's installed read-only LIBERTIFF
reader can. Import now tries that reader after a primary-reader failure, records
the actual reader and selects it explicitly for raw crop extraction. Validation
still requires a self-contained, single-band Float32 GeoTIFF with internal
coordinates. Raw compressed bytes are preserved.

The metric preview check also now follows GDAL's raster-to-CRS axis mapping.
EPSG:2193's authority axes are northing/easting, while these rasters map x/y to
easting/northing with `[2,1]`. This is accepted alongside ordinary `[1,2]` metric
grids without transposing terrain or changing any coordinate. The first unvoted
previews are preserved; corrected previews use a new run directory.

Target lists and original metadata:
`data/staging/source-expansion-linz-v1/`. Original acquisition run:
`data/review/source-expansion-linz-batch-v1/`. The corrected review queue is
`targets-review-v2.json`, with run `data/review/source-expansion-linz-batch-v2/`.
The latter reuses verified receipts without downloading the assets again.
Independent source-window comparisons and retained checks live with the runs.
All 9,437,184 native Float32 samples and provider masks match exactly in the
corrected previews, which all have metric 3D enabled. Gallery v5 combines those
four then-pending tiles with the fifteen earlier tiles; no automated votes were cast.

At the initial gallery checkpoint, the Mount St. Helens queue had retained
218,103,808 of 315,921,778 bytes. See the current [progress checkpoint](../progress.md)
for its later resume. Waioweka metadata remains a discovery lead only.
All acquisition uses public downloads without correspondence.
