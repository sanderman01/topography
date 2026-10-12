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
