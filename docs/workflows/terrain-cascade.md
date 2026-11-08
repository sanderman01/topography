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
  - closed depressions on land of at least 1 km² and 50 m deep (info only; the hydrology
    breaches or fills them)
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
after L2 and L3 comes from [Hydrology v3](#hydrology-v3), one level at a time
(`water.run.run_level` with the release binary, which it builds with Cargo when it is
stale): L1 lakes get a bed sloping down from their shore (0.1, at most 40 m) so the 32 m
level keeps them, then the 32 m level (wind from the layout) breaches and erodes L2's ground
and sets its sea, lakes and rivers. L3 refines that final ground, with the 32 m water as its
water channel (32 m rivers drawn about 20 m wide). The 4 m level runs on the kept raw L3
surface, guided by the 32 m level; its final ground is the world's elevation. Outputs stay in
`hydrology/run-32m` and `hydrology/run-4m`, with `hydrology/32m` and `hydrology/4m` linking
their `out/` (L4 reads the 4 m one; details in
[Generating fujix from the collage](#generating-fujix-from-the-collage)). The L1 128 m lake picker keeps 1% of the land as lakes and 1% as dry basins in
classes whose lake-to-catchment ratio r is at most 0.05 (`configs/hydrology/water-v3.toml`);
`--stream-km2` sets the 32 m river start (default 10 km²). `--l1-class-runs`, `--l2-class-runs` and `--l3-class-runs` mix
per-class expert runs (`cN` subdirectories) over a level's run. The output is a 4 m world
(`manifest.json`, layers, hillshades of each level) that
exports directly. Beside `water.u8` (the 4 m mask L4 conditions on; the export ignores it) it holds `water-surface.f32`
(NaN dry, sea 0), `water-body.u32`, `bodies.json` and the 128 m `moisture.f32` (placed by
the manifest's `moisture` entry); the rasters are hard links into `hydrology/4m` (copies
across file systems). A world's `bodies.json` lists the sea, lakes, rivers and dry basins of
at least 1 ha (a 1 m glacier holds thousands of tiny ice basins); a parent left out becomes 0.
At 1 m it comes from `water.run world`. A 32 km L3 takes about an hour. `--guidance w` applies class
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

`cascade_l4` refines that 4 m world to a raw 1 m surface in bands of rows, and
`water.run world` then makes the 1 m water world from it ([Hydrology v3](#hydrology-v3)):

```sh
$P -m topography_training.diffusion.cascade_l4 --run data/models/cascade-l4-run-8 \
    --world data/worlds/fujion-v3-chain-14 --out data/worlds/fujion-v3-chain-14-l4-raw --band-m 1024
PYTHONPATH=training/src $P -m topography_training.water.run world \
    --source data/worlds/fujion-v3-chain-14-l4-raw --out data/worlds/fujion-v3-chain-14-l4
```

Bands default to 1,024 m with a 256 m halo (`--band-m`, `--halo-m`, multiples of 64 m, the
halo no wider than a band); overlapping bands share their starting noise (drawn per tile of
the map's 1 m grid, as in L3) and are cross-faded, resuming exactly as in L3. `cascade_l4`
only diffuses: it writes a raw world (`manifest.json` of kind `world_raw`,
`elevation-raw.f32`, `snow-ice.u8`, `vegetation.u8`, `overview.png` and `detail.png`) with
no water. Under 4 m lakes and sea the ground is the refined 4 m ground (L4 learned land
only); under 4 m rivers too, unless `--keep-river-residual` keeps the diffused residual there
(B32 A/B). Bands, and windows inside a band, whose 4 m ground is all deep water (below
−50 m) are not diffused (the manifest's `deep_sea_bands` and `windows`). `--guidance-classes`
limits `--guidance` to windows holding those classes, as in the chain. Rerunning the same
command resumes after the last finished band.

`elevation-raw.f32` (4 bytes a cell, 17 GB at 64 km) is the input of `water.run world`, so
the 1 m water can be rerun after a hydrology fix without diffusion and without the GPU.
`--km N` limits the run to a central N × 1,024 m square for quick checks (`--centre X,Y`
places it); worlds without a `hydrology/4m` output must be regenerated, and older chain
worlds need `--class-id`. `--class-runs` mixes per-class expert runs over `--run`. At
12 steps and stride 192, a central 4 km takes about 14 min.

Memory and disk: output layers are memory-mapped and the 4 m world is read band by band, so
host memory depends on the band width and height. Measured at 64 km on CPU with the sampler
replaced by five band-size buffers (its working arrays): a 1,024 m band holds about 5.5 GB in
Python besides those buffers and the run peaked at 9.4 GB (page cache included). So run it in
a 12 GB cap; 2,048 m bands roughly double the Python share. The raw world takes 6 bytes a
1 m cell (26 GB at 64 km); `cascade_l4` checks the free space before it starts. The 1 m water
world needs its own room ([Settings, resuming and cost](#settings-resuming-and-cost)). The
chain checks for 130 bytes a 4 m cell before it starts (35 GB at 64 km: world layers and raw L3
about 44 B, the 4 m water work 54 B, erosion routings 28 B). Its L3 bands hold
about 4.6 GB at 64 km width (8,192 m bands); the 4 m level at 64 km (16,384²) peaked at
6.2 GB in Rust with erosion while Python holds about 2.5 GB, so run a whole-map chain in a
10 GB cap.

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
treeline and forest come from the collage's `layout.json`. The collage needs its provenance
sidecar `provenance32.u8` (`python -m topography_training.diffusion.collage provenance <dir>`).

Water is hydrology v3, one level at a time (`water.run.run_level`):

- **32 m**: `hydrology/run-32m/{grid,work,out,review,bin,level.json}`, with `hydrology/32m`
  a symlink to `run-32m/out`. Rain, breach, erosion (K by provenance), water, checks and
  review. L3 refines its final ground. A failed 32 m level exits 1 before L3 and writes no
  manifest.
- **L3**: the raw 4 m surface `l3-raw.f32` is kept. Under 32 m lakes and sea the ground eases
  down to the 32 m ground; under 32 m rivers too unless `--no-l3-river-clamp` (B32; the Task 15
  A/B tied, so the clamp stays on; recorded in `progress.json` and the manifest as
  `l3_river_clamp`).
- **4 m**: `hydrology/run-4m/…` on `l3-raw.f32` (symlinked into its grid), guided by the
  32 m level (hard coarse guide), with `hydrology/4m` linking `run-4m/out`. Its final ground
  is the world's `elevation.f32`.

The world adds `provenance.u8` (32 m cells of the L3 square) and records both levels'
`level.json` under the manifest's `water`. When the 4 m level fails, the world is written
as `manifest-failed.json`, the chain exits 1 and keeps `l2.npz` (the chain refuses to rerun
a failed world as it is). To recover after a fix:

1. Build, then replace the level's pinned binary: either delete
   `<world>/hydrology/run-4m/bin` (the level starts fresh, about 35 min at 64 km) or follow
   [Rerunning from a stage](#rerunning-from-a-stage) steps 1–5 to keep the unchanged stages.
   Without this the level reruns the old binary on its finished stages and fails again.
2. Rerun the level alone: `python -m topography_training.water.run level --run
   <world>/hydrology/run-4m --coarse <world>/hydrology/32m`.
3. Delete `<world>/manifest-failed.json`.
4. Set `hydrology_4m_done` to `false` in `<world>/progress.json`.
5. Rerun the same `cascade_chain` command: it resumes after L3 from `l2.npz`, takes the
   passed 4 m level and writes `manifest.json`.

The world also holds `description.json`
and `description.md`: for a whole-map L3 the collage's description measured on the carved
4 m surface (level `l3`: regions, passes and their bands, summit and caldera); for an L3
square, measured on the carved 32 m surface (level `l2`) plus `finer`, pass ground and
feature values on the square (`partial` where a feature reaches past it). `cascade_l4`
copies it. `--l3-km` with `--l3-centre X,Y` (map metres from the north-west corner, y south)
puts the L3 square anywhere; `cascade_l4 --km` with `--centre X,Y` does the same for L4,
whose default is the 4 m world's centre. Windows (and L4 bands) of deep sea only are not
diffused at L3 or L4.

Full run (it needs the owner's go). Class guidance 2 on the Grand Canyon windows only:

```sh
CAP=(systemd-run --user --scope -q -p MemoryMax=12G)
export PYTHONPATH=training/src
$P -m topography_training.diffusion.collage provenance data/worlds/fujix-collage-55
"${CAP[@]}" $P -m topography_training.diffusion.cascade_chain --collage data/worlds/fujix-collage-55 \
    --l3-run data/models/cascade-l3-run-5 --l3-class-runs data/models/cascade-l3-run-5-experts \
    --guidance 2 --guidance-classes 19 --out data/worlds/fujix-55-chain-v3
"${CAP[@]}" $P -m topography_training.diffusion.cascade_l4 --run data/models/cascade-l4-run-8 \
    --class-runs data/models/cascade-l4-run-8-experts --guidance 2 --guidance-classes 19 \
    --world data/worlds/fujix-55-chain-v3 --out data/worlds/fujix-55-l4v3b-raw
"${CAP[@]}" $P -m topography_training.water.run world \
    --source data/worlds/fujix-55-l4v3b-raw --out data/worlds/fujix-55-l4v3b
$P -m topography_training.diffusion.game_layers --world data/worlds/fujix-55-l4v3b
target/release/topography export heightmap data/worlds/fujix-55-l4v3b data/exports/fujix-55-l4v3b
```

`collage provenance` writes the sidecar once (it is additive). Each long command, rerun
unchanged after a crash or kill, resumes where it stopped. Run them as background jobs with a
log and an exit-code file ([AGENTS.md](../../AGENTS.md)).

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

These runs used the v2 hydrology; every report and band seam passed (after the fix of a cell widened out of a sink
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
  about 35 min of 4 m water with erosion (a 16,384² benchmark took 33 min, 701 s of it
  erosion) and 5 min of layers. The 32 m level takes about 2 min.
- L4: about 35 h of diffusion (41 h with guidance), against 55 h before the window skip.
  Then `water.run world`: about 5 h of CPU (4.9 h fresh at gate 2).
- RAM: L3 peaks at about 9 GB (bands about 4.6 GB; the 4 m level 6.2 GB in Rust while
  Python holds about 2.5 GB); L4 about 9.4 GB (earlier 64 km band measurement, 12 GB cap);
  the 1 m water world about 4.4 GB in Rust and up to 8 GB in the driver. Run each in a 12 GB
  cap.
- Disk: about 35 GB for the chain (130 bytes a 4 m cell checked), 26 GB for the raw L4
  (6 bytes a 1 m cell), and for the 1 m water world about 232 GB of work plus 55 GB
  published.

## Hydrology v3

Water as one field at every level ([spec](../superpowers/specs/2026-11-05-hydrology-v3-design.md),
[Plan A](../superpowers/plans/2026-11-05-hydrology-v3-a-water-model.md) for the 1 m model,
[Plan B](../superpowers/plans/2026-11-07-hydrology-v3-b-coarse-and-regeneration.md) for the
coarse levels): every cell has a ground height, a water surface or none, and a discharge.
Lakes and rivers are labels read off that field. One model runs at 32 m, 4 m and 1 m. Rust
(`world::water` in `topography-core`) runs exact graph passes on tiles with halos: the
priority-flood spill graph, the depression hierarchy and Fill-Spill-Merge, fall-line
receivers, flat ponds and accumulation, then breaches, labels and channel carving. Tiled runs
give the same bytes as one tile, so there are no bands and no seam rules. The 32 m and 4 m
levels add an erosion stage (`e`) and hand their final ground to the next diffusion level;
each finer level's own model sets its courses and levels, and the coarser level only guides
(the [coarse guide](#the-coarse-guide)). Where the code departs from the spec, the spec lists
it under "Plan A departures" and "Plan B departures".

Departure from the spec: the spec's GPU bank relaxation is dropped in Plan A. On an 8192²
probe at the deepest cut (1.33 m) the carved ground alone passed `walls` and `facets`, the
relaxed ground had more `drainage_rises`, and the hillshades looked the same. Plan A ships the
carved ground (`relaxation: none` in the manifest). The relaxation is kept in git at
`d0c0055` (`training/src/topography_training/water/relax.py`), should deeper cuts need bank
shaping (Plan B ruling B27: only if the owner flags banks at gate 4).

### Commands

`$P` as at the top of this page and `CAP` with `MemoryMax=12G`. Each driver resumes when run again with the same
arguments and pins a copy of `target/release/topography` in its run directory (`bin/`). A
new build starts again from scratch: delete that `bin` directory and the driver wipes every
cache. To keep the stages whose code did not change, see
[Rerunning from a stage](#rerunning-from-a-stage).

```sh
export PYTHONPATH=training/src
"${CAP[@]}" $P -m topography_training.water.run world \
    --source data/worlds/fujix-55-l4v3b-raw --out <new-world> [--tile 2048] [--threads 4]
"${CAP[@]}" $P -m topography_training.water.run level \
    --run <run-dir> [--coarse <coarser-level-out>] [--tile 2048] [--threads 4] [--river-km2 A]
"${CAP[@]}" $P -m topography_training.water.run window \
    --source data/worlds/fujix-55-l4v2 --level 1 --centre 27169,44546 --size 2048 --tile 512 --out <dir>
```

`world` needs an L4 world with a kept `elevation-raw.f32` (`cascade_l4` output) and a
`source_world` chain world whose `hydrology/4m/grid.json` carries the 128 m moisture field;
without real rain the run is refused. It never writes to the source and refuses an `--out`
inside it. It compares its result with the 4 m level as the [coarse guide](#the-coarse-guide).
`level` runs one coarse level ([The level runner](#the-level-runner)); `cascade_chain` calls
it for 32 m and 4 m. `window` cuts a test window: `--level 1` from the L4 world's raw surface,
`--level 4` and `--level 32` from a chain world (`--level 4` reads the chain's final 4 m
ground, so it shows the model's behaviour, not a level result; `--level 32` reads the
chain's `l2_raw`). Exit status: 0 passed, 1 a check failed (outputs kept; a world gets
`manifest-failed.json`), 2 refused or failed.

The Rust commands, by hand:

```sh
target/release/topography world rain --grid <grid-dir> [--wind-deg 225]
target/release/topography world water --grid <grid-dir> --work <work-dir> [--tile 4096] [--threads 4] [--water water.toml] [--river-km2 0.05]
target/release/topography world water-finish --grid <grid-dir> --work <work-dir> --ground <work-dir>/carved.f32 \
    --out <new-out-dir> [--tile 4096] [--threads 4] [--spots E,N;E,N]
```

`--spots` (metres; the driver passes the review spots) only feeds the `dam_drops_at_spots`
figure; it is not part of the work stamp.

### Rain (`world rain`)

`world rain` writes the trunk grid's 128 m moisture field from its own raw ground, before any
erosion: `moisture.f32` (rain over its 95th percentile on land, 0 to 1) and the grid.json
entries `moisture`, `discharge.mean_land_mm_h` and `precipitation` that `world water` reads.
The level runner calls it when the 32 m grid has no `moisture` entry; the 4 m and 1 m grids
copy the 32 m field. The model is Smith and Barstad (2004), the linear upslope model on a
128 m grid (FFT, padded with 0 m ground), clipped at 0 plus a background rate, then depleted
along the wind from the upwind edge (exp(−rained / budget)), which gives wet windward coasts
and a dry far lee. The wind is `--wind-deg` (degrees the wind comes from, clockwise from
north), else `wind_from_deg` in grid.json; without either, rain is uniform and moisture 0.5.
Sea is ground below 0 m connected 8-way to the grid edge. Judgment constants: 10 m/s wind,
N 0.005 s⁻¹, vapour scale height 2,500 m, condensation and fallout 300 s, background
1.5 mm/h, 3 mm column budget. On `fujix-collage-55` it reproduces v2's 32 m moisture field
byte for byte (ruling B12).

### Provenance

`python -m topography_training.diffusion.collage provenance <collage>` replays a collage's
surface steps from its layout and release and writes the sidecar `provenance32.u8` and
`provenance32.json` beside it (ruling B10; refused on a layout or region mismatch). One byte a
32 m cell:

| Bits | Meaning |
| --- | --- |
| 0–2 (kind) | 0 sea outside every region, 1 piece interior (one real GLO-30 piece, at least 1 km from any other kind), 2 blend zone, 3 painted feature, 4 exemplar fill |
| 64 | an inland basin below 0 m here may keep a lake below sea level (a `hot_dry` class at least `below_sea_coast_km` from open sea) |
| 128 | steep source: the collage is at least 30° steep (a figure tag) |

`cascade_chain` crops it into each level's grid as `provenance.u8` (`provenance_cell_m` 32),
with the collage's region ids as `region.u8` (a figure mask only), and the world keeps it as
`provenance.u8`. At 32 m and 4 m the kind sets the erosion's K; at 1 m only the flags are read
(no provenance scaling at 1 m). A grid without `provenance.u8` reads every cell as 0.

### Coarse levels: erosion

At 32 m and 4 m the `e` stage erodes the breached ground before the labels, in `routings`
rounds ([water table](#settings-resuming-and-cost) `[levels.N.erosion]`; Plan B rulings
[B1–B7, B23 and B33](../superpowers/plans/2026-11-07-hydrology-v3-b-coarse-and-regeneration.md#rulings-on-what-the-spec-leaves-open)).
Each routing:

1. **Model**: the full water model on the current ground (routing 0 on the breached ground),
   so depressions route to their outlets and sills incise.
2. **Stream power**: one implicit step (Braun and Willett 2013, n = 1),
   F = k_dt · A^m / L with A the discharge in m² (rain-weighted) and m = 0.5: one global,
   single-threaded pass in ascending ground order (a tie's receiver chain first), so it is
   exact and independent of the tiling. K depends on the provenance kind: piece interiors and
   glacier cells (class 10) take `interior_k` (0.05) × K and no diffusion; blend zones,
   painted features and fills take full K; the sea is never eroded, and a base below 0 m on a
   river into the sea counts as 0 m.
3. **Taper**: each stream-power cut of d ≥ 1 cm becomes a disc lowered by d · (1 − (r/R)²),
   R = max(2.5 cells, 2d / (tan(wall_deg) · spacing)), never below the floor and never under
   water or sea, so cuts are at least 5 cells wide.
4. **Diffusion**: explicit 4-neighbour Jacobi, α = `diffusion` (0.05) a routing, lowering
   only, and only near this routing's stream-power cuts (the footprint grows from cuts of at
   least 0.25 m by the taper radius plus a cell). It never makes a trough or a wall.
5. **Wall rounds**: up to 16 rounds halve the lowering of any changed cell steeper than
   max(wall_deg, raw slope) + 0.01°, alternating with a bed-grade pass so no new pit forms;
   a cut still beside a wall after that is reverted to the routing's start (figures
   `wall_cuts_removed`, `sills_reverted`, `wall_rounds`).

Caps per cell, below the breached ground (so breach gorges are excepted): `cap_m` on full-K
cells (30 m at 32 m, 10 m at 4 m) and `interior_cap_m` (3 m) on interior cells. The ground
before channels (`pre_channel`) is the last routing's `e<k>.f32`. Breaches at the coarse
levels follow the same shape rules: a gorge's taper radius is at least 2.5 cells, its discs
also cut steep raw ground, and a gorge that would leave a wall or fail `handoff_narrow` is
refused, so the depression stays filled. The 1 m level has no `e` stage.

Calibration (Task 9, 32 m): k_dt 7 × 10⁻⁴ keeps v2's value. It passes every hard check;
`seam_excess` (the density of lakes exiting within 2 km of a region border over that of
lakes exiting farther away) is 1.10, under the 1.5 target, and far blend-zone cells change by
a p99 of 1.95 m. Erosion takes 7 to 9 s a routing at 32 m and 701 s for both
routings of a 16,384² 4 m grid (peak 6.2 GB): the CPU solver is fast enough.

Calibration (Task 15, 4 m): on two real L3 squares (dam spots and canyon) every k_dt from
4.4 × 10⁻⁵ to 1.75 × 10⁻⁴ passes and `seam_excess` does not separate them, so the planned
8.75 × 10⁻⁵ stays. The L3 river clamp A/B tied (same checks, figures within 1 %).

### The level runner

`water.run level --run <dir>` runs one coarse level from a grid its caller prepared
(`cascade_chain` does this for `hydrology/run-32m` and `hydrology/run-4m`):

| Path | Content |
| --- | --- |
| `grid/` | `grid.json`, `elevation.f32` (at 4 m a symlink to the chain's `l3-raw.f32`), `classes.u8`, `provenance.u8`, `region.u8`, `moisture.f32` |
| `work/` | the stage files of `world water` (below) |
| `out/` | the published output of `world water-finish` |
| `review/` | review figures and `coarse-guide.json` |
| `bin/` | the pinned binary |
| `progress.json` | the driver's finished steps |
| `level.json` | `passed`, `failures` (the report's plus `coarse_guide`), `report`, `coarse_guide` summary and the review figures |

It runs `world rain` when the grid has no moisture, then `world water`, `world water-finish`,
the review and, with `--coarse`, the hard coarse guide against the next coarser level's
`out/`. Exit status 0 when every check and the guide pass, else 1 (2 refused).

### The coarse guide

The level runner and the 1 m driver compare each level with the next coarser v3 level
(`coarse-guide.json`, ruling B15). It is hard only for coarse lakes of at least 0.05 km²,
deeper than the finer level's breach cap plus 1 m (6 m from 32 m to 4 m, 3 m from 4 m to 1 m)
and at least 1 km from an inner window or square edge: such a lake fails when its fine
level differs by more than 5 m, when it is gone, or when it is doubled (two fine lakes each
over 25 % of its area). Every other coarse lake is a figure (`lakes_small`, `lakes_shallow`,
`lakes_near_edge`), as is the share of coarse rivers with a fine river within 100 m. A hard
failure withholds the level's pass (`level.json`) and, in a world, `manifest.json`
(`manifest-failed.json` instead), and the run exits 1.

### Grid and work directories

A grid directory holds `grid.json`, `elevation.f32` and optional `classes.u8`,
`provenance.u8`, `region.u8` and `moisture.f32` (little-endian, rows north to south).
`grid.json` gives `size` [w, h], `spacing_m`, `origin` [x, y] (the world's north-west corner
in the layout plane, x east, y north; default [0, h · spacing]), and optionally `offset_m`
[east, south] (a window inside that world), `classes_cell_m`, `provenance_cell_m` and
`wind_from_deg`. `world water` needs a `moisture` entry (`file`, `size`, `spacing_m`, `nw_m`,
`scale_mm_h`) and `discharge.mean_land_mm_h` (from `world rain`); the drivers add `window`
and `inputs` (class map and moisture provenance). At 1 m the grid is the raw surface with
the chain's 4 m classes (`classes_cell_m` 4).

`world water` writes the work directory stage by stage; each stage's tag digests its inputs
and settings, and `stamp.json` ties the directory to its grid, class map, moisture and binary,
so a rerun resumes and a changed input is refused:

| Stage | Files |
| --- | --- |
| `m1` model on raw | tile caches `m1-<tag>/` |
| `b` breach | `breached.f32` |
| `e` erosion (32 m and 4 m only): per routing k, a model run and stream power, diffusion, taper, and wall and narrow-cut rounds | `e<k>.f32`, `e<k>-*.{f32,u32,u8}`, `e-<tag>/` |
| `m2` model on the pre-channel ground (the last `e<k>.f32`, else `breached.f32`) | `m2-level.f32`, `m2-node.u32`, `m2-discharge.f32`, `m2-flow.u8` |
| `l` labels (lakes, rivers, river pools) | `body.u32`, `bodies.json` |
| `c` channels | `carved.f32`, `water-surface.f32`, `water-body.u32` |
| `m3` model on the carved ground | `m3-<tag>-*.{f32,u32,u8}` |
| `s` settle | rewrites `water-surface.f32` and `water-body.u32` in place; per-tile changes in `s-<tag>/` |
| figures | `stages.json` (stage tags, breaches, erosion summed over routings with `per_routing`, channels, nodes, leaves, lakes, ponds, area and depth histograms) |

`world water-finish` checks the carved ground with the m3 model and publishes `--out`. The m3
tag is keyed on the run's settings and the sha256 of the carved and pre-channel grounds (the
only rasters m3 reads), so a channels rerun that changes surfaces and labels only reuses m3. The
settle tag follows m3's and the channels' tags. `water-finish` recomputes every tag and refuses
a `stages.json` that does not match.

Settle caches each tile's changes before it rewrites the channels' rasters, so a kill between
the two writes resumes cleanly. Settle never runs on surfaces that another settle has already
rewritten. A channels run removes earlier `s-*` directories, and `world water` refuses to
settle again until the channels rerun.

### Outputs

The world's `elevation.f32`, `water-surface.f32`, `water-body.u32`, `bodies.json` and
`water.u8` keep their earlier names and formats, so `game_layers` and export are unchanged.
`snow-ice.u8`, `vegetation.u8` and `moisture.f32` come from the source and chain worlds.
`hydrology/1m` holds the published water output: those layers plus `discharge.f32` and
`flow.u8` (both from m3), `report.json` and `grid.json` with provenance. `discharge.f32` is
drainage area weighted by rain, km² at mean rain. `flow.u8` holds the D8 receiver in its low
4 bits (1 N, 2 S, 3 W, 4 E, 5 NW, 6 NE, 7 SW, 8 SE; 0 none), 16 on a channel at the river
start, 32 on a dry closed basin and 64 in a closed (endorheic) basin or lake.
`hydrology/review` holds the review figures (owner spots, longest straight runs, failing-check
clusters, lake-exit sheets, overview) and `coarse-guide.json`, the comparison against the 4 m
level ([The coarse guide](#the-coarse-guide)). A window or level run has the same under
`out/` and `review/`.

A level's `bodies.json` lists the sea, lakes (`level_m`, `exit_cell`, `parent`), rivers and
dry basins. Every full labelled lake starts a river at its outflow cell (the outlet
trickle, ruling B16), labelled down its path to the next water, the sea or the map edge
whatever its discharge, with a channel sized from that discharge; at most one river per lake.

The coastal water table (ruling B13): in a depression with ground below 0 m, the lake level
is never below min(0 m, its spill) unless every sub-zero cell carries provenance bit 64 (a
hot, dry class far from the sea). Where that floor governs, the lake covers the cells below
it, takes no extra supply and is not full, so no water flows on downstream. Ground is never
raised. Basins allowed below sea level keep their balance level.

### Checks

Every count must be 0. D is the water surface where wet, the pond level in ponds (m3), else the
final ground. Thresholds and rulings are in [Plan A](../superpowers/plans/2026-11-05-hydrology-v3-a-water-model.md)
(Task 13 and rulings R7, R11, R14, R16, R27, R29–R31) and, for the coarse levels and the newer
checks, [Plan B](../superpowers/plans/2026-11-07-hydrology-v3-b-coarse-and-regeneration.md#rulings-on-what-the-spec-leaves-open):

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
| `dam_steps` | within 50 m below a lake exit, D drops more than 1.5 m between adjacent path cells and raw does not drop as much. Hard at 1 m; at 32 m and 4 m a figure, `dam_steps_over_raw` (ruling R2: 1.5 m a cell is a gentle grade there) |
| `dam_drops` | on a labelled lake's exit walk (receivers, at most 150 m, to standing water, no receiver or the edge), a drop D over 1.5 m within 50 m of the exit is a knickpoint: D > 3 × the mean fall a cell over the next 100 m and D > 3 × the mean fall before it. At 32 m and 4 m it fails only where D exceeds the raw step there by over 1.5 m; 1 m ignores raw. Drops onto water and walks with fewer than 2 steps ahead are figures (`dam_drops_into_water`, `dam_drops_unjudged`); pond exits are figures (`dam_drops_ponds`, `dam_drops_any`) (B14) |
| `lake_below_sea` | a labelled lake's level is below −0.01 m and one of its cells lacks provenance bit 64 (B13) |
| `dam_rims` | a guard: an exit-side dry cell stands over level + 0.5 m and raw did not (v3 never raises ground) |
| `bed_rises` | on a dry m2 thalweg link where the ground before channels falls, the carved bed rises |
| `thalweg_beside` | beside a thalweg with Q ≥ 1 km², the lowest standing cell across the flow (to the first ridge 0.5 m up, ±30 m) is more than 1 m below this river, belongs to no tributary, and carries a second channel (see below) |
| `parallel_rivers` | figure (owner ruling 2026-11-06: raw-terrain straight/parallel rivers accepted), `figures.parallel_rivers_count`: along a river with Q ≥ 1 km², another flow line with Q ≥ 0.05 km² runs within 10 m and parallel (cosine ≥ 0.7) for over 100 m without joining it within 100 m |
| `straight_share` | figure (owner ruling 2026-11-06: raw-terrain straight/parallel rivers accepted), `figures.straight_share_count`: more than 5 % of river length lies in straight runs over 60 m (global, 0 or 1; the share itself is `figures.straight_share`) |
| `straight_runs` | figure (owner ruling 2026-11-06: raw-terrain straight/parallel rivers accepted), `figures.straight_runs_count`: a river thalweg stays within 1 m of its chord (any angle) for more than 150 m, unless the raw thalweg is straight there too (90 % of cells) |
| `facets` | more planar 5×5 windows (residual < 2 cm, slope > 5°) among changed cells than raw had (global) |
| `walls` | a changed cell's Horn slope exceeds max(wall_deg, its raw slope) + 0.01° (`wall_deg` 40° at 1 m, 35° at 4 m and 32 m) |
| `change_p99` | p99 of \|Δ\| over all cells > 1 m (global). At 32 m and 4 m a figure, `change_p99_all_cells_m`: their budget is `interior_p99` and `interior_max` (B4) |
| `change_max` | a cell's \|Δ\| > the breach cap + the erosion cap + 0.5 m (2.5 m at 1 m, 15.5 m at 4 m, 50.5 m at 32 m) |
| `interior_p99` | 32 m and 4 m: p99 of \|Δ\| over real piece-interior cells outside breaches > 1 m (global; figure `interior_change_p99_m`) |
| `interior_max` | 32 m and 4 m: an interior cell outside breaches changed by more than 3 m (figure `interior_change_max_m`) |
| `handoff_narrow` | 32 m and 4 m: the cut Δ = raw − final exceeds its 3 × 3 grey opening by more than max(0.5 m, 0.5 × Δ) and, on some axis, the final ground is a trough over that threshold deeper than raw's (the cut made the trough; B9, ruling R5; figure `handoff_excess_max_m`) |
| `erosion_new_pits` | 32 m and 4 m: a strict local minimum of the eroded ground that was none of the breached ground (B33) |
| `carve_pits` | (a) a dry cell lowered from raw by more than 1 cm inside an m3 node and in no m2 node (a pond only the cutting made), or (b) a one-cell pit the cutting deepened by more than 1 cm (B26; figure `carve_pool_cells` for wider pools) |
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
beside a lower written river passes. Plan B (Task 9) counts these as the river's own water, not
a second channel: the sea, a fed lake's outflow within 150 m, the river's own unlabelled
continuation, and a dry drainage end on a shore.

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
included), so a bed carved flat does not send the river off its labelled line. Rims (Plan B
Task 13): the floor beside a cut holds only for lakes and for ponds at least as deep as the
river, not for micro-pits beside the channel, and a pit-restore rule keeps channel cuts from
deepening a one-cell pit, so cutting on raw flats leaves no comb of pits (`carve_pits`).

Breach (gate 2 fix round 1): at 1 m, cells beside the gorge's path whose raw slope is at least
39° are not cut (path cells always are); each path cell's taper reaches max(half-width + 1,
2·cut / (tan 35° · spacing)) cells; and a gorge that would still leave a changed cell steeper
than max(wall_deg, raw) is refused, so the depression stays filled. At 32 m and 4 m the taper
is at least 2.5 cells, the discs also cut steep raw ground, and a gorge whose cut fails
`handoff_narrow` is refused too. `stages.breach.breaches_rejected_walls` and
`breach_refused_narrow` count the refused gorges and list the 20 deepest (node, cut, spill
cell, filled depth).

Two more figures: `dam_drops_at_spots` lists, per review spot, the exit-walk drops over 1.5 m
within 100 m with their raw drop (count and largest 10), and `unlabelled_main_river_cells`
counts dry m3 thalweg cells in no node with Q ≥ 1 km², plus runs of 10 or more such cells at
the river threshold.

Coarse-level figures: `seam_lakes` lists labelled lakes whose outlet saddle cell is blend,
painted or fill (lakes behind seams between collage pieces; B25);
`interior_change_p99_m` and `interior_change_max_m` are the interior budget's values and
`blend_change_p99_m` / `blend_change_max_m` the same for blend cells at least 2 km from a
region border; `stages.erosion` sums each routing's lowering, the hillslope lowering outside
the diffusion footprint and the wall-round figures (`per_routing` lists them by routing).

### Settings, resuming and cost

`configs/hydrology/water-v3.toml` (`schema_version = 3`, built in; `--water` replaces it)
gives each class's lake-to-catchment ratio `r` and `hot_dry` flag (canyons 2, 13 and 19,
badlands 7, dunes 8 and 16: a basin there at least `below_sea_coast_km`, 5 km, from open sea
may keep a lake below sea level), the game setting `lake_min_m2` (smaller standing water is
an unlabelled pond) and the river-width factor `m3s_per_km2`. Per level (`[levels.1]`,
`[levels.4]`, `[levels.32]`) it gives the river start `river_km2`, the breach cap
`breach_cap_m` (2, 5 and 20 m) and `wall_deg` (40°, 35°, 35°), and at 4 m and 32 m an
`[levels.N.erosion]` table: `routings` (2, 5), `k_dt` (8.75 × 10⁻⁵, 7 × 10⁻⁴), `m` (0.5),
`diffusion` (0.05), `cap_m` (10, 30 m), `interior_k` (0.05) and `interior_cap_m` (3 m).

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
| `erosion` | e |
| `bodies` | l |
| `channel` (channels or settle), or in `checks` the `SILL_M` that channels use | c |
| any other part of `checks` | none: rerun `water-finish` |

Settle rewrites the channels' rasters, so rerunning settle or m3 means rerunning from the
channels too. `world water` refuses to settle surfaces that are already settled.

Steps, with the run stopped (its background job has exited and written its `exit_code`);
`$W` is the run directory and `$H` is `$W/hydrology` for a world or `$W` for a window or a
level:

1. Build, then `D=$(sha256sum target/release/topography | cut -d' ' -f1)`.
2. Swap the pin: delete `$H/bin/topography-*` and copy the new binary to
   `$H/bin/topography-${D:0:12}`.
3. Set the new digest in the driver's `$W/progress.json` (`.binary = $D`). Keep only the steps
   before `water`: a world keeps `raw`, a window or level none. Set the same digest in the Rust stamp
   `$H/work/stamp.json` (`.code = $D`). Use `jq` to a temporary file, then `mv`.
4. In `$H/work`, delete the first stage to rerun and every later stage. Always delete
   `stages.json`:

   | Stage | Delete |
   | --- | --- |
   | b | `b-<tag>/`, `breached.f32` |
   | e | `e-<tag>/`, `e<k>.f32`, `e<k>-*` |
   | m2 | `m2-<tag>/`, `m2-{level,discharge}.f32`, `m2-node.u32`, `m2-flow.u8` |
   | l | `l-<tag>/`, `body.u32`, `bodies.json` |
   | c | `c-<tag>/`, `carved.f32`, `water-surface.f32`, `water-body.u32`, `s-<tag>/` |
   | m3 | `m3-<tag>/`, `m3-<tag>-*` (only when the model changed; m3 is keyed on the carved ground's content) |

5. Delete the published outputs: a world's `hydrology/1m`, `hydrology/review` and
   `manifest-failed.json`, a window's `out/` and `review/`, or a level's `out/`, `review/`
   and `level.json` (inside a chain world, then recover the chain as in
   [Generating fujix from the collage](#generating-fujix-from-the-collage)).
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
only. Worlds without the water layers (`water-surface.f32`, `water-body.u32`,
`bodies.json`) are refused: regenerate them with hydrology v3. A world whose manifest names a 4 m hydrology
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
