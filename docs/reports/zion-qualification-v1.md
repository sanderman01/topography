# Zion qualification

2026-09-21. The two gallery-selected Zion crops, `lower-corridor` and
`branching-canyons`, are technically qualified and prepared under
`supplier_qa_pilot_v1`. Their original whole-tile Yes decisions remain the human
selection. The additional source, water and quality findings are Codex technical
assessments and do not claim another vote or human technical certification.

Both crops use raw receipt
`88912ca879769bcb3876715f42a10e99a1476c2b597c52294170f0565e37c256`
and exact asset
`c9d67b7bf22bb819cdaa6b631d0880a1761569af64b16e61927a20af84e64ba4`.
The exact TNM tile is
`USGS_one_meter_x32y414_UT_ZionNP_QL2_2016`, from work unit
`UT_ZionNP_QL2_2016`. Existing source declarations and native verification pin
the unchanged Float32 1 m NAD83 / UTM zone 12N grid (EPSG:26912), NAVD88 metre
heights, crop windows and decoded elevation values. The source record is already
complete apart from its deliberately unresolved training-permission field, so no
metadata-only successor is required.

UGRC identifies the 2015–2016 Zion project as QL1/QL2 lidar acquired by USGS and
the National Park Service, with classified point clouds and 0.5 m and 1 m
bare-earth DEMs. Its retained Woolpert report documents automated ground
filtering, manual removal of anomalies from the ground class, survey-control
checks, bridge breaklines and DEM production. The combined project results report
0.152 m bare-earth DEM non-vegetated vertical accuracy at 95 percent confidence,
0.272 m vegetated vertical accuracy at the 95th percentile and 0.051 ft
between-swath RMSDz. These project statistics and nominal spacing do not prove
independently measured local fine-feature accuracy or an exact acquisition flight
for this tile.

The exact TNM discovery item identifies the distributed tile as a 3DEP product
and states that all 3DEP products are public domain. The current official USGS
3DEP page also says its products are free and without use restrictions. Codex
applies that dataset-specific evidence to intended terrain-model training while
retaining USGS, NPS, UGRC and Woolpert credit, source identity, links and
modification history. This is an implementer assessment rather than an agency
letter specific to machine learning.

The supplier report says no project hydro feature met its hydroflattening
criteria, so supplier processing cannot establish crop water boundaries. A
bounded USGS NHD query identifies perennial and ephemeral canyon flowlines, and
bounded public USGS imagery plus each native hillshade confirms that the
perennial lines follow the visible canyon-floor river route. Shadows, vegetation
and differing acquisition dates do not support precise banks. The masks
therefore classify only a 3 m radius around perennial NHD centerlines (FCode
46006): a conservative 6 m interior corridor rather than a claimed survey-date
wetted edge. Ephemeral NHD lines and every cell outside that supported interior
remain terrestrial. The mask does not assert channel width, surface level or
bathymetry.

| Crop | Qualified record | Prepared manifest | Water cells | Valid terrestrial cells |
| --- | --- | --- | ---: | ---: |
| Lower corridor | `bcacc161e3f23205535c762f45d517946870c63a7176c654567a1789a97c1153` | `8b10897b9230975e342ab15c5e5bd2aa65ef9fd77ee6a08a606b1c47ef97eb26` | 35,315 | 4,158,989 / 4,158,989 |
| Branching canyons | `7c6fd5d1adcc94f4246cefda6f30058f15986154ddf235b5209a25a163d662ec` | `ee80c81ffdf336bfc33aa88dac640740dd849e6be404cf149e65df694ceb7404` | 25,369 | 4,168,935 / 4,168,935 |

Both deterministic assessments report zero blockers, zero project repairs and no
optional feature annotations. They classify 100 percent of terrestrial cells as
finite, provider-valid supported ground under the pilot supplier-QA rule. Both
CLI prepared verifications report intact, registered and current outputs.
Independent decoding verifies all 8,327,924 terrestrial Float32 cells bit for bit
against the selected native reviews, all 60,684 water cells as deterministic zero
fill, and the expected water, validity, provider-validity, quality, repair and
feature masks. Evidence, masks, portable curation exports and numeric proofs are
retained under `data/review/zion-qualification-v1/`. Preparation remains
unreviewed output and does not create a training release or approve generated
terrain.
