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
- Fix: export water as coverage (8-bit fraction of each texel under water) or, better, as a
  signed distance to the shoreline in meters. Either can be resampled and thresholded without
  stairs.

### Rivers are carved along the mask's texels

Channels are cut into the heightmap exactly where the mask is set, so channel walls are vertical
steps of up to several meters that follow the texel staircase. Cross-sections are not smooth.

- Seen: the stream at (7188, 21708), whose banks render as a zigzag wall.
- Workaround: the game box-blurs heights within two vertices of water before building meshes,
  and fills channels up to the lowest bank (at most 4 m above the bed).
- Fix: carve with a smooth profile (for example a cosine cross-section) driven by distance to
  the river centerline, not by the binary mask.

### Lakes are flattened along the mask, and the two disagree

Lakes are flattened to their surface, but only where the mask is set, and the mask also covers
ground that is not flattened. In the lake tile at level 2 (4, 20), whose surface is 216.8 m, wet
texels range from 177.6 m (1st percentile) to 217.3 m (99th). Dry ground inside the outline
pokes out of the lake, and parts of the bed sit tens of meters lower.

- Workaround: the game takes each wet vertex's level from the lowest wet ground around it,
  never the highest, and treats only narrow water as flowing.
- Fix: flatten the whole lake area to one surface elevation, then derive the mask from the
  flattened area so both always agree.

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

## Wishlist for the next export

- Water as coverage or signed distance, not binary.
- Smooth river cross-sections, carved by distance to the centerline.
- Lakes flattened over their full area, with the mask derived from that area.
- Connected river networks.
- Water surface elevation, flow and type layers.
- Filtered levels of detail: averaged heights and coverage, majority classes.
- An overlap apron around each tile (about 48 texels of the neighbors' data). The game shapes
  ground and water near rivers with neighborhood operations, and without an apron it has to read
  up to eight neighbor files per tile so that adjacent tiles agree on their shared edge.
