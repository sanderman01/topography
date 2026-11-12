# World description v1

`description.json` describes a generated world in words and map geometry. The layout writes
its targets; each cascade level and the export add the values measured on their surface. The
game reads it for names, passes, rivers, lakes and water bodies. Status: written by
`collage.py` (level `collage`) and by `cascade_chain --collage`: level `l3` (the 4 m surface
after hydrology) when L3 covers the whole map, else level `l2` (the 32 m surface after
hydrology) with `finer` for the L3 square. `cascade_l4` copies the chain's, level unchanged.
`topography export heightmap` writes the export's ([heightmap export v2](heightmap-export-v2.md)),
level `export`, with the water fields filled. Each cascade level writing its own measured
description is planned ([fujix v1](../docs/plans/fujix-v1.md), sub-project 5).

| Field | Meaning |
| --- | --- |
| `kind`, `schema_version` | `world_description`, 1 |
| `world`, `level` | World name; the surface measured (`collage`, `l2`, `l3`, `l4`, `export`) |
| `cell_m`, `extent_m` | Grid spacing of that surface; map size |
| `coordinates` | Metres from the north-west corner, x east, y south |
| `flight` | `clearance_m` and `bands` (`name`, `top_m` of flight height) |
| `regions[]` | `id`, `name`, `class_id` (terrain legend v3), `terrain`, `references` (GLO-30 region ids), `outline_m`, `target` (`area_km2`, `floor_m`, `summit_m`), `measured` (`land_km2`, `p5_m`, `p95_m`, `max_m`) |
| `passes[]` | `id`, `name`, `at_m`, `target.band`, `measured.ground_m` (lowest ground within 500 m), `measured.band` |
| `features[]` | `id`, `kind`, `name`, `at_m` or `path_m`, `target` (the layout's fields), `measured` (volcano: `summit_m`, `caldera_floor_m`, `caldera_rim_m`, `caldera_breach_m`, `caldera_diameter_m`, `caldera_at_m`; paths: `min_m`, `max_m`) |
| `finer[]` | Optional. Values on a finer level's window: `level`, `cell_m`, `window_m` [x, y, width, height], `passes` (id: `ground_m`) and `features` (id: `measured` as above) for points inside the window; `partial: true` where a feature's summit or caldera window, or part of its path, lies outside. No band reach, which needs the whole map |
| `measured_level` | Export only: the `level` of the description it copied (`collage`, `l2`, `l3`), null when none was copied |
| `water_bodies[]` | Export only. One row per export id of the `water-body` tiles, 1 to *n*: `id`, `world_id` (the world's `bodies.json` id), `kind` (`sea`, `lake`, `river`, `ice`: a frozen lake, a flat ice surface at its level), `area_km2` (wet level-0 cells in the export), `bounds_m` [x0, y0, x1, y1] or null, `parent` (export id of the body it drains into, 0 none), `level_m` (sea, lakes and ice) |
| `rivers[]` | Export only. One per river body with a 4 m centreline in the export: `body` (export id), `length_km` (the whole river's, from the 4 m hydrology), `source_m` (head of its longest path), `mouth_m` (null when off the map), `paths_m` (polylines of nodes [x, y, width_m, surface_m, discharge_km2], every fourth 4 m node plus the ends) |
| `lakes[]` | Export only. One per lake or ice body with 4 m cells in the export: `body` (export id), `kind` (`lake` or `ice`), `level_m`, `area_km2`, `outline_m` (closed rings of [x, y], cell edges simplified by one 4 m cell) |

Notes on the fields:

- `outline_m` is the authored outline from the layout. The collage bends every outline
  with a smooth warp (up to about 2.4 km), so the built regions do not follow it exactly.
- Region figures (`measured`) come from the collage's region map, `region32.u8`: each cell
  belongs to the region with the largest blend weight, so every land cell counts in exactly
  one region and the regions' `land_km2` add up to the island's land. Later levels pass the
  same map, resampled to their grid, to `description.measure`.
- `measured.band` is a `[flight]` band name, or `above` when no band reaches the pass.
- The caldera is the largest closed hollow that reaches within half of
  `caldera_diameter_m` of the volcano's `at`. `caldera_floor_m` is its lowest ground,
  `caldera_breach_m` the lowest point of its rim, where it would spill (a breach, when the
  layout has one), and `caldera_at_m` the hollow's centre. The rim is measured on 72 rays
  from that centre, each taking its highest ground within 0.8 of `caldera_diameter_m`:
  `caldera_rim_m` is the median of those crest heights and `caldera_diameter_m` the
  diameter of a circle with the area inside the crest. `caldera_floor_m` is null when there
  is no closed hollow. `summit_m` is the highest ground within `radius_m`, on the rim when
  there is a caldera.

Export notes:

- The base is the first `description.json` in the world, its `source_world`, or the layout
  directory. It is copied only when its `extent_m` equals the export's (a full-map export);
  a crop export keeps only the water fields, with empty `regions`, `passes` and `features`
  and a `note` saying why.
- Ids: export id 1 is always the sea (present even when the map has no sea cell, then with
  null `bounds_m`). Ids are dense 16-bit and match the `water-body` tiles; `world_id` links
  to the world's bodies. `bounds_m` is null for an id with no cell in the export.
- Rivers and lakes come from the 4 m hydrology (the world's `hydrology.4m`, else its source
  world's). Streams and tarns that exist only at 1 m are in the rasters and in
  `water_bodies` but have no `rivers[]` or `lakes[]` entry. A named 4 m hydrology that is
  missing or does not overlap the export fails the export; a world with none gets empty
  lines and a note.
- Lake rings: outer rings run clockwise on the north-up map and holes the other way; fill
  with the even-odd rule. Each ring is simplified on its own, so a ring can touch or cross
  its hole or itself; even-odd fill tolerates that.
- `description.md` and check examples name ids as "export id N" or "world body N".

`description.md` is the same content as tables for people.
