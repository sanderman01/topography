# World description v1

`description.json` describes a generated world in words and map geometry. The layout writes
its targets; each cascade level and the export add the values measured on their surface. The
game reads it for names, passes, rivers, lakes and water bodies. Status: written by
`collage.py` (level `collage`); cascade levels, hydrology and the export are planned
([fujix v1](../docs/plans/fujix-v1.md)).

| Field | Meaning |
| --- | --- |
| `kind`, `schema_version` | `world_description`, 1 |
| `world`, `level` | World name; the surface measured (`collage`, `l3`, `l4`, `export`) |
| `cell_m`, `extent_m` | Grid spacing of that surface; map size |
| `coordinates` | Metres from the north-west corner, x east, y south |
| `flight` | `clearance_m` and `bands` (`name`, `top_m` of flight height) |
| `regions[]` | `id`, `name`, `class_id` (terrain legend v3), `terrain`, `references` (GLO-30 region ids), `outline_m`, `target` (`area_km2`, `floor_m`, `summit_m`), `measured` (`land_km2`, `p5_m`, `p95_m`, `max_m`) |
| `passes[]` | `id`, `name`, `at_m`, `target.band`, `measured.ground_m` (lowest ground within 500 m), `measured.band` |
| `features[]` | `id`, `kind`, `name`, `at_m` or `path_m`, `target` (the layout's fields), `measured` (volcano: `summit_m`, `caldera_floor_m`; paths: `min_m`, `max_m`) |
| `rivers[]`, `lakes[]`, `water_bodies[]` | Empty until hydrology v2. `water_bodies` will map each `water-body` id to its type (sea, lake, river), surface level and bounds |

`description.md` is the same content as tables for people.
