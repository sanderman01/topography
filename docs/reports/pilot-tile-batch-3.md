# Pilot tile batch 3

The owner answered **“all yes”** on 2026-09-21 for these four whole tiles.
All four are selected, technically qualified and prepared. They add another gorge
area and another river/floodplain area; nearby crops from each area stay together
in the [proposed numeric pilot](numeric-pilot-proposal-v1.md).

Open the local [four-tile review page](../../data/review/pilot-batch-3-2026-09-21/review-batch.html).
The original page and pinned batch JSON remain unchanged as the presented
snapshot. The exact response and four decisions are retained in its `decisions-v1/`
directory. No repeat source review is needed.

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
fine-feature fidelity. Water is classified separately during technical preparation.

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
footprints, source/review identities, windows and current accepted decisions.
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
These checks establish extraction fidelity; human acceptance and technical
qualification are recorded separately under
[supplier_qa_pilot_v1](../design/source-quality-pilot-v1.md).

## Qualification and preparation

Codex inspected the exact USGS OPR metadata for both source tiles, the retained
NOAA collection lineage and supplier QA, and the published
[3DEP unrestricted-use statement](https://www.usgs.gov/3d-elevation-program).
The source-specific permission interpretation retains attribution, modification
history and no-endorsement conditions. This is Codex's technical/permission
assessment, not a new human legal vote or a claim of an agency ML letter.

Public USGS imagery and native DEM hydrography support the river masks. Codex
delineated polygons, retained river islands and rasterized them at native cell
centres. The polygons, imagery requests, original extracts and classification
previews are retained. Boundaries are pilot interpretations with display-resolution,
canopy and date limits, not independently surveyed half-metre water boundaries.
No elevation threshold, water-level authoring or bathymetry inference was used.

All remaining land is finite and provider-valid and passes the delegated
supplier-QA rule. Each crop has 100% usable terrestrial coverage, zero project
repairs and zero optional feature annotations. Water uses positive-zero masked
fill; ground elevation is unchanged. The four crops remain separate native
regions, without resampling or cross-survey joining.

| Tile | Usable land cells | Water cells | Prepared manifest SHA-256 |
| --- | ---: | ---: | --- |
| guajataca-corridor | 1,021,001 | 27,575 | `81457fc51d46803edb14505c6c5c6f915fe22e64c833798b401a5de3da8cfcb6` |
| guajataca-bend | 1,036,065 | 12,511 | `ba4aeb8868fde5b12d28d3a1b19b223f2b2cd94a131e5a8291c4fa0196f77991` |
| arecibo-upper-meander | 915,821 | 132,755 | `d156fba2e441a063cf719a8c9eb47b863fd8c14431312f94b9c0c2a1fc578f1e` |
| arecibo-lower-meander | 861,447 | 187,129 | `9a3cc8a9b1302ea8e553573f284c92cb19f656af13a525a13051b441b4b4b8b6` |

`data/review/pilot-batch-3-qualification-v1/` retains policy/delegation, source
metadata, water evidence, masks, assessments, current curation records/portable
exports, build requests and results. `records.json` pins current IDs. Independent
decoding verified all 4,194,304 output samples: 3,834,334 valid ground values are
bit-identical, and 359,970 water cells contain exact masked fill. All seven raster
layers were checked. `prepare verify` reports intact, registered, current outputs.
The catalog has 35 records, 13 current heads and 12 prepared regions; the prior
batch's tile 6 remains rejected and preserved. No training dataset is released.
