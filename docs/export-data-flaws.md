# Flaws in exported terrain and mask data

Observed while building the airship game's landscape renderer on the export
`fujion-v3-chain-14-l4` (32 km at 1 m, `heightmap_export`, shared-edge tiles). Each entry says
what is wrong, where it was seen, how the game currently works around it, and what a future
training run or export should do instead. Map coordinates are meters east and south of the map's
north-west corner.

The game works around all of these, but every workaround costs shader time or blurs real detail.
Fixing them at the source is the better path.

## Water

### The water mask is binary and texel-stepped

The water layer is 0 or 255 per 1 m texel, with no partial coverage. River banks, lake shores and
the coastline are staircases with 1 m steps, and diagonal streams are chains of single texels.

- Seen: every river and lake; for example the stream at (9419, 8396) and the lakes around
  (8680, 15800).
- Workaround: the game reconstructs a smooth outline from a B-spline of a blurred mip of the
  mask, and hides what is left under foam.
- Fix: replace the on/off mask with a signed distance field: meters to the nearest shoreline,
  negative on land, positive in water. It resamples and thresholds without stairs at any level of
  detail, and the game would use it for:
  - blending terrain color into water and wet banks by distance, not by a blurred binary mask;
  - flow and foam: shore foam by distance to the bank, river width from the distance at the
    centerline, and flow fields that do not turn toward stair-stepped banks;
  - flattening: smooth bank profiles and water surfaces shaped by distance from the shore, instead
    of the game's own mask closing, blurring and sinking.

  Coverage (fraction of each texel under water) would be a smaller step, but loses the distance
  information the uses above need.

### Rivers are carved along the mask's texels

Channels are cut into the heightmap exactly where the mask is set, so channel walls are vertical
steps of up to several meters that follow the texel staircase. Cross-sections are not smooth.

- Seen: the stream at (7188, 21708), whose banks render as a zigzag wall.
- Workaround: the game box-blurs heights within two vertices of water before building meshes,
  and fills channels up to the lowest bank (at most 4 m above the bed).
- Fix: carve with a smooth profile (for example a cosine cross-section) driven by distance to
  the river centerline, not by the binary mask.

### Streams one texel wide, some ruler-straight across ridges

Some streams are a single texel wide, and some of those run as a perfectly straight 45-degree
line that ignores the terrain, crossing ground several meters higher than its surroundings
instead of following a valley.

- Seen: the stream through level 0 tile (15, 16), around (4030, 4305), on ground at 5 to 10 m.
- Workaround: the game widens water thinner than about 6 m before shaping and draws narrow water
  from a finer mask level; otherwise such streams rendered as a dark, near-transparent film.
- Fix: give streams a minimum width (or a width from flow accumulation) and route them along
  the terrain's drainage, never in straight lines.

### Lakes are flattened along the mask, and the two disagree

Lakes are flattened to their surface, but only where the mask is set, and the mask also covers
ground that is not flattened. In the lake tile at level 2 (4, 20), whose surface is 216.8 m, wet
texels range from 177.6 m (1st percentile) to 217.3 m (99th). Dry ground inside the outline
pokes out of the lake, and parts of the bed sit tens of meters lower.

- Workaround: the game takes each wet vertex's level from the lowest wet ground around it,
  never the highest, and treats only narrow water as flowing.
- Fix: give each lake one water surface elevation over its whole area and derive the mask from
  that area, so both always agree. Keep the lake bed below the surface: do not flatten the ground
  itself (`hydrology.py::drain()` currently replaces it with `np.where(keep, filled, h)`). The game
  needs the real depth for water color and to place boats.

### Some lakes have rectangular outlines

Several lakes have long straight edges and right angles, which look like processing artifacts
rather than shorelines.

- Seen: the lakes around (8680, 15800), clearly visible in the level 2 water tiles (7..9, 14..16).
- Workaround: none.
- Fix: find where the straight edges come from (a bounding box, a tile boundary or a crop during
  generation) and remove it.

### Streams stop short of the water they feed, or end mid-slope

Streams end a few texels before the lake they run into, leaving a dry gap.

- Seen: the stream entering the lake at about (8141, 16128) stops roughly 9 m short. The stream
  at about (7300, 21932) simply ends on a hillside.
- Workaround: the game closes gaps up to about 12 m in the water mask on load, except within
  24 m of a tile edge, where neighbors could disagree. Gaps on tile edges stay open.
- Fix: guarantee river connectivity in the export: each river reaches a lake, the sea, a
  confluence or the map edge.

### Rivers reach the coast above sea level

River beds stay 2 to 4 m above sea level until about 15 m from the coast, and the wet ground
between them and the sea lies 0 to 2 m above it. A river therefore ends in a step above a strip of
dry beach instead of running into the sea.

