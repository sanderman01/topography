# Fujix v1

Status: design agreed with the owner 2026-10-26. Sub-project 1 (the collage layout) is
implemented and built as `data/worlds/fujix-collage-54` (55 with class 19), pending the
owner's review. Sub-project 3 is implemented in code (class 19, reaches, dithered borders,
gates); its training runs, L3 run 5 and L4 run 8, are pending. The others are not started. Fujix is a new
64 km world that replaces Fujion (32 km) as the game's map. It follows the
[layout spec process](world-layout-spec-v1.md), except that the agent authors the layout as
a collage of real DEMs and the owner neither writes nor paints it.

## Why

The airship game re-specced its ships between 2026-10-15 and 2026-10-21
(`airshipgame/docs/game-design/vehicles.md`, `world.md`). The map is now 64 km with an island
about 56 km across, peaks up to about 5,000 m, four ships with rated ceilings of 1,500, 2,000,
3,700 and 4,530 m, and flight about 200 m above ground. Altitude limits cargo, not access.
Fujion's tiers (1,500 and 3,000 m ceilings, 100 m clearance, reachable or not) no longer match.

Fujion also left problems the owner wants fixed:

- Painting the layout in GIMP did not give good results, and TOML primitives before it showed
  through the generated terrain.
- Region borders are hard lines.
- Lake surfaces are not flat or stand above the ground beside them. Stream water climbs the
  banks. Small streams break away from the rest of their stream or from their source lake.
- The water mask is binary, so the game's renderer guesses surface heights.
- The canyon class mixes five canyons. The owner wants one that reads as the Grand Canyon.

## Sub-projects

Each one gets its own task list in `TASKS.md` and its own branch.

| # | Sub-project | Needs GPU | Depends on |
| --- | --- | --- | --- |
| 1 | Fujix layout: collage, description, reach in game bands | No | |
| 2 | Hydrology v2 in Rust | No | 1 for testing on fujix |
| 3 | Grand Canyon subclass and dithered borders, L3 and L4 retrained | About 15 h | |
| 4 | Export manifest v2 and new layers | No | 2 for the water layers |
| 5 | Generate fujix at 64 km and export | About 50 to 75 h | 1 to 4 |

Sub-project 3 needs neither the layout nor the hydrology, so it can run alongside 1 and 2.

## 1. Layout

`configs/layouts/fujix/layout.toml` (layout schema v2) plus a readable `layout.md`. North is
up. The map is 65,536 m square. The coast stays at least 4 km inside the edge.

### Shape

Fujion's X stays. A volcanic rift runs SW to NE through a central volcano, and an uplifted
belt runs NW to SE across the wind. The windward half is star-shaped, with arms running out as
peninsulas and fjords between them. The lee half is plateau and desert, open to the NE and E
coasts, so every region can be reached round the coast. There is more mountain and rough
ground everywhere than in Fujion, with flatter pockets between.

The prevailing wind is from the south-west. Snowline is 1,600 m windward and 2,100 m lee, and
treeline about 1,000 m, as in Fujion.

### Regions

| Area | Terrain | References (GLO-30) | Ground height |
| --- | --- | --- | --- |
| Centre | Volcano with a summit caldera about 2 km across, its floor ice-filled near 4,250 m and breached on the north-west toward the canyon head; glaciers on the upper cone; flank cones | Kibo on Kilimanjaro, Shasta for the flanks | 0 to 5,000 m |
| Rift, SW to NE | Cinder cones and lava fields running into the NE desert | Craters of the Moon, Etna | 300 to 1,500 m |
| SW arm | Alpine ridges, cirques, glaciers, horn peaks | Zermatt, Aoraki/Mount Cook, Aletsch | 800 to 4,400 m |
| W coast | Fjords between the arms, one outlet glacier with an outwash fan | Milford Sound, Geirangerfjord | Walls to 2,500 m |
| NW arm, belt | Knife ridges inland, sea cliffs on the coast, skerries and an archipelago offshore | High Tatras, Na Pali, Senja | 0 to 2,800 m |
| S coast | Drowned tower karst in a bay, cone karst inland | Ha Long Bay, Wanfenglin | 0 to 700 m |
| SE, belt | Limestone range, then wet lowland: a braided river from the volcano to a small delta, rolling farmland pockets | Dolomites, Rakaia, Tagliamento | 0 to 3,000 m |
| N, lee | Grand Canyon plateau: about 25 km of canyon running north-east from the volcano's north-west flank to the north coast, rims falling from about 2,400 to 1,500 m, the floor from about 900 m to the sea, a snowmelt river from the volcano through it | Grand Canyon only | 0 to 2,400 m |
| NE, lee | Badlands and mesas; star dunes banked against the end of the rift, crest dunes toward the coast | Monument Valley, Bardenas Reales, Great Sand Dunes, Taklamakan | 0 to 1,600 m |
| E, lee | Desert basin open to the coast, with oases | Sossusvlei, Erg Chebbi | 0 to 1,000 m |

