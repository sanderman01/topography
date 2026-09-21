# Pilot tile batch 3

Four new whole-tile candidates, prepared for selection on 2026-09-21. They add
another gorge area and another river/floodplain area; nearby crops from each area
must stay together in a future geographic split. No decisions or training split
have been assigned to this batch.

Open the local [four-tile review page](../../data/review/pilot-batch-3-2026-09-21/review-batch.html).
Reply yes/no for tiles 1–4, or “all yes”. No explanations or feature annotations
are required. The eight already accepted regions need no repeat source review.

| Number | Tile | Native window: x, y, width, height | Height range (m) | Candidate family |
| --- | --- | --- | --- | --- |
| 1 | Guajataca canyon corridor | `600,150,1024,1024` | 0.423–129.851 | Gorge, class 2 |
| 2 | Guajataca bend and walls | `200,1450,1024,1024` | 0.868–104.423 | Gorge, class 2 |
| 3 | Arecibo upper meander | `0,450,1024,1024` | 2.598–8.620 | River/floodplain, class 9 |
| 4 | Arecibo lower meander | `50,1750,1024,1024` | 3.608–10.436 | River/floodplain, class 9 |

Each crop is 512 × 512 m with native 0.5 m samples, projected EPSG:6566 and
declared PRVD02 metre elevations. Guajataca shows steep enclosing walls and a
valley corridor; Arecibo provides broad low-relief river terrain. These are
candidate labels based on regional context and inspected morphology, not measured
fine-feature fidelity. Water classification and any deterministic masked fill
belong to later technical preparation, if the owner selects these tiles.

## Public sources and provenance

The official [Guajataca-area description](https://www.discoverpuertorico.com/regions/west/quebradillas)
and [USGS Arecibo flood study](https://pubs.usgs.gov/publication/wri20014247)
provide regional leads. Exact source selection used the retained public NOAA
EPSG:6566 DEM index and source hillshades. No agency contact or point-cloud
processing was involved.

| Area | NOAA relative asset | Bytes | Raw receipt SHA-256 |
| --- | --- | ---: | --- |
| Guajataca | `prvi_f/USGS_OPR_PR_PRVI_F_2018_19QGA44507050.tif` | 28,169,127 | `83ce2f62d24ac95076e2a79392cdcf22ebf61e725a09579366cd3e4c91c63cd6` |
| Arecibo | `prvi_f/USGS_OPR_PR_PRVI_F_2018_19QGA71506600.tif` | 27,579,217 | `40b811386854ecf9573fce48a0e5fca29f0be279f9920073782f60a0ac2acfa6` |

Source examples in [`catalog/candidates/pilot-batch-3/`](../../catalog/candidates/pilot-batch-3/)
contain exact public URLs and retained collection-metadata references.
[Crop envelopes](../../catalog/areas/pilot-batch-3-crops.geojson) record geographic
footprints, source/review identities, windows and unset decisions.
The acquisition CLI enforced a 40 MiB per-asset cap, strong ETags and exact ranges.
Copy source examples beside the original NOAA `original.xml` before acquisition,
following the [existing workflow](../workflows/dem-acquisition.md).

Retained local directories are
`data/staging/pilot-batch-3-2026-09-21/` for acquisition inputs and display overviews,
and `data/review/pilot-batch-3-2026-09-21/` for native crops, interactive previews,
thumbnails, the pinned batch page/JSON and verification reports. Preserve the
presented page when later capturing decisions.

The existing `scripts/check_review.py` compared all 4,194,304 Float32 samples,
provider masks and browser payloads with the original source windows, including
grid/CRS and metrics. All four crops have finite provider-valid samples throughout.
These checks establish extraction fidelity, not human acceptance or usable
terrestrial coverage. Qualification will use
[supplier_qa_pilot_v1](../design/source-quality-pilot-v1.md) if selected.
