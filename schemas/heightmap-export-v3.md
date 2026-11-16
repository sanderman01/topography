# Heightmap export v3

Producer: `topography export heightmap <world> <new-output-dir> [intervals]
[shared_edge|power_of_two] [--masters] [--no-normals] [--roughness] [--normals-level L]
[--normals-apron A] [--from <previous-export> --layers <group,...>]`. Consumers: the game and
`topography export view`. The spec is [fujix v1](../docs/plans/fujix-v1.md) §4. The export is a
conversion of a world output (`world_preview` or `diffusion_world_output`, square, power-of-two
side, spacing at least 1 m); it computes no terrain. It is written to a stage and published only
when every check passes; a failed check names the check and up to three examples.

Version 3 changes from v2 (2026-11-16), each answering a flaw in
[export-data-flaws.md](../docs/export-data-flaws.md):

- **Height encoding:** 65,534 steps over the range and `h_min_m` a multiple of `step_m`, so
  0 m is a code (the sea decodes to exactly 0).
- **`water-surface`** has its own encoding: 2 cm steps from 0 m, wrapped every 65,536 codes
  (`wrap_m` 1,310.72 m), unwrapped against the decoded ground of the same texel. No code is
  reserved: where `water-body` is 0 the surface code is 0 and means nothing.
- **Extension** past the shore carries the body id; its surface is the distance-weighted mean
  of that body's wet texels. Coverage is real wetness only: wet ⇔ coverage > 0, and
  `water-body` is also set on extension texels (coverage 0).
- **Diagonal streams:** a dry corner between two texels of one river that touch only
  diagonally is filled at level 0, so streams are 4-connected.
- **`biome` and `land-cover`** are RGB tiles of the two strongest classes and a weight
  (R = a, G = b, B = share of a within a + b × 255), from the world's top-two class layers and
  area-meaned through the pyramid; the dithered single-class tiles and the mode filter are gone.
- **New layers:** `temperature` (°C) from the game level; `moisture` from the game level when
  the world's game layers write it, else from the first level of at least 16 m (was 128 m).
- **Normals:** apron 2 by default; roughness off unless `--roughness`.
- **Legends:** land-cover class 21 `riparian` (an extension code), a colour per land-cover
  class, and `derived_from` on the classes drawn from a fraction layer.
- **`description.json`:** river lines from the world's own 1 m hydrology (v2 used a source
  world's 4 m hydrology whose body ids differ, so every line and outline was misplaced), lake
  outlines from the world's `water-body` raster, every line and outline checked inside its
  body's `bounds_m`, river drainage, gradient and `steep`/`falls` flags, and `glaciers[]`.
- **Partial exports** (`--from`, `--layers`) and `input_digests`, `reused_layers` in the
  manifest.

A world whose game layers are still the dithered v1 `biome`/`land-cover` is refused: rerun
`game_layers`. A world without the hydrology v2 water layers (`water-surface.f32`,
`water-body.u32`, `bodies.json`) is refused too.

Example: [heightmap-export-v2.json](../examples/manifests/heightmap-export-v2.json), a v2
manifest (levels and legends cut short); v3 adds the fields above.

## Manifest

`manifest.json` in the export root.

