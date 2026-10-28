# Heightmap export v2

Producer: `topography export heightmap <world> <new-output-dir> [intervals]
[shared_edge|power_of_two] [--masters]`. Consumers: the game and `topography export view`.
The spec is [fujix v1](../docs/plans/fujix-v1.md) §4. The export is a conversion of a world
output (`world_preview` or `diffusion_world_output`, square, power-of-two side, spacing at
least 1 m); it computes no terrain. It is written to a stage and published only when every
check passes; a failed check names the check and up to three examples.

Version 2 changes from the unversioned v1 exports: an area-mean and mode pyramid replaces
nested decimation, every layer except `height` has a 4-texel apron, water comes as surface,
coverage and body tiles, the binary `water`, `snow-ice` and `vegetation` masks are gone, and
the manifest gains per-layer metadata, `legends`, `validation` and `description`. The viewer
reads only v2. A world without the hydrology v2 water layers (`water-surface.f32`,
`water-body.u32`, `bodies.json`) is refused: regenerate it.

Example: [heightmap-export-v2.json](../examples/manifests/heightmap-export-v2.json) (levels
and legends cut short).

## Manifest

`manifest.json` in the export root.

| Field | Meaning |
| --- | --- |
| `kind`, `schema_version` | `heightmap_export`, 2 |
| `size`, `spacing_m` | Level 0 samples per side (x, y) and their spacing |
| `origin_nw_corner` | Layout-plane metres of the north-west corner of the map (cell corner) |
| `crs` | WKT of the layout plane |
| `row_order` | North to south; texel (0, 0) is the north-west cell centre |
| `intervals`, `layout` | Intervals per tile side, a power of two from 16 to 4096 (default 256); `shared_edge` (default) or `power_of_two` |
| `apron_texels` | 4: the apron of every non-height layer |
| `tile_path` | `{layer}/L{level}/{prefix}_L{level}_{x}_{y}.png`, the prefix from `layers.{layer}.prefix` |
| `levels[]` | Per level: `level`, `spacing_m`, `samples_per_side`, `tiles_per_side`, `tile_texels` (full tile side without apron), `last_tile_texels` (the last column and row of tiles) |
| `lod` | The pyramid rule (below) |
| `encoding` | Height encoding: `h_min_m`, `h_max_m`, `step_m`, `decode`, `max_quantization_error_m`, `range_from` |
| `layers` | Per-layer metadata (below) |
| `legends` | Class tables of `biome` and `land-cover` (below) |
| `water_files` | What a missing water tile means |
| `validation` | Check counts, examples and figures (below) |
| `description` | `file` (`description.json`), `schema` (`world-description-v1`), `sha256` |
| `checksums` | `checksums.txt` holds the SHA-256 of every tile; `sha256sum -c checksums.txt` |
| `masters` | Full-world COGs written with `--masters`: only `elevation.tif` (Float32) |
| `world_output`, `world_output_manifest_sha256` | The world exported and its manifest digest |
| `attribution` | The world input's base and cover attribution, null when the input has none |

## Tiles

Every tile is a single-channel PNG, 8- or 16-bit (16-bit samples big-endian, as PNG stores
them). Level *k* has spacing 2^*k* times level 0, and its texel *i* sits at level-0 sample
*i*·2^*k*. Levels continue until one tile covers the world.

- **Geometry.** Tile (x, y) of a level starts at level sample (x·N, y·N), N = `intervals`.
  With `shared_edge` a full tile is N + 1 texels per side and neighbours repeat their border
  row and column; with `power_of_two` it is N. The world has N·*t* samples per side, so with
  `shared_edge` the last tile of each row and column has no sample to share and is N texels
  (256² at the far corner); `last_tile_texels` gives it.
- **Apron.** Tiles of every layer except `height` add `apron_texels` (A = 4) on every side:
  texel (A, A) is level sample (x·N, y·N), so a full tile is N + 1 + 2A = 265 texels per side
  and the last one 264. Apron texels past the world edge repeat the edge texel. Height tiles
  have no apron: 257² at 16 bits, as in v1.
- **Decode.** `value = offset + code · scale`, with `offset` and `scale` from the layer.
  Heights: `height_m = h_min_m + code · step_m`; `step_m = (h_max_m − h_min_m) / 65535`.
  The range covers ground, finite water surfaces and lake and sea levels.
- **Pyramid.** Each level comes from a global pyramid before tiles are cut, so tiles of one
  level agree across seams. Level *k*+1 texel *i* is the area mean of level *k* samples
  2*i*−1, 2*i*, 2*i*+1 with weights ¼, ½, ¼ per axis (index −1 clamps to 0). Class layers
  take the weighted mode over the same footprint (weights 1, 2, 4; a tie keeps the centre's
  class when tied, else the lowest id). Nothing is decimated.

## Layers