- Seen: the river mouth at about (4225, 12270).
- Workaround: near a river's end the game sinks that strip under the sea and eases the river's
  surface and bed down to sea level over about ten vertices.
- Fix: carve river beds down to sea level (or below) where they meet the sea.

### River mouths are shallow

Even once a river reaches sea level, its bed at the mouth lies just under the surface, much
shallower than the sea beside it. Rendered, the mouth is a pale strip of sandy shallows and shore
foam between the river and the sea, which looks wrong for a steep stream that should run straight
into open water.

- Seen: the river mouth at about (4225, 12270).
- Workaround: the game deepens the bed toward the sea, ending 2.5 m deeper at the coast, and lets
  the banks slope down into it.
- Fix: carve mouths as deep as the sea next to them, with depth depending on the river: deep for
  steep streams, shallow bars only where a flat river deposits sediment.

### No water surface elevation, flow direction or water type

The export has no layer for the water's surface elevation, flow direction or whether water is a
river, a lake or the sea. The renderer has to guess all three from the mask and the heights,
which is fragile:

- the flow direction, taken from noisy local levels, pointed toward banks instead of
  downstream until it was smoothed over about six vertices;
- lakes whose beds vary at coarse levels looked like rapids;
- sea and lakes are told apart by height near 0 m, which fails for coastal lakes.

Fix: export, per texel or per water feature:

- water surface elevation;
- flow direction and speed (or flow accumulation, from which both follow);
- water type: sea, lake, river.

### Coastline mask and heights disagree

Water texels along the coast lie up to about 2 m above sea level, and some land texels lie
below it.

- Workaround: the game treats wet ground within 2 m of sea level as sea, and draws the shoreline
  where the terrain rises through 0 m, ignoring the mask.
- Fix: derive the coastline mask from the heights (ground below 0 m and connected to the open
  sea), or adjust the heights to match the mask.

## Levels of detail

### Coarse levels are decimated, not filtered

Level k keeps every 2^k-th sample of level 0, for both heights and masks. Verified on level 1
tile (15, 40): its heights and water mask equal level 0 decimated by two exactly, and its heights
differ from a 2x2 average by up to 2.7 m.

- Thin streams flicker in and out of existence between levels, and coarse masks alias.
- Coarse heights carry level 0's noise unfiltered, so a lake's level can step between vertices.
- Fix: area-average heights and coverage masks for each coarser level. Class layers (biome, land
  cover) should take the most common class, not a point sample.

## Classes

### Unexplained rectangle of different ground near the south coast

A tile-shaped rectangle of different ground shows near (17000, 25300) at level 0. It is not a
level-of-detail mismatch and not a missing tile; every layer has the same tile count. Not yet
diagnosed. It may be a biome or land-cover tile with a different class.

- Fix: check the class layers around level 0 tiles (66, 98) to (67, 99) for a whole-tile
  class change.

## How the game will draw water

Decided in the game's water plan (`airshipgame/PLAN-water.md`):

- Rivers, streams and lakes are drawn by the terrain's own pixel shader. For each terrain pixel
  under water it intersects the view ray with the water surface, shades the water there and writes
  that surface's depth, so boats and other objects below the waterline are hidden. There are no
  separate inland water meshes, skirts or game-side shaping of the ground any more.
- The sea is a flat surface at sea level from the shoreline to the horizon, shaded with the same
  data.
- All the workarounds listed above are deleted once an export provides the data below.

## Water data contract for the next export