| Field | Meaning |
| --- | --- |
| `kind`, `schema_version` | `heightmap_export`, 3 |
| `size`, `spacing_m` | Level 0 samples per side (x, y) and their spacing |
| `origin_nw_corner` | Layout-plane metres of the north-west corner of the map (cell corner) |
| `crs` | WKT of the layout plane |
| `row_order` | North to south; texel (0, 0) is the north-west cell centre |
| `intervals`, `layout` | Intervals per tile side, a power of two from 16 to 4096 (default 256); `shared_edge` (default) or `power_of_two` |
| `apron_texels` | 4: the apron of every non-height layer except `normal` and `roughness` |
| `tile_path` | `{layer}/L{level}/{prefix}_L{level}_{x}_{y}.{ext}`, the prefix from `layers.{layer}.prefix`; `ext` is `png`, or `ktx2` for `normal` and `roughness` (`layers.{layer}.file_extension`) |
| `levels[]` | Per level: `level`, `spacing_m`, `samples_per_side`, `tiles_per_side`, `tile_texels` (full tile side without apron), `last_tile_texels` (the last column and row of tiles) |
| `lod` | The pyramid rule (below) |
| `start_levels` | Where each layer's tiles start; finer levels read the ancestor tile |
| `encoding` | Height encoding: `h_min_m`, `h_max_m`, `step_m`, `decode`, `max_quantization_error_m`, `range_from` |
| `layers` | Per-layer metadata (below) |
| `legends` | Class tables of `biome` and `land-cover` (below) |
| `water_files` | What a missing water tile means |
| `validation` | Check counts, examples and figures (below) |
| `description` | `file` (`description.json`), `schema` (`world-description-v1`), `sha256` |
| `checksums` | `checksums.txt` holds the SHA-256 of every tile; `sha256sum -c checksums.txt` |
| `masters` | Full-world COGs written with `--masters`: only `elevation.tif` (Float32) |
| `input_digests` | Per layer group (below): the SHA-256 of each world file it reads (from the world manifest) and the settings that shape its tiles |
| `reused_layers` | Partial exports only: layer group → the export its files were hardlinked from |
| `world_output`, `world_output_manifest_sha256` | The world exported and its manifest digest |
| `attribution` | The world input's base and cover attribution, null when the input has none |

## Tiles

Every tile is a PNG: single-channel 8- or 16-bit (16-bit samples big-endian, as PNG stores
them), or 8-bit RGB for `biome` and `land-cover`. Level *k* has spacing 2^*k* times level 0,
and its texel *i* sits at level-0 sample *i*·2^*k*. Levels continue until one tile covers the
world.

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
  Heights: `height_m = h_min_m + code · step_m`; `step_m = (h_max_m − h_min_m) / 65535`, with
  `h_min_m` = −*k*·`step_m` for an integer *k*, so 0 m is code *k*. The range covers ground,
  finite water surfaces and lake and sea levels.
- **Pyramid.** Each level comes from a global pyramid before tiles are cut, so tiles of one
  level agree across seams. Level *k*+1 texel *i* is the area mean of level *k* samples
  2*i*−1, 2*i*, 2*i*+1 with weights ¼, ½, ¼ per axis (index −1 clamps to 0). Class layers
  carry class shares (up to four per texel) through the same mean and re-pick the top two at
  every level. Nothing is decimated and nothing takes a mode.

## Layer groups and partial exports

Layers rebuild in groups: `terrain` (`height`, `water-surface`, `water-coverage`,
`water-body`, `elevation.tif`, and the description), `description` (`description.json` and
`.md` only), `normals` (`normal`, `roughness`), each game layer (`biome`, `land-cover`,
`vegetation-density`, `snow-cover`, `talus-density`, `temperature`) and `moisture`.

