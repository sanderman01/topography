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

The collage's `l2` replaces the L1 and L2 models for fujix (`l1` is its 128 m mean), with
L3 and L4 adding detail: see [Generating fujix from the collage](#generating-fujix-from-the-collage).

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

L3 runs in bands of rows across the field, north to south (`--l3-band-m`, default 8,192 m,
with a `--l3-halo-m` of 1,024 m above and below; both multiples of 512 m; a last band
shorter than the halo joins the one above), like L4: the starting noise is drawn per
256-cell tile of the map's 4 m grid, so a check square gets the noise the whole map gets at
that place, and the halo is cross-faded into the band above, so the result hardly depends on
the band height (a 16 km square in two bands and in one differ by 3 mm on average). Each
band's 4 m inputs (coarse surface, water, cover, classes, form channels) are computed with
a 512 m margin and cropped, and the raw surface goes to a memory-mapped `l3-raw.f32`;
layers and hillshades are then written in blocks of rows. The whole-map 4 m description is
the exception: it measures the memory-mapped surface but holds whole-map masks and labels
(about 3 to 4 GB at 64 km), within the 8 GB peak of the 4 m stage. Rerunning the same
command resumes after the last finished band, or at the 4 m hydrology, exactly as an
uninterrupted run would end, even after a kill in the middle of a band's write:
`l3-raw.f32`, `l2.npz`, `progress.json` (written atomically) and each band's bottom halo
rows (`overlap-N.f32`, which the next band cross-fades with) hold that state until the
manifest is written. `l3-raw.f32` (the raw L3 surface before the 4 m hydrology, 1 GB at
64 km) is kept after that, so the 4 m hydrology can be run again without diffusing again. Windows whose coarse ground is all below −50 m are not
diffused: L3 learned land only and its output is the coarse ground there anyway (a coastal
8 km square: 115 of 225 windows skipped, half the time, deep sea identical and land within
3 mm of a run without the skip). `--guidance-classes 19` limits `--guidance` to windows
holding class 19. `--l3-bands A:B` diffuses only bands A to B−1 and leaves the others
coarse (memory and speed checks). `arrays.npz` holds `l1`, `l2` and `l2_raw`; the 4 m
surface is the world's `elevation.f32`.

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
exports directly. Beside `water.u8` (the 4 m mask L4 conditions on; the export ignores it) it holds `water-surface.f32`
(NaN dry, sea 0), `water-body.u32`, `bodies.json` and the 128 m `moisture.f32` (placed by
the manifest's `moisture` entry); the rasters are hard links into `hydrology/4m` (copies
across file systems). A world's `bodies.json` lists the sea, lakes, rivers and dry basins of
at least 1 ha (a 1 m glacier holds thousands of tiny ice basins); a parent left out becomes 0.
At 1 m it is merged from the bands: water areas count delivered cells, a dry basin keeps its
largest band area, and each body's parent comes from the band whose core holds its end. A 32 km L3 takes about an hour. `--guidance w` applies class
guidance at every level (1: off); it evaluates the model a second time in each guided window. L1 runs trained on 128-cell crops are sampled in
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

Bands default to 1,024 m with a 256 m halo (`--band-m`, `--halo-m`); overlapping bands
share their starting noise (drawn per tile of the map's 1 m grid, as in L3) and are
cross-faded, resuming exactly as in L3; the halo may be at most two thirds of a band. Once the band below is generated, a band's rows are final and it gets its 1 m water
from `topography world hydrology` with `--coarse` the world's `hydrology/4m` and `--north`
the previous band ([Bands and seams](#bands-and-seams---north)), over its rows plus a halo as
wide as `--halo-m`; it keeps the core rows, and `hydrology-seam` checks each pair (a failure
stops the run). Under water the ground is the refined 4 m ground (L4 learned land only), so
bands, and windows inside a band, whose 4 m ground is all deep water (below −50 m) are not
diffused (the manifest's `deep_sea_bands` and `windows`). `--guidance-classes` limits
`--guidance` to windows holding those classes, as in the chain.
The output adds `water-surface.f32`, `water-body.u32`, `bodies.json` and the world's 128 m
`moisture.f32`; band reports stay in `hydrology/`. Rerunning the same command resumes after
the last finished band (diffusion or hydrology), and after a failure while finishing.

The diffused surface hydrology reads, `elevation-raw.f32` (4 bytes a cell, 17 GB at 64 km),
is kept: the manifest records it as `hydrology_input` with its digest. So hydrology can be
rerun after a hydrology fix without diffusion and without the GPU:

```sh
systemd-run --user --scope -q -p MemoryMax=6G \
    $P -m topography_training.diffusion.cascade_l4 --out data/worlds/fujix-55-l4 --hydrology-only
```

A band's Rust hydrology peaked at about 2.8 GB at 64 km; the cap keeps a runaway band from
taking the session down. It reruns every band's hydrology from the kept raw surface, north
to south, rewrites `elevation.f32`, the water layers, `bodies.json`, the pictures and the
manifest. It works on a finished world, on one whose seam audit failed and on one whose
inline hydrology stopped at a failed band. It opens the raw surface read-only and checks it
against the digest the manifest (`manifest.json`, else `manifest-failed.json`, or the first
rerun) recorded, and keeps the diffusion's `peak_vram_gb`, `deep_sea_bands` and `windows`.

Both the full command and the rerun pin the hydrology binary for the whole run: they copy
`TOPOGRAPHY_BIN` (if set, else the release build, brought up to date first) to
`hydrology/bin/topography-<sha12>` in the world and run that copy, so a later `cargo build`
cannot change the remaining bands. The manifest records it as `hydrology_binary` (with
`mixed`, every binary used, when a resumed full run changed it). Unset `TOPOGRAPHY_BIN` for a
rerun after a hydrology fix, or point it at the fixed build: an old pin left exported reruns
the old code. A rerun stopped part-way (no failed band, Python code unchanged) resumes with
the binary it pinned when `TOPOGRAPHY_BIN` is unset, so building a later checkout does not
restart it; to apply a hydrology fix there, set `TOPOGRAPHY_BIN` to the fixed build or pass
`--restart`. Earlier pins are removed only once the rerun has decided to resume or restart and
its raw surface checks out.

`progress.json` keeps the rerun's state under `rerun`, with the binary's digest and a digest
of the Python hydrology code (`hydrology.py` and the band step): the same command resumes
after the last band it finished while both are unchanged, and starts again from band 0 when
either changed (a code fix), or with
`--restart`. A band that fails its checks does not stop the rerun: its grid and north band
are copied to `hydrology/failed/band-NNN/` as a repro, the next band takes its output as the
north, and the rerun exits 1 after the last band listing every failed band without finishing,
so one pass finds every failing band. The next rerun starts again from band 0. A failure
while finishing, the seam audit included, only finishes again (an audit fix needs no bands). So after a
hydrology fix, rerun the same command. While a rerun is in progress or failed, the full
command refuses to run, so it cannot finish over half-rerun bands. The manifest is moved
aside while the rerun goes, so a half-rerun world does not look finished; its `game` entry is
kept and marked stale, since the game layers follow the ground: run `game_layers` again
afterwards. `--drop-raw` removes the raw surface when a run (or a rerun) finishes; a world
without it needs diffusion again to change its hydrology.

Each band's hydrology fails (and stops hydrology, while diffusion goes on, for any error)
on a step or slot at its seam with the band above or on ground cut deeper than allowed
([Bands and seams](#bands-and-seams---north)). After the last band, a seam audit checks the
whole world with the same rule: at every band boundary, with d the change hydrology made
(what shows, the water surface where wet, else the ground, less the raw surface; a lake's or
the sea's depth is no change of the land, so their ground counts there), a column
fails where d in one of the first four rows of the band differs from d in the last row above
by more than 5 m plus the 35° side slope's rise per further row (except where both cells are
water of one body), or where what shows less the raw surface (the water surface where wet,
lakes and sea too) lies 5 m below both neighbours (north and south, or west and east) in those
rows (a slot), as the band checks count them. A world without its raw surface falls back to steps of what
shows over 5 m and 4 times the rows beside them. The audit writes
`hydrology/seam-audit.json` and records the totals in the manifest (`seam_audit`). A world
that fails gets `manifest-failed.json` instead of `manifest.json`, so export and resumes see
it unfinished; fix hydrology and rerun `--hydrology-only`. The audit also runs on its own
(exit status 1 on a failing column):

```sh
$P -m topography_training.diffusion.cascade_l4 --out data/worlds/fujix-55-l4 --seam-audit
```

On the first fujix-55-l4 world (before the seam fixes) it counted 8,260 columns at 28 of the
63 boundaries, up to 206 m (at row 31,744).
`--km N` limits the run to a central N × 1,024 m square for quick checks; worlds without a
`hydrology/4m` output (made before hydrology v2) must be regenerated, and older chain
worlds need `--class-id`. `--class-runs` mixes per-class expert runs over `--run`. At
12 steps and stride 192, a central 4 km takes about 14 min.

Memory and disk: output layers are memory-mapped and the 4 m world is read band by band, so
host memory depends on the band width and height. Measured at 64 km on CPU with the sampler
replaced by five band-size buffers (its working arrays): a 1,024 m band holds about 5.5 GB in
Python besides those buffers, the run peaked at 9.4 GB (page cache included), and a band's
hydrology takes 2.8 GB in Rust while Python holds 1.6 GB. So run it in a 12 GB cap; 2,048 m
bands roughly double the Python share. The
output takes 15 bytes a 1 m cell (elevation, water surface and body, and three masks: 64 GB
at 64 km), plus 4 bytes a cell for the kept `elevation-raw.f32` (17 GB) and two
band directories (about 4 GB); `cascade_l4` checks the free space before it starts. L4 still
writes the 1 m binary `water.u8`, `snow-ice.u8` and `vegetation.u8` (12.9 GB at 64 km), which
the export ignores; dropping them is left to sub-project 5. The
chain needs about 44 bytes a 4 m cell while it runs (12 GB at 64 km). Its L3 bands hold
about 4.6 GB at 64 km width (8,192 m bands); the 4 m hydrology at 64 km takes 5.5 GB in Rust
while Python holds about 2.5 GB, so a whole-map chain peaks at about 8 GB (measured in a
10 GB cap).

Then add the game layers the export needs (the chain still draws its 4 m cover internally;
a chain world takes the same command before a 4 m export):

```sh
$P -m topography_training.diffusion.game_layers --world data/worlds/fujion-v3-chain-14-l4 --layout data/worlds/fujion-painted-l0-5
```

`game_layers` writes `game/` at 2 m (at the world's spacing when coarser): vegetation, snow
and talus densities (0 to 1) and dithered biome and land cover, and records them in the
world manifest's `game` entry; rerunning replaces them. `--layout` defaults to the layout
the world records. The world's place on the layout comes from its `world-input/world.json`
origin, so off-centre squares (`--centre`, `--l3-centre`) get their own cover. It refuses
worlds without a `moisture` entry or that leave the layout. `--rows` (default 256)
bounds memory, about 3.7 GB at 64 km (extrapolated); `--png out.png` also writes area-mean
previews (snow, vegetation, talus as RGB, and `out-landcover.png`). On the 8 km chain test
and the 4 km L4 test it took 10 to 12 s and 1.3 GB each.

- The cover reads the layout's forest share and snowline bilinearly and its class map by
  nearest cell, at positions warped by about 150 m, so cover has no 128 m grid steps. Snow
  and tree lines follow the ground: snow lies lower on pole-facing slopes (`hemisphere` in
  the paint settings) and in gullies and sheds from faces steeper than 40–55°; trees reach
  higher in gullies and on sun-facing slopes and thin out near their limit and on steep
  ground.
- `vegetation-density` is the layout's forest share times a moisture ramp (`MOIST_DRY` 0.15
  to `MOIST_WET` 0.45), a tree-line fade, a steep-slope fade, (1 − snow) and (1 − wet). The
  cover's `patchy` noise is not folded in.
- `biome` is the layout's terrain class as the cover reads it (warped by about 150 m), each
  class's share smoothed over about 64 m and dithered with blue noise. It does not come
  from the model's `classes.u8`.
- `land-cover` holds ESA WorldCover codes, a recorded judgment call, not observed cover.
  Each rule is a ramp: trees (10) take the top of a shared blue-noise value by
  `vegetation-density` and snow/ice (70) the bottom by `snow-cover`, so the two never
  overlap and keep their exact shares; bare rock (60) ramps in around 35°; above the tree
  line grassland (30) gives way to bare ground over 400 m; dunes, badlands, glacier ground,
  volcanoes and lava are bare from gentle slopes up; other open land takes its class's
  typical cover. Water (80) is a hard rule at a wet share of at least 0.25 (`WATER_WET`),
  not dithered, so 1 m rivers stay whole at 2 m; `snow-cover` is 0 on those texels. The blue-noise tile is 256 texels (512 m
  at 2 m); no lattice shows in the 8 km test pictures.
- Known tuning point: hydrology moisture sits low (Fujion median 0.11 to 0.14,
  `fujix-55-chain` 0.20 over the whole map, about 40% of it below 0.15), so forest below the
  tree line may be thin. Lower `MOIST_DRY` if the owner wants more trees.

### Generating fujix from the collage

`cascade_chain --collage` starts from a [collage](#collage-layouts) instead of the L1 and L2
models: L2 is the collage's 32 m `l2` (seabed included), L1 its `l1`, and the classes are the
dithered `class32.u8` (class 19 on the canyon plateau), read nearest at 4 m for L3 and
written as the world's `classes.u8` and `biome.u8`, so L4 gets them too. Wind, snowline,
treeline and forest come from the collage's `layout.json`. Then the usual 32 m hydrology,
banded L3, 4 m hydrology and world output follow. The world also holds `description.json`
and `description.md`: for a whole-map L3 the collage's description measured on the carved
4 m surface (level `l3`: regions, passes and their bands, summit and caldera); for an L3
square, measured on the carved 32 m surface (level `l2`) plus `finer`, pass ground and
feature values on the square (`partial` where a feature reaches past it). `cascade_l4`
copies it. `--l3-km` with `--l3-centre X,Y` (map metres from the north-west corner, y south)
puts the L3 square anywhere; `cascade_l4 --km` with `--centre X,Y` does the same for L4,
whose default is the 4 m world's centre. Windows (and L4 bands) of deep sea only are not
diffused at L3 or L4.

Full run (not yet started; it needs the owner's go). Class guidance 2 on the Grand Canyon
windows only:

```sh
CAP=(systemd-run --user --scope -q -p MemoryMax=12G)
"${CAP[@]}" $P -m topography_training.diffusion.cascade_chain --collage data/worlds/fujix-collage-55 \
    --l3-run data/models/cascade-l3-run-5 --l3-class-runs data/models/cascade-l3-run-5-experts \
    --guidance 2 --guidance-classes 19 --out data/worlds/fujix-55-chain
"${CAP[@]}" $P -m topography_training.diffusion.cascade_l4 --run data/models/cascade-l4-run-8 \
    --class-runs data/models/cascade-l4-run-8-experts --guidance 2 --guidance-classes 19 \
    --world data/worlds/fujix-55-chain --out data/worlds/fujix-55-l4
```

Either command, rerun unchanged after a crash or kill, resumes where it stopped. Run them as
background jobs with a log and an exit-code file ([AGENTS.md](../../AGENTS.md)).

Checks (2026-10-28; RAM is the cgroup's anonymous memory plus swap, as page cache from the
memory-mapped layers fills any cap; VRAM is torch's peak reservation):

| Run | Time | RAM | VRAM |
| --- | --- | --- | --- |
| L3 8 km (2,048²), volcano, experts (Task 1, one field) | 233 s, hydrology 7 + 6 s | 2.3 GB | 2.8 GB |
| L3 8 km, canyon, experts / base (Task 1) | 298 s / 218 s | 2.6 / 2.1 GB | 3.8 / 4.2 GB |
| L3 16 km (4,096²), centre, experts, two 8 km bands | 1,455 s (907 s, 548 s), 4 m hydrology 26 s | 2.6 GB | 6.2 GB system |
| The same as one band | 1,246 s | 2.8 GB | 6.1 GB system |
| L3 whole map, full width, bands 2 and 3 of 8 diffused | 2,819 s and 3,080 s a band; others 90-140 s | 4.6 GB | 6.0 GB system |
| The same: 16,384² 4 m hydrology, layers, 4 m description (resumed, 10 GB cap) | 397 s hydrology, 737 s in all | 8.1 GB | |
| L3 8 km volcano, guidance 1 / 2 | 278 s / 435 s | 2.4 / 2.3 GB | |
| L4 4 km, four 1,024 m bands, experts (Task 1) | 14 min (bands 194-223 s, hydrology 6-7 s) | 3.6 GB | 5.8 GB |
| L3 4 km canyon, guidance 1 / 2 on class 19 | 68 s / 114 s | | |
| L4 4 km canyon on each, guidance 1 / 2 on class 19 | 15 / 27 min (bands 195-238 s / 374-426 s) | 2.2 GB | 7.8 GB system |
| L4 2 km on the class-19 zone's edge, guidance 1 / 2 (93 of 154 windows guided) | 6 / 16 min | | |

Every hydrology report and band seam passed (after the fix of a cell widened out of a sink
basin without a receiver, found by the whole-map run). Two bands against one differ by 3 mm
on average over the 16 km square, the same near the seam as elsewhere. Guidance, compared on
one world at the same place and seed (`--guidance 2 --guidance-classes 19` at L3 and then
L4, against guidance 1 at both): on the canyon (36800,14000) the walls get continuous strata
ledges at 1 m where guidance 1 gives broken ones (1.1 m standard deviation, slopes alike;
`scratch/fujix-run/t2/canyon-l4-detail-g1-vs-g2c19.png`). Across the edge of the dithered
class-19 zone (a 2 km L4 at 32000,26000 with 93 of 154 windows guided) the difference fades
from 0.14 m inside to 0.05 m outside, which is the run-to-run noise, with no seam
(`edge-l4-g1-vs-g2c19.png`); on the volcano, guidance 2 everywhere changed nothing visible
(0.2 m at the summit). A guided window evaluates the model twice: about twice the time in
a class-19 window at L4. The first band of a run with class-limited guidance also pays a
one-off kernel search for the uneven batch sizes (several minutes on this GPU).

Full 64 km run. From the collage's 32 m surface: L3 has 18,796 windows in 8,192 m bands
(16,129 as one field), 6,522 of them deep sea and skipped, 2,288 holding class 19; L4 has
173,910 windows, 72,597 skipped and 16,136 holding class 19. A window took 1.0 to 1.4 s
(L3 without class 19), about 2.1 s (L3 with the class-19 expert) and about 1.25 s (L4),
and about 1.0 s (L3) and 1.3 s (L4) more where guided. So:

- L3: about 4 to 5.5 h (5 to 6 h with guidance), plus about 15 min of band preparation,
  7 min of 4 m hydrology and 5 min of layers.
- L4: about 35 h of diffusion (41 h with guidance) and 2.6 h of band hydrology: about
  38 h (44 h with guidance), against 55 h before the window skip.
- RAM: L3 peaks at about 8 GB (bands about 4.6 GB; the 4 m hydrology 5.5 GB in Rust while
  Python holds about 2.5 GB); L4 about 9.4 GB (earlier 64 km band measurement, 12 GB cap).
  Run both in a 12 GB cap.
- Disk: about 12 GB for the chain (44 bytes a 4 m cell while it runs), 82 GB for L4.

## Hydrology v2

Water as a surface height ([Fujix plan](../plans/fujix-v1.md) section 2), computed in Rust
(`world::hydrology` in `topography-core`) one grid at a time: orographic rain, lakes,
breached depressions, stream power at 32 m and discharge-sized channels. A finer level holds
the coarser level's network fixed (`--coarse`), and 1 m bands take the previous band's rows
(`--north`). The cascade calls it at 32 m, 4 m and in every 1 m band
([Generating a world](#generating-a-world)); `scripts/hydrology_grid.py` makes grids to run
it by hand:

```sh
training/.venv/bin/python -I scripts/hydrology_grid.py data/worlds/fujix-collage-55 <grid-dir>
target/release/topography world hydrology --grid <grid-dir> --out <new-out-dir> [--wind-deg 225]
target/release/topography world hydrology --grid <4m-grid> --coarse <32m-out> --out <4m-out>
target/release/topography world hydrology --grid <band-k> --coarse <4m-out> --north <band-k-1-out> --out <band-k-out>
target/release/topography world hydrology-seam --north <band-k-1-out> --south <band-k-out>
```

`hydrology_grid.py` takes a collage (`arrays.npz` `l2`, `class32.u8`, the layout's wind), or
with `--chain-l2`, `--world` or `--band` a chain world's 32 m level, a world as it is, or rows
of a 1 m world.

### Grids and outputs

A grid directory holds `grid.json`, `elevation.f32` and optional `classes.u8` (little-endian,
rows north to south). `grid.json` gives `size` [w, h], `spacing_m`, `origin` [x, y] (the
world's north-west corner in the layout plane, x east, y north; default [0, h · spacing]),
and optionally `offset_m` [east, south] (a band or window inside that world), `core_rows`
[start, end) (the rows a band delivers; the rest is halo), `classes_cell_m` and
`wind_from_deg`. The output directory must not exist; it is staged and renamed into place:

| File | Content |
| --- | --- |
| `grid.json` | the input plus `level`, `wind_from_deg`, `discharge`, `moisture` and `flow` entries and provenance (input and output SHA-256s, the coarse and previous band's files, water table, tool version) |
| `elevation.f32` | ground after breaching, stream power and channel carving |
| `water-surface.f32` | water surface in metres, NaN where dry, sea 0 |
| `water-body.u32` | 0 dry, 1 sea, then lakes and rivers; coarse ids kept at finer levels |
| `discharge.f32` | drainage area weighted by rain, km² at mean rain |
| `moisture.f32` | 128 m grid, rain over its 95th percentile on land, 0 to 1 |
| `flow.u8` | low 4 bits the D8 receiver (1 N, 2 S, 3 W, 4 E, 5 NW, 6 NE, 7 SW, 8 SE; 0 none), 16 centreline, 32 dry basin, 64 endorheic |
| `bodies.json` | sea, lakes (`level_m`, `exit_cell`, `parent`), rivers (`coarse`, `mouth_cell`, `drainage_km2`, `length_km`, `parent`) and dry basins (`endorheic`, `ice`; not in the raster); `next_id`, the first id the next band or level may use |
| `report.json` | checks, figures and `passed`; exit status 1 when a check fails (outputs are still written) |

`--river-km2` defaults by spacing: 10 km² at 16 m and coarser, 0.5 km² from 2 m, 0.05 km²
below. Water amounts per class come from `configs/hydrology/water-v1.toml` (built into the
binary; `--water FILE` replaces it): `lake_share`, `tarn_share` and `dry_share` of each class's
land. Timings and peak memory go to stderr only, so identical inputs give identical outputs.

### The trunk level (32 m)

- **Rain**: Smith and Barstad (2004) linear upslope model on a 128 m grid (FFT, padded with
  0 m ground), clipped at 0 plus a background rate, then depleted along the wind from the
  upwind edge (exp(−rained / budget)), which gives wet windward coasts and a dry far lee.
  The wind is `--wind-deg` (degrees the wind comes from, clockwise from north), else
  `wind_from_deg` in grid.json; without either, rain is uniform and moisture 0.5. Judgment
  constants (`moisture::PARAMS`, in `report.json`): 10 m/s wind, N 0.005 s⁻¹, vapour scale
  height 2,500 m, condensation and fallout 300 s, background 1.5 mm/h, 3 mm column budget.
- **Sea** is ground below 0 m connected 8-way to the grid edge. A priority flood (Barnes
  et al. 2014) finds the depressions.
- **Lakes**: within each class's `lake_share`, the largest depressions by volume (at least
  1 m deep and 4,096 m²). Glacier depressions (class 10) stay dry ice basins. **Dry basins**:
  within `dry_share`, the deepest compact depressions of dry classes. Every other depression
  is **breached** (Soille 2004 carving) by at most 30 m; deeper ones become lakes at the spill
  minus 30 m, or dry sinks in glacier and arid classes; ground below 0 m makes lagoons at 0 m.
- **Trench widening** cuts breach slots back to a 35° side slope, ring by ring, never below
  the trench, 0 m or a kept basin level. **Stream power** (32 m and coarser): five implicit
  iterations (Braun and Willett 2013), K·dt = 7 × 10⁻⁴, A^0.5; basins, glacier and ground
  draining onto it untouched.
- **Rivers** start at `--river-km2` of discharge, one id per river system. **Channels**: width
  2.5 m · Q^0.5, depth 0.5 m · Q^0.4 (Q in km² at mean rain), a parabolic section; the
  surface never rises downstream and stands at most at every dry bank beside or behind a
  channel cell (ground ahead along the flow is the next section's bank, a ruling: figure
  `river_ahead_exemptions`; not in the core's last four rows, which the next band takes, so it
  gets water and banks that agree). Lake exits hold the lake level (a sill), rivers never stand below
  an adjacent lake or the water they flow into, mouths ramp into the seabed, carved channels
  deeper than a cell's rise widen like trenches, and holes (dry cells over 0.5 m below every
  adjacent water) are raised.
- **Receivers** follow the fall line: of the two neighbours bracketing a cell's gradient
  (Horn's 3 × 3 slope) it takes one with a share equal to the angle between them, against a
  low-discrepancy threshold of its world column and row, so streams on smooth slopes follow
  the slope instead of grid lines, and overlapping bands route shared rows alike. Where that
  neighbour is not lower it takes the steepest descent; flats keep the flood order. A flood
  step left uphill (a pit lowered later, such as a trench end) can close a receiver cycle:
  its cells get their flood receivers back, and any cycle left spills like a depression
  (`receivers_spilled`).

### Finer levels (`--coarse`)

The finer grid must lie inside the coarse output, aligned to its cells. Moisture is the
coarse one (bilinear); `--wind-deg` is refused. The sea also starts from coarse sea cells; a
window edge is an outlet only at the world's edge or where the coarse flow leaves.

- **Coarse lakes** keep their id and level. Wet is fine ground below the level connected to
  the coarse lake's cells; no ground is dug out. A branch that leaks (grows more than a few
  coarse cells, or reaches the sea, an edge or a lower lake) is cut by a sill at the level;
  dry ground beside a lake below its level is raised (a shore). A lake cell beside the
  previous band's ground or water lower than the level leaks too (the water would run back
  across the seam). A lake of the previous band
  that the coarse level lacks (one made at this level, such as a river sink filled to its
  spill) goes on across the seam the same way, from that band's cells at its level.
- **Coarse rivers** keep their id and get a fine centreline: a priority flood over a corridor
  around the river's coarse cells starts where the coarse river ends and climbs upstream; each
  coarse head follows it down (a head whose fine cells are all sea starts at the first coarse
  cell down the river with land). Paths cross dry basin floors, a path ending in a coarse dry basin
  with no fine hollow continues down the routing, and the path is burned below a cap from the
  coarse surfaces. Its discharge is the coarse one. The corridor is the river's coarse cells
  and one ring around them. A path stuck on its edge (free ground just outside more than
  0.5 m lower: the finer valley floor lies beyond) would run along coarse cell lines and be
  burned there into a stair-stepped trench, so the coarse cell of that lower ground joins
  the corridor (with the order of the cell beside it) and the flood is made again, up to
  three times. Paths that never touch the edge are laid as before. Beyond the coarse
  river's 3×3 cells a path has no cap and only keeps descending. A path still stuck after
  three widenings is kept and counted: `coarse.rivers` reports `corridor_widened` (rivers
  by times widened), `rivers_widened` and their ids `widened_rivers`, `edge_cells`, `edge_runs_48m` (straight row or
  column runs of stuck cells of 48 m or more) and `edge_run_max_m` / `edge_run_at_cell`.
- **Depressions**: tarns at 4 m within `tarn_share`; at 1 m no new lakes and no new dry basins
  (a depression touching a coarse dry basin stays one, glacier depressions stay ice basins).
- **Tributaries** start at the finer `--river-km2`, with ids above every coarse id.
- Extra checks (all 0): `coarse_lakes_missing`, `coarse_rivers_missing_or_unrouted`,
  `water_above_coarse_surface` (a coarse river's fine water more than 1 cm above its coarse
  surface), `coarse_lake_leaks_left` and `guard_coarse_lake_levels_changed`.

### Bands and seams (`--north`)

A band's grid holds its core rows and a halo above and below; its top rows must lie in the
previous band's core, and its core starts where that core ends. Those rows are taken as they
are, so adjacent bands agree on the seam by construction. A taken cell draining into this
band feeds it; a taken river entering continues with its id. A taken cell whose flow there
comes back into this band is no outlet and no river end (a loop across the seam). A stream
draining back onto taken dry ground with the taken river up to three cells away is led along
the first rows into it, never climbing (`streams_joined_across_seam`); a new stream that would
end on taken dry ground, or join the taken river beside lower taken ground, is dropped
(`streams_pruned_at_seam`), and this band's dry cells beside taken rivers are raised to their
surface. `rivers_cut_at_seam` must be 0, and the cycle guard also counts the routing stitched
with the previous band's.

The seam: a band's first 64 rows (`SEAM_BLEND_ROWS`) start from a blend of the
previous band's ground there (its halo, continuous with the taken rows) into this band's own,
so the seam starts without a step in either direction; a cut that band made in its halo (a
channel its hydrology guessed there) comes in only down to the seam limit below. On finer
levels no cut goes deeper than
`BREACH_MAX_M` (30 m) below that starting ground, and near the seam no cut goes below the
previous band's dry ground in its last row less the 35° side slope over the distance (down to a
taken river's or lake's bed beside it, and right beside a taken lake or the sea down to its
level, such as a lake's exit notched to it), and in the first four rows after the seam never
below what the seam step rule (below) allows: the carve floor. Breaching keeps a depression it
cannot drain within that as a sink and fills a pocket instead (one whose fill would fail the
seam step rule below stays a sink); trench walls, seam joins and
channel beds stop there. Where a river's water would stand below the floor (a low bank
downstream, a route over a rise), its bed stays on the floor and the water is raised to stand
on it, upstream too (a backwater), and the dry banks beside are raised to the water (on a
river over a high rise this fills the valley behind it); not above the previous band's dry
ground beside it (not ground ahead along its flow, as for banks), a coarse path's surface cap,
or, in the first four rows after the seam, the height the seam step rule (below) allows for its
channel cells and any dry cell beside them, where the bed goes just below the water instead (bounded by `river_beds_below_carve_floor`). A channel cell
beside the centreline whose bed cannot reach below the water stays dry. A river
entering from the previous band that must be rerouted takes only a way whose cells can be
lowered to its water level within their floor (it never climbs); with no such way it ends at
its entry in a sink (`inflows_ended_in_a_sink`). Seam joins that would climb are not made.
Coarse river paths may cut deeper (the coarse
network sets where they run) but keep the seam limit, filling up to it where needed
(`path_fill_*`), and keep four cells from taken dry ground below their water when a way that
climbs no more allows. A coarse path that climbs more than 5 m above its lowest point upstream,
or runs along the first 16 rows more than 16 columns from where its river crosses the seam, is
laid again along the way to its end that climbs least, keeping off the seam there
(`paths_relaid`; along the seam any such way is taken, since a climb there could only be
cut below the floor). A coarse path whose ground in the core rises more than the 30 m carve
depth above its lowest point upstream (the halo's rows are laid again by the next band) ends
before that rise, flowing into that lowest cell; where the water there finds no way down but
back into the path, that cell is a river sink, which the river-sink rules fill to a lake at its
spill or keep dry (`paths_ended_before_a_deep_rise`, `path_hollows_ended_in_a_sink`). A path
whose head is that lowest point is left out (`paths_dropped_before_a_deep_rise`). Cells of a
path away from its coarse river get no surface cap (the coarse
surface says nothing of the ground there), so the burn only keeps them descending. A coarse
head the corridor flood misses (its river crosses the seam where the previous band laid it,
off the coarse cells) takes the way that climbs least to that band's water of its own river
before any lowest way to other water. A head whose coarse river comes from the previous band,
which did not hand it over, is no head while the river has another, and a path laid again off
the seam may leave the seam strip near its head as near its end. Other
river routes whose water would have to climb past what the floor lets carving lower turn to
their steepest other way down (`river_climbs_turned`). With no lower neighbour (a flat the
breach filled where the floor kept it from cutting), such a river goes on along the way whose
water level stays highest, over cells a channel can be cut in to that level, to where its
routing goes on without climbing, and through any water on without coming back onto the route
(`river_climbs_routed_on`); with no such way it ends there in a sink
(`river_climbs_ended_in_a_sink`). Checks,
with figures under `carving`: `ground_below_carve_floor` (dry ground below the floor),
`river_beds_below_carve_floor` (river cells more than 5 m below it, coarse paths included,
so a path burn deeper than about 35 m fails), `ground_raised_over_limit` (ground raised more
than 60 m above the input by any fill), `seam_steps` (columns where the change hydrology made,
shown height less the input, differs between the taken last row and one of the first four
rows by more than 5 m plus the side slope's rise per further row, except where both cells are
water of one body; a lake's or the sea's depth is no change of the land, so their ground counts
there) and `seam_slots` (cells in those rows whose change lies 5 m below both
neighbours, north and south or west and east). `slots` counts such cells over the whole band
as a figure.

On finer levels a sink that breaching left (not a coarse dry basin) that a river runs into
fills to its spill as a lake draining over it, so a river does not end in a dry hollow (such
as one below sea level behind a lagoon's rim, or one the seam floor kept from being breached).
Neither way does it fill above a coarse river path beside it whose water is held lower (by its
surface cap or the previous band's dry ground beside it, there or upstream): no lake grows over
a path cell, so the lake would stand above the river. Nor where its outflow runs into, or the lake lies beside, a coarse path whose water is held
above the level (on its bed floors or carve floor, or at the previous band's water it ends in).
Nor where the outflow, followed down its routing to other water or an end, crosses a carve floor
above the level: the river's water stands on that floor and would back up over the exit.
A sink whose cells do not all drain to that spill (a pocket below 0 m left inside a breached
depression, or a basin trench widening split), or that reaches the first three rows after the
seam, fills to its own lowest spill instead: a priority flood from its lowest cell takes ground
in rising order, and lower ground draining back in, until lower ground drains away to water, a
dry pass-through basin or an edge. Other hollows breaching left, and a lake breaching left that
drains in, fill with it; a lake beside it at or above the level takes it in; the previous
band's cells and coarse river paths are walls (a path below the level may take the spill).
The cells below that level become lakes at it, draining over the spill, and sink cells above it
dry ground draining onto them (`river_sinks_filled_to_own_spill`, with the reasons sinks were
left in `left_by_reason`). This runs again, up to four passes, while a lake made this way sends
a river on into a further sink. It stays a sink where that is not clean: the flood meets a
kept sink or other water draining in (such as a coarse lagoon whose outflow ends there) or the
grid edge; ground draining away, the previous band's included, or water lies below the level
beside the lake, or that band's ground beside its outflow (over the dry region at the level,
then the river's water while in the rows the seam checks; that ground cannot be raised; a lake
beside lower water would be a step in the surface), or that band's water its outflow runs into
stands above the level (the outflow would climb off it). A sink a climbing river ends in has no spill and
stays one. The figure `rivers_ending_in_a_dry_sink` counts the river ends still left in such
sinks, with examples (`at_cells`), apart from those into kept dry basins (endorheic by design).

`hydrology-seam` compares two band outputs and exits 1 on a failure: the shared rows must
match (ground and surface within 1 mm, body ids equal), and across the seam no centreline may
flow onto dry ground that is not a basin, no dry cell may sit more than 1 cm below an adjacent
lake or the sea, nor below the northern band's river beside it, except on that river's cascade
side (ahead along its flow where its water drops, judged by the northern band's own water or
ground there, its halo for a southern cell) down to the seam step rule's limit (the southern
band's own check uses that limit; `hydrology-seam`, without the input, allows 5 m there),
and a lake keeps one level.

Near the seam (judgment rules): a river joining the previous band's water in the first four rows
may stand up to `SEAM_JOIN_STEP_M` (1 m) below it (channel cells of that band's river hold
it too) where a taken bank, the seam step rule over the cell or a dry cell beside it, or a coarse
path's surface cap holds it down (also into a join cell of this band that joins it); a coarse path ending in a cell of that water that
cascades more than that onto a lower cell beside it goes on one free cell to the lower one (figure
`seam_join_steps`, count and largest; more fails `river_cells_rising_downstream`). Dry cells
beside the previous band's river are raised to its water, except on its cascade side, where they
rise only as far as the seam step rule allows. Carving in the core's last four rows (which the
next band takes) keeps the 35° side slope below the next band's first row, as that band's first
rows do, river beds included (rivers cross the seam no deeper; water and banks are raised as
needed); beside water in that row (a lake grown there, a coarse path at its cap) the limit
goes down to it plus the slope, and coarse river paths keep the coarse network's level there. The seam blend and the carve floor in the first four
rows never go below what the seam step rule allows. A lake exits over its main outflow (most flow; a coarse lake's dry main exit above its level is
notched to it by up to 0.5 m, or further where the carve floor allows); among
exits carrying that outflow (within 1 %), one where its water can go at its level (water, or
ground not above it) comes first; an exit on a band edge row is not
checked against the level (the lake may go on beyond it).

### Checks

All must be 0: lakes in more than one piece or with ground at or above their level, lake
exits off the level (by more than 1 cm, plus 1 mm of f32 rounding), river cells below an
adjacent lake, wet cells whose surface is not above
the ground, centreline surfaces rising downstream, rivers ending on dry land, dry land
draining uphill, dry cells more than 1 cm below an adjacent lake, sea or river (river banks
as above), dry cells in holes, on finer levels the carving and seam checks (above), and the
`guard_` checks (land not draining to an outlet, including receiver cycles; lakes with
several levels; beds rising downstream, except onto a bed held at its carve floor). Unit tests
(`cargo test -p topography-core hydrology`, run by `scripts/check.sh`) break each check.
`check_examples` gives up to three failing cells per check.

### Figures

| Run | Time | Peak RAM | Notes |
| --- | --- | --- | --- |
| fujix-collage-55, 32 m, 2048² | 4 s | 0.16 GB | 287 lakes, 114 rivers (294 km) |
| Fujion chain world, 4 m, 8192², `--coarse` | 94 s | 1.9 GB | 203 lakes, 461 rivers (616 km) |
| Fujion 1 m band, 32,768 × 1,536 | 45-55 s | 1.0-1.2 GB | about 350 rivers a band |
| fujix 64 km, 4 m, 16,384² `--coarse` | 6.2 min | 5.3 GB | the carved 32 m ground upsampled, with 1 m noise |
| fujix 64 km, 1 m band, 65,536 × 1,536 | 2.4 min | 2.7 GB | one 1,024 m band with 256 m halos |

In the cascade on an 8 km Fujion crop, generating each level from the carved level above
keeps cuts small: at 4 m all steps together lower ground at most 9 m, at 1 m at most 7 m.

## Hydrology v3 (1 m)

Water as one field over the whole 1 m world ([spec](../superpowers/specs/2026-11-05-hydrology-v3-design.md),
[Plan A](../superpowers/plans/2026-11-05-hydrology-v3-a-water-model.md)): every cell has a
ground height, a water surface or none, and a discharge. Lakes and rivers are labels read off
that field. Rust (`world::water` in `topography-core`) runs exact graph passes on tiles with
halos: the priority-flood spill graph, the depression hierarchy and Fill-Spill-Merge, fall-line
receivers, flat ponds and accumulation, then breaches, labels and channel carving. Tiled runs
give the same bytes as one tile, so there are no bands and no seam rules. It replaces the
band-by-band `--hydrology-only` rerun for 1 m worlds; the 32 m and 4 m levels still use v2.

Departure from the spec: the spec's GPU bank relaxation is dropped in Plan A. On an 8192²
probe at the deepest cut (1.33 m) the carved ground alone passed `walls` and `facets`, the
relaxed ground had more `drainage_rises`, and the hillshades looked the same. Plan A ships the
carved ground (`relaxation: none` in the manifest). The relaxation is kept in git at
`d0c0055` (`training/src/topography_training/water/relax.py`) for Plan B, should deeper cuts
need bank shaping.

### Commands

`$P` and `CAP` as at the top of this page. The driver runs the whole chain, resumes when run
again with the same arguments, and pins a copy of `target/release/topography` in the run
directory. A new build starts again from scratch: delete that `bin` directory and the driver
wipes every cache. To keep the stages whose code did not change, see
[Rerunning from a stage](#rerunning-from-a-stage).

```sh
PYTHONPATH=training/src "${CAP[@]}" $P -m topography_training.water.run world \
    --source data/worlds/fujix-55-l4v2 --out <new-world> [--tile 2048] [--threads 4]
PYTHONPATH=training/src "${CAP[@]}" $P -m topography_training.water.run window \
    --source data/worlds/fujix-55-l4v2 --level 1 --centre 27169,44546 --size 2048 --tile 512 --out <dir>
```

`world` needs an L4 world with a kept `elevation-raw.f32` and a `source_world` chain world
whose `hydrology/4m/grid.json` carries the 128 m moisture field; without real rain the run is
refused. It never writes to the source and refuses an `--out` inside it. `window` cuts a test
window: `--level 1` from the L4 world's raw surface, `--level 4` and `--level 32` from the
chain world (`--source data/worlds/fujix-55-chain`; the 4 m input is v2's carved 4 m ground,
so these show the model's behaviour, not a v3 4 m result). Exit status: 0 passed, 1 a check
failed (outputs kept; a world gets `manifest-failed.json`), 2 refused or failed.

The two Rust commands, by hand:

```sh
target/release/topography world water --grid <grid-dir> --work <work-dir> [--tile 4096] [--threads 4] [--water water.toml] [--river-km2 0.05]
target/release/topography world water-finish --grid <grid-dir> --work <work-dir> --ground <work-dir>/carved.f32 \
    --out <new-out-dir> [--tile 4096] [--threads 4] [--spots E,N;E,N]
```

`--spots` (metres; the driver passes the review spots) only feeds the `dam_drops_at_spots`
figure; it is not part of the work stamp.

### Grid and work directories

The grid directory is v2's ([Grids and outputs](#grids-and-outputs)) at 1 m: `elevation.f32`
(the raw surface), `classes.u8` (the chain's 4 m classes, `classes_cell_m` 4), `moisture.f32`
and `grid.json`. `grid.json` must have a `moisture` entry (`file`, `size`, `spacing_m`,
`nw_m`, `scale_mm_h`) and `discharge.mean_land_mm_h`; the driver adds `window` and `inputs`
(class map and moisture provenance).

`world water` writes the work directory stage by stage; each stage's tag digests its inputs
and settings, and `stamp.json` ties the directory to its grid, class map, moisture and binary,
so a rerun resumes and a changed input is refused:

| Stage | Files |
| --- | --- |
| `m1` model on raw | tile caches `m1-<tag>/` |
| `b` breach | `breached.f32` |
| `m2` model on the breached ground | `m2-level.f32`, `m2-node.u32`, `m2-discharge.f32`, `m2-flow.u8` |
| `l` labels (lakes, rivers, river pools) | `body.u32`, `bodies.json` |
| `c` channels | `carved.f32`, `water-surface.f32`, `water-body.u32` |
| `m3` model on the carved ground | `m3-<tag>-*.{f32,u32,u8}` |
| `s` settle | rewrites `water-surface.f32` and `water-body.u32` in place; per-tile changes in `s-<tag>/` |
| figures | `stages.json` (stage tags, breaches, channels, nodes, leaves, lakes, ponds, area and depth histograms) |

`world water-finish` checks the carved ground with the m3 model and publishes `--out`. The m3
tag is keyed on the run's settings and the sha256 of the carved and breached grounds (the only
rasters m3 reads), so a channels rerun that changes surfaces and labels only reuses m3. The
settle tag follows m3's and the channels' tags. `water-finish` recomputes every tag and refuses
a `stages.json` that does not match.

Settle caches each tile's changes before it rewrites the channels' rasters, so a kill between
the two writes resumes cleanly. Settle never runs on surfaces that another settle has already
rewritten. A channels run removes earlier `s-*` directories, and `world water` refuses to
settle again until the channels rerun.

### Outputs

The world's `elevation.f32`, `water-surface.f32`, `water-body.u32`, `bodies.json` and
`water.u8` have v2's names and formats, so `game_layers` and export are unchanged.
`snow-ice.u8`, `vegetation.u8` and `moisture.f32` come from the source and chain worlds.
`hydrology/1m` holds the published water output: those layers plus `discharge.f32` and
`flow.u8` (both from m3, bits as v2), `report.json` and `grid.json` with provenance.
`hydrology/review` holds the review figures (owner spots, longest straight runs, failing-check
clusters, lake-exit sheets, overview) and `coarse-guide.json`, the comparison against the 4 m
v2 network. A window run has the same under `out/` and `review/`.

### Checks

Every count must be 0. D is the water surface where wet, the pond level in ponds (m3), else the
final ground. Thresholds and rulings are in [Plan A](../superpowers/plans/2026-11-05-hydrology-v3-a-water-model.md)
(Task 13 and rulings R7, R11, R14, R16, R27, R29–R31):

| Check | Fails a cell (or lake) when |
| --- | --- |
| `drainage_rises` | a non-sea cell with a receiver (m3) has D(receiver) > D(cell) + 1e-4 m |
| `drainage_stuck` | a dry cell has no receiver, is not on the world edge and is not in a closed basin |
| `drainage_cycles` | the m3 model found cells on cycles |
| `lake_not_flat` | a lake cell's surface differs from its lake's level |
| `lake_off_sill` | a full lake's lowest saddle on the final surfaces is more than 0.01 m off its level |
| `lake_leaks` | a non-lake neighbour of a lake cell, other than its outlet, has D more than 0.01 m below the level |
| `lake_exit_surface` | the first cell of a full lake's outflow is more than 0.01 m off the level |
| `lake_balance` | a closed lake covers fewer cells than its supply needs |
| `dam_steps` | within 50 m below a lake exit, D drops more than 1.5 m between adjacent path cells and raw does not drop as much |
| `dam_rims` | a guard: an exit-side dry cell stands over level + 0.5 m and raw did not (v3 never raises ground) |
| `bed_rises` | on a dry m2 thalweg link where the ground before channels falls, the carved bed rises |
| `thalweg_beside` | beside a thalweg with Q ≥ 1 km², the lowest standing cell across the flow (to the first ridge 0.5 m up, ±30 m) is more than 1 m below this river, belongs to no tributary, and carries a second channel (see below) |
| `parallel_rivers` | figure (owner ruling 2026-11-06: raw-terrain straight/parallel rivers accepted), `figures.parallel_rivers_count`: along a river with Q ≥ 1 km², another flow line with Q ≥ 0.05 km² runs within 10 m and parallel (cosine ≥ 0.7) for over 100 m without joining it within 100 m |
| `straight_share` | figure (owner ruling 2026-11-06: raw-terrain straight/parallel rivers accepted), `figures.straight_share_count`: more than 5 % of river length lies in straight runs over 60 m (global, 0 or 1; the share itself is `figures.straight_share`) |
| `straight_runs` | figure (owner ruling 2026-11-06: raw-terrain straight/parallel rivers accepted), `figures.straight_runs_count`: a river thalweg stays within 1 m of its chord (any angle) for more than 150 m, unless the raw thalweg is straight there too (90 % of cells) |
| `facets` | more planar 5×5 windows (residual < 2 cm, slope > 5°) among changed cells than raw had (global) |
| `walls` | a changed cell's Horn slope exceeds max(40°, its raw slope) + 0.01° |
| `change_p99` | p99 of \|Δ\| over all cells > 1 m (global) |
| `change_max` | a cell's \|Δ\| > the breach cap + 0.5 m (2.5 m at 1 m) |
| `label_agreement` | the written rivers' thalwegs (m2) and m3's agree less than 0.95 within one cell (global) |
| `export_bodies` | lakes + rivers > 50,000 (global) |
| `wet_not_above_ground` | a labelled cell's surface (a lake's or the sea's `bodies.json` level) is not strictly above the final ground |
| `wet_without_surface` | a labelled cell has no surface |
| `surface_without_body` | a cell has a surface but no label |
| `lake_level_json` | a lake or sea cell's surface differs from its `bodies.json` `level_m` (as f32) |
| `wet_unknown_body` | a labelled cell's body is not a sea, lake or river of `bodies.json` (id 1, the sea, always counts) |

The last five are the heightmap export's water contract (`export/water.rs`), so a world that
passes `water-finish` is not refused by the export. Rivers over a bed left uncut (steep raw
reaches, rim floors, pond spill cells) run 0.005 m deep: half the checks' 1 cm tolerance, so
f32 rounding cannot carry such water past a lake exit's or a dam step's 1 cm (at 0.01 m it did,
by one ulp). Lake and sea cells whose ground reaches their level, and a lake's exit cell at the
level, are dry shore. `water-finish` writes lake levels to `bodies.json` as the exact f32 the
rasters hold.

`thalweg_beside` ruling (departs from the spec's "lowest raw ground"): the lower cell counts as
a second thalweg only where it, or a cell within 2 cells, carries a channel (m3 Q at the river
threshold, 0.05 km² at 1 m) that is not this river's own; dry lower ground across a valley
floor or fan does not fail. An unlabelled m3 thalweg cell (a short detour beside a pond) is
judged as the labelled water its path reaches within 150 m. The cost: a river perched above a
dry valley floor passes. Gate 1 rulings: the channel cell near the lower cell is this river's
own when it is water this river joins within 150 m or a thalweg joining this river within
150 m. Fix round 2: written water of another body (a lower river or lake) is no second
channel; only an unwritten m3 channel near the lower cell counts. The cost: a bench river
beside a lower written river passes.

Settle (`s`) lifts written water to the m3 routing (a river or pool cell inside a higher m3
pond stands at its level; a dry cell draining into higher labelled water is wetted) but leaves
labelled lakes at their label's level and extent. The figure `closed_lake_m3_level_gap` is
the largest |m3 level − label level| over closed lakes' cells; `stages.settled` names the
largest lift's cell and body. The channel restore rounds stop at a hard cap of 128 (31 seen at gate 1); a core
cell still changing after that is an error.

Carving keeps m2 ponds (fix round 2): a cut beside another m2 node (lakes included) stops
0.01 m above that node's level (gate 2 fix round 1: cut to the level itself it became a new
spill), and a cut on a node's own cell standing at its level (its spill cell) stops at that
level; ground below that floor is left as it is. A river cell cut below a lower lake beside it
stands at the lake's level, keeping its body. The figure `m2_pond_cells_lost` counts m2 node
cells in no m3 node and names the largest m2 level − m3 level (ground where dry) with its m2
node and cell. m3 keeps an m2 receiver that is not higher on the carved ground (ties
included), so a bed carved flat does not send the river off its labelled line.

Breach (gate 2 fix round 1): cells beside the gorge's path whose raw slope is at least 39° are
not cut (path cells always are); each path cell's taper reaches max(half-width + 1,
2·cut / tan 35°) cells; and a gorge that would still leave a changed cell steeper than
max(40°, raw) is refused, so the depression stays filled. `stages.breach.breaches_rejected_walls`
counts the refused gorges and lists the 20 deepest (node, cut, spill cell, filled depth).

Two more figures: `dam_drops_at_spots` lists, per review spot, the exit-walk drops over 1.5 m
within 100 m with their raw drop (count and largest 10), and `unlabelled_main_river_cells`
counts dry m3 thalweg cells in no node with Q ≥ 1 km², plus runs of 10 or more such cells at
the river threshold.

### Settings, resuming and cost

`configs/hydrology/water-v2.toml` (built in; `--water` replaces it) gives each class's
lake-to-catchment ratio `r`, the game setting `lake_min_m2` (smaller standing water is an
unlabelled pond), the river-width factor `m3s_per_km2` and per level the river start and
breach cap.

Kill and rerun the same command to resume: finished stages and tiles are kept. Run under a
memory cap (`MemoryMax=10G` for 16,384² windows and the world). The work directory takes
about 49 bytes per cell.

Measured at gates 1 and 2 (2026-11-06, fujix-55-l4v2, real rain; the 2048² and tile-8192
rows from gate 1, the 16,384² tile-2048 row from gate 2 fix round 3):

| Run | Time | Peak RAM (Rust) | Disk | Notes |
| --- | --- | --- | --- | --- |
| 2048² window, `--tile 512 --threads 4` | 10 s | 0.33 GB | 0.3 GB | driver peak 0.5 GB |
| 16,384² window at the canyon, `--tile 2048 --threads 4` | 11 min (+41 s finish) | 3.0 GB | 18 GB (13 GB work) | 74 lakes, 790 rivers; every check 0 |
| 16,384² window, `--tile 8192 --threads 1` | 37 min | 5.3 GB | 18 GB | byte-identical outputs to the 2048 tiling; same checks (gate 1) |
| world 65,536², `--tile 2048 --threads 4`, fresh | 4.9 h (17,568 s) | 4.1 GB | 196 GB work | first gate-2 run, before the review's per-strip reads (driver 11.5 GB) |
| world, rerun from the breach (m1 kept) | 4.5 h (16,177 s) | 4.0 GB | | gate 2 rerun 1; driver 6.9 GB |
| world, rerun from the channels (m1, b, m2, l and m3 kept) | 65 min (3,905 s) | 4.4 GB | | gate 2 rerun 3 under `MemoryMax=10G`; driver 7.9 GB |

Tile passes dominate time. The global graph (about 70 MB serialized at 16k, so about 1.1 GB
for the world) scales with the leaves left after contraction, which removes only about 1 % of
depressions at 1 m. Each model run reads the ground about seven times (passes A, B, L, C, D, E,
F).

### Rerunning from a stage

A new build normally starts a fresh run (delete `bin`). After a code fix, the stages before
the first changed one can be kept. This saves hours on the world: gate 2's three world reruns
kept m1, then m1 to m3 (see `scratch/hydrology-v3/gate2/fix*/rerun-plan.md`). It is a manual
procedure.

**Warning: keep only stages whose code did not change.** Tags digest each stage's inputs and
settings: the tags of the stages it reads, the river pools and the grounds' contents. They do
not see code. Changed code, including a judgment constant that is not in a tag (such as
`LAKE_DEPTH_M`, `GORGE_RATIO`, `CHANGED_M` or `DRY_MIN_KM2`), leaves the tag as it was, and a
kept stage then silently holds the old code's output. Go by
`git diff <old>..<new> -- crates/topography-core/src/world/water/`:

| Changed | First stage to rerun |
| --- | --- |
| the model (`pipeline`, `balance`, `hierarchy`, `flow`, `receivers`, `flats`, `watershed`, `contract`, `nodes`, `rain`, `table`) or shared code (`geom`, `layers`, `store`, `mod.rs` constants) | m1: start fresh |
| `breach`, `hydraulic` (widths), or in `checks` the `CHANGED_M`, `horn_deg` or `is_wall` that breach uses | b |
| `bodies` | l |
| `channel` (channels or settle), or in `checks` the `SILL_M` that channels use | c |
| any other part of `checks` | none: rerun `water-finish` |

Settle rewrites the channels' rasters, so rerunning settle or m3 means rerunning from the
channels too. `world water` refuses to settle surfaces that are already settled.

Steps, with the run stopped (`pgrep -af '[t]opography_training.water.run'` shows nothing);
`$W` is the run directory and `$H` is `$W/hydrology` for a world or `$W` for a window:

1. Build, then `D=$(sha256sum target/release/topography | cut -d' ' -f1)`.
2. Swap the pin: delete `$H/bin/topography-*` and copy the new binary to
   `$H/bin/topography-${D:0:12}`.
3. Set the new digest in the driver's `$W/progress.json` (`.binary = $D`). Keep only the steps
   before `water`: a world keeps `raw`, a window keeps none. Set the same digest in the Rust stamp
   `$H/work/stamp.json` (`.code = $D`). Use `jq` to a temporary file, then `mv`.
4. In `$H/work`, delete the first stage to rerun and every later stage. Always delete
   `stages.json`:

   | Stage | Delete |
   | --- | --- |
   | b | `b-<tag>/`, `breached.f32` |
   | m2 | `m2-<tag>/`, `m2-{level,discharge}.f32`, `m2-node.u32`, `m2-flow.u8` |
   | l | `l-<tag>/`, `body.u32`, `bodies.json` |
   | c | `c-<tag>/`, `carved.f32`, `water-surface.f32`, `water-body.u32`, `s-<tag>/` |
   | m3 | `m3-<tag>/`, `m3-<tag>-*` (only when the model changed; m3 is keyed on the carved ground's content) |

5. Delete the published outputs: a world's `hydrology/1m`, `hydrology/review` and
   `manifest-failed.json`, or a window's `out/` and `review/`.
6. Relaunch the same driver command. The log should show the kept stages loading from cache
   (passes done in well under a second) and no `removing …/work` line.

For a checks-only change, do steps 1 to 3, keep `water` in `progress.json`, and drop `finish`
and the steps after it. Then do step 5 and relaunch.

## Export

```sh
target/release/topography export heightmap data/worlds/fujion-v3-chain-14-l4 data/exports/fujion-v3-chain-14-l4
target/release/topography export view data/exports/fujion-v3-chain-14-l4
```

Run `game_layers` first (above); without `game/` the export has height, water and moisture
only. Worlds without the hydrology v2 water layers (`water-surface.f32`, `water-body.u32`,
`bodies.json`) are refused: regenerate them. A world whose manifest names a 4 m hydrology
that is missing or does not overlap the export fails; a world without a `moisture` entry
exports without moisture and prints a line. The contract is
[heightmap export v2](../../schemas/heightmap-export-v2.md).

Tiles are written as `{layer}/L{level}/{prefix}_L{level}_{tx}_{ty}.png`, for example
`height/L0/height_L0_0_0.png` and `land-cover/L1/landcover_L1_0_0.png`. Layers (prefix =
layer name unless noted):

- `height`: 16-bit, 257² `shared_edge` tiles from level 0, no apron;
- `water-surface` (16-bit, the height encoding), `water-coverage` (8-bit fraction) and
  `water-body` (16-bit export ids matching `description.json`): from level 0, written only
  for tiles that hold water; a missing file means no water. Beyond 4 texels past the shore
  `water-surface` is code 0 (no surface);
- `biome`, `land-cover` (prefix `landcover`), `vegetation-density`, `snow-cover`,
  `talus-density`: from 2 m, from the world's `game/`;
- `moisture`: from about 128 m (on small test worlds the coarsest level only).

Every level comes from a global area-mean pyramid (mode for classes), and every layer but
height has a 4-texel apron. The last tile of a row or column is one sample short (256²
heights at the far corner). The manifest carries per-layer metadata, the `biome` and
`land-cover` legends with `present` flags, and `validation`: surface at or above ground where
wet, one level per lake, coverage matching surface over ground and identical shared edges
and aprons; a failing check stops the export before it is published. `description.json`
adds water bodies, river lines and lake outlines ([world description
v1](../../schemas/world-description-v1.md)).

The export holds the tiles, `manifest.json`, `description.json` and `.md`, and
`checksums.txt` (SHA-256 of every tile; verify with `sha256sum -c checksums.txt` in the
export). Add `--masters` to also write a full-world Float32 `elevation.tif`; it duplicates
the world output, so it is only needed when the export travels without it.

The export is staged in `.{output name}.stage-*` beside the output and renamed into place
once every check passes. It first checks for about 2 bytes free per level-0 sample (10 with
`--masters`; 8.6 GB at 64 km) and scans the world once, failing in minutes on water that
does not sit above its ground. A killed export (Ctrl-C, OOM) leaves its stage behind:
remove it by hand.

The viewer shows heights, water from `water-coverage`, snow from `snow-cover`, and land cover
or biome colours with a legend of the overview's shares; double-click loads 1 m detail.

Export streams the world row by row through the pyramids, so memory grows with the world's
width, not its area. Measured 2026-10-28 on the `t6` Fujion test worlds (a GPU job was running,
so wall times are pessimistic):

| World | Tiles | Size | Wall | CPU | Peak RSS |
| --- | --- | --- | --- | --- | --- |
| 8 km chain, 4 m (2,048²) | 700 | 9.0 MB | 4.1 s | 3.6 s | 49 MB |
| 4 km L4, 1 m (4,096²) | 1,379 | 21.8 MB | 14.4 s | 13.3 s | 71 MB |

Scaled to 64 km at 1 m (256 times the 4 km L4's cells): about 5.6 GB (height 4.9 GB, water
surface 0.2 GB, the rest 0.5 GB), about an hour of one core, and a peak well under 1 GB
(linear in width: about 0.7 GB, not measured). The test crop is all steep glaciated summit,
the worst case for PNG size; Fujix is about 40% deep sea, which compresses far better, so it
should land inside the 4 to 6 GB of [fujix v1](../plans/fujix-v1.md) §5.
