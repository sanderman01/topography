# Class 7–8 USGS source discovery

Discovery performed 2026-09-21 for the missing badlands (class 7) and dunes
(class 8) pilot families. This is a bounded source shortlist, not human terrain
acceptance, technical qualification, or training admission. Source selection uses actual raster overviews before publishing review windows.

## Selected bounded queue

The final download queue is
`data/review/family-selection-v1/download-targets-v2.json`. It contains two public
USGS 3DEP 1 m DEM assets totaling 556,953,224 advertised bytes, below a 600 MB
aggregate cap and 300 MB per-source caps.

| Family | Selected source | Advertised bytes | Selection basis |
| --- | --- | ---: | --- |
| Dunes (8) | `USGS_1M_13_x45y418_CO_SanLuisJuanMiguel_2020_D20`, ScienceBase `62171f45d34ec739b2dbf3c2` | 272,168,908 | Actual overview shows dune ridges in the northwest; select two adjacent windows away from the eastern missing-data margin. |
| Badlands (7) | `USGS_1M_13_x70y486_SD_Southwest_NRCS_SD_2018_D18`, ScienceBase `62e36b27d34e394b653652e2` | 284,784,316 | Actual overview shows an eroded wall and branching gullies along the southern edge; select two adjacent windows across that band. |

These areas extend the master plan's US dune-field and badlands acquisition
leads. They add two missing terrain families after the named Grand Canyon, Zion
and Milford examples from [locations](../locations.md). They are not substitutes
for all the locations that remain on that list.

The original queue under `data/staging/family-expansion-v1/` is preserved.
It selected Badlands `x70y485`, directly south of the final tile; overview
inspection showed weaker relief. That asset was not downloaded. Original source
inputs and partial dunes transfers remain intact. The new queue incorporates
corrected work-unit metadata and the raster-declared CRS before any gallery vote.
It prefers the same public bucket's documented regional S3 endpoint after slow
connections on the global endpoint; local transfer diagnostics are retained in
`data/review/family-transfer-diagnosis-v1/`. No downloader code changed.

## Numeric product and QA evidence

The National Map response describes both assets as project-based 3DEP 1 m
topographic bare-earth DEMs, derived exclusively from high-resolution lidar,
with metre horizontal and vertical units and NAVD88 elevations. USGS documents
project-based 1 m DEMs as UTM products made from QL2-or-better lidar, and states
that all 3DEP products are free of charge and without use restrictions:
<https://www.usgs.gov/3d-elevation-program/about-3dep-products-services>.

The exact TNM item points to vendor metadata collection
`CO_SanLuisJuanMiguel_5_2020`, whose report identifies WU_ID 216068. That report
documents acquisition from 2020-05-28 through 2020-10-04, QL2 collection,
NAD83(2011) / UTM zone 13N (EPSG:6342), NAVD88/GEOID18 metres, at least
2 pulses/m², and a one-metre 32-bit floating-point hydro-flattened bare-earth
raster exported from classified ground lidar. It also documents calibration,
classification, breakline and accuracy QA. Exact vendor index and report:
<https://prd-tnm.s3.amazonaws.com/index.html?prefix=StagedProducts/Elevation/metadata/CO_SanLuisJuanMiguel_2020_D20/CO_SanLuisJuanMiguel_5_2020>
and
<https://prd-tnm.s3.amazonaws.com/StagedProducts/Elevation/metadata/CO_SanLuisJuanMiguel_2020_D20/CO_SanLuisJuanMiguel_5_2020/reports/Lidar_Report/140G0220F0113_CO_SanLuisJuanMiguel_2020_D20_WU_ID216068_Lidar_Mapping_Report.pdf>.
These exact WU5 findings are included in the final source declaration. The
initial queue inputs remain unchanged as historical records.

The South Dakota Block 5 report documents QL2+ collection at 4 pulses/m²,
NAD83(2011) / UTM zone 13N, NAVD88/GEOID12B metres, classification with manual
inspection and peer review, and a one-metre hydro-flattened bare-earth raster
generated from class-2 ground points and 3D breaklines. Its QA includes coverage,
point-spacing and vertical-accuracy checks:
<https://prd-tnm.s3.amazonaws.com/StagedProducts/Elevation/metadata/SD_Southwest_NRCS_SD_2018_D18/SD_Southwest_B5_2018/reports/SD_Southwest_NRCS_SD_2018_D18_Block5_Lidar_Mapping_Report_195000.pdf>.
The public project collection report identifies Spring 2018, but the exact Block
5 flight-date range remains uncaptured and is stated that way in metadata.

USGS explains that downloadable-product XML metadata and WESM spatial metadata
carry product and work-unit detail, including acquisition dates, project links
and 3DEP conformance status:
<https://www.usgs.gov/ngp-standards-and-specifications/3dep-product-metadata>
and <https://www.usgs.gov/3d-elevation-program/3dep-spatial-metadata>.

## Native raster checks

Both selected distributed GeoTIFFs declare EPSG:26913 (NAD83 / UTM zone 13N),
1 m cells, 10,012 × 10,012 samples, Float32 elevation and north-up rows. This
raster declaration is retained separately from the supplier's EPSG:6342 processing
reference; the project does not reproject or snap their fractional native origins.
Public overview inspection alone is not an integrity or valid-ground claim.
Both full assets are now downloaded and verified, with immutable receipts under
`data/review/family-download-v2/`. Four native review crops passed exact sample,
validity and grid comparisons; see the [review batch](family-expansion-review-v1.md).

Owner choices and technical qualification remain separate from source discovery. Exact South Dakota Block 5 flight dates remain
uncaptured; the recorded date is the broader published Spring 2018 scope. Public
permission and successful processing do not create human terrain acceptance.
