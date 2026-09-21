# Grand Canyon expansion review candidates

Follow-up: the owner accepted all three crops. A gallery save bug required
[recovering those explicit Yes choices](gallery-vote-recovery-v1.md); technical
qualification remains pending. The following describes their publication state.

2026-09-21. Three additional Grand Canyon native-grid crops await whole-tile
Yes/No gallery review. No selection vote, technical qualification, curation or
dataset admission was created. The previously accepted crop from the adjacent
western source remains unchanged at native window `[8300, 200, 1536, 1536]`.

The western source was considered first. Although its southern valid strip can
fit several crops, much of the separated terrain available there is developed
plateau rather than varied canyon morphology. The expansion therefore uses the
adjacent public USGS tile
`USGS_1M_12_x40y400_AZ_GrandCanyonNP_2019_B19` (source ID
`66102e8ad34e633466504c6d`, declared size 328,774,898 bytes). Exact TNM metadata
identifies a native 1 m bare-earth DEM in NAD83 / UTM zone 12N with metre heights
referenced to NAVD88. Exact work-unit metadata identifies survey
`AZ_GrandCanyonNP_2019_B19 / AZ_GrandCanyonNP_2_2019`, collection from
2019-09-29 through 2019-10-05, Atlantic's ground filtering and manual editing,
0.5 m source DEM production, and USGS-NGTOC acceptance on 2022-06-24. The exact
TNM item and work-unit metadata support U.S. public-domain reuse; their identity,
attribution and temporal limits are retained.

The existing list-driven batch downloader completed the source with raw receipt
`a76471b72402da5a97b9c638868a67b537c561c9d4b61c7abf4a38dfe5f410f6`.
Before gallery review, a metadata-successor declaration produced receipt
`11afe2ce6ca0c158edb35ce3c3837ab3cdb22b8cc1282bac7b6b3be2e7fc4e7f`.
Both receipts resolve to the identical 328,774,898-byte asset, SHA-256
`84b35005c1b650af5f4370616a4f5c9186825944670a7ad0d88fcb86ee93ed92`,
and identical raster inspection. The first receipt and review outputs remain
preserved; the successor review is the final gallery source.

Selection used an 800 × 800 source-wide overview, followed by full-resolution
checks of each proposed window. Crop processing was sequential with a 64 MiB
GDAL cache and one GDAL worker.

| Candidate | Native window `[x, y, width, height]` | Height range, metres |
| --- | --- | ---: |
| Northwest layered cliffs | `[500, 500, 1536, 1536]` | 944.70–1,629.74 |
| Central river corridor | `[3600, 3600, 2048, 2048]` | 741.75–1,588.73 |
| Southeast river bend | `[7300, 7000, 1536, 1536]` | 749.93–1,266.34 |

The three windows do not overlap. They sample layered upper canyon walls, a
wide central corridor with the river and tributary slopes, and a lower-relief
river bend with opposing cliffs. Names describe visible morphology for review;
they are not feature annotations.

Independent full-resolution GDAL reads compared **8,912,896 Float32 samples**
byte for byte among the raw source windows, initial reviews and metadata-successor
reviews. Every provider mask also matched across all three; all 8,912,896 cells
are provider-valid and finite. The actual grid is 10,012 × 10,012 Float32 cells,
EPSG:26912, with 1 m spacing and geotransform
`[399993.999990934, 1, 0, 4000006.0000184234, 0, -1]`. The declaration records
horizontal and vertical metre units and NAVD88 (EPSG:5703), GEOID12B. These checks
establish identity, numeric copying and source validity only. They do not establish
terrestrial/water masks, local cliff-detail accuracy, fitness for terrain
overhangs, human acceptance or training admission.

The selected crops cover three separated parts of one 10,012 × 10,012 source
tile. They do not establish complete Grand Canyon, Colorado River corridor or
project coverage. The source-wide overview also shows an irregular
provider-invalid area along part of the western edge; none of the selected
windows intersects it.

Retained artifacts:

- Source and queues: `data/staging/grand-canyon-expansion-v1/`.
- Download receipt, overview and review batch:
  `data/review/grand-canyon-expansion-v1/`.
- Preserved initial gallery input and comparison:
  `data/review/grand-canyon-expansion-v1/gallery-input.json` and
  `data/review/grand-canyon-expansion-v1/native-comparison.json`.
- Final metadata-successor gallery input:
  `data/review/grand-canyon-expansion-v1/gallery-input-v2.json`.
- Raw/v1/v2 identity comparison:
  `data/review/grand-canyon-expansion-v1/native-comparison-v2.json`.

The final gallery input contains only these three pending candidates. It is intended
to be combined with the owner's next immutable gallery snapshot; gallery build
and service were deliberately left to that integration step.