`--from <previous-export> --layers a,b` rebuilds the named groups and hardlinks every other
group's files from the previous export (copying where a link fails). Each reused group's
`input_digests` entry in the previous export must equal the current world's, or the export
fails naming the group: add it to `--layers`. The previous export must be schema 3 with the
same `intervals`, `layout`, `size`, `spacing_m` and origin; exports made before schema 3 have
no input digests and cannot be reused (run a full export). The new manifest copies the reused
groups' `layers` entries, legends, `encoding` (when `terrain` is reused) and `description`
(when `description` is reused); `checksums.txt` keeps the reused files' lines; the report's
`reused_from` names the previous export, its groups, whether it passed and the checks and
figures it `copied` (the reused groups' checks and figures, such as `diagonal_fills` or
`bc_error_budget`, are the previous export's), and `passed` is true only when both did.
Shared-edge checks run per rebuilt layer. Each group's digest includes an
`exporter_version`, bumped whenever the code changes that group's output, so tiles of older
exporter code are never reused.

## Layers

`layers.{name}` holds `prefix`, `format`, `scale`, `offset`, `decode`, `unit`,
`finest_spacing_m`, `levels` [first, last], `filter`, `source` (`generated`, `hydrology` or
`procedural`), `apron_texels`, `missing`, `meaning` where the unit does not say it and, for
class layers, `legend`.

| Layer | Prefix | Encoding | Finest | Coarse filter | Missing file |
| --- | --- | --- | --- | --- | --- |
| `height` | `height` | 16-bit, the height encoding | Level 0 | Area mean | Never |
| `water-surface` | `water-surface` | 16-bit, 2 cm wrapped (below) | Level 0 | Mean within one body; lakes and sea at their level | No water in the tile |
| `water-coverage` | `water-coverage` | 8-bit fraction, scale 1/255 | Level 0 | Area mean | No water in the tile |
| `water-body` | `water-body` | 16-bit export id | Level 0 | None (sampled, below) | No water in the tile |
| `biome` | `biome` | RGB: terrain-v3 ids a, b, weight | Game level (2 m) | Share mean, top two | Never |
| `land-cover` | `landcover` | RGB: WorldCover codes a, b, weight | Game level (2 m) | Share mean, top two | Never |
| `vegetation-density` | `vegetation-density` | 8-bit fraction | Game level (2 m) | Area mean | Never |
| `snow-cover` | `snow-cover` | 8-bit fraction | Game level (2 m) | Area mean | Never |
| `talus-density` | `talus-density` | 8-bit fraction | Game level (2 m) | Area mean | Never |
| `temperature` | `temperature` | 8-bit, °C = code · 0.25 − 30 | Game level (2 m) | Area mean | Never |
| `moisture` | `moisture` | 8-bit fraction | Game level, else 16 m | Area mean | Never |

- The game layers come from the world's `game/` directory, written by `game_layers` after
  generation: `{biome,land-cover}-{a,b,weight}.u8`, the three densities, `temperature.u8` and
  `moisture.u8`. They start at the game level (level 1 of a 1 m world, level 0 of a coarser
  one). All are procedural.
- **Class layers.** R is the dominant class *a*, G the runner-up *b* (= *a* where the texel
  has one class), B the share of *a* within *a* + *b* × 255 (128 to 255). Blend *a* and *b*
  with weight B/255. At every level the exporter area-means the class shares under the
  footprint and picks the top two again, so a border keeps its width and mix at every level
  of detail. A class is `present` in the legend when it is *a*, or *b* with B < 255, at the
  finest level.
- `vegetation-density` is the canopy share of the texel: 1.0 is continuous canopy over the
  whole texel. Closed forest reads lower (about 0.65) because forest covers only its layout
  share of a region and fades at its edges.
- `temperature` is the mean annual air temperature.
- `moisture` comes from the game layers when the world has `game/moisture.u8`. Otherwise its
  first level is the first with spacing of at least 16 m (L4 of a 1 m world; on smaller test
  worlds the coarsest), bilinear from the world's 128 m grid. A world with neither exports
  without the layer (the exporter prints a line).

## Normals and roughness

Baked lighting layers for the coarse levels: normals on by default (`--no-normals` drops
both), roughness only with `--roughness` (it changed shading by under 2 % and the game no
longer reads it).

| Layer | File | Content |
| --- | --- | --- |
| `normal` | KTX2, `VK_FORMAT_BC5_UNORM_BLOCK` (141) | Unit world-space normal: x east, y north, z up |
| `roughness` | KTX2, `VK_FORMAT_BC4_UNORM_BLOCK` (139) | Toksvig slope deviation σ, 0 for a plane |

- **Files.** KTX 2.0, one 2D image, one mip level, no supercompression, a basic data format
  descriptor (BC4/BC5 colour model, BT.709 primaries, linear transfer) and the keys
  `KTXorientation` = `rd` (row 0 is the north edge, column 0 the west edge) and `KTXwriter`.
  Bevy loads them with its `ktx2` feature; BC4/BC5 need a GPU with BC texture support
  (desktop DX12 and Vulkan; Metal on macOS, not iOS).
- **Levels.** From `--normals-level` (default 2, 4 m on a 1 m world) to the last level;
  `layers.{layer}.levels` gives [first, last]. Finer levels have no tiles: the game derives
  normals from the height tiles there.
- **Lattice.** Cell-centred: texel *i* of level *k* covers the level-*k* cell between height
  samples *i* and *i*+1, so sampling at (*i* + 0.5)/*N* lands on the cell centre. Level *k*
  has 2^(*m*−*k*) − 1 cells per side on a 2^*m* world. Tile (x, y) texel (A, A) is cell
  (x·N, y·N), with N = `intervals` and A = `--normals-apron` (default 2, must be even so a
  tile is whole 4×4 blocks). A full tile is N + 2A texels per side (260² by default), aprons
  repeat the neighbours' texels and past the world edge the edge texel, so filtering crosses
  tile borders without a step. The last tile of each row and column holds one cell fewer
  than N; it is padded to a multiple of 4 by repeating its edge texel.
  `layers.{layer}.tile_levels[]` gives per level `cells_per_side`, `tiles_per_side`,
  `tile_texels`, `last_tile_valid_texels` and `last_tile_texels` (padded).
- **Source.** Every level-0 cell gets the unit normal of the gradient across it from its
  four corner ground samples (the Float32 ground the height tiles encode; water has its own
  surface layer). A level-*k* texel is the box mean **M** of the 2^*k* × 2^*k* cell normals
  it covers, from one global pyramid, so tiles agree across seams.
- **Decode.** Normal: `x = 2R − 1`, `y = 2G − 1`, `z = sqrt(max(0, 1 − x² − y²))`, then
  normalize. Stored is **M**/|**M**|. Roughness: `σ = R`, with m = |**M**| and
  σ = min(sqrt((1 − m)/m), 1) (Toksvig): 0 on a plane, growing with the spread of the slopes
  under the texel.
- **Encoder.** Deterministic BC4 (BC5 = two BC4 blocks), 8-value mode, endpoints fitted by a
  small search and least-squares refinement.

## Water

- **Ids.** `water-body` holds dense export ids: 0 none, 1 always the sea, then lakes, ice and
  rivers in world-id order. `description.json` maps them to world bodies (`world_id`). More
  than 65,535 bodies fails the export. Glaciers (`bodies.json` kind `glacier`) have no water
  cells and no export id; `description.json` lists them in `glaciers[]`.
- **Wet texels.** At level 0 a cell is wet when it has a body and its surface lies above its
  ground: coverage 255, else 0. Coverage is the area mean at coarser levels; wet ⇔ coverage
  > 0 at every level (a share that rounds to 0 is written as 1). Lakes, ice (frozen lakes)
  and the sea take their `bodies.json` level, so their code is the same at every level;
  rivers keep their raster surface. Ice counts as coverage; its kind `ice` tells the game to
  draw ice, not water.
- **Diagonal fill.** Where two level-0 texels of one river touch only at a corner and both
  other corners are dry, the corner with the lower ground becomes wet: the river's body,
  coverage 255 and the mean of the two surfaces, raised to at least 5 cm above that texel's
  ground (ground is never carved). Where that would put the surface more than 0.6 m above
  both diagonal surfaces the corner stays dry. `validation.figures.diagonal_fills` and
  `diagonal_fills_skipped` count them.
- **Coarser levels.** `water-body` is sampled: the centre sample's body when that sample is
  wet, else the wet body with the largest footprint weight. Ids are never averaged. River
  surfaces are the weighted mean over footprint samples of the same body.
- **Extension.** A dry texel within 4 texels (Chebyshev, `extended_texels`) of water takes the
  body of the nearest wet texel and, as surface, the 1/d² weighted mean of that body's wet
  texels within 4 texels (exactly the level for lakes and the sea). Its coverage stays 0.
  Further out `water-body` is 0 and there is no surface. An extension whose surface would be
  600 m or more from its ground (the sea below a cliff on a coarse level) is dropped.
- **Surface encoding.** `code = round(surface_m / 0.02) mod 65536` (`scale` 0.02, `offset` 0,
  `wrap_m` 1,310.72). Decode where `water-body` > 0: `s0 = code · 0.02`, then
  `surface_m = s0 + n · 1310.72` with `n = round((ground_m − s0) / 1310.72)`, `ground_m` the
  decoded `height` of the same texel and level. This is exact while |surface − ground| <
  655 m. The export fails when a level-0 wet texel is 600 m or more from its ground
  (`surface_far_from_ground`), and drops such extension texels. On coarse levels mixed shore
  texels can be that far from the mean ground (`coarse_surface_far_from_ground` per level,
  split into flat bodies and rivers): river texels are written clamped to ground ± 600 m so
  they decode to the right wrap; sea, lake and ice texels keep their level, so take their
  surfaces from `water_bodies[].level_m` by id.
- **Files.** The three water tiles of a tile exist only where a texel of the tile or its apron
  has `water-body` > 0 (water or its extension). A missing file means coverage 0, body 0 and
  no surface.

## Legends

`legends.biome` (`legend`: `topography terrain-v3`, from `catalog/labels/terrain-v3.toml`) and
`legends.land-cover` (ESA WorldCover v200 with topography extensions) list `classes[]` of
`id`, stable `key`, `name` and `present`. Id 0 is `unknown`. `present` is true when the class
occurs at the layer's finest level. Land-cover classes also carry `color` (WorldCover's
palette; `#6fbf3f` for riparian), `extension: true` for codes outside WorldCover (21
`riparian`: vegetation along lakes and larger rivers in dry biomes) and `derived_from` where
the class is drawn from a fraction layer, which is the better source for it (`tree_cover`
from the forest part of `vegetation-density`, `riparian` from its riparian band (density
0.25 floor), `snow_ice` exactly from `snow-cover`). `water` (80) takes a whole texel at a wet
share of 0.25 or more. Keys never change meaning; a retired id is not reused.

