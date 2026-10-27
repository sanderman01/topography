# Terrain cascade workflow

Implements the [terrain cascade](../plans/terrain-cascade-v1.md): L1 (128 m) →
L2 (32 m) → L3 (4 m) → L4 (1 m), each a residual diffusion model over the level
above. `P` below is `data/models/diffusion-env-v1/.venv/bin/python` (set up as in
[training/README.md](../../training/README.md#terrain-diffusion)). Training and
generation are GPU workloads: warn before running them.

Run heavy jobs inside a memory-capped scope so an out-of-memory job cannot take the
session down: `CAP=(systemd-run --user --scope -q -p MemoryMax=7G)`, then `"${CAP[@]}" $P ...`.

Current models (2026-10-27), all on terrain legend v3 with form channels:

| Level | Run | Training data | Class gates |
| --- | --- | --- | --- |
| L1 128 m | `cascade-l1-run-6` | `context-v5` (GLO-30 + GEBCO) | pass |
| L2 32 m | `cascade-l2-run-8` | `context-v5` | pass |
| L3 4 m | `cascade-l3-run-5` + experts `cascade-l3-run-5-experts` | `lidar-v3` (lidar averaged to 4 m), dithered borders | pass (Grand Canyon vs plain under floor, see below) |
| L4 1 m | `cascade-l4-run-8` + experts `cascade-l4-run-8-experts` | `diffusion-v9` (1 m library tiles), dithered borders | pass |

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

Results (2026-10-27; L3 stopped at step 7,156, L4 at 7,650). Both pass the class swap,
recognition (L3 own 0.26 vs swapped 0.14; L4 0.38 vs 0.11), roughness (hidden rank 0.83
and 0.90) and the dithered-border gate. Both fail Grand Canyon vs 13 (L3 0.58 between
classes vs 0.67 between seeds; L4 0.45 vs 0.59) and L3 fails vs plain (1.30 vs floor 1.56),
so `class_gates` exits 1: alone, the runs treat class 19 as its parent 13.

Grand Canyon experts (2026-10-27): class-19-only fine-tunes of these runs (`--only-class 19
--init-from`, no mixed windows, lr 1e-4; L3 2 h, 2,740 steps; L4 3 h) in
`cascade-l3-run-5-experts/c19` and `cascade-l4-run-8-experts/c19`, used as a `ClassMixture`
over the base run (class gates `--class-runs`). With them Grand Canyon vs 13 passes (L3 0.81
between classes vs 0.76 between seeds; L4 1.36 vs 0.79), the dithered border passes, and the
other gates are unchanged. L3 vs plain stays under its floor (1.09 vs 1.56, above seed noise
0.76); accepted, since the floor uses real centroids of windows with different coarse input.

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
    --l3-run data/models/cascade-l3-run-5 --out data/worlds/fujion-v3-chain-14
```

L3 covers the whole field by default, the size the layout specifies; `--l3-km N`
limits it to a central N × 1,024 m square (a derived region is 48 km, so use it there).
The chain writes the class per 4 m cell (`classes.u8`) for L4 and the cover.

After L1 the land is made to drain in Python (`hydrology.drain`: largest depressions kept
as lakes, others carved; hollows of crater, karst and dune cells stay dry basins). Water
after L2 and L3 comes from [Hydrology v2](#hydrology-v2) (`topography world hydrology`, the
release binary, which the chain builds with Cargo when it is stale): L1 lakes get a bed
sloping down from their shore (0.1, at most 40 m) so the 32 m run keeps them, then the 32 m
run (wind from the layout) carves L2's ground and sets its sea, lakes and rivers. L3
refines that carved ground, with the 32 m water as its water channel (32 m rivers drawn
about 20 m wide), and its ground under lakes and rivers eases down to the carved 32 m
ground. The 4 m run (`--coarse` the 32 m output) then holds the 32 m network fixed, adds
tarns and tributaries and carves the 4 m ground; that is the world's elevation. Outputs
stay in `hydrology/32m` and `hydrology/4m` (L4 reads the 4 m one). Water amounts follow
the region class (`configs/hydrology/water-v1.toml`, which Rust and Python both read);
`--lake-share` and `--tarn-share` set them for every class, `--stream-km2` the 32 m river
start (default 10 km²). `--l1-class-runs`, `--l2-class-runs` and `--l3-class-runs` mix
per-class expert runs (`cN` subdirectories) over a level's run. The output is a 4 m world
(`manifest.json`, layers, hillshades of each level) that
exports directly. Beside `water.u8` (kept until export v2) it holds `water-surface.f32`
(NaN dry, sea 0), `water-body.u32`, `bodies.json` and the 128 m `moisture.f32` (placed by
the manifest's `moisture` entry); the hydrology layers are hard links into
`hydrology/4m`. A 32 km L3 takes about an hour. `--guidance w` applies class
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
$P -m topography_training.diffusion.cascade_l4 --run data/models/cascade-l4-run-8 \
    --world data/worlds/fujion-v3-chain-14 --out data/worlds/fujion-v3-chain-14-l4 --band-m 1024
```

Bands default to 2,048 m with a 256 m halo (`--band-m`, `--halo-m`); overlapping bands
share their starting noise and are cross-faded; the halo may be at most two thirds of a
band. Once the band below is generated, a band's rows are final and it gets its 1 m water
from `topography world hydrology` with `--coarse` the world's `hydrology/4m` and `--north`
the previous band ([Bands and seams](#bands-and-seams)), over its rows plus a halo as wide
as `--halo-m`; it keeps the core rows, and `hydrology-seam` checks each pair (a failure
stops the run). Under water the ground is the refined 4 m ground (L4 learned land only).
The output adds `water-surface.f32`, `water-body.u32`, `bodies.json` (merged from the bands,
areas from the delivered cells) and the world's 128 m `moisture.f32`; band reports stay in
`hydrology/`. Output layers are memory-mapped, so host memory depends on the band, not the
world. Across 32 km, 2,048 m bands exceed a 7 GB cap, so use `--band-m 1024` there.
Rerunning the same command resumes after the last finished band (diffusion or hydrology).
`--km N` limits the run to a central N × 1,024 m square for quick checks; worlds without a
`hydrology/4m` output (made before hydrology v2) must be regenerated, and older chain
worlds need `--class-id`. `--class-runs` mixes per-class expert runs over `--run`. At
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
discharge-sized channels. A finer level holds the coarser level's network fixed (`--coarse`,
[below](#finer-levels)), and 1 m bands carry the previous band's rows (`--north`,
[below](#bands-and-seams)). The cascade calls it at every level from 32 m down
([Generating a world](#generating-a-world)); `hydrology_grid.py` below makes grids for
running it by hand.

```sh
training/.venv/bin/python -I scripts/hydrology_grid.py data/worlds/fujix-collage-55 scratch/hydro-v2/fujix55-grid
target/release/topography world hydrology --grid scratch/hydro-v2/fujix55-grid --out scratch/hydro-v2/fujix55-out
```

`hydrology_grid.py` turns a collage (`arrays.npz` `l2` and `class32.u8`) into a grid
directory (with `wind_from_deg` from the layout copy in the collage's `layout.json`); with
`--chain-l2`, `--world` or `--band` it takes a cascade chain world's 32 m level, a world as it
is, or rows of a 1 m world ([figures](#fine-level-figures) shows the commands). A grid
directory holds `grid.json` (`size` [w, h], `spacing_m`, `origin`, optional `offset_m`,
`core_rows`, `classes_cell_m` and `wind_from_deg`), `elevation.f32` and optional
`classes.u8` (little-endian, rows north to south). `origin` [x, y] is the world's north-west corner in the layout plane (x east,
y north; default [0, h · spacing]); `offset_m` [east, south] places a band or window inside
that world, so its north-west corner is (origin x + east, origin y − south); `core_rows`
[start, end) are the rows a band delivers (default all; the rest is halo). The output
directory must not exist; it is staged and renamed into place. It holds `grid.json` (input
plus `level`, `wind_from_deg`, a `discharge` entry, a `moisture` entry, a `flow` entry and
provenance: input and output SHA-256s, the coarse level's and previous band's files, water
table, tool version), `elevation.f32` (ground after breaching, stream power and channel
carving), `water-surface.f32` (NaN where dry, sea 0), `water-body.u32` (0 dry, 1 sea, then
lakes, then rivers), `discharge.f32`, `moisture.f32`, `flow.u8` (low 4 bits the D8 receiver,
0 none; 16 river centreline, 32 dry basin, 64 endorheic basin; what a finer level and the
next band read), `bodies.json` and `report.json`. Exit status 1 means a validation check
failed; the outputs are still written so the report can be read. `--river-km2` defaults by
spacing: 10 km² at 16 m and coarser, 0.5 km² from 2 m, 0.05 km² below.

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
  [configs/hydrology/water-v1.toml](../../configs/hydrology/water-v1.toml), which Python's
  `hydrology.WATER` reads too), the largest depressions by volume, at least 1 m deep and 4,096 m².
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
  breach cut, less the slope's rise per further ring, so widening dies out (a cell reached
  again from another side keeps that bound from its ground before widening). The ring count
  follows the deepest cut: ceil(cut / (tan 35° · spacing)) + 1, so a 30 m cut at 1 m gets 44
  rings. Trench cells are walls too, so a shallow breach crossing a deep one is lowered with
  the valley instead of standing as a fin. A lowered wall cell drains into that neighbour, so
  heights fall strictly towards the trench and no pits appear. Never below the trench, 0 m
  or a kept basin level beside it. On the trunk level all basins are kept; on finer levels
  coarse lakes, tarns and basins on a coarse dry basin (such as an ice caldera) are kept,
  while other fine dry and ice basin cells may be lowered and then leave their basin
  (`trenches.basin_cells_lowered`). The sea and cells from the previous band are untouched.
  Glacier ground widens like any other (ice is a surface layer), so a breach through a
  glacier rim is a valley, not a slot. On finer levels burned coarse river paths widen the
  same way.
- Channel widening, at every level, after channel carving: channel cells carved more than
  one cell's rise (spacing · tan 35°) widen the same way from their water surface
  (`channel_widening`). Ground is never lowered below the surface of a river it is a bank
  of (as the bank check: not ground ahead along the river's flow), more than 0.5 m below
  every river beside it (a hole) or a kept basin level beside it. Lowered cells drain into
  the channel; their discharge is not recomputed.
- Receivers, at every level: after breaching and widening, every land cell outside basins
  drains to its steepest-descent neighbour on the final ground (drop over distance, a diagonal
  √2 long; a basin neighbour counts at its level). The flood's discovery order picks the lowest
  neighbour, which is nearly always a diagonal (about 90 % of receivers) and drew 45° streams
  across slopes; steepest descent gives about 50 % diagonals. Cells with no strictly lower
  neighbour (flats) keep the flood receiver, so every cell still drains and no cycle can form.
  `receivers_set_by_steepest_descent` counts the cells changed.
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
  seabed. Three exceptions:
  - a lake's exit cell (a river cell a lake drains into) holds its surface at the lake
    level, a sill. Dry ground beside it or its channel below the level is raised to the
    level (a rock sill), and so is any dry cell draining into a raised one that is now below
    it, so nothing drains uphill;
  - a centreline never stands below an adjacent lake's level or the water it flows into (a
    sill, the sea at 0 m, the previous band's river): it rises to it, and lower surfaces
    upstream rise too (a backwater; an outflow running along its lake's shore stays at the
    level). Dry ground (dry basin cells too) beside a raised channel below its surface is
    raised as at a sill, and so is any dry cell left beside a lake below its level (a
    divide). `channels.centreline_cells_raised_to_a_lake` counts the raised centreline cells
    (0 on fujix; on finer levels they are where the fine valley lies below the coarse
    water). Beds never rise downstream: each centreline bed is at most every bed upstream
    (only lowering);
  - the basin a river drains into is not its bank, so mouths into sinks are not cut down to
    the sink floor. `mouth_max_drop_m` reports the largest plunge from a mouth to the floor
    beside it.
- Holes: a dry cell (dry basin cells too) more than 0.5 m below every adjacent water surface
  (sea, lake or river; not inside the basin an adjacent river enters) is raised to the lowest
  of them, and so is any dry cell draining into a raised one that is now lower, repeated while
  holes remain.
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
(trench count and cells, deepest breach cut, wall cells lowered, volume, rings used and
allowed, and the steepest wall of each slot cell, cut more than one cell's rise, before and
after: median, 90th percentile, largest and where, and how many exceed 60°; trench cells
count as walls), `channel_widening` (the same for carved channels),
`stream_power` (cells and volume lowered, the largest drop and where, and median, 90th
percentile and largest drop on river cells that breaching or widening did and did not cut)
and `channels` (channel cells beside the centreline, carved cells, deepest cut and where,
carve volume, the largest bank-forced surface drop, seabed cells lowered past mouths, widest
and deepest channel). `check_examples` gives up to three failing cells ([column, row]) per
failed check. Timings and the peak resident memory go to stderr only, so identical inputs
give identical outputs. A lake piece touching a band edge row is not counted as a second
piece (it may join the rest beyond the band).

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
and the summit caldera's ice and walls unchanged and dry. On the 8192² 4 m chain world run
alone (as a trunk level, without `--coarse`) it takes 54 s and 1.2 GB peak: widening lowered
102,000 wall cells at most 27 m (up to 11 rings) and brought slot walls from a median 51°
(90th percentile 71°) to 35° (35°); 45,600 channel cells beside 21,600 centreline cells
(widest 25 m), carved at most 10.2 m; the ahead rule leaves 2,664 cells, 1 by more than
3 m, at most 4.2 m; no holes. A 1024² exact flat takes under 0.1 s to breach.

The fujix figures above predate steepest-descent receivers. With them (about 4 s): 50 % of
land receivers are diagonal (89 % before); 113 river systems, 287 km (37 to the sea, 72 to
lakes, 4 to sinks); lakes and dry basins unchanged; the deepest channel cut 15 m and all
steps together at most 35 m (0.48 km³); the ahead rule leaves 4,063 cells, 1,825 by more
than 3 m, at most 46 m (416, 1490), as streams now run straight down steep flanks; 2 holes
raised, by at most 3.6 m. The Grand Canyon river (25.6 km, 48 km²) still ends in its north
coast lagoon, every check is 0, and the caldera's 2,141 glacier cells are unchanged and dry.

### Finer levels

```sh
target/release/topography world hydrology --grid <4m-grid> --coarse <32m-out> --out <4m-out>
```

`--coarse` is the previous level's `--out`. The finer grid must lie inside it, aligned to its
cells (the spacing a whole multiple, `origin`/`offset_m` on a fine-cell boundary), or the run
stops with an error. Only the coarse rows and columns over the grid, plus one cell around,
are read. The coarse network is then held fixed:

- Moisture: the coarse `moisture.f32` is reused (bilinear, placed by its `nw_m`) and copied to
  the output, never recomputed; discharge weights are moisture × the coarse `scale_mm_h` / the
  coarse `mean_land_mm_h`, so discharge stays in the coarse level's km² at mean rain.
  `--wind-deg` is refused.
- Sea: input ground below 0 m connected to the world edge, to a coarse sea cell or to the
  previous band's sea. A window edge is an outlet only where it is the world's edge or the
  coarse cell there drains out across it; coarse discharge entering across a window edge is
  added to the edge cells it enters (`edge_inflow_cells`).
- Coarse lakes keep their id and level. Wet is fine ground below the level, 8-connected to
  seed cells (fine cells below the level inside the coarse lake's cells, and the previous
  band's cells of the lake); no ground is dug out, so coarse lake cells whose fine ground is
  above the level stay dry. The region grows breadth first over ground below the level, not
  over a coarse river's cells (the river keeps its course), up to four coarse cells out from
  the seeds; still growing there counts as a leak, as does water beside the sea for a lake
  above 0 m, at the world's or the grid's edge, or beside a lake with a lower level. A leaking
  branch is cut at its narrowest breadth-first layer between one and two coarse cells out
  (and nearer than its nearest leak) by a sill raised to the level plus 0.05 m; a leak nearer
  the seeds is raised where it is, except on a band edge, where the lake may go on. Cut-off
  ground stays out of lakes as high, and the rounds repeat (at most 8); a leak left after them
  fails `coarse_lake_leaks_left` (a lake still growing at the bound included). One piece per
  lake (the largest, and any touching a band edge or the previous band's cells); smaller
  pieces stay dry. Dry ground left beside a lake below its level, and a coarse river's cells
  beside it below the level plus 0.05 m, are raised to that (a shore). A lake with no fine
  ground below its level keeps its lowest cell 0.25 m below. The coarse river leaving a lake starts beside it in the coarse exit cell, at the
  level. A depression the flood reaches from a lake joins it; one around a lake is breached
  past the cap (not below 0 m; its ground below the lake's level joins the lake), so the lake
  drains. At the end, a dry cell with 6 or more cells of one lake around it (a sill or divide
  row the lake later surrounded) goes back to its input ground and joins the lake when that is
  below the level (`notch_cells_joined_to_a_lake`). `coarse.lakes` reports
  `bed_max_lowering_m` (only the one-cell case), sill and shore cells raised with the largest
  raise and example cells, cut-off pieces left dry, distinct leaking cells cut off and
  `leaks_left`. `lake_exits_off_the_coarse_exit` counts lakes whose fine exit is more than one
  coarse cell from the coarse one, with examples (a figure, by ruling).
- Coarse rivers keep their id and get a fine centreline, largest first. Its corridor is the
  fine cells of the river's coarse cells and their neighbours (and of the river it joins,
  and all of the lake it ends in, whose fine water may lie further in), not other coarse
  rivers or lakes, the sea, earlier paths, or ground beside a lake other than the one it ends
  in or leaves. A priority flood over the corridor starts where the
  coarse river ends: the sea, its lake, the fine path of the river it joins, the dry basin it
  flows into, the previous band's cells of it, or the grid edge within a coarse cell of where
  it leaves the grid. Each flood branch carries the highest coarse order it has reached
  (centreline steps to the river's end) and enters no cell more than one step below, so the
  flood climbs the river upstream; a stretch that leaves the grid and comes back is not routed
  back to where it left. Corridor cells the river's own end cannot reach (another river's path
  cuts across) end at that river instead. Each coarse head (a centreline cell without a
  centreline donor in the grid) starts a path at the fine cell whose path climbs least (then
  the lowest), which follows the flood down; a low head on a steep cell whose path climbed to
  the stem would have the stem burned down to it. The surface cap along a path is the
  highest coarse surface of the river over the coarse cell it is in and the 8 around it, from
  the river's coarse cells not downstream of where the path is (the coarse order of the last
  river cell it passed, or of the upstream-most river cell beside it), and never below the
  coarse surface interpolated along the path between the middles of its runs through the
  river's coarse cells. A downstream cell's surface beside a steep coarse drop would otherwise
  cut a gash (132 m at 4 m before this rule). The path is burned in (a running minimum from
  its head, at most the cap, not below a lake floor beside it or 0 m), and each cut then
  tapers off upstream at the 35° trench side slope instead of ending in a cliff, before the
  fine flood, so the fine routing follows it. After breaching the path is burned again (cuts
  breaching made along it), and both burns' cells widen like breach trenches; after widening
  and the steepest-descent step it is routed and burned a last time. Its discharge is the
  coarse one. `coarse.rivers` reports paths, cells, burned cells, the deepest burn below the
  ground the path had (about the input ground) and where, the deepest later burn
  (`burn_after_breach_max_m`) and where, and rivers whose head the flood did not
  reach (counted only in the grid's core rows: a band's halo is made again by the next band).
- Depressions: no new lakes but tarns at 4 m (from 2 m spacing: the class `tarn_share` as
  the lake budget), none at 1 m, and no new dry basins. A depression touching a coarse dry
  basin (`flow.u8`) stays a dry basin (endorheic if the coarse one was), glacier depressions
  stay ice basins, and none is kept in or beside a coarse lake or (but these) beside a coarse
  river path. Deeper than the cap, or below 0 m, a depression becomes a dry sink, not a lake.
- Tributaries start from the fine discharge at `--river-km2` (0.5 km² at 4 m, 0.05 km² at
  1 m), each a system with its own id above every coarse id (`bodies.json` `coarse` false,
  `parent` the body it joins); reaching a coarse river, it ends there (`rivers_to_rivers`).
  Channels as on the trunk level; stream power only from 32 m.
- Checks added on finer levels (all must be 0): `coarse_lakes_missing` (a coarse lake with
  fine cells here but no fine lake cell), `guard_coarse_lake_levels_changed`,
  `coarse_rivers_missing_or_unrouted` (a coarse river with a centreline here but no fine
  centreline, or a head the flood did not reach) and `water_above_coarse_surface` (a fine
  centreline of a coarse river more than 1 cm above the highest of its coarse surfaces over
  its coarse cell and the 8 around it, or above its path's cap where that is higher) and
  `coarse_lake_leaks_left` (above). Unit tests break each of them. `coarse.consistency` lists
  the cells and rivers. `total_lowering` gives the deepest lowering and where.

### Bands and seams

The 1 m level runs in bands of rows (as `cascade_l4.py` generates it): a band's grid holds its
core rows and a halo above and below (`offset_m`, `core_rows`). Outputs cover the whole band
grid; the cascade keeps the core rows. Bands run north to south, each with `--north` set to
the previous band's output:

```sh
topography world hydrology --grid <band-k> --coarse <4m-out> --north <band-k-1-out> --out <band-k-out>
topography world hydrology-seam --north <band-k-1-out> --south <band-k-out>
```

The grid's top rows must lie in the previous band's core, and its core must start where that
core ends. Those rows are taken as they are (ground, surface, body ids, discharge, flow) and
never changed, so adjacent bands agree on the seam by construction; the check then shows the
cells this band makes are consistent with them. A taken cell that drains into this band's
cells feeds it its discharge; a taken river entering continues with its id (a coarse river as
the head of its path, a tributary down the routing while its discharge holds), and the taken
dry cells around its entry are not outlets, so it cannot turn straight back. Other taken
cells beside this band's cells are outlets. A new stream that would run onto taken dry ground
(the previous band did not see enough discharge for it) is dropped
(`band.streams_pruned_at_seam`); beds draining into a taken channel stay at or above its bed.
Checks run on this band's cells and on taken cells against them; `rivers_cut_at_seam` (a
taken centreline flowing into this band onto dry ground that is not a basin) must be 0.

`hydrology-seam` compares two band outputs and exits 1 on a failure: the rows of the northern
core that the southern grid holds must match (ground and surface within 1 mm, body ids
equal); across the seam (the last northern core row against the first southern one) no
centreline may flow onto dry ground that is not a basin, no dry cell may sit more than 1 cm
below an adjacent lake or the sea, and a lake keeps one level. Dry cells below an adjacent
river across the seam are left to the southern band's own checks (its grid holds both sides
with each channel cell's centreline); the tool reports those not ahead of the wet cell's own
flow as a figure. Body kinds come from both bands' `bodies.json` (a river may run only in the
northern rows). `halo_agreement_share` is how often the northern band's halo guess matched
the southern core. Unit tests break each seam check and `rivers_cut_at_seam`.

### Fine-level figures

Measured on the Fujion chain: a 32 m coarse level from `fujion-v3-chain-14` (`arrays.npz` `l2`,
1024², most common 4 m class per 32 m cell, wind from 225°), the 8192² 4 m world against it,
and two adjacent 1 m bands from `fujion-v3-chain-14-l4` (32,768 × 1,024 cells: 512 core rows,
256-row halos) against the 4 m output. The grids come from `scripts/hydrology_grid.py`:

```sh
py="training/.venv/bin/python -I scripts/hydrology_grid.py"; g=scratch/hydro-v2/grids; o=scratch/hydro-v2/r4
$py --chain-l2 data/worlds/fujion-v3-chain-14 $g/c14-32
$py --world data/worlds/fujion-v3-chain-14 $g/c14-4
$py --band data/worlds/fujion-v3-chain-14-l4 16128 1024 256 $g/band-A
$py --band data/worlds/fujion-v3-chain-14-l4 16640 1024 256 $g/band-B
run="systemd-run --user --scope -q -p MemoryMax=5G target/release/topography world hydrology"
$run --grid $g/c14-32 --out $o/c14-32
$run --grid $g/c14-4 --coarse $o/c14-32 --out $o/c14-4
$run --grid $g/band-A --coarse $o/c14-4 --out $o/band-A
$run --grid $g/band-B --coarse $o/c14-4 --north $o/band-A --out $o/band-B
target/release/topography world hydrology-seam --north $o/band-A --south $o/band-B
```

| Run | Time | Peak RAM | Result |
| --- | --- | --- | --- |
| 32 m, 1024² | 1.0 s | 34 MB | passed: 148 lakes, 49 rivers, 83 km |
| 4 m, 8192², `--coarse` | 83 s | 1.27 GB | passed: 204 lakes (148 coarse, 56 tarns), 463 rivers (49 coarse), 596 km |
| 1 m band A, `--coarse` | 46 s | 0.73 GB | passed: 12 coarse lakes, 350 rivers (23 coarse), 102 km |
| 1 m band B, `--north` A | 37 s | 0.89 GB | passed: 5 lakes, 203 rivers (17 coarse), 57 km |
| `hydrology-seam` A, B | 0.2 s | small | passed: 256 shared rows identical, nothing cut, halo guess 99.7 % |

Times were measured while a GPU training job ran on the same machine and vary by up to a
third between runs. About 50 % of land receivers are diagonal at every level. At 4 m: 91
tributaries end in a coarse river; 41 centreline cells were raised to a lake or the water
below; lake beds were lowered only for one lake without fine ground below its level
(0.32 m); 290 sill cells (at most 9.4 m) and 507 shore cells (at most 9.3 m) were raised,
where coarse lake levels stand above the fine ground around them; 17,244 leaking cells were
cut off, no leak is left; 150 cells raised inside a lake joined it (at the end, from the
input ground); 7 lake exits lie more than a coarse cell from the coarse exit. 4 m memory is
about 20 bytes a cell, so 16,384² (64 km) needs about 5 GB. At 1 m, sills rise at most 2.8 m and shores 1.5 m; band B took 14
river inflows from band A and dropped 7 new streams at the seam; a 1 m band of 32 km is under
1 GB and under a minute, so a whole 32 km world (64 bands of 512 rows) is about an hour, and
64 km bands about 2 GB.

Deep cuts by source (largest, metres): before the cap rule (fix round 1), after it, with
slots through glacier widened (round 3) and now that trench cells, fine basins and channel
cuts widen too, over as many rings as the cut needs (current):

| Source | 4 m before | 4 m after | 4 m r3 | 4 m now | 1 m A now | 1 m B before | 1 m B r3 | 1 m B now |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Coarse path burn | 68 | 19 | 19 | 19 | 9 | 30 | 30 | 30 |
| Later path burn (after breaching or widening) | | | 4.1 | 4.1 | 0.5 | | 1.8 | 1.8 |
| Channel carving | 33 | 20 | 20 | 20 | 7.5 | 16 | 8 | 6.9 |
| Trench and burn widening | 68 | 27 | 28 | 28 | 8.5 | 9 | 30 | 30 |
| Channel widening | | | | 20 | 7.5 | | | 6.9 |
| Lake bed | 0.3 | 0.3 | 0.3 | 0.3 | 0.3 | 0 | 0 | 0 |
| All steps together | 132 | 31 | 31 | 31 | 11 | 31 | 31 | 31 |
| Cells lowered over 10 m | 9,780 | 8,209 | 9,044 | 9,474 | 540 | 860 | 10,952 | 14,095 |
| Cells lowered over 50 m | 628 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |

The Fuji summit crater (coarse lake 149 at 3,386.7 m) drains through a breach in its glacier
rim. Trench widening used to skip glacier walls, which left that exit as a slot one cell
wide and 20 to 31 m deep at every level; round 3 widened it through glacier but left a
fin and, at 1 m, vertical walls. Now, at 4 m, the shallow breach cells that crossed the
widened valley as a blade 20 m high ((3447, 4359) to (3450, 4356)) are lowered with it: the
section along row 4358 falls from 3,429 m to the river at 3,386 m and climbs to 3,422 m at
the side slope, and no wall in the exit (3420–3480, 4340–4375) is steeper than 54° (81°
before). At 1 m (band B) the cut needs 44 rings (16 were allowed), and the ice-basin cells
on its east side are lowered too (892 basin cells left their basin): along row 822 both
walls now climb at 35° to the original ground, where round 3 left an 18 m step on the west
side and a 29 m wall on the east. In the exit (13770–13830, 790–880) no cell is a fin and 6
cells keep a wall over 60° (at most 68°, at (13770, 832), where widening dies out on the
crater wall); 522 did before. All steps together still lower at most 31 m; band B's cells
over 10 m rose from 10,952 to 14,095 (0.5 M m³ in all). Cells more than 10 m above both
opposite neighbours with a cut under 3 m: 66 → 13 at 4 m, 1 → 0 in band A, 121 → 0 in band B.
The 2 new ones at 4 m ((4634, 6127), (4633, 6131), about 10.3 m) lie on a river falling
12 m per 4 m cell down a 70° cliff: a bank cell raised to the river surface above it (a
hole) stands beside walls that channel widening lowered.

Channel cuts deeper than one cell's rise now widen: at 4 m 9,238 such cells, 11,600 wall
cells lowered (at most 20 m, 0.4 M m³); their walls over 60° fell from 874 to 141 (steepest
83° → 76°). The 20 m gorge at (4634, 6141) is a cascade down that cliff; 311 cells around
it are now lowered (up to 20 m), but banks may not drop below the river surface they hold,
so walls there stay steep. Slot walls left over 60° are mostly natural steps in the input
that a cut no deeper than the trench's cannot remove (for example the 78° at (3908, 1952),
a 15 m terrace edge beside a burned path), and at 1 m a dry breach line under a wide river
whose channel cells 7 m above it were not lowered ((26410, 693), 82°, as in round 3). On
fujix the caldera's 632 ice-basin cells and all 2,141 glacier cells within 4 km of the
summit are unchanged and dry; 27 cells elsewhere changed (shallow trench cells, at most
10.5 m lower or 3.4 m higher than before), and the 32 m Fujion level is unchanged. Peak
memory rose by about 2 % (1.27 GB at 4 m).
The 132 m at 4 m (3775, 6418) was not uncarved coarse ground: the path ran a coarse column
beside a river falling 45 m per 32 m cell, and the cap took the surface of the river cell
downstream of it (the coarse level had cut there only 16 m). The rule above removes it
(11.5 m there now). The largest left at 4 m are path burns (19 m at (2358, 3800)) and their
widening, and channels; together at most 31 m at (3911, 1951). In band B the 30 m burn at
(13797, 853) is the outflow of a coarse summit lake whose level lies about 30 m below the 1 m
ground around it: the head is held at the lake level and the path cut down to it. The test
worlds were generated from uncarved coarser ground (the 4 m world from the 32 m `l2` before
hydrology, the 1 m world from the 4 m world before it), so where the coarse level cut down
(breach trenches, stream power, channels, lakes held below their surroundings) the finer level
cuts again; generating each level from the carved coarse ground (Task 6) should shrink those,
while channel cuts follow discharge and stay. Task 6 should also not flatten water per coarse
cell: the 4 m test world holds 128 m flat water terraces (values in 32-cell strips), and lakes
and tarns on them inherit block shapes; tributaries on those flats still run straight.
Sills form straight runs where a leaking branch is cut on a breadth-first layer (now its
narrowest layer); placing them at the highest cells of the leak front would need a saddle
search and is left. PNG crops:
`scratch/hydro-v2/c14-4m-tributaries.png` (4 m, 4 km square at cells 1024, 5120, fix round 3),
`scratch/hydro-v2/c14-4m-steepest-comparison.png` (that crop with flood-order receivers on
the left, steepest descent on the right) and `scratch/hydro-v2/band1m-tributaries-seam.png`
(1 m, 1 km × 768 m at column 16,384 of band B, the seam at row 256); coarse rivers blue,
tributaries cyan with their centreline bright.

### In the cascade

Verified on the Fujion layout (`fujion-painted-l0-5`; L1 run 6, L2 run 8, L3 run 5 with its
Grand Canyon expert, L4 run 8 with its expert): the chain with `--l3-km 8`, then
`cascade_l4 --km 4 --band-m 1024` (four bands, each with its seam check), every hydrology run
and seam passed:

| Step | Time | Peak RAM (capped 7 GB) | Result |
| --- | --- | --- | --- |
| Chain (L1, L2, 32 m, L3 8 km, 4 m) | 5.8 min (hydrology 2 s and 8 s) | 2.7 GB | 66 lakes and 39 rivers (78 km) at 32 m; 9 lakes and 80 rivers (34 km) at 4 m, 1.5 % water |
| L4, 4 km at 1 m | 15.7 min (hydrology 6-7 s a band) | 2.7 GB | 37 to 310 rivers a band, 0.13-0.30 % water |

Generating each level from the carved level above shrinks the cuts: on this 8 km crop at 4 m
all steps together lower ground by at most 20 m (31 m on the uncarved test world above), path
burns 0 m (19 m), trench walls 6.8 m (28 m), with 255 cells cut more than 10 m; at 1 m at most
7.4 m and none over 10 m (11 m and 31 m before). Compared with the old Python water on the
same layout (`fujion-v3-chain-14`), lakes now have beds below a level surface (no flat
128 m terraces), the Fuji summit crater keeps its lake with an outflow, and the 4 m world has
a branching river network where the old one drew a single stream. 1 m tributaries on smooth,
evenly sloping cone flanks still follow D8 fall lines as straight parallel lines. PNGs:
`scratch/hydro-v2/t6/water-4m-old-vs-new.png`, `water-4m-8km.png` and `water-1m-4km.png`
(sea navy, lakes blue, rivers cyan).

The run found and fixed five robustness gaps in the Rust tool: wall cells that trench widening
pointed back at a trench end formed receiver cycles (they now keep their old receiver, or in
channel widening take the steepest lower neighbour not draining into them, and any cycle left
spills like a depression); a coarse path ending in a coarse dry basin with no hollow at the finer
level continues down the routing; coarse paths run over dry basin floors (at 1 m a glacier is a
field of thousands of small ice basins, which had left a coarse river missing); a stream
turning back across a seam onto the previous band's ground beside its river joins that river;
and body lists over 1 MiB are read.

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
