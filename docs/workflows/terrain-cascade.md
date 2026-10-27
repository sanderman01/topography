# Terrain cascade workflow

Implements the [terrain cascade](../plans/terrain-cascade-v1.md): L1 (128 m) →
L2 (32 m) → L3 (4 m) → L4 (1 m), each a residual diffusion model over the level
above. `P` below is `data/models/diffusion-env-v1/.venv/bin/python` (set up as in
[training/README.md](../../training/README.md#terrain-diffusion)). Training and
generation are GPU workloads: warn before running them.

Run heavy jobs inside a memory-capped scope so an out-of-memory job cannot take the
session down: `CAP=(systemd-run --user --scope -q -p MemoryMax=7G)`, then `"${CAP[@]}" $P ...`.

Current models (2026-10-11), all on terrain legend v3 with form channels:

| Level | Run | Training data | Class gates |
| --- | --- | --- | --- |
| L1 128 m | `cascade-l1-run-6` | `context-v5` (GLO-30 + GEBCO) | pass |
| L2 32 m | `cascade-l2-run-8` | `context-v5` | pass |
| L3 4 m | `cascade-l3-run-4` | `lidar-v3` (lidar averaged to 4 m) | pass |
| L4 1 m | `cascade-l4-run-7` | `diffusion-v9` (1 m library tiles) | pass |

## Training data

- L1 and L2: [GLO-30 context regions](context-regions.md), release `context-v5`.
- L3: lidar regions from `context build` with `source = "lidar"` plans
  (`catalog/plans/lidar-regions-v*.toml`): USGS 3DEP and LINZ tiles averaged to
  4 m with a validity mask, voted and released like the GLO-30 regions.
- L4: the 1 m library release from `topography dataset diffusion` (see the
  [diffusion workflow](terrain-diffusion.md#1-training-release)). Its tiles are
  split by geographic bands inside each tile.

## Training, gates and blind review

```sh
COMMON=(--class-embedding --scale-shift --balanced --mix 0.25 --forms)
$P -m topography_training.diffusion.train --release data/datasets/context-v5/release.json --level l1 \
    --run-dir data/models/cascade-l1-run-6 --channels 64,96,128,192,256 "${COMMON[@]}" \
    --crop 128 --coarse-dropout 0.5 --hours 4
$P -m topography_training.diffusion.train --release data/datasets/context-v5/release.json --level l2 \
    --run-dir data/models/cascade-l2-run-8 --channels 64,96,128,192,256 "${COMMON[@]}" \
    --coarse-dropout 0.25 --coarse-blur 0.5 --hours 6
$P -m topography_training.diffusion.train --release data/datasets/lidar-v3/release.json --level l3 \
    --run-dir data/models/cascade-l3-run-4 --channels 64,96,128,192,256 "${COMMON[@]}" \
    --coarse-dropout 0.25 --coarse-blur 0.5 --hours 6
$P -m topography_training.diffusion.train --release data/datasets/diffusion-v9/manifest.json --level l4 \
    --run-dir data/models/cascade-l4-run-7 --channels 64,128,256,384,512 "${COMMON[@]}" \
    --coarse-dropout 0.25 --coarse-blur 0.5 --hours 8
$P -m topography_training.diffusion.class_gates --run-dir data/models/cascade-l4-run-7 \
    --out data/models/cascade-l4-run-7/gates-1
$P -m topography_training.diffusion.cascade_eval --run-dir data/models/cascade-l4-run-7 \
    --out data/models/cascade-l4-run-7/eval-1
```

L4 run 7 peaks at about 9 GB of VRAM. `class_gates` checks that the class matters
([gates](../plans/class-conditioning-v1.md#gates)). Generated residuals of validation
regions must be recognised as their class, and their per-class roughness must follow
the real ranking with the coarse input hidden. It writes `gates.json` and a class grid
hillshade, and exits 1 when a gate fails.

Refining levels (L2–L4) all use conditioning layout 3: the level above as a
per-window unit-range shape channel, its east and north slopes, absolute height /
4,096 m, water, snow/ice, vegetation and the class one-hot. Each level sees the
level above as block means on a randomly shifted grid, upsampled and smoothed
(L2 4× blocks, L3 8×, L4 4×). L4 leaves water out of the loss, since lidar
records water surfaces, not ground. Training drops the class (10%) and, for
layout 3, the shape and slopes (10%) and snow/ice and vegetation (10%), so
guidance can push against them at generation time. Rerunning `train` with the
same run directory resumes.

The class recipe: `--class-embedding` adds a learned class embedding to every block,
`--scale-shift` lets it scale and shift every norm (AdaGN), `--balanced` draws windows
class-balanced by region, `--mix` joins two classes in a share of windows, and
`--coarse-blur` blurs the coarse input so the class, not only the level above, sets
the detail.

Runs started after 2026-10-07 use terrain legend v3 (`catalog/labels/terrain-v3.toml`,
[biome forms](../plans/biome-forms-v1.md)): 19 class channels (20 from L3 run 5 and L4
run 8, with class 19), sources relabelled to
subclasses by `catalog/labels/subclasses-v1.toml` (for example Zion and
Canyonlands as layered canyons), ice cells as glaciers and quiet ground as plain, per
cell. A class trains at a level only with at least three sources there; the run's
`class_map` sends every other class to its first trained fallback. Older runs keep 13
channels and see subclasses as their parents.

`--forms` adds six form channels (`forms.py`): roughness, flat share, peak density, hollow
share and texture direction. Training takes them from the truth (dropped for
`--form-dropout`, default 25%, of windows); generation takes them from the run's per-class
table, with dune direction from the layout's wind. L1 can train on `--crop 128`, so
the chain samples it in overlapping local windows.

### Grand Canyon and dithered borders (fujix)

Class 19, `grand_canyon` (parent 13, fallback 13), trains only on the Grand Canyon: GLO-30
`grand-canyon`, lidar `grand-canyon` and the five `Grand Canyon` 1 m tiles. Class 13 keeps
the other canyons. Runs from before class 19 send it where they send 13 (13-channel runs:
to 2), so their configs and Fujion inputs still work. L1 and L2 are not retrained; fujix's
32 m surface is real Grand Canyon.

The lidar region is a single accepted 4096² region, so `subclasses-v1.toml` splits it into
four 2048² reaches (`[[reaches]]`; ids `grand-canyon-r0` to `-r3`, row-major from the
north-west), each its own source. The south-west reach (r2, canyon and rim plateau) is
held out; the north-west one is only 23% surveyed and gives few windows. Reaches apply to
training releases only (`cascade.load_regions`); other readers see the whole region. At L4
the five tiles are split by northing bands as usual.

`--dither-m 3000` dithers the class borders of the per-cell labels and of mixed windows
with blue noise (`borders.py`, shared with the collage), across a band drawn per window
from 0 to 3 km (fujix's `border_m` is 2 to 4.5 km), hard in a quarter of the windows. The
dither works on the 32 m label grid, like the collage's `class32.u8`. The truth under a
dithered band still follows the original border, so the models learn a gradual
transition across the band, not cell-by-cell interleaving. With `--balanced` the reaches
of one region count as one region, so each reach is drawn by its share of windows. Per-cell
labels of a reach are worked out on the whole region (plain threshold included).

`class_gates` adds, for runs that train class 19 (`--gc-windows`, default 8 validation
windows, class 19 first; under `grand_canyon` in `gates.json`, counted in the exit status):

- Grand Canyon against 13 and against plain. Residuals of the two classes must be further
  apart in the classifier's features than two seeds of class 19, and at least 70% as far
  apart as the real classes' centroids. Plain's real centroid comes from real windows that
  are at least 80% plain by per-cell label; without a real centroid the floor is 70% of the
  real 19-to-13 distance. Where the level does not train plain (L4 maps it to 0, the
  class-dropped input), vs_plain is skipped.
- A dithered border: Grand Canyon west, the other class east (plain, else 16 if trained,
  else 13), dithered across half the window. In the band the output must lie between the
  two classes' uniform outputs (closer to each than they are to each other), and cells on
  the Grand Canyon side of the band must come out closer to the Grand Canyon output more
  often than cells on the far side.

Gates rerun on runs from before the reaches hold out reach r2, which those runs trained
on, so their results are slightly optimistic.

Runs to launch (about 6 h and 8 h, GPU, 7 GB RAM cap):

```sh
CAP=(systemd-run --user --scope -q -p MemoryMax=7G)
COMMON=(--class-embedding --scale-shift --balanced --mix 0.25 --forms --dither-m 3000)
"${CAP[@]}" $P -m topography_training.diffusion.train --release data/datasets/lidar-v3/release.json --level l3 \
    --run-dir data/models/cascade-l3-run-5 --channels 64,96,128,192,256 "${COMMON[@]}" \
    --coarse-dropout 0.25 --coarse-blur 0.5 --hours 6
"${CAP[@]}" $P -m topography_training.diffusion.train --release data/datasets/diffusion-v9/manifest.json --level l4 \
    --run-dir data/models/cascade-l4-run-8 --channels 64,128,256,384,512 "${COMMON[@]}" \
    --coarse-dropout 0.25 --coarse-blur 0.5 --hours 8
"${CAP[@]}" $P -m topography_training.diffusion.class_gates --run-dir data/models/cascade-l3-run-5 \
    --out data/models/cascade-l3-run-5/gates-1
"${CAP[@]}" $P -m topography_training.diffusion.class_gates --run-dir data/models/cascade-l4-run-8 \
    --out data/models/cascade-l4-run-8/gates-1
```

The first L3 run after the label change rebuilds the cached 4 m fields of every lidar
region (`data/cache/cascade-fields/`).

`cascade_eval` writes a blind review (`review.html`; keep `key.json` and
`compare.html` closed until the votes are in), terrain statistics, a
nearest-training-window distance and a same-seed check: different windows
generated with one seed should not correlate (run 4 of the old 1 m model: 0.99;
L4 run 5: 0.29).

## Authoring a layout

A world starts from a written layout ([layout spec](../plans/world-layout-spec-v1.md);
Fujion is the example in `configs/layouts/fujion/`): `brief.md` from the owner,
`layout.toml` and its `layout.md` summary from the agent. Render it to L0 (128 m class
map, height envelope and feature lines) and check it:

```sh
target/release/topography world layout configs/layouts/fujion/layout.toml data/worlds/fujion-l0-1
$P -m topography_training.diffusion.layout overview data/worlds/fujion-l0-1
$P -m topography_training.diffusion.layout reach data/worlds/fujion-l0-1
$P -m topography_training.diffusion.layout refs data/worlds/fujion-l0-1 --release data/datasets/context-v5/release.json
```

`overview` draws the annotated map, `reach` checks the altitude tiers and `refs` makes
a sheet of the real reference regions at world scale.

The owner then paints the layout. `paint export` writes one GIMP file, `layout.xcf`,
with the layers height, roughness and classes (legend v3 palette `terrain-v3.gpl`)
and an optional `hold` layer; `paint import` reads it back as an L0 that holds only
ground height, terrain classes and roughness. Lakes, rivers and troughs are left to
the models and drainage, and a 1.5 km band along the map border is always sea. GIMP's
`gimp-console` must be installed.

```sh
$P -m topography_training.diffusion.paint export data/worlds/fujion-l0-1 data/worlds/fujion-paint
$P -m topography_training.diffusion.paint import data/worlds/fujion-paint data/worlds/fujion-painted-l0-1
```

To move a painted layout to the legend v3 subclasses, copy it with renamed classes and
import the copy:

```sh
$P -m topography_training.diffusion.paint relabel data/worlds/fujion-paint data/worlds/fujion-paint-v3 \
    --map 2:13,1:14,5:15,8:16 --small 11:17:3 --hemisphere south
$P -m topography_training.diffusion.paint import data/worlds/fujion-paint-v3 data/worlds/fujion-painted-l0-3
```

A painted volcano can be replaced by a real one: `paint transplant` centres a real cone
(`--region`, default `fuji`, from the GLO-30 release) on the painted summit, scales it to
the painted cone and summit height, fades it out over 1.5 km at the edge of the volcano
biome (classes 11, 10 and 4), and redraws the glaciers on it with their painted area
(lower on pole-facing slopes and in gullies). `--base` gives the same map before the cone
was painted:

```sh
$P -m topography_training.diffusion.paint transplant data/worlds/fujion-paint-v3 data/worlds/fujion-paint-v3b \
    --base data/worlds/fujion-paint/layout.before-volcano.xcf
```

A painted outline that looks drawn (a half-disc bay, say) can be broken up with
`paint roughen`: it warps the layers within 3 km of one class's outline by smooth noise
(about 2 km at a 6 km wavelength, plus a finer octave), away from the map border:

```sh
$P -m topography_training.diffusion.paint roughen data/worlds/fujion-paint-v3b data/worlds/fujion-paint-v3c --class 15
```

Drowned tower karst (15) is painted in shallow sea: import sets its seabed to −40 to
−10 m (fading out over about 1 km past the painted area) and keeps its relief offshore.
Its Ha Long exemplar also reaches 2 km onto neighbouring karst and 800 m onto any other
shore, so L1 raises towers above the water and the coast breaks into coves and islets.

## Collage layouts

A layout v2 (`configs/layouts/fujix/layout.toml`, see [fujix v1](../plans/fujix-v1.md))
is written by the agent, not painted. It lists regions, each with an outline, a terrain
class and one or more pieces: crops of real GLO-30 regions that are rotated, scaled
(within 1/1.5 to 1.5), remapped in height and tilted. It also has `[flight]` bands
(clearance and band tops), `[[passes]]` that each name the band that first reaches them,
and `[[features]]` (the volcano and its caldera, rivers and canyons). Positions are metres
from the north-west corner.

`collage` builds the 32 m surface directly, and needs no GPU:

```sh
$P -m topography_training.diffusion.collage configs/layouts/fujix/layout.toml \
    --release data/datasets/context-v5/release.json --out data/worlds/fujix-collage-54
```

It bends every outline with one fixed-seed smooth warp. Pieces fade out inside their crop
edge, ground past the pieces is filled smoothly, and region borders are blended with a
Laplacian pyramid while classes are dithered across them. A smooth seabed starts at the
coast, and ground past the island outline sinks to it. Passes above their band get a
broad dip. Each volcano with caldera fields gets a bowl cut into its summit dome, centred on
the summit plateau. The rim follows the dome's own crest, with its scallops. The upper
inner walls carry the outer flank's gullies, mirrored in the crest; the lower walls are
smooth down to an ice floor with faint texture, and the crest is rounded by a one-cell
blur. Ground is only lowered, except that
rim notches lower than 300 m above the floor are filled. A breach is a notch from the floor
through the rim and down the gully below it, ending where the ground falls below its floor.
All change fades out between 2 and 2.5 km from the caldera's centre. The build writes into
`<out>.partial` and renames it when done; an error removes it, and a killed build leaves it
for you to remove. Outputs:

- `layout.json` and the L0 layers (`class.u8`, `mid.f32`, `relief.f32`, `coast.f32`,
  `snowline.f32`, `forest.u8`, `lake.u8`, `lake-level.f32`), readable by `layout.load`
- `class32.u8`, the 32 m class map (terrain legend v3, glacier on caldera floors)
- `region32.u8`, the 32 m region map: 1 to N in the layout's region order for the region
  with the largest blend weight, 0 for sea outside every outline. Every land cell has a
  region.
- `arrays.npz` with `l1` and `l2`
- `collage.json`: layout and release paths and hashes, sources, the `class32` and
  `region32` files (size, cell, dtype, legend), provenance (warp seed and octaves,
  blue-noise seeds, Git commit and whether tracked files were changed) and timing
- `report.txt`, with WARN lines for failed checks:
  - land fraction, distance of land from the map edge, and coast cells pressed against
    the edge clamp
  - per region: piece coverage of the outline, largest patch under 5° (caldera ice
    excluded) and land area
  - seams: steepest slopes along each border against those inside both regions (WARN
    above 1.3 times)
  - volcano margin: the summit against the highest land outside the volcano's region
    (WARN under 400 m)
  - caldera: floor within 100 m of its target; the rim at least 200 m above the floor
    except within 30° of the breach, whose sill is the lowest rim; the floor reached by
    exactly the band its height needs, with the breach at least 50 m under that band's
    ground limit; the area changed by more than 50 m and how far out, at most 3 m of change
    in the last 200 m before 2.5 km, and nothing changed beyond
  - every pass reached by exactly its band, with any dip
  - each feature with `river = true`: its drainage path (priority flood) from the lowest
    ground within 300 m of its path's first point to the sea, its length, where it meets
    the sea (WARN more than 500 m from the path, or where the path past that point runs
    over ground above 30 m) and its largest rise above ground it already passed (WARN above
    30 m; GLO-30 river floors carry pits of 20 m or so)
  - for a breached caldera, where water from the breach drains and where it joins a river
    (info)
  - closed depressions on land of at least 1 km² and 50 m deep (info only; hydrology v2
    breaches them)
- `reach-l2.txt` and `reach-l2.png`, which bands reach which ground
- `overview.png` and `pieces.png`, the annotated map and the piece footprints
- `description.json` and `description.md`
  ([world description v1](../../schemas/world-description-v1.md)): region land and heights
  from `region32.u8`, passes, summit and caldera

`layout reach` works on the result:

```sh
$P -m topography_training.diffusion.layout reach data/worlds/fujix-collage-54 \
    --arrays data/worlds/fujix-collage-54/arrays.npz
```

The collage's `l2` is meant to replace the L1 and L2 models for fujix (`l1` is its 128 m
mean), with L3 and L4 adding detail. That is planned for sub-project 5: no chain entry
point reads the collage's `l2`, `class32.u8` or `region32.u8` yet.

### Layout v2 fields

Heights and distances are metres; positions are `[x, y]` from the north-west corner.

| Key | Meaning |
| --- | --- |
| `schema_version`, `kind` | `2`, `"world_layout"` |
| `name`, `extent_m` | World name; map side (a multiple of 32 m) |
| `hemisphere` | `north` or `south`, for the cover's pole-facing slopes |
| `wind_from_deg` | Prevailing wind (degrees, from), which sets the windward and lee sides |
| `snowline_windward_m`, `snowline_lee_m`, `treeline_m` | Snowline on each side; treeline |
| `coast_margin_m`, `seabed_edge_m` | Land stays this far inside the map edge; seabed depth at the edge |
| `[flight]` | `clearance_m` and `bands`: `{ name, top_m }` with rising `top_m` (flight height), at most four |
| `[[regions]]` | `id`, `name`, `class_id` (legend v3), `terrain`, `outline` (polygon), `border_m` (blend width, default 3,000), `forest` (0 to 1), optional `area_km2`, `floor_m`, `summit_m` targets |
| `[[regions.pieces]]` | `source` (release region), `at` (crop centre in the world), `size_m` (crop width east and height south, source metres), `offset_m` (crop centre from the source centre), `rotate_deg` (clockwise), `scale` (1/1.5 to 1.5), `remap` (source lo, hi to world lo, hi, linear), `tilt_m_per_km` (east, south), `class_id` (instead of the region's) |
| `[[passes]]` | `id`, `name`, `at`, `band` (a `[flight]` band name) |
| `[[features]]` | `id`, `kind`, `name`, `note`. Volcano: `at`, `radius_m`, `summit_m`, optional `caldera_diameter_m` and `caldera_floor_m` (both or neither; the diameter is roughly the rim crest's), optional `breach_azimuth_deg` (clockwise from north) and `breach_m` (its sill, from floor + 10 m up to below floor + 300 m; both or neither, and only with a caldera), optional `breach_width_m` (default 600: the notch's width about a third of the way up its walls, its floor a quarter of that). Gorge: `path` (head to mouth), `depth_m`, `river` (traced to the sea by the report) |

## Generating a world

`cascade_chain` builds L1 → L2 → L3 from a painted L0 (`--layout`) or, as a stand-in,
from a layout derived from a real context region (`--release` and `--region`):

```sh
$P -m topography_training.diffusion.cascade_chain --layout data/worlds/fujion-painted-l0-5 \
    --l1-run data/models/cascade-l1-run-6 --l2-run data/models/cascade-l2-run-8 \
    --l3-run data/models/cascade-l3-run-4 --out data/worlds/fujion-v3-chain-14
```

L3 covers the whole field by default, the size the layout specifies; `--l3-km N`
limits it to a central N × 1,024 m square (a derived region is 48 km, so use it there).
The chain writes the class per 4 m cell (`classes.u8`) for L4 and the cover.

After L1 the land is made to drain (largest depressions kept as lakes,
others carved; hollows of crater, karst and dune cells stay dry basins); after L2
compact deep hollows become tarns and streams are traced. Streams flow through lakes
and dry basins to the sea (only karst basins swallow them), follow smoothed paths, are
continued to the water after L3 refines the coast, and their beds are carved to descend.
Water amounts follow the region class (`hydrology.WATER`) unless overridden with
`--lake-share`, `--tarn-share` and `--stream-km2`. The output is a 4 m world
(`manifest.json`, layers, hillshades of each level) that
exports directly. A 32 km L3 takes about an hour. `--guidance w` applies class
guidance at every level (1: off). L1 runs trained on 128-cell crops are sampled in
overlapping local windows, and generation takes the form channels from the run's
per-class table (`python -m topography_training.diffusion.forms --run-dir <run>`
recomputes it without retraining).

For authored layouts, L1 holds the painted shape above about 400 m (the `hold` layer
and the map border), painted craters (class 4) at 128 m, and fills painted calderas,
craters, cinder cones and drowned tower karst (classes 4, 17 and 15) with the 128 m relief of real exemplars
from `catalog/exemplars/exemplars-v1.toml` (`--no-exemplars` turns this off). The
catalog is rebuilt from a context release with
`$P -m topography_training.diffusion.exemplar find --release data/datasets/context-v5/release.json`;
check the picks by eye. Craters are made at least as deep as painted; painted crater
hollows always stay dry basins through drainage.

`cascade_l4` refines that 4 m world to 1 m in bands of rows:

```sh
$P -m topography_training.diffusion.cascade_l4 --run data/models/cascade-l4-run-7 \
    --world data/worlds/fujion-v3-chain-14 --out data/worlds/fujion-v3-chain-14-l4 --band-m 1024
```

Bands default to 2,048 m with a 256 m halo (`--band-m`, `--halo-m`); overlapping bands
share their starting noise and are cross-faded. Output layers are memory-mapped, so
host memory depends on the band, not the world. Across 32 km, 2,048 m bands exceed a
7 GB cap, so use `--band-m 1024` there. Rerunning the same command resumes after the
last finished band. Water keeps the smooth 4 m surface. `--km N` limits the run to a
central N × 1,024 m square for quick checks; older chain worlds need `--class-id`. At
12 steps and stride 192, 32 km takes about 17 h (32 bands of about 31 min); a central
4 km about 14 min.

Then redraw the cover at 1 m from the layout and add land cover:

```sh
$P -m topography_training.diffusion.cover --world data/worlds/fujion-v3-chain-14-l4 --layout data/worlds/fujion-painted-l0-5
$P -m topography_training.diffusion.landcover --world data/worlds/fujion-v3-chain-14-l4
```

## Hydrology v2

Water as a surface height ([Fujix plan](../plans/fujix-v1.md) section 2). The Rust command
works on one grid at a time, with orographic precipitation, stream power at 32 m and
discharge-sized channels; holding a coarser network fixed (`--coarse`) and the cascade
calling it come later, and the chain still uses `hydrology.py`.

```sh
training/.venv/bin/python -I scripts/hydrology_grid.py data/worlds/fujix-collage-55 scratch/hydro-v2/fujix55-grid
target/release/topography world hydrology --grid scratch/hydro-v2/fujix55-grid --out scratch/hydro-v2/fujix55-out
```

`hydrology_grid.py` turns a collage (`arrays.npz` `l2` and `class32.u8`) into a grid
directory: `grid.json` (`size` [w, h], `spacing_m`, `origin`, optional `classes_cell_m`,
optional `wind_from_deg` from the layout copy in the collage's `layout.json`),
`elevation.f32` and optional `classes.u8` (little-endian, rows north to south). The output
directory must not exist; it is staged and renamed into place. It holds `grid.json` (input
plus `level`, `wind_from_deg`, a `discharge` entry, a `moisture` entry and provenance: input
and output SHA-256s, water table, tool version), `elevation.f32` (ground after breaching,
stream power and channel carving),
`water-surface.f32` (NaN where dry, sea 0), `water-body.u32` (0 dry, 1 sea, then lakes, then
rivers), `discharge.f32`, `moisture.f32`, `bodies.json` and `report.json`. Exit status 1
means a validation check failed; the outputs are still written so the report can be read.
`--coarse` is not implemented yet.

Precipitation comes first, from the input ground:

- Smith and Barstad (2004) linear upslope model, solved by FFT on a 128 m grid (the input
  block-averaged, 4 x 4 cells at 32 m; ground below 0 m counts as 0 m), padded by at least
  half its size each side with 0 m ground so the FFT does not wrap. Negative rates (descending
  lee air) are clipped to 0, then a uniform background rate is added. Land touching an upwind
  grid edge meets a cliff to 0 m in that padding, which puts a spurious rain spike on that
  edge; island worlds such as fujix have sea there. Edge-replicated padding tapered to 0 m is
  the fix when land-to-edge worlds use it.
- Depletion: the air enters full at the upwind grid edge and loses what rains out over land
  (non-sea cells, as the sea flag) on its way (an upwind march along the wind on the 128 m grid). Rain at each cell is scaled
  by the fraction left, exp(−rained mm / budget), with rained mm = rate · distance / wind
  speed. This gives the large-scale gradient: wet windward coasts and ranges, a dry far lee.
- Wind: `--wind-deg D` is the direction the wind comes from, clockwise from north (225 = from
  the south-west); without it, grid.json `wind_from_deg`; without either, rain is uniform,
  discharge equals drainage area and `moisture.f32` is a neutral 0.5 everywhere (judgment
  value: neither wet nor dry for vegetation).
- Judgment constants (`moisture::PARAMS`, recorded in `report.json` `precipitation.params`):
  wind 10 m/s, moist buoyancy frequency 0.005 s⁻¹, water vapour scale height 2,500 m,
  condensation and fallout times 300 s each (the short end of the usual 200 to 2,000 s, so
  rain lands on the windward flanks instead of drifting 20 km over the crest),
  uplift sensitivity 0.004 kg m⁻³, background 1.5 mm/h and a column water budget of 3 mm
  (tuned on fujix so the SW is wet and the NE lee dry; a real column holds more, but these
  are steady storm rates). Only relative rain matters downstream. On a world much wider than
  fujix's 32 km the far lee comes out drier, since the budget runs out over more land.
- `moisture.f32` is that 128 m grid (size and spacing in grid.json `moisture`): precipitation
  divided by its 95th percentile over land cells (`scale_mm_h`), clipped to 0 to 1, so most
  land spans the range for vegetation. It is written at every level for now; finer levels
  and bands will reuse the coarse level's file (`--coarse`, later), since a band run would
  restart the depletion at the band's upwind edge.
- `discharge.f32` is the drainage area weighted by precipitation over its land mean, in km² at
  mean rain (precipitation bilinear between 128 m cell centres). Rivers start at 10 km² of
  that, so wet windward catchments start rivers on smaller areas and the dry lee on larger
  ones; lake selection and river depth use it too. `report.json` `precipitation` gives the
  land mean, the 95th percentile and land means of rain and moisture by map quadrant.

Steps:

- Sea is input ground below 0 m connected 8-way to the grid edge (`cascade.open_sea` labels
  4-way; D8 flow needs 8). It is not recomputed after carving, so cuts never become sea
  inlets. The sea and the grid edge are the outlets.
- A priority flood (Barnes et al. 2014) gives the filled surface; each connected region
  filled above the ground is a depression with one spill level and outlet.
- Lakes: within each class's `lake_share` of its land (classes from
  [configs/hydrology/water-v1.toml](../../configs/hydrology/water-v1.toml), a copy of
  `hydrology.WATER`), the largest depressions by volume, at least 1 m deep and 4,096 m².
  A depression belongs to its most common class. Nested depressions count as one, at the
  outer spill: when that is over budget, a smaller inner lake that would fit is not kept
  (a known limitation, revisited with finer levels).
- Glacier basins (most common class 10) stay dry ice basins at their own ground, never lakes
  and never breached; flow passes through them. This keeps the fujix summit caldera intact.
- Dry basins: within `dry_share`, the deepest compact depressions (at least 2 m deep,
  bounding box side² / area at most 4) of dry classes with less than the river threshold
  flowing in. Flow passes through them; one that a river reaches after breaching becomes a
  lake.
- Every other depression is breached by a second priority flood: when the flood first
  reaches it, the path it came along is lowered once to the depression's lowest ground, and
  cells inside that are lower than their path lower it again (Soille 2004 carving), never
  filling. Equal heights are crossed as a flat without carving, so ground descends or stays
  level along every receiver. Kept basins are flat at their level in this flood, and the path
  into one is lowered to its level.
- Breaching cuts at most `BREACH_MAX_M` = 30 m (judgment constant, the same at every level for
  now). The pit class (the class at the depression's lowest cell) decides what happens to a
  deeper one:
  - deeper than 30 m below its spill, in a glacier or arid class (`lake_share` ≤ 0.002:
    canyons, badlands, dunes): a dry sink where flow ends (endorheic), even when its ground
    is below 0 m;
  - deeper than 30 m elsewhere: lakes at the spill lowered by 30 m, but not below 0 m or the
    level of a lake the outlet drains into (so no dry ground beside the cut lies below it), one
    per piece of ground below that level, with the outlet cut down to it;
  - at most 30 m deep but with ground below 0 m, in any class: lakes at 0 m (lagoons), with
    the outlet cut to 0 m. Breaching therefore never cuts below sea level.
- Trench widening, at every level (`erosion.rs`): breach trenches (cells breaching lowered)
  would be one-cell slots up to 30 m deep, so their dry walls are cut back to a 35° side
  slope (judgment constant), ring by ring outward. Each wall cell is lowered to the side
  slope from its lowest neighbour already in the valley, but by no more than the trench's
  breach cut, less the slope's rise per further ring, so widening dies out within a few
  cells (two rings at 32 m, up to 11 at 4 m; at most 16). A lowered wall cell drains into
  that neighbour, so heights fall strictly towards the trench and no pits appear. Never below
  the trench, 0 m or a basin level beside it; basins, the sea, glacier and all ground
  draining onto glacier or into an ice basin are untouched.
- Stream power, on grids of 32 m or coarser only (`erosion.rs`): five iterations of the
  implicit stream-power law (Braun and Willett 2013, FastScape) with n = 1, no uplift:
  each land cell, in receiver order from the outlets up, moves to
  (z + F · z_receiver) / (1 + F) with F = K·dt · A^0.5 / step length, A the discharge in m²
  (km² at mean rain × 10⁶) and K·dt = 7 × 10⁻⁴ per iteration (judgment constants, tuned on
  fujix so real valley floors move about 2 m). Steep steps, such as collage seams and cliffs
  under a river, retreat and grade; trench floors are flat, so stream power barely changes
  them (widening shapes their walls instead). The sea is base level
  at 0 m and a kept basin at its level; nothing goes below 0 m, cells beside a basin stay at
  or above its level (spills and shores hold), and basin cells, glacier cells and all ground
  draining onto glacier or into an ice basin are not touched, which keeps the summit caldera
  and its walls exactly as they were.
- Lakes grow over carved ground below their level that drains into them, so one level and
  wet exactly where the level is above the ground still hold next to a cut.
- Rivers start at 10 km² of discharge (`--river-km2`); those cells are the centreline. One
  body id per river system: all river cells upstream of one mouth, which ends in the sea, a
  lake, a sink, a glacier basin (flow passes through and a new river starts at its exit) or
  the grid edge.
- Channels, at every level (judgment constants in `rivers.rs`): width W = 2.5 m · Q^0.5 and
  depth D = 0.5 m · Q^0.4, Q the discharge in km² at mean rain: 8 m by 1.3 m at 10 km², 33 m
  by 4 m at fujix's largest (178 km²); at 4 m the 0.5 km² start is 1.8 m by 0.38 m. The
  section is parabolic and each cell is carved by its mean depth over the section, so a
  channel narrower than a cell is one shallower cell (at 32 m all but the largest rivers).
  Cells beside the centreline join the channel where the section reaches into them with at
  least a tenth of D, nearest centreline cell first, but not the sea, basins, cells next to a
  lake, or ground more than D above the centreline's final surface (in a gorge the channel
  stays narrower; such a cell is above every surface beside it, so it is a valid bank).
  They share their centreline cell's surface and body id.
- Water surface along the centreline: its ground, at most every dry bank of any of its
  channel cells and at most every surface upstream, so it never rises downstream. A bank is
  a dry neighbour beside or behind the channel cell along the flow; ground ahead (a positive
  step along the flow direction) is the next cross-section's bank, which keeps a river on a
  steep slope from being cut down to the next cell's ground. This cascade rule is an
  accepted change to "water cannot climb a bank" (ruling, Task 4 review: the export uses
  1 m and 32 m water is not rendered). It is bounded: `report.json` figure
  `river_ahead_exemptions` gives the dry cells it lets sit below an adjacent river (count,
  more than 3 m, deepest and where), and no dry cell may be a hole (below). Where a low bank still pulls the
  surface below the ground, the drop tapers upstream by at most 5 % of the step length per
  cell, so the bed ramps down over many cells instead of a one-cell notch. The bed is the
  surface minus the cell's depth. Into the sea, near sea level (surface at most 4 D), the bed
  of the last four cells descends to −D at the mouth and continues four cells into the
  seabed. Two exceptions:
  - a lake's exit cell (a river cell a lake drains into) holds its surface at the lake
    level, a sill. Dry ground beside it or its channel below the level is raised to the
    level (a rock sill, the only ground raising), and so is any dry cell draining into a
    raised one that is now below it, so nothing drains uphill;
  - the basin a river drains into is not its bank, so mouths into sinks are not cut down to
    the sink floor. `mouth_max_drop_m` reports the largest plunge from a mouth to the floor
    beside it.
- Holes: a dry cell more than 0.5 m below every adjacent water surface (sea, lake or river;
  not inside the basin an adjacent river enters) is raised to the lowest of them, and so is
  any dry cell draining into a raised one that is now lower, repeated while holes remain.
  `holes_filled` reports the cells, the largest raise and the volume.

`bodies.json` lists the sea (id 1), lakes (`exit_cell`: the first cell the lake's routed flow
enters outside it, `parent`: the body that flow reaches), rivers (`area_km2`, `drainage_km2`:
the discharge at the mouth, `length_km`, `parent`) and dry basins (`kind` `dry`,
`endorheic`, `ice`; their ids are not in `water-body.u32`, which is 0 on dry ground).

`report.json` checks (all must be 0): lakes in more than one piece or with ground at or above
their level, lake exits off the level (an outflow river more than 1 cm from it, or dry exit
ground more than 1 cm above it), river cells more than 1 cm below an adjacent lake, wet cells
whose surface is not above the ground, centreline surfaces rising downstream (including into
a lake or the sea), rivers ending on dry land, dry land draining uphill, dry cells more than
1 cm below an adjacent lake, sea or river surface (for rivers, only banks as above: not
inside the basin the river enters, not ahead along the flow) and dry cells in holes. Channel
cells are left out of the uphill check: they lie under water whose surface is flat across
the section and never rises along the centreline, so their beds may hold pools. A unit test
breaks each of the 15 checks on a small scene with a centreline and one channel cell. Three
`guard_` checks (land not draining to an outlet, lakes with several levels, centreline beds
rising more than 1 cm into the next centreline cell) hold by construction and only catch
bugs. Figures include lakes and dry basins (over budget, sinks, ice), `total_lowering`
(cells, cells over 10 m, volume and deepest, all steps together against the input), river
counts and lengths, `river_ahead_exemptions`, `holes_filled`, and per step: `trenches`
(trench count and cells, deepest breach cut, wall cells lowered, volume, rings, and the
steepest wall of each slot cell, cut more than one cell's rise, before and after),
`stream_power` (cells and volume lowered, the largest drop and where, and median, 90th
percentile and largest drop on river cells that breaching or widening did and did not cut)
and `channels` (channel cells beside the centreline, carved cells, deepest cut and where,
carve volume, the largest bank-forced surface drop, seabed cells lowered past mouths, widest
and deepest channel). Timings and the peak resident memory go to stderr only, so identical
inputs give identical outputs.

On fujix-collage-55 (2048², 32 m, wind from 225°) it takes about 3 s and 110 MB: land precipitation
averages 2.4 mm/h (95th percentile 7.6 mm/h). Land means by quadrant are 3.9 (SW), 3.1 (SE),
2.3 (NW) and 0.57 mm/h (NE), moisture 0.49, 0.40, 0.30 and 0.08. The wettest regions are the
SW sounds (6.0 mm/h), the SW arm, the W sound and the SE lowland (about 4 mm/h); the NE dunes,
rift and east desert get 0.5 to 0.8 mm/h. The volcano gets 6.2 mm/h within 5 km windward of
the summit and 2.7 mm/h within 5 km in its lee; the canyon head on its north-west flank gets
1.9 mm/h, 2.5 times the east desert. Result: 287 lakes (36 km², 1.6% of the land; 138 over
budget, some of them lagoons), 168 dry basins (64 sinks, 47 km²; 29 ice), 117 river systems
(298 km: 34 to the sea, 78 to lakes, 5 to sinks). Breaching cut 3,732 trenches (11,195
cells), at most 30 m deep; widening lowered 4,921 wall cells (at most 30 m, 0.03 km³, two
rings), and on the 540 slot cells the steepest wall went from a median 45° (90th percentile
52°, max 68°) to 35° (35°, 59°). Stream power lowered uncut river cells by a median 2.2 m
(90th percentile 7.8 m) and trench floors by a median 0.01 m; the largest drop, 28 m, is on
a river down a cliff of about 50 m per cell on the volcano (cell 915, 1159), and it moved
0.36 km³ in all, mostly fractions of a metre on hillsides. Channels: 7,548 cells, all one
cell wide at 32 m (widest 33 m, deepest 4.0 m), 8.4 million m³ carved; low banks pulled a
surface down at most 31 m (it was 61 m before the ahead rule), the deepest channel cut is
36 m at a mouth at the foot of a coastal cliff (371, 1466) and 82 cells are cut more than
10 m. The ahead rule leaves 3,804 dry cells below an adjacent river, 1,429 by more than 3 m,
at most 33 m beside a river down a 60 % volcano flank (914, 1159); 2 holes were raised, by
at most 4.8 m. The Grand Canyon river (25 km, 47 km² at its mouth) descends evenly from
805 m to its lagoon at the north coast with no bed rise. 17 lip cells were raised, by at
most 5.2 m; mouths plunge at most 14.5 m into sinks. All steps together lowered ground by at
most 54 m (that coastal cliff), 0.46 km³. All checks 0, every river exit at its lake level,
and the summit caldera's ice and walls unchanged and dry. On the 8192² 4 m chain world it
takes 54 s and 1.2 GB peak: widening lowered 102,000 wall cells at most 27 m (up to 11
rings) and brought slot walls from a median 51° (90th percentile 71°) to 35° (35°); 45,600
channel cells beside 21,600 centreline cells (widest 25 m), carved at most 10.2 m; the
ahead rule leaves 2,664 cells, 1 by more than 3 m, at most 4.2 m; no holes. 16,384² at 4 m
should need about 5 GB. A 1024² exact flat takes under 0.1 s to breach.

## Export

```sh
target/release/topography export heightmap data/worlds/fujion-v3-chain-14-l4 data/exports/fujion-v3-chain-14-l4
target/release/topography export view data/exports/fujion-v3-chain-14-l4
```

Tiles are written as `{layer}/L{level}/{prefix}_L{level}_{tx}_{ty}.png`, for example
`height/L0/height_L0_0_0.png` and `vegetation/L0/veg_L0_0_0.png` (prefixes `height`,
`water`, `snow`, `veg`, `landcover`, `biome`). `biome` holds the terrain class id per
cell (authored layouts; written by the chain and by `cover`), and the viewer's Colours
menu shows it with a legend. For authored layouts the chain reads the layout's forest share and snowline
bilinearly, and its class map by nearest cell, at positions warped by about 150 m, so
cover has no 128 m grid steps. Snow and tree lines then follow the ground: snow lies
lower on pole-facing slopes (set `hemisphere` in the paint settings) and in gullies and
sheds from faces steeper than 40–55°; trees reach higher in gullies and on sun-facing
slopes, thin out near their limit and on steep ground, and leave patchy edges.

`landcover` (run after L4, above; `--treeline-m` sets the tree line) adds `land-cover.u8` with ESA WorldCover codes derived by rule (a recorded
judgment call, not observed cover):
- water 80, snow/ice 70 and forest 10, taken from the world's own masks;
- bare 60 above 35° or more than 400 m above the local tree line, grassland 30 just
  above it (bare classes stay bare);
- dunes, badlands, lava and glacier forelands bare, canyons and badlands shrub 20,
  other open land grassland;
- class borders warped by about 150 m.

The export holds the tiles, `manifest.json` and `checksums.txt` (SHA-256 of every
tile; verify with `sha256sum -c checksums.txt` in the export). Add `--masters` to also write
full-world COGs (Float32 `elevation.tif` and the mask layers). They duplicate the
world output, so they are only needed when the export travels without it.

Export streams the world layers row by row: a 32 km world at 1 m exports in
about 13 minutes with a peak under 200 MB. The 4 m chain output exports the same
way for a quick 3D look before L4.
