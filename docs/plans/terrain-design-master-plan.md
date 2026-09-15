# Terrain collection and generation design concept

Status: preset/metadata validation, raw DEM acquisition/import and native-grid review crops are implemented; regional preparation and model work remain planned. Sources checked 13 September 2026. Builds on `docs/biomes.md`, `docs/locations.md`, and `docs/elevation-data-sources.md`.

## Intended result

Create a human-directed terrain workshop for a dirigible game: collect detailed Earth elevation, curate a reusable terrain library, and use conditional diffusion to assemble an original world with a working target of nine deliberately placed terrain biomes (§1 allows that count to shift with what acquisition actually finds). The result should offer recognizable landmarks, concealed routes, confusing junctions, narrow passages, and meaningful choices about when to climb.

The central design decision is to separate the world's large-scale structure from its local detail. An editable base surface establishes mountain ranges, drainage, coastlines, and routes. Diffusion adds terrain conditioned on that surface and the biome layout. Human edits and accepted regions remain reproducible inputs to subsequent builds.

Keep an authoritative elevation surface in floating-point metres throughout processing. Produce 16-bit heightmaps as controlled exports and optional training intermediates. A grayscale image is a storage representation of elevation, not the definition of the terrain.

This plan covers acquisition, cleaning, labeling, training experiments, generation, editing, validation, and export. Bevy integration, QuadTree subdivision, vegetation, weather simulation, a complete flight model, and underwater bathymetry remain outside its scope. This is a solo project; every human checkpoint below is performed by the same person, so checkpoints are a workflow discipline, not a multi-role handoff.

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

Store these as stable integer IDs with a versioned color legend. The proposed colors are editable before the label schema is frozen. Water and missing data are separate masks, so they do not consume extra biome classes.

For every area of interest, record its polygon, biome/subtype, approximate feature dimensions, elevation range, landmark candidates, and the specific flying experience it might support. Keep “interesting geography” and “usable source data” as separate scores.

Review both overhead views and low-oblique views at representative clearances, initially 32, 64, and 128 m above terrain. Flight-envelope settings should include airship width, length, clearance margin, speed, minimum turning space, climb rate, and optional ceiling. Do not assume that an attractive slot canyon can fit the vehicle. A fixed-altitude view is also useful: a camera that automatically follows terrain conceals climb requirements.

Use relief, ridge density, channel branching, passage width, line-of-sight occlusion, and skyline distinctiveness to rank candidates. Keep the human decision final. Include memorable landmarks and recovery areas as well as confusing terrain; uninterrupted difficulty would reduce both navigability and visual variety. Fog and wind are scenario annotations here, not features supplied by the DEM.

## 2. Source acquisition strategy

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

Retain Yangshuo, Ha Long Bay, Tsingy, Raja Ampat, Ngorongoro, and other desired locations in a research backlog when suitable data has not been verified. Record the best known resolution and why it failed. Search regional agencies, geological surveys, and documented research surveys before changing the target. If detailed data remains unavailable, explicitly select a morphological substitute or commission/acquire better measurements later; do not disguise upsampling as a successful acquisition.

