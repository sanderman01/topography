# Milford qualification

2026-09-21. The three gallery-selected Milford crops, `north-fjord-wall`,
`fjord-head` and `glacial-valley`, are technically qualified and prepared under
`supplier_qa_pilot_v1`. Their original whole-tile Yes decisions remain the
human selection. The source, water and quality findings are Codex technical
assessments and do not claim another vote or human technical certification.

`north-fjord-wall` and `fjord-head` use raw receipt
`c7fcca164f69fc404f388dc717a418b9148b314281ff7b2a6e899345c5728e94`
and LINZ tile `CB08_10000_0104`. `glacial-valley` uses raw receipt
`7f7050c521ddeb3dac1f1a8f18eadf93e83baab37e68641ec846e4786e9449f7`
and tile `CB08_10000_0105`. Existing native comparison pins all 12,582,912
Float32 values, provider-validity cells and 1 m EPSG:2193 grids exactly. The
retained collection metadata identifies NZVD2016 metre heights.

The archived Aerial Surveys report documents Optech Galaxy PRIME acquisition,
coverage and strip-overlap checks, automatic iterative ground classification
followed by manual correction, and Natural Neighbour interpolation from ground
points through a TIN to the raster DEM. It says hydro flattening followed part
7 of the January 2020 New Zealand National Aerial LiDAR Base Specification. The
archived report's Block 4 section gives 4 points/m² and reports 0.001 m mean vertical
difference, 0.043 m standard deviation and RMSE, and 0.084 m CI95 against
surveyed open-land checks. Exact tile-to-Block-4 membership was not established;
these statistics are archived report context. These project figures and nominal
grid spacing do not establish independently measured local fine-feature accuracy.

The exact archived OpenTopography collection metadata applies CC BY 4.0 and
names Environment Southland as licensor. Its broad grants permit sharing and
adaptation for any purpose, subject to attribution, a license link and an
indication of changes. Codex applies those terms to intended terrain-model
training while retaining credit to Environment Southland Regional Council and
LINZ (2024), DOI `10.5069/G9TM78B1`, Aerial Surveys, tile identity and
modification history. No separate machine-learning phrase or correspondence is
needed for this technical assessment.

One bounded public OpenStreetMap query retains coastline and mapped water
geometry for the three crop bounds. It includes coastline ways attributed to
the LINZ mainland coastline source and named Cleddau, Tūtoko and Bowen river
geometry. The native hillshades corroborate the mapped fjord edge and river
routes. The masks classify the supplier's exact -0.5 m hydro-flattened
fjord surface where the mapped coastline crosses these crops, the mapped
Cleddau water polygon, and a conservative 3 m radius around mapped river
centerlines in the two fjord crops. In `glacial-valley`, where the public
Cleddau/Tūtoko route crosses a visibly broad hydro-flattened open channel, the
bounded corridor is 75 m on each side of the mapped centerlines. The supplier
flat and mapped coastline together establish a water class; this is not a
general elevation threshold. The bounded corridors are not claimed survey-date
banks. The masks do not assert water level, submerged terrain or bathymetry.

The retained hydrography response is © OpenStreetMap contributors and identifies
the Open Database License (ODbL). Its geometry is used to derive these masks.
Portable evidence therefore retains OpenStreetMap attribution and the ODbL
source/license notice; downstream reuse must assess and satisfy any applicable
ODbL database obligations separately from the elevation collection's CC BY 4.0
terms.

| Crop | Qualified record | Prepared manifest | Water cells | Valid terrestrial cells |
| --- | --- | --- | ---: | ---: |
| North fjord wall | `823d1c642979b13497a580bf903c0834ad95e3f9f358a8376b3f1991b5840215` | `7a82cd03238fc56f62df2589cdeb90b0d6c9b810c929f078207677fb607f7859` | 402,817 | 3,791,487 / 3,791,487 |
| Fjord head | `cc0ac61227dd99de48a5c195ece8ad42905eb9c027d365ad400ad143ae6abfb5` | `56c68804c7bb1ea8d02e8b8cd9065fe78e9202d206a14048e2f1c6c088f54131` | 966,216 | 3,228,088 / 3,228,088 |
| Glacial valley | `e578313635c986551f1e4bff572b91346631e51d7ccbc9b7c6cad6b40eb5a4bb` | `14146395a642accd6c622d8930880d00f41febebc93761cd9a08360840eb88e3` | 513,668 | 3,680,636 / 3,680,636 |

All assessments report zero project repairs, no optional feature
annotations, no invalid or nonfinite terrestrial cells, and 100 percent valid
supported terrestrial coverage under the pilot supplier-QA policy. Each published
curation record carries 10 evidence objects, preserving the original human Yes
and identifying the additional findings as Codex assessments.

The CLI prepared verifications report all three outputs intact, registered and
current. Independent decoding verifies all 10,700,211 terrestrial Float32 cells
bit for bit against the selected native reviews, all 1,882,701 water cells as
deterministic zero fill, and the expected water, validity, provider-validity,
quality, repair and feature masks.

The masks, source findings, target list, download receipt, curation records,
portable exports, numeric proofs and reproducible scripts are retained under
`data/review/milford-qualification-v1/`. Preparation remains unreviewed output
and does not create a training release or approve generated terrain.
