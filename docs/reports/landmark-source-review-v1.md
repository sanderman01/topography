# Mount St. Helens and Grand Canyon review batch

2026-09-21. Four new native crops await whole-tile gallery selection. Three show
Mount St. Helens in Washington, USA; one shows a Grand Canyon side canyon in
Arizona, USA. They have no human acceptance or technical preparation admission.

The owner pointed out [the example locations](../locations.md). That list now
explicitly guides discovery. Grand Canyon and Zion Narrows were selected to add
deep canyon morphology outside the current Puerto Rico and Banks Peninsula
surveys. USGS discovery returned two Grand Canyon and five Zion candidates at
1 m. A bounded 600 MB queue chooses one source from each, keeping original
responses and source identities. Other examples remain marked unqueried in this
pass; no coverage claim is made for them.

The Mount St. Helens source was already queued and the owner explicitly requested
its resume. It completed at 315,921,778 bytes, with asset SHA-256
`38ea2810ec62b42194e273bf510b16eba0df7a46c7565bc0b75ccb6a664e9bce` and raw receipt
`10d39ba242193053284ffc87b89d80f758c1f6b0fce51c6622d96568eb4eda0f`.
The range timeout fix and actual retry are recorded in [progress](../progress.md).

Crop selection used the actual raster grid, whole-source hillshade and
[USGS's published volcano location](https://www.usgs.gov/volcanoes/mount-st.-helens).
The earlier exploratory offsets were replaced in a new queue before any human
review. The native 1 m source is NAD83 / UTM zone 10N, with NAVD88 metre heights
declared in the exact product metadata. Crops cover a southern crater rim and
interior, eastern gullies, and southwestern ravines. The crater crop does not
contain the entire crater.

The Grand Canyon source `USGS_1M_12_x39y400_AZ_GrandCanyonNP_2019_B19` is
107,186,344 bytes. Its bounding rectangle intersects the requested area, but
most of the actual raster is provider-invalid. A complete northeastern crop
contains branching cliffs and a side canyon. This source does not establish
continuous coverage of the Colorado River corridor. Source metadata declares
native 1 m NAD83 / UTM zone 12N and NAVD88 metre heights. Importing a separate
declaration carrying those metadata-derived units preserves the original bytes
and discovery receipt; its review receipt is
`d6ca2357a2646641744b8c4f8312a4827a4dde1b8292461947ca4abd4e780d0c`.

| Crop | Native dimensions | Height range, metres |
| --- | --- | ---: |
| St. Helens southern crater rim | 2,048 × 2,048 | 1,778.94–2,537.78 |
| St. Helens eastern gullies | 2,048 × 2,048 | 1,351.67–2,473.07 |
| St. Helens southwestern ravines | 2,048 × 2,048 | 1,176.32–2,089.26 |
| Grand Canyon northern side canyon | 1,536 × 1,536 | 1,028.84–1,727.64 |

All four crops have finite provider-valid values everywhere and metric 3D enabled.
Independent full-resolution reads compared **14,942,208 Float32 samples** and
every provider mask exactly. These are numeric checks, not proof of local ground
quality or heightfield support for overhangs. Supplier QA, terrain/water masks and
dataset-specific admission remain separate follow-up work for selected crops.

Retained artifacts:

- St. Helens acquisition: `data/review/source-expansion-batch-v1/`;
  revised crop queue: `data/staging/source-expansion-v1/targets-review-v2.json`;
  native previews: `data/review/st-helens-review-v2/`.
- Named-location shortlist, provider responses and queues:
  `data/staging/location-shortlist-v1/`; downloads:
  `data/review/location-shortlist-download-v1/`; Grand Canyon previews:
  `data/review/location-shortlist-review-v1/`.
- Exact source/crop comparisons: `data/review/landmark-crop-verification-v1/`.
- Gallery input: `data/review/library-gallery-input-v6.json`; immutable gallery:
  `data/review/library-gallery-v6/gallery.json`.

Chromium inspection passed without voting; all 19 existing catalog heads, 46
records and 17 prepared entries remained unchanged. Gallery v6 has 17 accepted,
two rejected and four pending tiles.

Zion's 393,181,417-byte asset remains a download in progress at this checkpoint;
it is not part of this four-crop review. The source lists and downloader handle
resume, and previous accepted terrain and gallery decisions are preserved.