`layers.{name}` holds `prefix`, `format`, `scale`, `offset`, `decode`, `unit`,
`finest_spacing_m`, `levels` [first, last], `filter`, `source` (`generated`, `hydrology` or
`procedural`), `apron_texels`, `missing` and, for class layers, `legend`.

| Layer | Prefix | Encoding | Finest | Coarse filter | Missing file |
| --- | --- | --- | --- | --- | --- |
| `height` | `height` | 16-bit, the height encoding | Level 0 | Area mean | Never |
| `water-surface` | `water-surface` | 16-bit, the height encoding | Level 0 | Mean within one body; lakes and sea at their level | No water in the tile |
| `water-coverage` | `water-coverage` | 8-bit fraction, scale 1/255 | Level 0 | Area mean | No water in the tile |
| `water-body` | `water-body` | 16-bit export id | Level 0 | None (sampled, below) | No water in the tile |
| `biome` | `biome` | 8-bit terrain-v3 class id | 2 m | Area mode | Never |
| `land-cover` | `landcover` | 8-bit ESA WorldCover code | 2 m | Area mode | Never |
| `vegetation-density` | `vegetation-density` | 8-bit fraction | 2 m | Area mean | Never |
| `snow-cover` | `snow-cover` | 8-bit fraction | 2 m | Area mean | Never |
| `talus-density` | `talus-density` | 8-bit fraction | 2 m | Area mean | Never |
| `moisture` | `moisture` | 8-bit fraction | About 128 m | Area mean | Never |

- The five game layers come from the world's `game/` directory, written by `game_layers`
  after generation; they start at 2 m (level 1 of a 1 m world, level 0 of a coarser one).
  `biome` and `land-cover` are dithered from blue noise at 2 m, so borders read as a fine mix.
  `vegetation-density`, `snow-cover` and `talus-density` are procedural.
- `moisture` exists at levels with spacing of at least 128 m; on worlds too small for that
  (test crops) only at the coarsest level. Its first level is bilinear from the world's 128 m
  grid. A world without a `moisture` entry exports without the layer (the exporter prints a
  line).

## Water

- **Ids.** `water-body` holds dense export ids: 0 dry, 1 always the sea, then lakes and
  rivers in world-id order. `description.json` maps them to world bodies (`world_id`). More
  than 65,535 bodies fails the export.
- **Finest level.** Coverage is 255 where a cell has a body and its surface encodes at or
  above the ground, else 0. Lakes and the sea take their `bodies.json` level, so their code is
  the same at every level; rivers keep their raster surface.
- **Coarser levels.** Coverage is the area mean. `water-body` is sampled: the centre sample's
  body when that sample is wet, else the wet body with the largest footprint weight, 0 only
  when the footprint is dry. Ids are never averaged, and coverage above 0 always has a body.
  River surfaces are the weighted mean over footprint samples of the same body.
- **Extension.** A dry texel within 4 texels of water carries the nearest wet texel's surface;
  further out the surface is the ground height (depth 0), and coverage marks it dry.
- **Files.** The three water tiles of a tile exist only where a texel of the tile or its apron
  has coverage above 0. A missing file means coverage 0, body 0 and surface equal to the
  ground.

## Legends

`legends.biome` (`legend`: `topography terrain-v3`, from `catalog/labels/terrain-v3.toml`) and
`legends.land-cover` (`ESA WorldCover v200`) list `classes[]` of `id`, stable `key`, `name` and
`present`. Id 0 is `unknown`. `present` is true when the class occurs at the layer's finest
level. Keys never change meaning; a retired id is not reused.

## Validation

`validation.passed` is true in every published export. `checks` counts failures; any nonzero
count fails the export:

| Check | Fails where (level 0 unless noted) |
| --- | --- |
| `wet_not_above_ground` | A body with a finite surface at or below the ground |
| `wet_without_surface` | A body without a finite surface |
| `surface_without_body` | A finite surface outside every body |
| `lake_levels_off` | A lake or sea cell whose surface does not encode to its level's code |
| `lake_levels` | A lake or sea with more than one surface code at one level (any level) |
| `coverage_mismatch` | Coverage that does not match surface at or above ground |
| `shared_edge_mismatches` | Duplicated border or apron texels that differ between neighbours (any layer, any level) |

`examples` keeps up to three per failing check, naming "world body N" or "export id N".
`figures` reports `coarse_wet_below_ground` per level (texels with coverage ≥ 0.5 whose
surface is below the mean ground, expected on mixed shore texels and not a failure) and
`lake_levels_checked`.

## Not exported

The game's water contract also asks for shoreline distance, water type fractions and
velocity. The game derives them: the shore is where surface minus ground crosses zero, the
type comes from the body id, and velocity follows surface slope and depth (Manning) and is
zero in lakes. The world keeps surface, body and ground, so adding them needs only a
re-export. From the vegetation requests, `hollow`, `treeline-margin` and `needleleaf` are not
exported.