The volcano stays the highest point by at least 400 m. Its base radius is about 10 km, about
7% of the map (Fujion's took about 19%). No cone sits inside the caldera, and the caldera holds
ice, not a lake.

Flatter ground: the SE farmland, intermontane basins north of the SW arm, desert pans in the E
and the plateau top at the canyon rim. Each region keeps at least one patch under 5 degrees for
a settlement. No large floodplains.

### Passes

Bands follow the game's flight heights, ground height plus 200 m
(`airshipgame/docs/game-design/world.md`, "Passes and routes").

| Band | Ground at the pass | Where |
| --- | --- | --- |
| Low | Up to 1,300 m | The coast is always followable. Each arm has a low pass far out on it, a detour |
| Middle | 1,300 to 2,300 m | Saddles between the volcano and the SW and NW ranges; across the plateau. Low ships can fly the canyon where it is wider than their turning circle |
| High | 2,300 to 3,500 m | Cols in the alpine arm; the volcano's shoulder near 3,000 m |
| Top | 3,500 to 4,300 m | The highest cols beside the horn peaks. Only a light liner or racer reaches the caldera floor |

### Collage

A new module, `topography_training.diffusion.collage`, builds the world from `layout.toml`:

1. Each region lists its references with a crop centre, a rotation, a horizontal scale of at
   most 1.5 times and a height remap from the reference's floor and summit to the region's
   envelope. Heights come from accepted regions of the context release (`context-v5`, which
   has Kilimanjaro); regions the owner rejected (Timanfaya, Lofoten, Algodones) are not used.
2. Each region gets a weight from its signed distance to its border and that border's width:
   about 300 m at canyon rims and range fronts, 2 to 4 km elsewhere. A Laplacian pyramid blends
   heights, with wide bands for large-scale shape and narrow bands for detail, so ridges do not
   double and no steps appear. A blue-noise dither of the same weights picks each cell's class.
3. Land is pushed down to seabed in the outer 4 km. The seabed slopes to about -200 m at the
   map edge, coarse only.
4. Pieces are placed so real cols fall where passes belong. A col that is too high gets a
   smooth dip with a wavelength of at least 3 km and a depth of at most 300 m. The check lists
   every dip.
5. The collage is built at 32 m, the resolution of GLO-30. It is the L2 surface, and L1 is its
   128 m mean, so fujix skips the L1 and L2 models. The L3 and L4 models add detail.

Checks: area per region, volcano margin, coast distance, every pass in its band, and a seam
score from slope breaks along borders. `layout.py reach` moves from two walls to the game's
four ceilings, 200 m clearance and cargo bands; `[tiers]` becomes `[flight]` with
`clearance_m` and the band edges 1,500, 2,500, 3,700 and 4,500 m. Outputs are the overview
map and the reference sheet, for the owner's Yes/No.

### Description

The descriptive part of `layout.toml` runs through to the export:

- **Layout.** Region names, terrain, references and envelopes. Passes with name, location,
  target ground height and band. The volcano, caldera and canyon.
- **Generation.** Each level writes `description.json` with the target and the value measured
  on that level's surface: summit and caldera heights, each pass's saddle height and band,
  region areas. Rivers (length, source, mouth) and lakes (surface level) join once hydrology
  makes them. The check flags drift, such as a pass that leaves its band at L4.
- **Export.** `description.json`, with its schema in `schemas/` and a link from
  `manifest.json`, plus a readable `description.md`. Geometry is in map metres: region
  polygons, pass points, feature points and lines, river polylines with width and surface
  height per node, lake polygons with their levels, and a table of water bodies giving each
  `water-body` id its type. This also covers the optional vector data
  in the game's water contract.

## 2. Hydrology v2

Water is a surface height. A cell is wet only where that surface is above the ground. The
wet extent follows from the two heights, so a lake with one level is flat, and water cannot
climb a bank.

| Step | Level | What happens |
| --- | --- | --- |
| Moisture | 128 m | Orographic precipitation from the SW wind, with the Smith and Barstad (2004) linear upslope model. It feeds discharge, the coarse `moisture` layer, vegetation and snow |
| Trunk network | 32 m | Priority-flood drainage weighted by precipitation. Depressions become lakes at their spill level, within the existing per-class lake budgets (`Water` in `hydrology.py`); the rest are breached, not filled. A few iterations of implicit stream-power erosion (Braun and Willett 2013) cut valleys across collage seams and barely change real ones. Rivers start at about 10 km² of drainage |
| Tributaries | 4 m, then 1 m | Each level routes flow on its own surface with the coarser network held fixed, and adds streams from about 0.5 km² at 4 m and 0.05 km² at 1 m. Every stream ends at a confluence, a lake or the sea |
| Channels | Each level | Width and depth from discharge, roughly W ∝ Q^0.5 and D ∝ Q^0.4, so rivers grow downstream. A smooth cross-section is carved below a surface that never rises downstream. Mouths slope down into the sea |
| Lakes | Each level | One level per lake id at every level. After each level, the bed inside stays below that level, the shore outside stays above it, and the outlet is carved to it |
| Water layers | Each level | Surface height and body id are stored. Coverage is derived at export |
| Validation | Each level | Surface at or above the bed where wet, one level per lake, a connected network with no orphan streams |

The models do not change: water stays out of the loss. Hydrology runs between levels and
between L4 bands.

It is written in Rust, as `world::hydrology` in `topography-core`, and run by the cascade
through `topography world hydrology`. The Python `heapq` flood is already slow at 32 m, and 1 m
across 64 km is 4.3 billion cells. `hydrology.py` is retired once v2 runs.

## 3. Grand Canyon subclass and dithered borders

- New subclass 19, key `grand_canyon`, parent 13. It trains only on Grand Canyon cells
  labelled canyon (57% of the region per [biome forms](biome-forms-v1.md)), from GLO-30,
  lidar and 1 m tiles, with one reach held out for the geographic split. Class 13 keeps its
  five canyons. Fujix's 32 m surface is real Grand Canyon, so only L3 and L4 need the class.
  If there is too little data to pass the gates, more public Grand Canyon tiles are acquired.
- Classes already enter each model as one-hot channels, so a dithered border adds no inputs.
  Training applies the same blue-noise dither, with random band widths, across the natural
  borders in the per-cell labels, such as canyon and plateau. The models then see at training
  time the input that fujix gives them.
- Gates, per [class conditioning](class-conditioning-v1.md): swap `grand_canyon` against 13 and
  against plain ground, and a dithered border against a hard one, at L3 and L4. Review on fujix
  as well as on held-out real tiles.

Implemented (2026-10-26; how to run it in [the cascade workflow](../workflows/terrain-cascade.md#grand-canyon-and-dithered-borders-fujix)):
the single accepted Grand Canyon lidar region is trained as four reaches, three train and
the south-west one held out, so class 19 has three L3 sources without a new vote. The five
1 m Grand Canyon tiles keep their northing bands. Training dithers borders with
`--dither-m 1500` (bands of 0 to 1.5 km, hard in a quarter of windows). `fujix-collage-55`
is build 54 with the canyon plateau as class 19: identical heights and report.

## 4. Export

Manifest `schema_version` 2. Height tiles do not change: 1 m, 16-bit, 257 squared,
`shared_edge`. Every other layer gets a 4-texel apron, per the game's water contract in
[export-data-flaws.md](../export-data-flaws.md). Every level comes from a global pyramid before
tiles are cut: area mean for heights and continuous layers, mode for classes. Nothing is
decimated.

| Layer | Encoding | Finest level | Coarse filter | Notes |
| --- | --- | --- | --- | --- |
| `height` | 16-bit, as now | 1 m | Mean | |
| `water-surface` | 16-bit, the height encoding | 1 m | Mean within one body | Sea at 0, extended 4 texels past the shore |
| `water-coverage` | 8-bit area fraction | 1 m | Mean | Streams narrower than a texel keep their share |
| `water-body` | 16-bit id | 1 m | None, sampled | Ids match `description.json`, which gives each body's type: sea, lake or river |
| `biome` | 8-bit ids, dithered borders | 2 m | Mode | |
| `land-cover` | ESA WorldCover codes, dithered borders | 2 m | Mode | |
| `vegetation-density` | 8-bit, 0 to 1 | 2 m | Mean | Procedural, from moisture, treeline and slope |
| `snow-cover` | 8-bit, 0 to 1 | 2 m | Mean | Soft edges |
| `talus-density` | 8-bit, 0 to 1 | 2 m | Mean | Procedural, scree fans below steep rock |
| `moisture` | 8-bit, 0 to 1 | About 128 m | Mean | Coarse levels only |

- Land cover is dithered like the biome layer. Each hard rule in `landcover.py` becomes a ramp:
  forest from `vegetation-density`, snow from `snow-cover`, bare rock over a slope band around
  35 degrees, grass to bare ground over a band above the tree line. A blue-noise value per
  texel picks the class, so forest edges, snow lines and scree margins read as a fine mix
  instead of a line. The blue noise is the collage's, moved to a shared helper.
- Water files exist only for tiles that hold water. The manifest states that a missing water
  file means no water in that tile.
- `manifest.json` gains `legends` in the shape the game asked for in
  [game-vegetation-data-requests.md](../game-vegetation-data-requests.md), with stable keys and
  id 0 named `unknown`. Each layer gets metadata: prefix, format, scale, offset, unit, finest
  spacing, coarse filter, procedural or observed, and the meaning of a missing file.
- The binary `water`, `snow-ice` and `vegetation` masks are dropped. The game keeps its
  fallbacks for v1 exports.
- The exporter checks: surface at or above the bed where wet, one level per lake at every
  level, coverage at the finest level matching surface above ground, identical shared edges.

The game's water contract also asks for shoreline distance, type fractions and velocity. They
are left out because the game can derive them: the shore is where surface minus ground crosses
zero, type comes from the body id, and velocity follows the surface slope and depth (Manning)
and is zero in lakes. The world keeps surface, body id and ground, so adding them later needs
only a re-export. From the game's vegetation requests, `hollow`, `treeline-margin` and
`needleleaf` are not exported.

## 5. Generation and cost

Estimates scale Fujion's measured 32 km run (`fujion-v3-chain-14-l4`) by four.

| Resource | Estimate | Notes |
| --- | --- | --- |
| Export | 4 to 6 GB | Height about 2.6 GB. 7 to 10 GB if every layer were dense at 1 m |
| Workspace | 40 to 55 GB | 1 m height f32 is 17 GB; each u8 layer 4.3 GB |
| GPU, generation | L3 about 4 h, L4 about 45 to 70 h | L4 skips bands that are all sea (new); 70 h without that |
| GPU, training | About 15 h | L3 about 6 h, L4 about 8 h, more if a gate fails |
| VRAM | About 9 GB peak | As now |
| RAM | Under 7 to 10 GB | 512 m L4 bands, since bands span the map width. Hydrology at 4 m on the whole map, about 4 to 6 GB |
| CPU | Collage minutes, hydrology about 1 to 3 h, export a few hours | Measured on the collage and a 4 km L4 check first |

A central 4 km L4 check (about 15 min) runs before the full L4. Every long run waits for the
owner's go. Old Fujion worlds can be pruned afterwards, with the owner's say-so.

## Not in scope

- Lakes or rivers placed by hand. Hydrology makes them.
- Painting in GIMP. `paint.py` stays for other worlds.
- Settlements, routes and jobs, which belong to the game.
- Retraining L1 and L2 for the new classes.
