# Great Sand Dunes and Badlands review batch

2026-09-21. Four native 2,048 × 2,048 m crops are ready for whole-tile Yes/No
review. Two show Great Sand Dunes in Colorado, USA; two show the Badlands in
South Dakota, USA. They introduce dunes and badlands candidates alongside the
five currently prepared terrain families. They have no owner acceptance,
technical qualification or training admission yet.

The [source discovery report](family-expansion-discovery-v1.md) records selection,
public permission, supplier QA and coordinate metadata. The batch tool downloaded
both selected USGS 3DEP assets, totaling 556,953,224 bytes, and retained immutable
receipts with local SHA-256 hashes and HTTP ETags. No provider-published SHA-256
was available. The original partial download and superseded target list remain
preserved; no downloader code changed.

| Crop | Source tile suffix | Native window `[x, y, width, height]` | Height range, metres |
| --- | --- | --- | ---: |
| Western dune ridges | `x45y418` | `[400, 300, 2048, 2048]` | 2,466.42–2,619.43 |
| Eastern dune margin | `x45y418` | `[2448, 300, 2048, 2048]` | 2,457.52–2,642.39 |
| Western escarpment | `x70y486` | `[1800, 7950, 2048, 2048]` | 831.47–918.01 |
| Branching gullies | `x70y486` | `[3848, 7950, 2048, 2048]` | 827.81–910.67 |

Each pair is adjacent on its source grid and spans 4,096 × 2,048 m. The Badlands
windows follow the eroded southern wall of the selected source, retaining its
transition to flatter terrain. The dunes windows avoid the source's eastern
missing-data margin. The previews show striping on some dune slopes; retain this
observation for technical qualification rather than treating visual selection
as proof of measurement fidelity. No smoothing or elevation repair was applied.

Direct comparisons verified all **16,777,216 Float32 samples**, explicit
provider-validity cells, native crop transforms and coordinate metadata against
the immutable source windows. All selected cells are finite and provider-valid.
The native EPSG:26913 grids retain their fractional origins, metre coordinates,
north-up rows and 1 m spacing. Supplier processing uses EPSG:6342; that declaration
does not replace the distributed raster CRS. NAVD88 heights remain in metres.
Water and terrestrial quality remain unclassified in these review artifacts.

Retained local evidence:

- Final download queue: `data/review/family-selection-v1/download-targets-v2.json`.
- Download receipts and completion markers: `data/review/family-download-v2/`.
- Review queue: `data/review/family-selection-v1/review-targets-v1.json`.
- Direct native comparison: `data/review/family-selection-v1/native-comparison.json`.
- Review artifacts and gallery input: `data/review/family-review-v1/`.
- Gallery snapshot: `data/review/library-gallery-v9/gallery.json`.

Gallery v9 preserves the previous 31 entries and appends four pending choices:
35 tiles total, with 29 Yes and 2 No. Chromium inspection passed without voting.
All 71 curation records, 34 heads and 28 prepared entries remained unchanged
during publication and inspection. The manifest SHA-256 is
`3fc06b40d3c859cffc897d27e83f7f809c3a0be890058f16913bfc4f87c5d919`.
Publication, browser and preservation evidence is retained under
`data/review/milford-family-continuation-v1/`. Qualification and preparation require the owner's
whole-tile selections and separate technical checks. The existing numeric
release remains unchanged.