## Validation

`validation.passed` is true in every published export. `checks` counts failures; any nonzero
count fails the export:

| Check | Fails where (level 0 unless noted) |
| --- | --- |
| `wet_not_above_ground` | A body with a finite surface at or below the ground |
| `wet_without_surface` | A body without a finite surface |
| `surface_without_body` | A finite surface outside every body |
| `lake_levels_off` | A lake or sea cell whose surface is more than 1 cm (half a water step) from its level |
| `lake_levels` | A lake or sea with more than one surface code at one level (any level) |
| `coverage_mismatch` | Coverage that is not 255 where wet and 0 where dry; coverage > 0 without a body (any level) |
| `surface_far_from_ground` | A wet texel whose surface is 600 m or more from its ground |
| `shared_edge_mismatches` | Duplicated border or apron texels that differ between neighbours (any layer, any level) |
| `bc_gross_errors` | A baked texel whose BC5 normal is more than 30° off its Float32 normal or whose BC4 roughness is off by more than 0.25 |

`examples` keeps up to three per failing check, naming "world body N" or "export id N".
`figures` reports `coarse_wet_below_ground` per level (texels with coverage ≥ 0.5 whose
surface is below the mean ground, expected on mixed shore texels and not a failure),
`coarse_surface_far_from_ground` per level (`flat` and `river` wet texels 600 m or more from
the mean ground), `diagonal_fills`, `lake_levels_checked` and, with normals,
`bc_error_budget`: per level `texels`, `angle_p99_deg`, `angle_max_deg` and
`angle_over_10_deg` (decoded BC5 normal against the Float32 normal) and, with roughness,
`roughness_p99_abs` and `roughness_max_abs` (decoded BC4 against Float32), measured over
every valid texel, plus the `fail_above` limits. Percentiles are upper bin edges (0.001°,
0.0001). Partial exports add `reused_from` (above).

Measured on an 8 km crop of fujix-55-l4v6 (1 m, 1,751 to 4,975 m, 2026-11-13): angle p99
1.96° at L2 rising to 3.6° at L5, max 21.6° (32 of 4.2 M L2 texels above 10°, all on steep
blocks whose x and y span much of the range, where the rebuilt z amplifies the 3-bit
steps); roughness p99 0.008 to 0.013, max 0.058.

## Not exported

The game's water contract also asks for shoreline distance, water type fractions and
velocity. The game derives them: the shore is where surface minus ground crosses zero, the
type comes from the body id, and velocity follows surface slope and depth (Manning) and is
zero in lakes. The world keeps surface, body and ground, so adding them needs only a
re-export. From the vegetation requests, `hollow`, `treeline-margin` and `needleleaf` are not
exported. The world's 1 m `glacier.u8` mask feeds the game layers (snow) and is not exported.
