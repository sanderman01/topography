# World description v1

`description.json` describes a generated world in words and map geometry. The layout writes
its targets; each cascade level and the export add the values measured on their surface. The
game reads it for names, passes, rivers, lakes and water bodies. Status: written by
`collage.py` (level `collage`) and by `cascade_chain --collage`: level `l3` (the 4 m surface
after hydrology) when L3 covers the whole map, else level `l2` (the 32 m surface after
hydrology) with `finer` for the L3 square. `cascade_l4` copies the chain's, level unchanged. Hydrology
bodies and the export are planned ([fujix v1](../docs/plans/fujix-v1.md)).

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
| `rivers[]`, `lakes[]`, `water_bodies[]` | Empty until hydrology v2. `water_bodies` will map each `water-body` id to its type (sea, lake, river), surface level and bounds |

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

`description.md` is the same content as tables for people.
