# Milford Sound review batch

2026-09-21. Three native 2,048 × 2,048 m crops are ready for whole-tile review.
They add fjord walls and glacial-valley terrain from the Milford Sound example in
[locations](../locations.md). They have no owner acceptance, technical
qualification or training admission yet.

The queued batch downloaded two public LINZ Southland 1 m DEM assets, totaling
64,225,962 bytes. Both match their published SHA-256 values. The files contain
missing-data areas, so selection used their actual coverage and terrain previews.
All selected windows are finite and provider-valid.

| Crop | Source tile | Native window `[x, y, width, height]` | Height range, metres |
| --- | --- | --- | ---: |
| Northern fjord wall | CB08_10000_0104 | `[2600, 2300, 2048, 2048]` | −0.50–1,075.51 |
| Fjord head | CB08_10000_0104 | `[2752, 4500, 2048, 2048]` | −0.50–1,160.38 |
| Adjoining glacial valley | CB08_10000_0105 | `[0, 4500, 2048, 2048]` | 9.76–1,292.05 |

The last two crops are adjacent on the same survey grid, spanning a combined
4,096 × 2,048 m area. They remain separate review choices. The northern crop
does not overlap them. This covers part of the fjord head and adjoining valley,
not the complete Milford Sound landscape.

Exact raster inspection establishes EPSG:2193, metre coordinates, mapped axes
`[2, 1]` and north-up 1 m cells. The distributor identifies the Southland survey's
vertical reference as NZVD2016 and its collection period as December 2020–January
2024; exact tile flight days remain unknown. It also records CC BY 4.0 and
Environment Southland attribution requirements. These declarations and the
public survey report are archived from the
[Southland collection record](https://portal.opentopography.org/datasetMetadata?otCollectionID=OT.042023.2193.3).
Training permission is left unassessed for later curation.

Basic source metadata was completed before publishing the reviews. Both enriched
receipts retain their original asset bytes. GDAL's existing LIBERTIFF path reads
the LERC-compressed files. Native review adds no resampling or elevation repair.
Independent comparisons verified all **12,582,912 Float32 samples**, explicit
provider-validity cells and native crop transforms against the source windows.

Water remains unclassified in these review artifacts. Provider water-surface
values are not underwater ground; accepted crops will need separate masks during
qualification. Heights alone do not identify water. No flight-clearance or
overhang-fidelity claim is made.

Retained evidence:

- Download queue: `data/staging/fjord-shortlist-v1/download-targets.json`.
- Completed downloads: `data/review/milford-download-v1/`.
- Archived source metadata and declarations: `data/review/milford-source-metadata-v1/`.
- Selection, native windows and exact comparisons: `data/review/milford-selection-v1/`.
- Review queue: `data/review/milford-selection-v1/review-targets.json`.
- Review artifacts and gallery input: `data/review/milford-review-v1/`.

Gallery v8 appends these three pending choices to the existing 28 entries without
changing those entries or their decisions. The live gallery shows 26 Yes, 2 No
and 3 pending. Chromium inspection passed without voting, and all 65 curation
records, 31 heads and 25 prepared entries were unchanged by inspection.
Publication and preservation evidence is retained under
`data/review/canyon-fjord-continuation-v1/`.
