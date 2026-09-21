# Mount St. Helens qualification

2026-09-21. Gallery v6 records three whole-tile **Yes** selections for Mount
St. Helens. Technical qualification under `supplier_qa_pilot_v1` prepares the
two flank crops and excludes the southern crater-rim crop. Selection and
preparation therefore have different counts: **three selected, two prepared**.

The exact TNM tile is
`USGS_1M_10_x56y512_WA_FEMAHQ_2018_D18`, raw asset SHA-256
`38ea2810ec62b42194e273bf510b16eba0df7a46c7565bc0b75ccb6a664e9bce`.
The retained exact tile XML, actual TNM `vendorMetaUrl` listing and published
Washington FEMA 3DEP 2018 supplier report identify the shared source. The report
documents QL2 ground classification, automated and manual processing, flight-line
calibration, triangulated 1 m bare-earth DEM production, hydroflattening and
0.119 m non-vegetated vertical accuracy at 95 percent confidence against 128
withheld checkpoints. Its cumulative QL2 acquisition range is 2018-08-05 through
2019-05-06; it does not establish the exact acquisition day for this tile, and no
date is inferred from the project name.

Exact product metadata and raster inspection establish a native 1 m NAD83 / UTM
zone 10N grid (EPSG:26910), metre heights and provider-declared NAVD88 vertical
datum. The exact TNM product description calls 3DEP products public domain, and
the official USGS 3DEP page inspected on the review date states that all 3DEP
products are free and without use restrictions. Codex applies that basis to
intended terrain-model training while retaining USGS/FEMA/NRCS/QSI credits,
source identity, provenance, modification history and no-endorsement language.
This is an implementer assessment, not an ML-specific agency letter.

The original receipt omitted admission-required survey, date, horizontal-CRS and
processing fields. The two eligible flanks therefore use a metadata-only successor
receipt, `770fa285a67de27f9810be35c3ec52c3a3a98784baee5c7a0314be57f093ecad`.
It preserves the exact asset and crop windows while adding only evidence-backed
metadata. The original review artifacts, heads and gallery vote bytes remain
preserved. The owner's original Yes is carried to each successor as explicit
evidence for the exact unchanged terrain; no fresh vote or human technical
certification is claimed. The independent integration proof in
[landmark continuation](landmark-continuation-v1.md) checks asset identity,
windows, CRS/axis mapping, vertical interpretation and exact TIFF identities;
its retained proof SHA-256 is
`ace43282f4a51b5bfde05f320cc1111ed883c91c1b558800243812f9f13c043c`.

| Gallery selection | Technical result | Record SHA-256 | Prepared manifest SHA-256 |
| --- | --- | --- | --- |
| Southern crater rim | Excluded | `4ebfe7ec2d2e7aafbfe54eca47b0b9b0b8b8da0384e0251c69bd79a37ab1e442` | None |
| Eastern gullies | Eligible and prepared | `cb61f1d9630908fa36a6eb9eae8c5fbd040180b47804fc554f198356091c2253` | `3f44dbcbce3a0756cbcc0177357d85a0bcc31d40cda7948e2abc8c6b4f10ba98` |
| Southwestern ravines | Eligible and prepared | `4477c7733839cd6329ec4042d9b19563a0bcfaae124d8c714c5aa0b73f2708fc` | `0dd739b2ee58ec872bab865eb5fb1dfeb04f6554cc337ad689f4950662a2d630` |

The crater crop visibly contains the USGS-documented Crater Glacier around the
lava domes. A lidar/DEM surface over glacier or seasonal snow does not establish
measured ground beneath it. No authoritative crop-aligned glacier mask supports
retaining the required 99.5 percent usable terrestrial ground, so the conservative
quality mask excludes the whole crop. The glacier is terrestrial for the water
mask and is not classified as river water. Its original Yes remains a morphology
selection, and no replacement surface or project repair was invented.

No defining snow/ice surface, open water or concrete local defect was identified
in the two native flank hillshades. All 4,194,304 cells per flank are finite,
provider-valid and admitted as terrestrial under supplier QA, with zero project
repairs and no optional feature annotations. Seasonal snow/ice uncertainty remains
an explicit source limitation; supplier QA and nominal spacing do not prove
measured local fine-feature accuracy.

Independent native verification decoded and compared 8,388,608 Float32 cells.
Each successor elevation and validity TIFF is byte-identical to its originally
selected crop; each prepared Float32 cell is exact, and provider validity, water,
quality, repairs and feature masks have the expected complete values. Both CLI
prepared verifications pass. The enriched raw receipt rehashes successfully.
Evidence, masks, portable curation exports, exact equivalence and verification
records are retained under `data/review/st-helens-qualification-v1/`.
