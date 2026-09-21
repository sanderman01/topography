# Zion canyon review batch

2026-09-21. The queued Zion source finished downloading. Two disjoint native
2,048 × 2,048 m crops now show winding canyon walls and branching passages for
whole-tile gallery review. They have no human selection or preparation admission.

The source is `USGS_one_meter_x32y414_UT_ZionNP_QL2_2016`, selected from actual
USGS results for the Zion Narrows example in [locations](../locations.md).
Its 393,181,417-byte asset has SHA-256
`c9d67b7bf22bb819cdaa6b631d0880a1761569af64b16e61927a20af84e64ba4`.
The original discovery and first review receipts remain preserved. The final
review declaration records metre units, native 1 m spacing and NAVD88 from the
exact product description, the actual raster CRS, and the project acquisition
range from the [Utah provider's Zion description](https://gis.utah.gov/products/sgid/elevation/lidar/).
That November 2015–July 2016 range covers the combined QL1/QL2 project; the exact
tile's flight dates remain unknown. Its review receipt is
`88912ca879769bcb3876715f42a10e99a1476c2b597c52294170f0565e37c256`.
Source metadata and its archived evidence are in `data/review/zion-source-metadata-v1/`.

The raster uses EPSG:26912, NAD83 / UTM zone 12N, with native north-up origin
319,994 E, 4,140,006 N. The provider's rectangular footprint includes a large
northern void. Crop selection inspected the actual source hillshade and excluded
that void. Neither resampling nor elevation repair was applied to the crops.

| Gallery tile | Native source window, x/y/width/height | Height range, metres |
| --- | --- | ---: |
| Winding southern canyon | 6000 / 7400 / 2048 / 2048 | 1,389.25–2,101.19 |
| Branching northern canyons | 4900 / 4000 / 2048 / 2048 | 1,446.50–2,073.74 |

All 8,388,608 cells are finite and provider-valid, and both metric previews are
enabled. Native hillshades were inspected. Exact numeric comparisons are retained
in `data/review/zion-native-verification-v1/native-comparison.json`.
This establishes preservation of supplied values, not measured canyon-floor
accuracy. Supplier QA, river masks and dataset permission remain technical
qualification work after selection. The heightfield cannot represent overhangs.

Titles describe visible morphology rather than asserting exact named junctions
or passable flight routes. The [NPS Narrows description](https://www.nps.gov/zion/planyourvisit/thenarrows.htm)
and [canyon map](https://www.nps.gov/zion/planyourvisit/upload/Narrows-Map-2017.pdf)
provide geographic context; they are not elevation-quality evidence.

Retained inputs and outputs:

- Source queue and completed download: `data/staging/location-shortlist-v1/download-targets.json`
  and `data/review/location-shortlist-download-v1/`.
- Final crop queue: `data/staging/location-shortlist-v1/zion-review-v2.json`.
- Selection overview and rationale: `data/review/zion-selection-v1/`.
- Final native review crops and gallery input: `data/review/zion-review-v2/`;
  the initial `data/review/zion-review-v1/` remains preserved.

The original numeric dataset, accepted terrain and prior gallery votes remain
separate from this review batch. No model job was started.
