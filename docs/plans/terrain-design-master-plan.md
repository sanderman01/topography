# Terrain collection and generation design concept

Status: preset/metadata validation, raw DEM acquisition/import, native-grid review, curation, regional quality checks, bounded same-survey native preparation, evaluation-only dataset releases and CPU numeric evaluation are implemented. Thirty real regions are qualified under the supplier-QA pilot rule; the original twelve belong to the numeric release. The first image-codec experiment completed and rejected its tested representation. Model-training admission and terrain-specific modeling remain open. Sources checked 13 September 2026, with subsequent dataset checks in the linked reports; implementation checkpoint updated 22 September 2026. Builds on `docs/biomes.md`, `docs/locations.md`, and `docs/elevation-data-sources.md`.

Project stance (2026-09-25): this is game terrain. Prefer plausible results,
automation and judgment calls over scientific evidence gathering; where later
sections demand stricter evidence, tooling defaults and recorded judgment apply.

## Intended result

Create a human-directed terrain workshop for a dirigible game: collect detailed Earth elevation, curate a reusable terrain library, and use conditional diffusion to assemble an original world with a working target of nine deliberately placed terrain biomes (§1 allows that count to shift with what acquisition actually finds). The result should offer recognizable landmarks, concealed routes, confusing junctions, narrow passages, and meaningful choices about when to climb.

The central design decision is to separate the world's large-scale structure from its local detail. An editable base surface establishes mountain ranges, drainage, coastlines, and routes. Diffusion adds terrain conditioned on that surface and the biome layout. Human edits and accepted regions remain reproducible inputs to subsequent builds.

Keep an authoritative elevation surface in floating-point metres throughout processing. Produce 16-bit heightmap textures as controlled exports and optional training intermediates. A grayscale image is a storage representation of elevation, not the definition of the terrain.

This plan covers acquisition, cleaning, labeling, training experiments, generation, editing, validation, and export, including snow/ice classification and vegetation masks where available. Bevy integration, QuadTree subdivision, vegetation placement/rendering/growth, weather simulation, a complete flight model, and underwater bathymetry remain outside its scope. This is a solo project; every human checkpoint below is performed by the same person, so checkpoints are a workflow discipline, not a multi-role handoff.

Being solo also caps the scope of everything above: the plan's many provider adapters, quality gates, and a possible custom autoencoder are a menu to draw from as needed, not a checklist to fully complete before the pilot means anything. Keep the real-patch-and-blending baseline (§7) a genuine fallback, not just a comparison point — if diffusion training does not clear its feasibility gate within a reasonable number of attempts, ship the world using curated real terrain and blending instead of treating diffusion as mandatory. Prefer adding a small number of high-value agency adapters over integrating every table row in §2, and prefer a narrow first source region per biome over broad simultaneous coverage.

## Constraints that shape the approach

