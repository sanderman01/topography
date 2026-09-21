# Pilot tile batch 2

Prepared for whole-tile selection on 2026-09-21. The three original Río Abajo
tiles are already selected and prepared. This batch adds six candidates from
three other areas using the same publicly downloadable NOAA DEM collection.
No decision has been recorded for these new tiles.

Open the local [six-tile review page](../../data/review/pilot-batch-2-2026-09-21/review-batch.html).
Reply with yes or no for numbers 1–6, or “all yes”. No feature annotations or
explanations are required. Each tile is 512 × 512 m at native 0.5 m spacing;
the page links to the existing interactive terrain viewer.

| Number | Tile | Source window: x, y, width, height | Elevation range (m) | Candidate family |
| --- | --- | --- | --- | --- |
| 1 | Camuy western hollows | `100,400,1024,1024` | 273.340–320.630 | Karst, class 1 |
| 2 | Camuy eastern slopes | `1850,250,1024,1024` | 200.089–306.028 | Karst, class 1 |
| 3 | Central gorge western bend | `600,1850,1024,1024` | 483.482–584.041 | Gorge, class 2 |
| 4 | Central gorge eastern ravines | `1750,1550,1024,1024` | 478.815–572.009 | Gorge, class 2 |
| 5 | Southern river northern channel | `600,50,1024,1024` | 69.813–108.955 | River/floodplain, class 9 |
| 6 | Southern river southern terrace | `700,1750,1024,1024` | 57.019–133.684 | River/floodplain, class 9 |

These are broad candidate labels, not completed per-cell geology or water
classification. The river candidates include foothills, fields and channels;
the gorge candidates include branching slopes and river bends. Human source
selection may precede technical qualification. Native grid spacing does not
claim measured fine-feature accuracy.

## Public sources and retained provenance

The [US Forest Service karst report](https://research.fs.usda.gov/treesearch/2864)
provides the regional karst lead. The official tourism description of
[Cañón de San Cristóbal](https://www.discoverpuertorico.com/profile/canon-de-san-cristobal/12007)
provides the gorge-area lead; crop selection comes from the inspected DEM,
not from an asserted protected-area boundary. River candidates were selected
from terrain morphology in a southern Puerto Rico DEM tile.

The official [NOAA EPSG:6566 VRT index](https://noaa-nos-coastal-lidar-pds.s3.amazonaws.com/dem/USGS_PostMaria_PuertoRico_DEM_2018_9101/USGS_PostMaria_PuertoRico_DEM_2018_m9101_EPSG-6566.vrt)
located the three source files. Original collection metadata declares PRVD02
metre elevations and the DEM's GEOID12B processing. All files were acquired
through the existing bounded CLI with strong ETag/range checks and a 40 MiB
per-asset cap. No point-cloud run or new tool installation was needed.

| Area | NOAA relative asset | Bytes | Raw receipt SHA-256 |
| --- | --- | ---: | --- |
| Camuy | `prvi_d/USGS_OPR_PR_PRVI_D_2018_19QGA58005700.tif` | 24,504,432 | `bde601ec88fd893aa449cdf404a37fb1324dbbddf23b7866c01e1648d5fd8d81` |
| Central gorge | `prvi_h/USGS_OPR_PR_PRVI_H_2018_19QGA16503600.tif` | 24,547,343 | `e1b50b473aacdd8d04da38ddfd5436264c7e109c8c3918967340594a4d0b6872` |
| Southern river | `prvi_h/USGS_OPR_PR_PRVI_H_2018_19QGV15001950.tif` | 26,080,258 | `52c42480ed92bb5cb1dde6bc9096a066a71caaeec6f7929a8a16a3ec4a99a0f5` |

Portable source descriptions are in
[`catalog/candidates/pilot-batch-2/`](../../catalog/candidates/pilot-batch-2/).
[Crop envelopes](../../catalog/areas/pilot-batch-2-crops.geojson) retain geographic
footprints, windows, review identities and unset decisions. The two crops from
each area belong to one future geographic split group; do not randomly split
nearby crops. No training/validation partition or dataset release exists yet.

Ignored local artifacts:

- `data/staging/pilot-batch-2-2026-09-21/`: exact source inputs, metadata, discovery
  manifest, receipt locators and display-only whole-source overviews.
- `data/review/pilot-batch-2-2026-09-21/`: six native reviews, display thumbnails,
  batch HTML/JSON, and three source-to-crop/browser comparison reports.
- `data/raw/`: immutable acquired assets, metadata and receipts.

`review-batch.json` pins the presented page, each review, interactive preview and
thumbnail. Preserve that presentation when later capturing the human response.
Thumbnails and 600-pixel source overviews are display products only; elevation
and viewer samples retain the native grid.

## Reproduce and verify

Copy the selected source TOML beside the original NOAA collection metadata named
`original.xml`, as in the [karst acquisition example](rio-abajo-karst-trial.md#reproduce-from-the-repository-root).
Acquire using `topography acquire SOURCE.toml configs/runtime.example.toml 41943040`,
then run `topography review RECEIPT.json NEW_DIRECTORY X,Y,1024,1024` with the
windows above. Preserve existing directories. The source descriptions retain
permission as unknown; qualify any selected crops with dataset-specific evidence
and [supplier_qa_pilot_v1](../design/source-quality-pilot-v1.md) before preparation.

`scripts/check_review.py` compared all 6,291,456 decoded elevation samples and
provider-mask cells with their source windows and browser payloads. All six have
1,048,576 finite provider-valid samples. These checks establish extraction and
display fidelity, not terrestrial/water classification or training admission.