Narrowed 2026-10-26 in [fujix v1](plans/fujix-v1.md#4-export): the export carries the water
surface elevation, water coverage and water body id, plus a body table with each body's type
in `description.json`. The game derives the shoreline distance, type fractions and velocity
(`airshipgame/PLAN-water.md` §0 and §3.1). The table below is the game's original request.

### Required layers

Per level and tile, on the same grid as the heights:

| Layer | Meaning |
| --- | --- |
| Ground elevation | The bed under water, not flattened. Lakes keep their bathymetry. |
| Water surface elevation | One value per lake; a continuous profile along rivers, also through junctions and into lakes; 0 for the sea. Extended a few texels past the shoreline (nearest water value) so it can be filtered across the waterline. |
| Shoreline signed distance | Meters to the nearest shoreline, positive in water, negative on land, saturated at a documented distance (for example 64 m). |
| Water coverage | Area fraction of each texel under water, filtered per level independently of the distance field, so streams narrower than a texel keep their contribution at coarse levels. |
| Water type fractions | Fractions of sea, lake and river per texel that sum to the coverage. Never a majority-filtered class: minority types must survive filtering. |
| Water body id | A stable id per connected lake or river system, sampled without filtering. Lets the game keep one surface per lake and never blend levels of two different lakes. |
| Surface velocity | Map X and Z in m/s, with exact zero for still water. Document how it is estimated (routing, Manning n, assumed channel geometry). |

Plus metadata in the manifest: encodings, units, datum, sample placement, apron size, a schema
version, and what missing data means.

### Optional layers (do not block the export on them)

- Breaking potential for rapids (for example slope times discharge), 0 to 1.
- Sea exposure: how open a coast is to swell, 0 to 1.
- Optical subtype (clear, dark humic, turbid or glacial) and shore material (rock, sand, mud).
- Vector data per map: river polylines (width, surface elevation, discharge per node), lake
  polygons and levels, waterfall points. Useful for VFX, audio and labels.

### Repairs before deriving the layers

The distance field and the other layers only help if the geometry they come from is right:

1. Separate bed and water surface (see "Lakes are flattened along the mask").
2. Connected river networks: every river reaches a lake, the sea, a confluence or the map edge.
3. Routing along the terrain's drainage; no straight streams across ridges.
4. No rectangular lake outlines.
5. Lake outlets and river levels consistent with the lake's surface.
6. Sea defined by connection to open water, not by height alone.
7. River beds carved with a smooth cross-section below a continuous surface, width and depth from
   discharge. No universal minimum width; state the source resolution limit instead.
8. Mouths with a plausible profile down into the sea, bars only where a flat river deposits
   sediment.
9. Shore profiles by shore type (rock, sand, mud), not one beach rule everywhere.

### Filtering rules for coarser levels

- Build each level from a global pyramid before cutting tiles, so neighbors agree on shared edges.
- Area-average heights and coverage; never decimate.
- Never average water surface elevations across different water bodies.
- Do not let coarse terrain fill channels or lake margins that are still visible at that level;
  export error bounds or another way for the game to detect it.
- Keep each lake's surface elevation identical at every level.

### Aprons

The game no longer reshapes ground near water, so the large neighborhood apron (about 48 texels)
is no longer needed. An apron only has to cover the game's filter kernel: start with 4 texels at
every level (cubic B-spline sampling needs 2).

### Validation the exporter should run

- Water surface at or above the bed wherever coverage is set.
- One surface elevation per lake id, at every level.
- Distance field sign agrees with coverage at the finest level.
- Velocity exactly zero in lakes.
- Shared tile edges identical between neighbors at every level.

### Not needed

Horizon or sky-openness data: deferred.

## Export `fujix-55-l4v9`

Observed on 2026-11-15 while moving the game to `fujix-55-l4v9` (65.5 km at 1 m, schema 2). The
water flaws above are fixed in this export: real lake beds, one level per lake at every level,
continuous river mouths, a surface extended past the shore, and every validation check at zero.
What is left:

### `lakes[].outline_m` disagrees with `water_bodies[]` and the tiles

In `description.json`, some `lakes[]` entries have an `outline_m` far from the lake. The
`water_bodies[].bounds_m` of the same body agree with the `water-body` tiles.

- Seen: lake body 688 (level 1060 m). Its `outline_m` is near (47590, 25810), where the ground is
  445 m and there is no water. Its `bounds_m` are x 43059..51005, y 43444..46564, as in the tiles.
  Lakes 417 and 644 show the same mismatch.
- Workaround: the game reads `water_bodies[]` (kind, `level_m`, `bounds_m`) and ignores
  `lakes[].outline_m`.
- Fix: write both from the same body, in export ids, and validate that each outline lies inside
  its body's bounds.

### Class layers are white-noise dithered, and the mode filter does not preserve the dither

At L1 (2 m), `biome` and `land-cover` borders are an independent random draw per texel along a
probability ramp: about 100 m wide for biomes, 45 to 55 m for cover, with isolated texels up to
300 m into the other class. Coarser levels take the mode, which collapses the ramp to an edge of
1 to 4 texels and drops sparse classes.

- Seen: `biome_L1_3_21` against `biome_L3_0_5`; `landcover_L1_50_40`. Over one 2 km square,
  grassland (cover 30) is 6.0% of L1 and 3.5% of L3.
- Effect: a blend drawn from the tiles changes width and class mix with the level of detail, so
  colors and plant density pop when tiles split.
- Workaround: the game filters one-hot class channels itself over a fixed radius in meters.
- Fix: export the blend, not a sample of it. For example the two strongest class ids plus a
  weight per texel, or a fraction per class, area-averaged per level like coverage. If the dither
  stays, state its ramp width in the manifest and build coarse levels from the ramp, not from the
  mode of the dithered texels.

### The sea surface decodes to -0.006 m

`water-surface` shares the height encoding (`h_min_m -498.777`, `step_m 0.0835185`), in which 0 m
is not a code. The sea is code 5972, which decodes to -0.006 m. Lake levels are quantized the same
way, by up to 0.042 m.

- Workaround: the game takes sea and lake levels from `water_bodies[].level_m` by body id.
- Fix: either keep it and say so in the manifest (flat bodies take `level_m`, the tile value is
  approximate), or choose `h_min_m` so that 0 m falls on a code.

### Minor

- Coarse levels have wet texels whose mean ground is above the mean surface
  (`coarse_wet_below_ground` in `report.json`: 156k texels at L1, 151 at L8). Expected from area
  means in narrow channels; the game clamps depth to zero there.
- `moisture` exists only at L7 and L8, and the class and density layers start at L1, normal and
  roughness at L2. The game reads an ancestor tile for finer levels. Fine as is; listed so it is
  a decision, not an accident.
- 79 `ice` bodies are exported as a flat surface at the lake level; the game has no ice material
  yet.
- `normal` and `roughness` have no apron (`apron_texels: 0`), while every other layer has 4. A
  filtered lookup clamps at the file edge, so neighbouring tiles meet with a step of one cell's
  change. The files are BC-compressed, so the game cannot assemble a window across files on the
  CPU. A one-texel apron (padded to the BC block size) would remove the step.

### Found while drawing the water and terrain (same export)

- **Axis-aligned notch at the main river's mouth.** The sea mask at about (43253..43300,
  56800..56850) has a straight north-south edge 40 m long and a straight top, with a round pool
  3 m deep inside. It shows from 600 m up.