ControlNet provides learned spatial conditioning; it does not guarantee that every generated feature follows a segmentation boundary. Use a fixed biome ownership map, a terrain-trained control branch, explicit geometric constraints, and human acceptance to enforce the desired result. Arbitrary biome colors need paired training; an existing photographic segmentation ControlNet will not already know their meanings. [ControlNet paper](https://arxiv.org/abs/2302.05543), [ControlNet training documentation](https://huggingface.co/docs/diffusers/training/controlnet).

LoRA changes model weights efficiently, but does not change the precision or semantics of the data pipeline or repair a lossy image autoencoder. For example, the standard Diffusers image LoRA training script converts its input images to RGB. Uploading 16-bit DEMs to that workflow does not establish a precision-preserving elevation model. Treat image-model adaptation as an experiment with measurable acceptance criteria. [Diffusers training implementation](https://github.com/huggingface/diffusers/blob/main/examples/text_to_image/train_text_to_image_lora.py).

A heightfield has one height per horizontal position. It can approximate steep slopes and pillars, but cannot represent overhangs, arches, caves, or two surfaces above one another. Even a genuine 1 m DEM can miss the interior of a narrow canyon when airborne measurements cannot see its floor. Select features that survive this representation; flag exceptions for possible future mesh work.

Grid spacing is not effective feature resolution. A raster resampled from 30 m to 1 m has no new measured detail. Small pillars and passage widths need several reliable samples across them. Ground-return density, interpolation, vegetation filtering, and survey geometry matter alongside nominal resolution.

## 1. Select terrain for flight

The existing notes specify five families. Retain those and provisionally add alpine ridges, badlands, dunes, and braided river basins. Here, “biome” means a terrain generation class; it does not imply a complete ecological biome.

Nine is a working target, not a hard requirement. Two escape valves apply per class, both to be resolved during the pilot rather than assumed upfront:

- **Accept reduced fidelity for a specific class.** If a class cannot clear the native 0.5–1 m gate anywhere searched, it may be admitted at the secondary 2 m tier instead of dropped, provided it is explicitly flagged as reduced-fidelity in its metadata and reviewed for whether its defining features (passage widths, pillar separations) still survive at that resolution. This is a per-class exception, not a general relaxation of §4's gates.
- **Add or swap classes opportunistically, within reason.** If acquisition turns up a strong high-detail candidate for a terrain family outside this list, add it as a tenth-plus class rather than discarding the data, and if a listed class turns up nothing usable even at reduced fidelity, replace it rather than blocking the whole pilot on it. Each added class still carries real cost — acquisition, balancing, labeling, evaluation, and an added slot in the model's conditioning vocabulary — so weigh a genuinely distinctive candidate against that cost rather than adding classes for their own sake, especially on a solo project (§7). Keep the label schema (§1 table below, §6) versioned so classes can be added or retired between world revisions without invalidating past accepted work for unaffected classes; never reuse a retired class ID, and treat a class-vocabulary change as a schema migration tied to a specific dataset/model/world-revision version, not a free edit.

The following locations are search candidates, not claims of verified high-resolution coverage at each landmark. Agency product availability is documented in the next section; exact footprints still need inspection.

| ID / provisional color | Terrain family | Flight experience to seek | Candidate areas |
| --- | --- | --- | --- |
| 1 / purple | Karst towers, pinnacles, and depressions | Repeated silhouettes, enclosed hollows, hidden exits between isolated hills | Yangshuo and Tsingy as visual references; Puerto Rico karst, Slovenia's Kras, and El Torcal in Spain as acquisition leads |
| 2 / orange | Gorges and slot-like canyons | Blind bends, branching passages, constrictions, climb-out decisions | Verdon; Zion and Colorado Plateau gorges |
| 3 / cyan | Fjords and glacial troughs | Long corridors interrupted by side valleys, hanging valleys, and enclosing walls | Geirangerfjord; Milford Sound and other covered New Zealand valleys |
| 4 / dark red | Volcanic calderas and lava terrain | Rim crossings, bowls, isolated cones, local roughness | Mount Aso; Réunion; Canary Islands; Hawaii |
| 5 / teal | Archipelagos and dissected coasts | Similar-looking channels, headlands that hide landmarks, sparse height cues | Reine/Lofoten; Swedish archipelagos; covered New Zealand coasts |
| 6 / red | Alpine ridges and cirques | Saddles, ridge crossings, false passes, disconnected valleys | French Alps; Norwegian mountain terrain |
| 7 / brown | Badlands, mesas, and eroded gullies | Dense branching routes, misleading parallel drainages, abrupt escarpments | Bardenas Reales; covered US badlands and plateau margins |
| 8 / yellow | Dunes and sand ridges | Repeated forms and weak landmarks interrupted by distinctive ridges | Dune du Pilat; covered US dune fields; English coastal dunes |
| 9 / blue | Braided river basins and floodplains | Channel choice, meanders, terrace edges, open recovery areas | Covered New Zealand braided rivers; English floodplains |

Store these as stable integer IDs with a versioned color legend. The proposed colors are editable before the label schema is frozen. Water, snow/ice, vegetation and missing data use separate masks; they do not consume extra biome classes. A tile's biome label does not establish whether any of these covers is present.

For every area of interest, record its polygon, biome/subtype, approximate feature dimensions, elevation range, landmark candidates, and the specific flying experience it might support. Keep “interesting geography” and “usable source data” as separate scores.

Use overhead, oblique and fixed-altitude views as optional aids for judging terrain. Terrain suitability is the owner's whole-tile Yes/No decision. Vehicle dimensions, movement limits, clearance requirements and formal gameplay acceptance thresholds are outside this plan; they are not pending pilot decisions.

Use relief, ridge density, channel branching, passage width, line-of-sight occlusion, and skyline distinctiveness to rank candidates. Keep the human decision final. Include memorable landmarks and recovery areas as well as confusing terrain; uninterrupted difficulty would reduce both navigability and visual variety. Fog and wind are scenario annotations here, not features supplied by the DEM.

## 2. Source acquisition strategy

Use only source data publicly available for download. Never send emails or
prepare email drafts, and do not ask the owner to contact people or institutions
for data. Unpublished deliveries and commissioned measurements are outside this
project's acquisition scope. If a needed asset has no public download, use
adequate existing public evidence, select another public source, or record the
limitation without making correspondence a dependency.

Search national mapping agencies first for numeric bare-earth DEMs, often called DTM, MNT, or MDT. Use classified LiDAR point clouds when delivered DEMs are missing or inadequately processed. Keep DSMs, rendered hillshades, imagery, and bathymetry distinct from ground elevation.

| Agency / source | Confirmed product or access route | Proposed role and remaining checks |
| --- | --- | --- |
| USGS, United States and Puerto Rico | Project-based 1 m LiDAR DEMs through The National Map; seamless 1 m production is still expanding | Core adapter for karst, canyon, volcanic, and badland searches. Query coverage for each polygon; neither project seams nor complete national 1 m coverage should be assumed. [USGS products](https://www.usgs.gov/3d-elevation-program/about-3dep-products-services) |
| IGN, France | LiDAR HD MNT at 0.5 m, with classified point clouds and downloadable tile URL lists | Priority for Verdon, alpine terrain, karst plateaus, and volcanic terrain where released. Distinguish available deliveries from planned coverage. [IGN delivery specification](https://geoservices.ign.fr/sites/default/files/2024-09/DL_LiDAR_HD_1-0.pdf) |
| IGN/CNIG, Spain | Third-coverage MDT at 0.5 m in COG; older 2 m and 5 m products are separate collections | Priority for karst, badlands, and Canary Islands searches. Pin the collection and confirm the exact released footprint. [CNIG elevation catalog](https://centrodedescargas.cnig.es/CentroDescargas/modelos-digitales-elevaciones) |
| Kartverket, Norway | Høydedata provides project data; DTM1 is the 1 m terrain product | Priority for fjords, Lofoten, and alpine terrain. Check measurement method, project quality, and access for selected assets. [Geonorge elevation access](https://www.geonorge.no/kartdata/datasett-i-geonorge/hoydedata/), [Kartverket specification identifying DTM1](https://register.geonorge.no/data/documents/Produktspesifikasjoner_dtm-10-terrengmodell_v2_produktspesifikasjon-kartverket-dtm-20190815_.pdf) |
| LINZ, New Zealand | National 1 m DEM and individual surveys; public AWS COGs with STAC metadata | Strong first adapter for glacial valleys, rivers, and coasts. Resolve survey provenance and actual coverage rather than assuming Milford Sound is fully available. [LINZ access](https://www.linz.govt.nz/products-services/data/types-linz-data/elevation-data/access-elevation-data), [official catalog guidance](https://github.com/linz/elevation) |
| Lantmäteriet, Sweden | Markhöjdmodell is a 1 m terrain grid; download access uses STAC/COG infrastructure | Useful for archipelagos and glacial landforms. Inspect current access requirements. June 2026 delivery changes introduced 10 × 10 km files, so avoid hardcoded assumptions about the older tile sizes. [Product catalog](https://www.lantmateriet.se/geodataprodukter), [delivery update](https://www.lantmateriet.se/sv/geodata/vara-produkter/Produktnyheter/Geografisk-information/markhojdmodell-nedladdning-utokas-med-mer-innehall/), [access infrastructure](https://www.lantmateriet.se/sv/geodata/vara-produkter/Produktnyheter/Allmanna-nyheter/vardefulla-datamangder---forandringar-pa-gang/) |
| GURS / E-prostor, Slovenia | National LiDAR program and 3D data viewer; agency documents a LiDAR-derived 1 × 1 m relief model | Strong karst acquisition lead. Inspect downloadable ground-point/DEM products and their metadata; a point-cloud import may be necessary. [Agency remote-sensing information](https://www.e-prostor.gov.si/podrocja/drzavni-topografski-sistem/daljinsko-zaznavanje/) |
| GSI, Japan | DEM1A from airborne laser surveys, alongside distinct 5 m and 10 m products | Check Mount Aso and other volcanic areas against DEM1A coverage. “1 m” is a nominal geographic mesh: actual spacing varies by axis and latitude, so validate it in metres after projection. [GSI product FAQ](https://web2.gsi.go.jp/kiban/faq.html) |
| Environment Agency, England | Composite bare-earth DTM at 1 m, covering approximately 99% of England | Useful for floodplains, coastal dunes, and an accessible ingestion pilot. Composite data includes different acquisition dates and resampling history. [Official dataset](https://www.data.gov.uk/dataset/01b3ee39-da3f-47b6-83da-dc98e73a461f/lidar-composite-digital-terrain-model-dtm-1m) |
| National Land Survey, Finland | Open 2 m laser-derived elevation model with documented quality classes | Optional secondary tier for glacial terrain. It is not native 1 m training data. [Product specification](https://www.maanmittauslaitos.fi/en/maps-and-spatial-data/datasets-and-interfaces/product-descriptions/elevation-model-2-m) |

Puerto Rico deserves an early karst trial. USGS documents a 1 m working DEM used to study its karst, but the linked release is hillshade/TPI imagery. Follow its provenance to numerical elevation rather than ingesting the visualization as heights. It is an acquisition lead and morphological reference, not proof that its hills reproduce Yangshuo or Tsingy's particular geometry. [USGS karst release](https://www.usgs.gov/data/2015-2017-lidar-derived-imagery-karst-areas-puerto-rico-1-meter-resolution).

Retain Yangshuo, Ha Long Bay, Tsingy, Raja Ampat, Ngorongoro, and other desired locations in a research backlog when suitable data has not been verified. Record the best known resolution and why it failed. Search public downloads from regional agencies, geological surveys, and documented research surveys before changing the target. If detailed data remains unavailable, explicitly select a publicly downloadable morphological substitute or leave the location in the backlog; do not disguise upsampling as a successful acquisition.

Correct two assumptions in the existing source notes when implementing the catalog: USGS 1 m products are not universally available or universally seamless, and global satellite products are not replacements for detailed ground LiDAR. Copernicus DEM is a surface model, while ArcticDEM is also derived from optical surface reconstruction. Keep them in a reference tier unless specifically accepted for an appropriate coarse-scale use. [USGS products](https://www.usgs.gov/3d-elevation-program/about-3dep-products-services), [Copernicus DEM description](https://dataspace.copernicus.eu/explore-data/data-collections/copernicus-contributing-missions/collections-description/COP-DEM), [ArcticDEM description](https://www.pgc.umn.edu/data/arcticdem/).

For each provider, the collection tool should discover coverage, list matching assets, estimate download size, fetch selected assets with resume/retry support, verify checksums, and import manually downloaded assets. Prefer documented APIs, STAC catalogs, download indexes, and direct files. A provider without a suitable automated route can initially use a human-exported URL list. Keep account credentials outside manifests.

Save original metadata, source URLs, dataset and survey identifiers, acquisition dates, CRS and height datum, units, resolution, ground-point density when known, processing history, and license/attribution information. Record reuse requirements for training, edited maps, and redistribution without assuming every dataset hosted on the same portal has identical terms.

Include companion water, snow/ice and vegetation layers in source discovery and
the batch download list. Prefer supplied masks or polygons and documented LiDAR
classifications; use other public glacier inventories, land-cover products or
dated imagery when suitable. Record each layer's source, license, date, extent,
resolution and class definitions independently of the DEM. Project-wide class
counts do not establish coverage of a particular crop. Check seasonal snow and
permanent ice separately when assessing completeness; a generic snow-free survey
description does not establish that a glacier is absent. Use a combined snow/ice
mask as the required minimum; retain subtypes when the source supports them.
Vegetation extent is optional where reliable public data is unavailable. Neither
an empty download nor absence of classified returns proves absence of cover.

**Licensing gate.** Before any dataset enters the prepared or training pool, confirm its license permits ML training use. A dataset whose license prohibits, restricts, or leaves ambiguous the right to train models on it is excluded outright, regardless of resolution or geographic value; do not admit it under a "review later" status. Record the specific clause or confirmation relied on next to the dataset's metadata. This gate is evaluated per dataset, not per portal or per agency, since a single portal can mix terms across products.

## 3. Data organization and tool boundaries

Use Bash for small orchestration tasks around existing tools and Rust for the persistent catalog, provider adapters, configuration validation, quality metrics, tile indexing, and build orchestration. Use GDAL/PROJ for raster and coordinate operations and PDAL for point-cloud processing instead of implementing those algorithms from scratch. [GDAL raster warping](https://gdal.org/en/stable/programs/gdalwarp.html), [PDAL raster output](https://pdal.io/en/stable/stages/writers.gdal.html).

Model training will probably use the established Python/PyTorch ecosystem. Keep it behind a documented batch interface driven by the same manifests; the preference for Bash/Rust need not require developing a new diffusion training framework.

Keep these artifact groups distinct:

| Artifact | Purpose |
| --- | --- |
| Source registry and area catalog | Provider capabilities, candidate polygons, coverage status, metadata, human decisions |
| Immutable raw assets | Original files and metadata, identified by checksums |
| Prepared regional rasters | Physical elevation, water/snow-ice/vegetation assessments and masks, validity and quality masks, transformation records |
| Review assets | Hillshades, contours, oblique previews, metrics, annotations, accepted crop polygons |
| Dataset releases | Approved tiles, labels, spatial splits, normalization settings, source lineage |
| Model runs | Model versions, controls, training configuration, checkpoints, evaluations |
| World revisions | Layout, base terrain, source placements, generated regions, edit layers, locks |
| Exports | Height tiles, masks, metadata, attribution, overview rasters, validation report |

Use a small local catalog, such as SQLite, with portable manifest exports. Large rasters live in files, not inside the catalog. Every derived artifact identifies its inputs, configuration, tool versions, and approval state. An input or parameter change invalidates only its dependents.

Original data remains immutable. “Discard” means exclusion from the approved dataset, with a recorded reason; removing raw cache files is a separate explicit housekeeping operation.

## 4. Clean and curate before slicing

Source expansion must use [batch acquisition and a review gallery](batch-source-workflow-v1.md).
Maintain versioned target lists and let the tool manage bounded downloads, retries,
verified resume, crop previews and status summaries. Do not orchestrate each file
with agent-issued curl calls. Keep download and raster-processing stages separate
so workload readiness remains explicit. The gallery provides one place to browse
tiles, inspect terrain and save whole-tile Yes/No decisions directly to the catalog;
show prior votes and preserve them for unchanged artifacts. Avoid separate review
links and conversational vote transcription as the normal workflow.

[Automatic preparation](../design/automatic-preparation-v1.md) reuses pinned
source assessments to run technical checks when tiles enter the library. A saved
Yes queues native preparation in the gallery's background worker. Show Preparing,
Ready or Needs technical attention; retain the vote through technical exceptions
without requesting repeat approval. Evidence interpretation remains a source-level
technical task. Successful preparation does not create training admission.

### Snow/ice and vegetation masks

These are game layers, not scientific classifications (policy `game_cover_v2`,
2026-09-25, superseding the stricter evidence rules first planned here).

- Derive snow/ice and vegetation automatically from coarse public land cover:
  ESA WorldCover 10 m (class 70 snow/ice; vegetated classes) plus current GLIMS
  glacier outlines. About 10 m source detail is enough for 1 m terrain.
- Mark snow/ice where the area commonly has it. Ignore seasonality, survey dates
  and boundary uncertainty. No cell is left unknown: ambiguous or missing cover
  becomes a recorded judgment (normally absent).
- Water also comes automatically: the tile's water mask plus WorldCover class 80
  and OpenStreetMap water (ODbL attribution recorded). Water wins: water cells
  are never snow/ice or vegetation.
- Masks follow the elevation crop grid by nearest-neighbour resampling and stay
  categorical; vegetation and snow may overlap.
- Snow/ice cells are excluded from bare ground with deterministic fill; the
  measured surface is kept separately. Vegetation never excludes ground.
- The gallery worker produces cover masks and the v2 product for every
  accepted tile, including legacy tiles, without per-tile profiles or votes.
- Earlier products stay on disk; accepted tiles move to `game_cover_v2`
  successors automatically.

Process each real-world survey region in its appropriate projected metric CRS. Resolve horizontal units, vertical units and datum, axis orientation, nodata, and pixel alignment explicitly. Use an available verified vertical transformation when joining surveys that need one; otherwise isolate the incompatible surveys and flag the uncertainty. Do not silently treat ellipsoidal and sea-level heights as interchangeable.

Create the aligned regional mosaic before extracting training tiles. Prefer the best supported survey in an overlap according to declared criteria. Estimate residual offsets on stable ground, examine difference rasters, and reject incompatible joins. Averaging displaced ridges produces false terrain; broad smoothing is not a substitute for correcting alignment.

If rasterizing LiDAR, select classified ground returns, inspect ground-return density and unsupported cells, and preserve breaklines where supplied. Request a source buffer around crops so interpolation is not truncated at tile edges. A sparse point cloud cannot justify a detailed DEM merely because the rasterizer accepts a 1 m output setting.

Resample once to an aligned working grid when practical. Use appropriate filtering when downsampling and inspect interpolation around cliffs for overshoot or rounding. Keep labels and validity masks categorical. Never use RGB color management, gamma, histogram equalization, or photographic enhancement on numerical elevations.

Initial quality policy, adjustable per terrain family:

| Gate | Proposed decision |
| --- | --- |
| Effective resolution | Prefer native 0.5–1 m ground data. Admit other nominal products only after checking actual spacing and feature preservation. Keep 2 m data in an explicitly approved secondary tier; exclude 5–30 m originals from fine-detail training. |
| Missing ground | Require at least 99.5% valid terrestrial ground cells for the initial training pool. Under the planned mask contract above, mapped water and snow/ice are separate excluded domains. Vegetated bare ground remains eligible. Evaluate support automatically; optional annotated landmarks or passages must have no unsupported ground gaps. Do not require manual feature annotations. |
| Cover masks | Generate snow/ice and vegetation masks automatically under `game_cover_v2`; they never block qualification. |
| Interpolation | Permit small, reviewed repairs with a repair mask. Large voids, invisible canyon floors, or interpolated pillar fields trigger exclusion or another source search. |
| Artifacts | Flag stripes, terraces, spikes, pits, duplicate edges, vegetation remnants, buildings, and implausible survey-boundary steps. Compare against context before rejecting natural steepness. |
| Feature fidelity | Human reviews the entire tile and chooses yes or no. The technical workflow assesses sampling and feature fidelity from available evidence, reporting concrete limitations without asking the human to label individual features. |
| Traceability | Missing units, ambiguous surface type, or unresolved source provenance blocks admission until clarified. |
| Licensing | Confirmed ML-training permission required for admission. A dataset that blocks or does not clearly permit training use is excluded outright, not deferred. |

For the pilot, apply [supplier_qa_pilot_v1](../design/source-quality-pilot-v1.md)
as the evidence basis for released native bare-earth DEMs. Trust documented
supplier QA for finite, provider-valid ground except identified local defects;
retain the 99.5% coverage requirement and source limitations. Routine supplier
interpolation beneath forest is acceptable and does not require a separate
confidence target or repeated human review. This is an explicit pilot choice,
not a claim of measured fine-feature accuracy. The implementer makes routine
qualification decisions; human source review remains whole-tile yes/no.

Do not automatically fill all depressions. Karst sinkholes and volcanic craters are desired features. Hydrological corrections should be masked, optional operations tied to intended rivers and lakes, with before/after review.

Present batches of tile previews for a whole-tile yes/no decision. Keep multiple hillshade directions, contours, cross-sections and oblique 3D views available for inspection without making them a mandatory checklist. Technical assessment and mask construction belong to the tools and implementer, not to the human reviewer. Preserve the human decision for an unchanged tile while technical work continues; a yes does not invent passing technical findings. Recropping, repair and feature annotations are optional follow-up actions only when the owner requests them. Clean examples, exceptional landmarks, ordinary connecting terrain, and real biome transitions all belong in the library.

## 5. Physical scale, tiling, and elevation encoding

Recommended world preset: 32,768 × 32,768 m with 1 m cells, yielding 32,768 × 32,768 samples. This is 32.768 km per side, slightly larger than the requested approximate 32 km. Also provide an exact 32,000 m preset: at 1 m it has 32,000 samples, or with 32,768 samples its spacing is 0.9765625 m. Configuration must select a consistent relationship among extent, cell count, and spacing rather than independently accepting conflicting values.

| Setting | Initial preset | Configurable alternatives |
| --- | --- | --- |
| World extent | 32,768 m per side | Rectangular extents and exact 32 km |
| Final spacing | 1 m | Other physical spacings, including 0.5 and 2 m |
| Training tile size | 512 × 512 cells | 1,024 or 2,048 as a measured stretch target, subject to VRAM (§7) |
| Final file tile size | 256 × 256 intervals (257² texels, `shared_edge`) | Configurable power of two; independent of training/inference windows |
| Generation window / stride | 1,024 / 512 cells | Overlap selected through seam experiments |
| Layout painting resolution | 512 × 512 | Other resolutions, plus vector boundaries |
| Generation scales | 16 m, 4 m, 1 m | A versioned scale pyramid |

Use powers of two for convenient defaults; retain agency-native tile dimensions until preparation. Do not distort geography merely to fit power-of-two source tiles.

The 512 × 512 layout painting resolution is a human sketching overview only — at the recommended 32,768 m extent that is 64 m per pixel, too coarse to authoritatively place narrow passages, riverbanks, or fine coastal boundaries. The authoritative ownership map is a finer raster or vector boundary set derived from (and editable alongside) this coarse sketch, not the sketch itself.

A 1,024-cell tile at 1 m covers 1.024 km per side. It cannot teach the complete structure of a large caldera or fjord system alone. Derive aligned training examples at 4 m and 16 m from accepted detailed surveys to supply wider context. These intentionally downsampled examples are different from pretending an originally coarse DEM contains fine detail.

Use Float32 GeoTIFF/COG in physical metres for prepared and generated elevation. Maintain an explicit validity mask. For 16-bit grayscale exports, use a documented linear offset and scale shared across the whole world; nodata lives only in the separate validity mask, never as a reserved height code, so all 65,536 codes remain valid height values across the chosen range. A 4,096 m vertical range across 65,535 intervals has approximately 0.0625 m quantization steps. This is encoding precision, not a claim about survey or model accuracy.

Do not independently min/max normalize each tile. That would make a flat field and a mountain occupy the same intensity range and destroy cross-tile comparability. Training should use a documented fixed transform per model level, fitted on training data and frozen for validation. Record clipping rates. For residual generation, normalize the difference from the coarse surface with its own fixed physical scale and retain the coarse surface as an input.

Define cell-center coordinates, north/south row orientation, horizontal axes, world origin, and elevation units in the export contract. The export delivers heightmap textures, not meshes or vertex data; the game's own quadtree LOD renderer builds geometry from them. This preserves a clean handoff without designing the renderer here.

The master mosaic and all internal processing stay cell-center rasters (§5's cell counts, e.g. 32,768² for the full extent). Each texel is one master sample read directly from that surface, so export adds only height encoding, not resampling. Tiles span a configurable power-of-two number of intervals and use one of two configurable layouts. The default follows Microsoft Flight Simulator 2020's DEM tiles: 256 intervals in the `shared_edge` layout, 257 × 257 texels at 16 bits.

- **`shared_edge`**: 2ⁿ + 1 texels per full side (257² by default). Adjacent tiles repeat their border row/column bit-identically, so each tile is self-contained and seams close without neighbour access. This is the common heightfield convention (Unity terrain; Microsoft Flight Simulator 2020).
- **`power_of_two`**: 2ⁿ texels per full side (256²) with no duplication; the renderer reads the closing row/column from the neighbouring tile. Dimensions suit texture arrays, atlases, and GPU block compression (which needs multiples of four).

Both layouts use the same global sample lattice and encoding, so switching layout never changes a height value. World-edge and partial-tile behavior is defined explicitly with the first exporter.

## 6. Build the training dataset and controls

Label accepted geographic regions before cropping. Use human polygons supported by geology, slope, landform analysis, and imagery; a DEM alone does not reliably identify geology. Automatically proposed labels remain editable. Keep uncertain areas masked out of the supervised label loss.

Each example contains elevation, the categorical biome mask, validity/quality masks, physical spacing, source identity, and any coarse elevation or structural controls needed by the model. Derive the RGB conditioning image from the integer labels. Scale categorical masks with nearest-neighbor rules; prevent antialiasing and JPEG compression from inventing colors. Transition widths belong in a separate control layer, not in ambiguous label colors.

Dataset manifests must also carry snow/ice and vegetation masks or their explicit
absence/availability declarations from §4. Crop, transform and augment them with
the terrain; propagate unknown coverage. Exclude snow/ice surface heights and
their ground-fill cells from bare-ground learning and evaluation. Vegetation
classification is optional metadata unless a model explicitly consumes it; mask
unknown vegetation out of that supervision only, not out of valid ground losses.
Do not turn missing vegetation into a negative training label. Handle all-absent,
partially known and all-ground-masked windows explicitly. Record coverage and
mask provenance in each release; do not silently upgrade existing releases.

Include mixed-biome windows and genuine transitions. Match the coarse and simplified masks seen during training to the kind of low-resolution layout the human will paint. If a model only sees one solid color per tile, it has little evidence for learning regional placement inside mixed tiles.

Use geographical train/validation/test splits established before extracting overlapping tiles. Keep adjacent crops, repeated surveys, alternate resolutions, and augmentations of the same landform in one split. Add a separation buffer based on the largest physical context window. Report independent source regions and surveyed area as well as tile counts; overlapping tiles are not independent evidence.

Balance by biome and independent area. Limit domination by one huge survey. Prefer matched rotations and reflections for all paired layers; avoid arbitrary image resizing that changes physical feature scale. Any deliberate horizontal or vertical scaling must update metadata and controls and should be reviewed for altered gameplay geometry.

**Scale-eligibility for reduced-fidelity classes.** A class admitted at the secondary 2 m tier (§1, §4) may supervise only the 4 m and 16 m pyramid levels, never the 1 m level; upsampling 2 m source data to claim 1 m detail would let source resolution become a hidden feature the model keys on (learning that a biome is intrinsically smoother rather than that it was measured coarsely). Its 1 m output stays band-limited to what the 4 m/16 m residual can plausibly refine, or is left to explicitly separate procedural or cross-class fine-detail work, never treated as measured 1 m ground truth. Conditioning the model on physical spacing is a useful signal but does not by itself remove this confound.

The ownership map is authoritative and must contain every finalized class ID with configured minimum areas; the class count is nine as a working target but may finish higher or lower per §1's acquisition-driven exceptions. Generated terrain still requires separate checks for whether those regions actually exhibit the requested forms. Support disjoint regions of the same class; class count and region count are different concepts.

Terrain families are not mutually exclusive on the ground — a fjord can be alpine, a volcanic island can also be an archipelago, a karst plateau can be coastal. The ownership map still assigns exactly one primary class per cell, chosen by a documented, versioned precedence order (for example: water body > karst > volcanic > fjord/glacial > alpine > archipelago/coast > badlands > dunes > braided river, adjustable during the pilot). Carry any secondary character (coastal, alpine, etc.) as an additional structural control channel, not as a competing exclusive class, so a cell's conditioning is not lost just because it didn't win the primary label.

## 7. Diffusion development with an early feasibility gate

First assemble a small curated pilot spanning karst, gorges, and a smoother river or coastal class. Aim initially for several independent areas per class, then expand according to validation results. Do not commit to a training-tile count before measuring data diversity, memory, and reconstruction quality.

**GPU budget.** The available hardware has 16 GB VRAM; this is a fixed ceiling for the pilot and initial full builds, not a target to negotiate upward. Reference ControlNet training guidance reports roughly 16 GB fitting a 512² tile at batch size 1 with gradient checkpointing and an 8-bit optimizer, against a documented ~38 GB default configuration — so 512² (§5's training-tile default) is the defensible initial preset, and 1,024²/2,048² are measured stretch targets, not starting assumptions. Distinguish LoRA fine-tuning memory from full ControlNet training memory (the latter optimizes an entire added branch), and distinguish where reduced precision is applied: parameter precision, optimizer-state quantization, activation precision, and output/decoding precision each have different effects on fit and on numeric fidelity. In particular, a 4,096 m elevation range mapped into a low-precision (e.g. FP16) representation can leave only ~1–2 m of representable spacing near the range extremes; keep the numeric-precision acceptance test in the paragraph below in metres, not just "does it fit in memory." If a candidate approach does not fit within 16 GB at its required precision, treat that as a rejection criterion, not a reason to assume future hardware.

Evaluate the proposed LoRA approach first, but check its numerical representation before expensive training. Pass held-out ramps, flats, sharp ridges, and real DEM patches through the loader and, where applicable, encoder/decoder. Measure height error in metres, slope changes, terracing, and preservation of narrow features. Loading must retain the original numeric values within the declared encoding tolerance.

If an image-based latent model passes the terrain reconstruction checks, adapt it using LoRA and train a compatible ControlNet on the paired terrain controls. If the image autoencoder destroys critical detail, further denoiser LoRA training cannot recover that lost representation reliably. Evaluate a terrain-specific single-channel autoencoder or direct scalar-field diffusion with spatial conditioning. Changing the latent representation requires matching denoiser training; it is not a drop-in decoder swap. This branch is a larger modeling effort and should have its own benchmark and compute estimate.

Condition the terrain generator on biome labels, coarse elevation, spacing, desired relief, and selected structural controls. Prioritize river centerlines and levels, ridge lines, passes, coastlines, and protected landmark heights. Prefer coarse-to-fine residual prediction so new detail does not freely redefine the entire mountain or basin.

Keep standard diffusion training losses and evaluate terrain-aware auxiliary losses only where needed: reconstructed height/slope, multi-scale shape, control compliance, and overlap agreement. Avoid a blanket smoothness objective that would erase the very pinnacles and narrow gorges sought for gameplay.

Compare generated output with a baseline assembled from real terrain patches and simple blending. Evaluate held-out landform distributions, constraint adherence, undesirable repetition, source memorization, and human low-altitude preference. Generated terrain has no unique true DEM, so physical height error is appropriate for reconstruction tests, not as the sole quality score for novel worlds.

This baseline is a real fallback, not only a comparison point. If diffusion training does not clear its feasibility gate (numeric reconstruction, VRAM fit at acceptable precision, and human-judged terrain quality) within a bounded number of experiment iterations, build the world from curated real patches, sculpting, and blending instead — the implementation sequence (§11) should proceed on that basis rather than stall waiting for a diffusion architecture to work.

Record model and dataset versions, normalization, seeds, scheduler, control strengths, training settings, checkpoints, GPU memory, runtime, and software versions. Use checkpointed experiments with identical evaluation layouts. Promote a model only after a human accepts examples and the defined numeric and structural checks pass.

## 8. Compose, generate, and join the world

Start by painting the current candidate-biome layout and sketching large ridges, water bodies, river outlets, caldera rims, and preferred routes. Create a coarse base surface from editable sculpting and selected DEM patches. Source placements retain crop, translation, rotation, scale, elevation offset, blend mask, and provenance. Keep Earth coordinates in the source library and explicit transforms into the fictional local world.

Resolve the large terrain structures at 16 m, refine at 4 m, and generate final detail at 1 m. At the recommended extent these grids are 2,048, 8,192, and 32,768 cells per side. Review each scale before using it as the constraint for the next. All source placements and residual layers use common elevation units; there is no per-window contrast adjustment.

Use MultiDiffusion-style inference to coordinate overlapping windows during denoising. Its defining operation combines window predictions at each diffusion step into a shared update; generating unrelated completed images and then feathering their borders is a different process. The method is evidence for coordinated image generation, not proof of valid drainage or geological continuity in DEMs. [MultiDiffusion project and method](https://multidiffusion.github.io/).

At each level, use spatially consistent initial noise, shared world-coordinate controls, normalized overlap weights, and stable processing rules. Include context outside final crop boundaries and handle outer edges and four-window corners explicitly. If using a latent model, also validate tiled decoding seams. Store height and slope differences across joins; inspect feature continuity through the whole overlap.

Prevent broad elevation drift through the coarse surface and height anchors. Keep river elevations consistent downstream except where a designed waterfall applies; keep lake surfaces level. Ocean, lake, and river surfaces are separate from ground elevation. Ordinary terrestrial LiDAR does not supply underwater topography.

**Water is out of scope as bathymetry.** Since flight happens above the surface, bed elevation is not a gameplay requirement here; do not source, author, or infer any underwater surface. Represent each water body as a mask plus an authored water-surface function (a flat elevation for lakes/ocean, a monotonic downstream profile for rivers, with explicit drops where a waterfall is designed). Cells under the water mask carry a fixed deterministic fill value in the elevation raster, not a sloped or shaped bed; that fill value is excluded from training loss, structural controls, seam/continuity checks, and export validation alike, exactly like other masked-invalid cells. If a future need for underwater flight or diving arises, that is a separate follow-on effort with its own data sourcing and its own bed representation, not an extension of this plan.

Carry snow/ice and available vegetation layers through source placement, joins
and local edits using categorical rules and the same spatial transforms as the
terrain. Keep overlapping cover layers independent; do not average their IDs or
infer vegetation from a biome. A mask for a generated or edited area must be
authored or derived with recorded provenance; do not retain stale Earth-source
labels after changes that invalidate them. Worlds that intentionally have no
snow/ice or vegetation record that absence explicitly. Any retained snow/ice
surface is a separate surface layer, with no claim to ground beneath it. Mask
support does not add vegetation placement/rendering or seasonal simulation.

Final join repair can use constrained local blending, boundary-conditioned inpainting, or a gradient-domain correction within a marked seam band. It must preserve locked heights and named landmarks. Recheck water connections and nearby passages afterward; a visually invisible seam can still block a river or erase a saddle.

Exact placement is achieved operationally: retain the ownership map, enforce explicit constraints, reject unsuitable regional outputs, and allow local replacement or sculpting. Do not claim a learned model can guarantee a particular geological shape at every pixel.

## 9. Human editing and partial reruns

Use QGIS for initial selection, annotation, and numeric inspection, alongside a lightweight terrain preview capable of oblique views and repeatable camera paths. A custom editor can follow if these tools prove cumbersome. Image editors may paint categorical controls, but elevation editing must preserve physical values and export metadata.

For source curation, combine visual selection into a batch review: show each whole tile and record yes or no. Do not turn source quality or crop selection into repeated manual reviews of individual terrain features. Technical failures remain separate and should be reported in plain language. Keep later checkpoints for dataset release, model evaluation, coarse-world composition, regional generation and export review. Annotation, variant comparison and region locking are optional editing capabilities, not source-admission chores.

Store edits as ordered, reversible layers: additive height changes, masked replacements, landmark placements, water constraints, control-mask edits, and protected regions. Preserve both the original generated proposal and the edited revision. An exported raster from another tool becomes an explicit imported edit with a known extent and encoding.

A regional rerun selects a polygon, generation scale, model version, controls, and seed. Include a context halo and a writable transition band; freeze everything beyond that band. Condition on locked surroundings during generation, then copy the original values back outside the permitted edit domain to guarantee that those values remain unchanged. Validate the boundary afterward. Record the resulting variant separately until accepted.

Reruns follow actual dependencies. A small biome change may need only fine-detail regeneration around its boundary. Moving a major river or caldera rim can invalidate downstream water levels and several finer regions. Show the affected area before running; do not promise that every edit is local. Changing the source dataset or model does not silently overwrite already accepted world regions.

Cache by source checksums, configuration, model version, and controls. Interrupted downloads, preprocessing, and model runs resume from completed artifacts. Deterministic seeds support repeatability, but exact regeneration across different GPU/software stacks should not be assumed; retaining accepted output files is the strongest reproducibility guarantee.

## 10. Capacity, acceptance, and export

A 32,768² raster contains 1,073,741,824 cells. One uncompressed Float32 elevation layer is 4 GiB; a UInt16 export is 2 GiB; a byte biome mask is 1 GiB. Nine full-resolution Float32 one-hot biome channels would consume 36 GiB, so derive conditioning windows on demand instead of storing that representation globally.

The world contains 16,384 delivery tiles of 256 intervals (257² texels). At a 512-cell stride, 1,024-cell inference windows yield 63 × 63 = 3,969 windows before extra edge padding. Each is revisited across denoising steps and model passes. Measure a representative regional run and extrapolate total compute, temporary storage, and I/O before scaling up. Global state and accumulation buffers may require storage in RAM or on disk; model windows alone do not account for all memory.

Use windowed reads/writes and overview pyramids. Keep staging space configurable and make compression error bounds explicit if using lossy numeric compression. Do not promise that a 4 GiB raster implies a 4 GiB generation working set. Generation runs against the fixed 16 GB VRAM budget (§7): the 3,969-window full-extent run is a long sequential (or batched-within-budget) job on this hardware, not a one-shot pass, so estimate wall-clock time from a measured per-window cost rather than assuming throughput.

Before promoting a world revision, verify:

- Correct extent, spacing, orientation, dimensions, units, and complete valid output coverage.
- Snow/ice masks wherever present, evidenced absence where not present, and explicit known/unknown vegetation coverage; aligned categorical layers and correct ground exclusions after every crop, placement and edit.
- Presence and minimum area of all finalized biome classes, plus human confirmation of recognizable terrain in each.
- No nodata leaks, clipping plateaus, unexplained spikes, or visible grid patterns.
- Height/slope agreement at seams, continuous named ridges and routes, and consistent water levels and connections.
- Preserved locked regions and human review of the composed terrain.
- Export quantization within half the chosen encoding step when there is no clipping; agreement between texel and master elevation values; bit-identical duplicated borders in the `shared_edge` layout.
- Complete provenance, versioned controls, source attribution, and a reproducible build manifest.

Keep exact invariants, such as untouched pixels outside an edit domain, separate from the owner's visual judgment of the terrain. The implementer handles technical data, numeric and export checks; human review does not require a gameplay suitability checklist or calibrated terrain-acceptance thresholds.

Deliver a Float32 master mosaic or equivalent indexed tile collection, configurable 16-bit heightmap texture tiles, biome/water/validity mask textures, snow/ice masks wherever present, available vegetation masks, explicit absence/availability declarations, encoding and coordinate metadata, review overviews, and the world manifest. Keep cover masks categorical at their declared cell grid and tile them with the same layout as heights; never average them into invented classes. Export tiles are packaging units; they must not determine how the world was trained or generated.

## 11. Proposed implementation sequence

The first item is underway: [loadable presets](../../configs/README.md), a
[candidate legend](../../schemas/legend-v1.md), a
[source-description contract](../../schemas/source-v1.md), and
[technical checks and whole-tile review](../design/pilot-acceptance-v1.md) are
implemented or documented. These metadata checks do not complete the
real-terrain milestone.

The second item has an initial [acquisition/import implementation](../workflows/dem-acquisition.md)
with raw integrity, resumable HTTPS staging and small native-grid review crops.
A real coastal USGS tile was imported; the later pilot establishes source
qualification and scalar numeric evaluation as described below. The
[early Río Abajo karst trial](../reports/rio-abajo-karst-trial.md) traced a native
0.5 m survey tile to three crops with overhead/profiles/low-altitude previews.
Broad karst morphology is inspectable; measured fine-feature accuracy remains
unclaimed. All three crops now have human selection and technical qualification
under the delegated supplier-QA pilot rule. The later
[east-ridges whole-crop review](../reports/rio-abajo-whole-crop-review.md) measures
direct ground returns across one crop and records the owner's approval of the
dataset-specific permission basis. Counts do not establish terrestrial quality.
The [curation workflow](../workflows/source-curation.md) now captures evidence,
crop-wide labels and explicit decisions in SQLite, with immutable history and
portable metadata exports. The [regional quality workflow](../workflows/regional-quality.md)
now measures whole-crop masks and checks preparation eligibility against pinned
current decisions. [Native regional preparation](../workflows/aligned-preparation.md)
composes eligible adjacent reviews from the same survey without resampling,
preserves valid Float32 values and distinct masks, and registers immutable outputs.
This path is verified with synthetic fixtures and twenty-eight real prepared regions
whose valid ground samples remain bit-identical to accepted native terrain;
water uses deterministic masked fill. The owner selected
five of six tiles in [batch 2](../reports/pilot-tile-batch-2.md); rejection preserves
the sixth tile's data. All four tiles in [batch 3](../reports/pilot-tile-batch-3.md) from two further
areas are now selected and prepared. A [numeric pilot proposal](../reports/numeric-pilot-proposal-v1.md)
groups the six areas for calibration/validation. The owner's assent to initial
evaluation is captured with technical responsibility left to the implementer.
The [numeric workflow](../workflows/numeric-evaluation.md) publishes an evaluation-only
release and runs a reusable Rust/Python scalar check. The
[first image-codec experiment](../reports/image-codec-pilot-v1.md) completed twelve
real windows and four synthetic cases; the tested representation was rejected
for metre-scale height errors and narrow-feature loss. One separately approved
[scalar fitting trial](../reports/scalar-codec-fit-v1.md) also completed and was
technically rejected. Its fitting-only release preserves the original evaluation
scope; no production training is admitted. Continue the world pilot with curated
real patches and blending under the fallback above. Prepared outputs retain their
separate unreviewed state. Cross-survey alignment, repairs, output acceptance,
production model-training releases and per-cell label editing remain pending.
The owner rejected the [first coarse world proposal](../reports/world-layout-proposal-v1.md)
at 16 m for repetition, visible joins and an uninteresting layout. The current
[source coverage](../reports/pilot-source-coverage-v1.md) supports initial technical
checks, not a representative world-quality pilot. Expand independent geographic
coverage and connected context under §§6–7 before another world-quality review.
The later authored experiment remains internal; the reusable world composer and
fine-resolution pilot are incomplete.

Source expansion added two accepted wider Puerto Rico contexts and three
[qualified Banks Peninsula crops](../reports/banks-peninsula-qualification-v1.md).
Use [the example locations](../locations.md) explicitly when selecting subsequent
discovery targets; record actual footprint and resolution limits, without
assuming every named landmark has qualifying data. The batch queue and gallery
remain the acquisition and whole-tile selection interface. The later
[landmark continuation](../reports/landmark-continuation-v1.md) adds a qualified
Grand Canyon side canyon and two Mount St. Helens flanks. Its selected crater rim
remains excluded from ground preparation because it contains glacier surface.

The owner specifically liked the Grand Canyon side-canyon crop. Three additional
[Grand Canyon crops](../reports/grand-canyon-expansion-qualification-v1.md) and two
[Zion crops](../reports/zion-qualification-v1.md) are now accepted, qualified and
prepared. Retain the preference for varied canyon contexts in later expansion;
additional Grand Canyon crops remain one survey project. Continue broadening the
other families. Three [Milford Sound crops](../reports/milford-qualification-v1.md) are
now accepted, qualified and prepared, including an adjacent fjord-head and valley
pair. Four verified native crops from Great Sand Dunes and Badlands in
[gallery v9](../reports/family-expansion-review-v1.md) now have owner Yes decisions.
Both Badlands crops are now qualified and prepared by the
[automatic preparation workflow](../reports/automatic-preparation-rollout-v1.md),
bringing the prepared library to thirty regions across six broad families.
Both dunes choices remain Yes with unresolved striping reported as technical
attention. Gallery v10 preserves all choices and reports preparation status.

The versioned snow/ice and optional vegetation implementation in §4 passes
synthetic workflow verification and independent review. See the
[implementation checkpoint](../reports/surface-cover-workflow-v2.md).
Next, backfill public source evidence before
claiming that existing tiles satisfy those added requirements. Prioritize
the accepted St. Helens crater as a snow/ice case, plus mixed and confirmed-absent
tiles. Reuse public source layers and preserve existing selections. Existing
products remain unchanged until new derived revisions have been verified.

- [ ] Establish the initial candidate-class legend (nine as a working target), world presets and source metadata contract.
- [ ] Implement one numeric DEM acquisition path and one manual-import path; produce reviewable regional rasters before expanding provider coverage.
- [x] Run an early karst acquisition trial, tracing source data through to several inspectable high-detail crops. Keep a failed candidate report if the geometry is not supported.
- [ ] Add the highest-value agency adapters, regional quality checks, human crop selection, and label editing.
- [x] Version and implement snow/ice and optional vegetation quality/preparation contracts, automatic profiles, gallery status and numeric dataset propagation.
- [ ] Automate source-layer acquisition, add useful optional cover overlays and backfill accepted tiles without repeating review.
- [ ] Verify present, confirmed-absent, partial, unavailable, fully ice-covered and fully vegetated cases, including boundary/date uncertainty, missing artifacts, downstream mask propagation and zero-ground windows.
- [x] Release a geographically split pilot dataset and complete numeric loader/autoencoder round-trip evaluation.
- [ ] Compare LoRA plus a trained control branch against a terrain-specific representation if needed; benchmark quality, memory, and runtime.
- [ ] Build a 4,096 × 4,096 m pilot with several biome boundaries, a connected water feature, one protected landmark, and a demonstrably local rerun.
- [ ] Expand to all finalized classes (nine as a working target, adjusted per §1's acquisition-driven exceptions) and test coherent coarse-to-fine generation at the full extent, with human review between scales.
- [ ] Finalize editing, restart/rebuild behavior, export validation, provenance, and documentation.

The first milestone should be an inspectable collection of real terrain and a successful numeric precision test. Those results determine the diffusion architecture and data requirements before committing to a full-world training run.
