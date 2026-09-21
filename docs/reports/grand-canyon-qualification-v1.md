# Grand Canyon qualification

2026-09-21. The one gallery-selected crop, `grand-canyon--northeast-cliffs`, is
technically qualified and prepared under `supplier_qa_pilot_v1`. The original
whole-tile Yes record (`9139ba5b...53ee`) and all prior raw, review, gallery and
catalog bytes remain preserved.

The TNM discovery response points to the exact vendor metadata package for
`AZ_GrandCanyonNP_2019_B19 / AZ_GrandCanyonNP_2_2019`. Its USGS validation report
says the delivered work unit is expected to meet 3DEP requirements and accepts
the bare-earth DEM, point cloud, breaklines, reporting metadata and spatial
metadata. Vendor metadata records acquisition from 2019-09-29 through 2019-10-05,
classified-ground filtering with visual inspection and manual editing, 0.5 m
Float32 source DEM production, and NAVD88/GEOID12B heights. The distributed tile
is the native 1 m EPSG:26912 Float32 product. Supplier QA and nominal spacing do
not prove independently measured local cliff-detail accuracy.

The exact TNM item identifies the tile as a 3DEP product and states that all 3DEP
products are public domain; the project DEM metadata states no access restrictions
and no use constraints. Attribution and source identity are retained. This is the
implementer's dataset-specific training-permission assessment, not a new human or
institutional legal decision.

Because the original raw declaration omitted admission metadata, the existing
import/review workflow created a metadata-only successor. It pins the same raw
asset SHA-256 `434d7794...5381` and window `[8300,200,1536,1536]`; its elevation
and provider-validity TIFFs are byte-identical to the owner-selected review. The
same-ID replacement gallery row is retained beside the evidence, and the original
Yes is carried without another visual vote.
The [integration report](landmark-continuation-v1.md) pins the primary agent's
additional full comparison of actual grid/CRS/axes and NAVD88-metre interpretation
alongside the identical native raster bytes.

All 2,359,296 cells are finite, provider-valid terrestrial cells. No open water is
visible in the native full-crop hillshade; this does not rule out small or
intermittent water. No repair, resampling, optional feature annotation or elevation
change was applied. CLI quality assessment, eligibility and prepared verification
pass. Independent decoding verifies every Float32 sample and every prepared mask.

Current curation record: `744f143b...e95e`. Prepared manifest:
`3827ef8e...7fbc`. Evidence and portable curation export are under
`data/review/grand-canyon-qualification-v1/`. Preparation does not create a
training release or approve generated terrain.