- **Rills on steep faces.** 10 977 river bodies; on the face at (18358, 53006) about twenty
  parallel streams 1 to 2 m wide run at slope 2.0. Prune by catchment or discharge, or mark them
  so the game can draw them as wet rock.
- **Steep final reaches.** The main river (body 8437) drops 22.5 m over its last 88 m. Rivers no
  longer reach the coast above sea level, but the grade into the sea is a torrent.
- **River surface in 8 cm steps.** `water-surface` shares the height code (0.0835 m), so calm
  rivers are staircases; the game averages over 6 m. A finer step for this layer, or a float
  layer, removes that.
- **Surface extension without a body.** The 4 texels past the shore have body 0 and step up to
  4.9 m between neighbours. Carry the body id into the extension and extend along the channel.
- **Diagonal one-texel streams** are 8-connected only at L0. Make channels 4-connected or two
  texels wide.
- **`snow_ice` and `tree_cover` are dithers of the snow and vegetation fractions.** The game
  ignores `snow_ice` and draws forest from `vegetation-density`. Say in the legend which classes
  derive from a fraction, or stop dithering them.
- **`vegetation-density` scale.** Closed forest is 0.65, found by measurement. State what 1.0
  means.
- **`roughness`** changes shading by under 2 %; the game no longer reads it. Drop it or encode
  something with range.
- **Flow.** Width, discharge or a falls flag per reach would replace the game's slope thresholds
  for rapids and falls.
- **Class levels and distance.** With mode-filtered class levels the game reads L1 classes up to
  L2 tiles; from L3 (about 3 km) the class mix shifts: 5.7 % mean luminance on dunes and cinder,
  25 % of a border band changes by over 10 %. Class fractions per level remove it.

### Found while tuning biomes and land cover (same export, 2026-11-16)

- **`talus-density` is a slope band, not deposition.** Measured at L4 over the whole map: mean
  density by 10° slope bin is 0.001, 0.004, 0.043, 0.35, 0.205, 0.012, 0.001 (0° to 70°). Within
  the 25° to 40° band, ridges (top 10 % of height minus the 112 m local mean) carry 0.42, valley
  floors 0.39 and mid-slopes 0.20; footslopes at 10° to 25° carry 0.003. In the game this puts
  rubble and boulders along ridge crests and almost none in gullies or at the foot of cliffs.
  Talus should come from a source and a sink: debris sheds from steep or exposed rock (cliffs,
  crests) and collects downslope where the grade drops below the angle of repose, in gullies,
  on fans and on valley floors. Crests and convex ground should be near zero.
- **No vegetation along water in the dry biomes.** `vegetation-density` is exactly 0 in
  `badlands`, `star_dunes`, `crest_dunes` and `grand_canyon`, including within 16 m of rivers and
  lakes (about 86 000 bank texels at L3). Add a narrow riparian band, 10 to 40 m wide, of modest
  density (0.1 to 0.3) around lakes and perennial rivers there, and a `land-cover` class to
  match. The game adds its own band from the water fields for now.
- **No temperature layer, and `moisture` only at L7 and L8 (128 m).** The game picks plant
  species by biome alone, so southern karst and western fjords differ only by biome palette. A
  `temperature` layer (latitude, altitude, coast) and `moisture` down to about L4 would let it
  pick species by climate: tropical in the warm wet south, conifers on cold slopes.