Correct two assumptions in the existing source notes when implementing the catalog: USGS 1 m products are not universally available or universally seamless, and global satellite products are not replacements for detailed ground LiDAR. Copernicus DEM is a surface model, while ArcticDEM is also derived from optical surface reconstruction. Keep them in a reference tier unless specifically accepted for an appropriate coarse-scale use. [USGS products](https://www.usgs.gov/3d-elevation-program/about-3dep-products-services), [Copernicus DEM description](https://dataspace.copernicus.eu/explore-data/data-collections/copernicus-contributing-missions/collections-description/COP-DEM), [ArcticDEM description](https://www.pgc.umn.edu/data/arcticdem/).

For each provider, the collection tool should discover coverage, list matching assets, estimate download size, fetch selected assets with resume/retry support, verify checksums, and import manually downloaded assets. Prefer documented APIs, STAC catalogs, download indexes, and direct files. A provider without a suitable automated route can initially use a human-exported URL list. Keep account credentials outside manifests.

Save original metadata, source URLs, dataset and survey identifiers, acquisition dates, CRS and height datum, units, resolution, ground-point density when known, processing history, and license/attribution information. Record reuse requirements for training, edited maps, and redistribution without assuming every dataset hosted on the same portal has identical terms.

**Licensing gate.** Before any dataset enters the prepared or training pool, confirm its license permits ML training use. A dataset whose license prohibits, restricts, or leaves ambiguous the right to train models on it is excluded outright, regardless of resolution or geographic value; do not admit it under a "review later" status. Record the specific clause or confirmation relied on next to the dataset's metadata. This gate is evaluated per dataset, not per portal or per agency, since a single portal can mix terms across products.

## 3. Data organization and tool boundaries

Use Bash for small orchestration tasks around existing tools and Rust for the persistent catalog, provider adapters, configuration validation, quality metrics, tile indexing, and build orchestration. Use GDAL/PROJ for raster and coordinate operations and PDAL for point-cloud processing instead of implementing those algorithms from scratch. [GDAL raster warping](https://gdal.org/en/stable/programs/gdalwarp.html), [PDAL raster output](https://pdal.io/en/stable/stages/writers.gdal.html).

Model training will probably use the established Python/PyTorch ecosystem. Keep it behind a documented batch interface driven by the same manifests; the preference for Bash/Rust need not require developing a new diffusion training framework.

Keep these artifact groups distinct:

| Artifact | Purpose |
| --- | --- |
| Source registry and area catalog | Provider capabilities, candidate polygons, coverage status, metadata, human decisions |
| Immutable raw assets | Original files and metadata, identified by checksums |
| Prepared regional rasters | Physical elevation, validity and quality masks, transformation records |
| Review assets | Hillshades, contours, oblique previews, metrics, annotations, accepted crop polygons |
| Dataset releases | Approved tiles, labels, spatial splits, normalization settings, source lineage |
| Model runs | Model versions, controls, training configuration, checkpoints, evaluations |
| World revisions | Layout, base terrain, source placements, generated regions, edit layers, locks |
| Exports | Height tiles, masks, metadata, attribution, overview rasters, validation report |

Use a small local catalog, such as SQLite, with portable manifest exports. Large rasters live in files, not inside the catalog. Every derived artifact identifies its inputs, configuration, tool versions, and approval state. An input or parameter change invalidates only its dependents.

Original data remains immutable. “Discard” means exclusion from the approved dataset, with a recorded reason; removing raw cache files is a separate explicit housekeeping operation.

## 4. Clean and curate before slicing

Process each real-world survey region in its appropriate projected metric CRS. Resolve horizontal units, vertical units and datum, axis orientation, nodata, and pixel alignment explicitly. Use an available verified vertical transformation when joining surveys that need one; otherwise isolate the incompatible surveys and flag the uncertainty. Do not silently treat ellipsoidal and sea-level heights as interchangeable.

Create the aligned regional mosaic before extracting training tiles. Prefer the best supported survey in an overlap according to declared criteria. Estimate residual offsets on stable ground, examine difference rasters, and reject incompatible joins. Averaging displaced ridges produces false terrain; broad smoothing is not a substitute for correcting alignment.

If rasterizing LiDAR, select classified ground returns, inspect ground-return density and unsupported cells, and preserve breaklines where supplied. Request a source buffer around crops so interpolation is not truncated at tile edges. A sparse point cloud cannot justify a detailed DEM merely because the rasterizer accepts a 1 m output setting.

Resample once to an aligned working grid when practical. Use appropriate filtering when downsampling and inspect interpolation around cliffs for overshoot or rounding. Keep labels and validity masks categorical. Never use RGB color management, gamma, histogram equalization, or photographic enhancement on numerical elevations.

Initial quality policy, adjustable per terrain family:

| Gate | Proposed decision |
| --- | --- |
| Effective resolution | Prefer native 0.5–1 m ground data. Admit other nominal products only after checking actual spacing and feature preservation. Keep 2 m data in an explicitly approved secondary tier; exclude 5–30 m originals from fine-detail training. |
| Missing ground | Require at least 99.5% valid terrestrial cells for the initial training pool, with no unsupported gaps through defining landmarks or passages. Water is evaluated separately. |
| Interpolation | Permit small, reviewed repairs with a repair mask. Large voids, invisible canyon floors, or interpolated pillar fields trigger exclusion or another source search. |
| Artifacts | Flag stripes, terraces, spikes, pits, duplicate edges, vegetation remnants, buildings, and implausible survey-boundary steps. Compare against context before rejecting natural steepness. |
| Feature fidelity | Human inspects passage widths, pillar separations, and ridge continuity against available point clouds and reference imagery. |
| Traceability | Missing units, ambiguous surface type, or unresolved source provenance blocks admission until clarified. |
| Licensing | Confirmed ML-training permission required for admission. A dataset that blocks or does not clearly permit training use is excluded outright, not deferred. |

Do not automatically fill all depressions. Karst sinkholes and volcanic craters are desired features. Hydrological corrections should be masked, optional operations tied to intended rivers and lakes, with before/after review.

Use multiple hillshade directions, slope and curvature maps, contours, cross-sections, and oblique 3D previews. Have the reviewer accept, reject, recrop, repair, or annotate an area. Allow accepted subregions inside otherwise rejected surveys. Clean examples, exceptional landmarks, ordinary connecting terrain, and real biome transitions all belong in the library.

## 5. Physical scale, tiling, and elevation encoding

Recommended world preset: 32,768 × 32,768 m with 1 m cells, yielding 32,768 × 32,768 samples. This is 32.768 km per side, slightly larger than the requested approximate 32 km. Also provide an exact 32,000 m preset: at 1 m it has 32,000 samples, or with 32,768 samples its spacing is 0.9765625 m. Configuration must select a consistent relationship among extent, cell count, and spacing rather than independently accepting conflicting values.

| Setting | Initial preset | Configurable alternatives |
| --- | --- | --- |
| World extent | 32,768 m per side | Rectangular extents and exact 32 km |
| Final spacing | 1 m | Other physical spacings, including 0.5 and 2 m |
| Training tile size | 512 × 512 cells | 1,024 or 2,048 as a measured stretch target, subject to VRAM (§7) |
| Final file tile size | 1,024 × 1,024 cells | Independent of training/inference windows |
| Generation window / stride | 1,024 / 512 cells | Overlap selected through seam experiments |
| Layout painting resolution | 512 × 512 | Other resolutions, plus vector boundaries |
| Generation scales | 16 m, 4 m, 1 m | A versioned scale pyramid |
| Review clearance | 32, 64, 128 m | Vehicle- and scenario-specific values |

Use powers of two for convenient defaults; retain agency-native tile dimensions until preparation. Do not distort geography merely to fit power-of-two source tiles.

The 512 × 512 layout painting resolution is a human sketching overview only — at the recommended 32,768 m extent that is 64 m per pixel, too coarse to authoritatively place narrow passages, riverbanks, or fine coastal boundaries. The authoritative ownership map is a finer raster or vector boundary set derived from (and editable alongside) this coarse sketch, not the sketch itself.

A 1,024-cell tile at 1 m covers 1.024 km per side. It cannot teach the complete structure of a large caldera or fjord system alone. Derive aligned training examples at 4 m and 16 m from accepted detailed surveys to supply wider context. These intentionally downsampled examples are different from pretending an originally coarse DEM contains fine detail.

Use Float32 GeoTIFF/COG in physical metres for prepared and generated elevation. Maintain an explicit validity mask. For 16-bit grayscale exports, use a documented linear offset and scale shared across the whole world; nodata lives only in the separate validity mask, never as a reserved height code, so all 65,536 codes remain valid height values across the chosen range. A 4,096 m vertical range across 65,535 intervals has approximately 0.0625 m quantization steps. This is encoding precision, not a claim about survey or model accuracy.

Do not independently min/max normalize each tile. That would make a flat field and a mountain occupy the same intensity range and destroy cross-tile comparability. Training should use a documented fixed transform per model level, fitted on training data and frozen for validation. Record clipping rates. For residual generation, normalize the difference from the coarse surface with its own fixed physical scale and retain the coarse surface as an input.

Define cell-center coordinates, north/south row orientation, horizontal axes, world origin, and elevation units in the export contract. Distinguish raster cells from mesh vertices: a future mesh with 1,024 intervals needs 1,025 edge-inclusive vertices. Future border/halo samples must be read from the same authoritative surface. This preserves a clean handoff without designing the QuadTree now.

The master mosaic and all internal processing stay cell-center rasters (§5's cell counts, e.g. 32,768² for the full extent). The export layer is a separate, explicit conversion: each delivered tile is a shared-edge vertex grid, storing 1,025 × 1,025 edge-inclusive vertices for a 1,024 × 1,024-cell tile, with adjacent tiles required to share border vertices exactly (bit-identical, both read from the same authoritative cell-center surface). This resolves the cell/vertex distinction at the one boundary where it is otherwise easy to leave implicit.

## 6. Build the training dataset and controls

Label accepted geographic regions before cropping. Use human polygons supported by geology, slope, landform analysis, and imagery; a DEM alone does not reliably identify geology. Automatically proposed labels remain editable. Keep uncertain areas masked out of the supervised label loss.

Each example contains elevation, the categorical biome mask, validity/quality masks, physical spacing, source identity, and any coarse elevation or structural controls needed by the model. Derive the RGB conditioning image from the integer labels. Scale categorical masks with nearest-neighbor rules; prevent antialiasing and JPEG compression from inventing colors. Transition widths belong in a separate control layer, not in ambiguous label colors.

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

Final join repair can use constrained local blending, boundary-conditioned inpainting, or a gradient-domain correction within a marked seam band. It must preserve locked heights and named landmarks. Recheck water connections and nearby passages afterward; a visually invisible seam can still block a river or erase a saddle.

Exact placement is achieved operationally: retain the ownership map, enforce explicit constraints, reject unsuitable regional outputs, and allow local replacement or sculpting. Do not claim a learned model can guarantee a particular geological shape at every pixel.

## 9. Human editing and partial reruns

Use QGIS for initial selection, annotation, and numeric inspection, alongside a lightweight terrain preview capable of oblique views and repeatable camera paths. A custom editor can follow if these tools prove cumbersome. Image editors may paint categorical controls, but elevation editing must preserve physical values and export metadata.

Provide human checkpoints after candidate discovery, source quality review, crop/label selection, dataset release, model evaluation, coarse-world composition, regional generation, and export review. Batch operations should remain runnable between checkpoints. Reviewers can accept, reject, annotate, compare variants, or lock a region without restarting the workflow.

Store edits as ordered, reversible layers: additive height changes, masked replacements, landmark placements, water constraints, control-mask edits, and protected regions. Preserve both the original generated proposal and the edited revision. An exported raster from another tool becomes an explicit imported edit with a known extent and encoding.

A regional rerun selects a polygon, generation scale, model version, controls, and seed. Include a context halo and a writable transition band; freeze everything beyond that band. Condition on locked surroundings during generation, then copy the original values back outside the permitted edit domain to guarantee that those values remain unchanged. Validate the boundary afterward. Record the resulting variant separately until accepted.

Reruns follow actual dependencies. A small biome change may need only fine-detail regeneration around its boundary. Moving a major river or caldera rim can invalidate downstream water levels and several finer regions. Show the affected area before running; do not promise that every edit is local. Changing the source dataset or model does not silently overwrite already accepted world regions.

Cache by source checksums, configuration, model version, and controls. Interrupted downloads, preprocessing, and model runs resume from completed artifacts. Deterministic seeds support repeatability, but exact regeneration across different GPU/software stacks should not be assumed; retaining accepted output files is the strongest reproducibility guarantee.

## 10. Capacity, acceptance, and export

A 32,768² raster contains 1,073,741,824 cells. One uncompressed Float32 elevation layer is 4 GiB; a UInt16 export is 2 GiB; a byte biome mask is 1 GiB. Nine full-resolution Float32 one-hot biome channels would consume 36 GiB, so derive conditioning windows on demand instead of storing that representation globally.

The world contains 1,024 non-overlapping 1,024² delivery tiles. At a 512-cell stride, 1,024-cell inference windows yield 63 × 63 = 3,969 windows before extra edge padding. Each is revisited across denoising steps and model passes. Measure a representative regional run and extrapolate total compute, temporary storage, and I/O before scaling up. Global state and accumulation buffers may require storage in RAM or on disk; model windows alone do not account for all memory.

Use windowed reads/writes and overview pyramids. Keep staging space configurable and make compression error bounds explicit if using lossy numeric compression. Do not promise that a 4 GiB raster implies a 4 GiB generation working set. Generation runs against the fixed 16 GB VRAM budget (§7): the 3,969-window full-extent run is a long sequential (or batched-within-budget) job on this hardware, not a one-shot pass, so estimate wall-clock time from a measured per-window cost rather than assuming throughput.

Before promoting a world revision, verify:

- Correct extent, spacing, orientation, dimensions, units, and complete valid output coverage.
- Presence and minimum area of all finalized biome classes, plus human confirmation of recognizable terrain in each.
- No nodata leaks, clipping plateaus, unexplained spikes, or visible grid patterns.
- Height/slope agreement at seams, continuous named ridges and routes, and consistent water levels and connections.
- Preserved locked regions and successful representative low-altitude route inspections at the chosen vehicle envelope.
- Export quantization within half the chosen encoding step when there is no clipping; agreement between tiled and master elevation values.
- Complete provenance, versioned controls, source attribution, and a reproducible build manifest.

Tune numerical terrain tolerances against accepted real-source examples and the vehicle's clearance needs during the pilot. Keep exact invariants, such as untouched pixels outside an edit domain, separate from subjective judgments such as whether a canyon is enjoyable.

Deliver a Float32 master mosaic or equivalent indexed tile collection, configurable 16-bit height tiles, biome/water/validity masks, encoding and coordinate metadata, review overviews, and the world manifest. Export tiles are packaging units; they must not determine how the world was trained or generated.

## 11. Proposed implementation sequence

The first item is underway: [loadable presets](../../configs/README.md), a
[candidate legend](../../schemas/legend-v1.md), a
[source-description contract](../../schemas/source-v1.md), and
[initial acceptance criteria](../design/pilot-acceptance-v1.md) are implemented
or documented. Vehicle values and measured pilot tolerances remain explicitly
unset. These metadata checks do not complete the real-terrain milestone.

The second item has an initial [acquisition/import implementation](../workflows/dem-acquisition.md)
with raw integrity, resumable HTTPS staging and small native-grid review crops.
A real coastal USGS tile was imported; regional preparation, quality admission,
karst suitability and numeric model/loader evaluation remain open.

- [ ] Establish the initial candidate-class legend (nine as a working target), vehicle review envelope, world presets, source metadata contract, and acceptance criteria.
- [ ] Implement one numeric DEM acquisition path and one manual-import path; produce reviewable regional rasters before expanding provider coverage.
- [ ] Run an early karst acquisition trial, tracing source data through to several inspectable high-detail crops. Keep a failed candidate report if the geometry is not supported.
- [ ] Add the highest-value agency adapters, regional quality checks, human crop selection, and label editing.
- [ ] Release a geographically split pilot dataset and complete numeric loader/autoencoder round-trip evaluation.
- [ ] Compare LoRA plus a trained control branch against a terrain-specific representation if needed; benchmark quality, memory, and runtime.
- [ ] Build a 4,096 × 4,096 m pilot with several biome boundaries, a connected water feature, one protected landmark, and a demonstrably local rerun.
- [ ] Expand to all finalized classes (nine as a working target, adjusted per §1's acquisition-driven exceptions) and test coherent coarse-to-fine generation at the full extent, with human review between scales.
- [ ] Finalize editing, restart/rebuild behavior, export validation, provenance, and documentation.

The first milestone should be an inspectable collection of real terrain and a successful numeric precision test. Those results determine the diffusion architecture and data requirements before committing to a full-world training run.
