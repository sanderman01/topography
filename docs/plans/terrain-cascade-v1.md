# Terrain cascade v1

Status: planned 2026-09-29. Steps 2–7 done by 2026-10-03: L1–L4 pass blind
review, the chain and banded 1 m generation work, and export streams. Step 8, the
first cascade world, is in progress; step 1 (authored layouts) is open (see
[Progress](#progress-2026-10-03) and the [workflow](../workflows/terrain-cascade.md)). It extends the
[terrain diffusion pilot](terrain-diffusion-v1.md) into a coarse-to-fine
generator and replaces its "follow-on coarse structure model" section. The
[master plan](terrain-design-master-plan.md) §§5, 8 and 11 point here. Curated
real patches and blending stay the fallback (master plan §7).

## Why

The caldera-island test world (2026-09-29) used a procedural sketch as its 32 m
base and let the 1 m model add detail. The owner rejected it: fjords, glaciers,
rivers, ranges and the caldera looked like brush strokes, and the canyon country
and karst looked flat. Two things caused that.

- Large landforms came from hand-written primitives, not from real terrain. No
  amount of 1 m detail gives a painted stroke a drainage network.
- The 1 m model only adds a residual of about ±8 m times a few standard
  deviations. Karst towers 100–200 m tall and canyon walls hundreds of metres
  high have to exist in the surface it refines. The 32 m base had averaged the
  towers away, and the sketch never had real canyon relief.

Generating at 1 m would not have fixed either. Large structure has to come from
models trained on real terrain at large scales, with each finer level adding
what the level above cannot hold. This is the fractal-noise idea: each octave
adds detail at its own scale and leaves the larger shapes alone.

## Levels

| Level | Cell | 32 km grid | Adds | Trained on |
| --- | --- | --- | --- | --- |
| L0 layout | 256 m | 128² | Biome regions, height envelope (below 0 m where sea is wanted), hero-feature lines | Nothing. Authored from the [layout spec](world-layout-spec-v1.md) |
| L1 world | 128 m | 256² | Island shape, ranges, main valleys, drainage divides, caldera | GLO-30 averaged to 128 m |
| L2 regional | 32 m | 1,024² | Valley systems, fjords, canyon networks, mesas, cones | GLO-30 |
| L3 local | 4 m | 8,192² | Spurs, gullies, karst towers, cliff bands, crevasse fields | 1 m lidar averaged to 4 m |
| L4 detail | 1 m | 32,768² | Fine detail | The existing 1 m library (run 5) |

Steps between levels are 2×, 4×, 8× and 4×. Each model sees the level above as a
smooth surface and generates the residual. L1 covers the whole 32 km world in one
256² window, so island-wide shape is consistent without window joins.

Every level uses the pilot's machinery: pixel-space EDM diffusion, a U-Net with
attention at the two coarsest resolutions, 256² training crops, MultiDiffusion
windows, host-memory conditioning (added 2026-09-29) and fixed physical
normalization per level. Models above 1 m can be smaller than the 1 m U-Net,
because their data is smoother and far smaller.

### Conditioning per level

Each level gets:

- the level above, upsampled and smoothed, as a unit-range local shape channel
  plus east and north slope channels (the fix proposed for run 5, issue 1);
- absolute height / 4,096 m;
- lakes and rivers (from L2 down). There is no sea input: the sea is ground below
  0 m connected to open water, so coastlines come from the terrain (see
  [Water](#water));
- the biome class map (one-hot, dropped 10% of the time for guidance);
- snow/ice and vegetation coverage (from L2 down);
- at L1 only, the L0 height envelope (low and high bound per 256 m cell) and
  hero-feature lines rasterized as distance fields.

Training derives every condition from the real data itself, so the model learns
how real terrain relates to a layout. For example the envelope comes from the
local minimum and maximum of the true surface at 256 m, including the seabed. At
generation time the author paints them.

### Water

Terrain continues under water (master plan §8, 2026-10-02). DEMs record water
surfaces, so training targets join GLO-30 land to GEBCO bathymetry below sea level:
L1 learns the seabed as it is (GEBCO at about 450 m against L1's 128 m), and L2 and
L3 learn a smooth seabed, the GEBCO surface upsampled and blended into the coast,
so that only the coastline position and near-shore slopes matter. Generated sea is
then the ground below 0 m connected to the field edge or a layout ocean.

Lakes and rivers come from the generated land: a drainage pass after L1 keeps the
largest closed depressions as lakes (a land share set by the layout, 2% by default)
and carves the rest into draining valleys; streams are traced on the L2 surface by
drainage area and passed to L3 as water. Generated L1 had about ten times the closed
depressions of real terrain, so this pass is required, not cosmetic.

## Data

### GLO-30 context regions (L1, L2)

Copernicus GLO-30 is global, public, 30 m, and small. Pick 6–10 regions per
terrain class, about 40–60 km across, from [the example locations](../locations.md)
and similar places. Examples: Grand Canyon and Canyonlands for gorges and mesas,
Crater Lake, Aniakchak and Ngorongoro for calderas, Fiordland and western Norway
for fjords. Store each region at 32 m (about 10 MB) with WorldCover-derived
cover at the same grid. 100 regions take about 1 GB.

Regions carry one or more class polygons drawn from obvious landform outlines. A
region can mix classes, which teaches transitions. Unlabelled parts are class 0,
not excluded ground.

GLO-30 is a surface model, so forests add their canopy height. At 32 m and above
this is tolerable, and it never supervises L3 or L4 (master plan §2).

Before training, record the GLO-30 licence in
`configs/provider-licences-v1.toml` and confirm it allows training use with
attribution.

### GEBCO bathymetry (L1, L2 below sea level)

The GEBCO global grid (15 arc-second, public, free use with attribution) supplies the
seabed under each GLO-30 region's sea. The training release reads it over HTTPS
(public CEDA GeoTIFF tiles, no bulk download) as a `seabed` layer per region, so
accepted regions and their votes stay unchanged. Training joins it where GLO-30
shows sea (0 m ground reaching GEBCO depths): the land slope continues down from the
coast to the deepest GEBCO within 1 km, and GEBCO takes over 1–3 km out, so the join
has no step or flat ring. Lakes are not filled from GEBCO.

### Reduced lidar regions (L3)

L3 needs real 4 m structure over areas larger than the current 1 m tiles (most
are 2–4 km). Use public 1 m lidar DEMs over areas of 100–400 km²: USGS 3DEP 1 m
and LINZ 1 m cover the canyon, volcanic, fjord and dune families. Read the
source COGs over HTTPS and average straight to 4 m, keeping only the 4 m result,
the source list and checksums. A 20 × 20 km region is then 100 MB instead of
1.6 GB. This is a deliberate change to the raw-storage rule: large-area reduced
sources keep receipts and URLs, not the raw files, and can be re-derived. The
existing 1 m library, averaged to 4 m, adds to this data.

Both new source kinds go through the gallery as whole-region Yes/No votes on an
overview. They use the automatic checks already in place: voids, artifacts, and
the flatness filter that skips crops more than 88% flat.

### 1 m library (L4)

The existing Ready tiles and release `diffusion-v8`. No new acquisition is
needed to start.

## Generation

Estimates for a 32 km world, from the 2026-09-29 runs (12 steps, stride 192,
about 16 windows per second on the RX 9060 XT):

| Level | Windows | GPU time |
| --- | --- | --- |
| L1 128 m | 1 | seconds |
| L2 32 m | ~25 | about 1 min |
| L3 4 m | ~1,850 | about 45 min |
| L4 1 m | ~29,000 | about 11 h |

The 12-step setting matched 32-step slope statistics on the pilot area (40.1° vs
39.7°). L4 at 32 km needs more host memory than the machine has (four
32,768² Float32 fields is 17 GB). Before the first full L4 run, generate L4 in
bands a few kilometres wide with an overlap halo, writing finished rows to a
memory-mapped output.

The owner reviews L1–L2 as a 3D overview within minutes of a layout change, and
L3 in under an hour. L4 runs once, overnight, after L3 is accepted. Regional
reruns (master plan §9) reuse the same levels on a polygon with a halo.

## Storage

- New sources: about 1 GB of GLO-30 regions and 5 GB of 4 m regions.
- Models: keep the final EMA weights and the resume checkpoint of the current run
  per level (about 1.7 GB each at the 1 m size, less above). Delete intermediate
  EMA snapshots and superseded runs' weights once a run is promoted. Keep configs,
  logs, evaluations and review records.
- Releases: keep the current release per level. Rebuild older ones from the
  gallery if needed.
- Worlds: a 32 km world's working Float32 files are about 8 GB at 1 m. Keep the
  accepted revision and its export, and delete rejected revisions after review.
- Shipped terrain: the 2 m caldera-island export was 770 MB, so a 32 km world at
  1 m should export to about 3 GB, inside the 10 GB budget.

## Steps

Each step ends with a check and, where terrain is involved, an owner review.

1. **Layout spec process.** Write the [layout spec](world-layout-spec-v1.md)
   template and extend `world sketch` to emit L0 rasters (class map, land/sea,
   envelope, feature lines) and an annotated overview, instead of a detailed
   height surface.
2. **GLO-30 context regions.** A catalog plan per class, download, reduce to
   32 m, gallery vote per region. Record the licence.
3. **L1 + L2 models.** Release builder, training (short runs, likely under 2 h
   each), blind review of real and generated 32 km (L1) and 8 km (L2) overviews.
4. **Reduced lidar regions** for L3: plans, streamed reduction to 4 m, gallery
   vote.
5. **L3 model.** Training, blind review of 1 km patches at 4 m.
6. **L4 run 5.** Retrain the 1 m model on the 4 m surface of its own tiles, with
   slope channels and cover/condition dropout (diffusion pilot issues 1–3).
   Blind review as before.
7. **Banded L4 generation** and memory-mapped output.
8. **First cascade world** from an accepted layout spec: review L1–L2, then L3,
   then run L4 overnight and export.

Steps 2–3 and 4–5 are independent and can alternate while GPU runs are going.

## Progress (2026-10-03)

- 74 GLO-30 context regions built, 64 accepted (`context build`, `context serve`).
  36 lidar regions at 4 m built and accepted (USGS 3DEP, LINZ; 3 dropped for no
  public coverage).
- L1 run 1 (128 m, 2 h): blind review 7/7. L2 run 1 (32 m, 3 h): 30/32, both No were
  open-sea windows. L3 run 1 (4 m, 3 h): 32/32.
- `cascade_chain` turns a layout derived from a held-out region (Howe Sound) into a
  48 km world at 32 m and a 16 km area at 4 m in about 20 minutes of GPU time, with
  drainage, lakes and streams.
- Owner review of the chain: clear progress, but still less real than the source;
  coasts showed a gentle strip between mountains and water. The strip came from
  treating the sea as a mask over a flat 0 m surface. Variants with a blurred
  coast (L1), a sharp coast without the sea in the loss (L1b runs 1 and 2) and the
  sea-level clean-up steps that went with them are superseded by the water approach
  above. Next: GEBCO layer, retrain L1 and L2 with continuous terrain, then rerun
  the chain.
- GEBCO seabed layer built (release `context-v2`, 64 GLO-30 regions, same split);
  L1 layout 5 drops the land/sea channel; the chain takes the sea as ground below
  0 m connected to the field edge, and heightmap export encodes the seabed.
- L2 run 2 (32/32 Yes in blind review) and L1 run 2 (7/7 Yes) trained on continuous
  terrain. Chain 7 (Howe Sound): slopes run straight into the sea with no flat
  ring, islands and drowned valleys come from the terrain.
- Water by terrain class (`hydrology.WATER`, judgment values from the regions' inland
  water share): lakes after L1, compact deep hollows kept as small lakes after L2
  (tarns; chain 9 has about 230 on Howe Sound), stream start area and width. Dry
  classes keep few lakes and narrow, rare streams (Canyonlands 0.5% water, Erg Chebbi
  about 1% before streams were narrowed). Open: rivers entering from outside the
  world (Green and Colorado in Canyonlands) need a layout input; generated mesas and
  dunes are softer than real.
- L4 run 5 (1 m on the 4 m surface of the `diffusion-v8` tiles, `train --level l4`;
  8 h, 12,484 steps): unit-range shape and slope inputs, 10% coarse and cover
  dropout, water out of the loss. Same-seed residual correlation across 8 windows
  0.29 (run 4: 0.99); mean slope 19.6° as real, curvature std 0.28 vs 0.35 m.
  Blind review 32/32 Yes (2026-10-03).
- Banded L4 (`cascade_l4`): refines a `cascade_chain` 4 m world to 1 m in row bands
  (default 2,048 m with a 256 m halo), with starting noise shared where bands overlap
  and a cross-fade over the halo. Output layers are memory-mapped and resumable per
  band; water keeps the smooth 4 m surface. A 2 km Canyonlands test shows no visible
  band joins (fade rows about 12% smoother) and exports through `export heightmap`.
  Speed about 30,000 cells/s (12 steps, stride 192): roughly 3 h for 16 km, 12 h for
  32 km. `export heightmap` streams layers row by row: a synthetic 32 km 1 m world
  exported in 13 minutes with a 164 MB peak (21,845 tiles, 8 LOD levels).
- First cascade world: Howe Sound derived layout, 32 km. L1 10 s, L2 2.5 min, L3
  (8,192² at 4 m, per-window conditioning) 59 min →
  `data/worlds/cascade-world-1-howe-sound`, 4 m export for review. Awaiting the
  owner's look before L4.

- **Class ignored (2026-10-03).** A class-swap test on the painted Fujion world showed
  that L1, L2, L3 and L4 all ignore the terrain class (correlation between classes
  0.99–1.00, against 0.33–0.67 for a seed change), so every biome got the same
  texture. The blind reviews could not catch it because they generated from real
  coarse inputs. Retraining with a class signal the models cannot ignore, new gates
  and more data per class: [class conditioning](class-conditioning-v1.md).

## Gates and fallbacks

- Every level must pass the class-swap and class-recognition gates of the
  [class conditioning plan](class-conditioning-v1.md) before its blind review.

- If L1/L2 cannot reproduce recognizable large landforms after two runs, compose
  L2 from real GLO-30 patches placed per the layout and blended (master plan §8)
  and continue at L3.
- If L3 is not accepted, run L4 on the L2 surface as today, knowing towers and
  cliff bands stay muted.
- If generated worlds repeat features across windows (the 0.99 correlation seen
  in run 4), fix it at the level where it appears before continuing down.

## Open choices

- L1 cell size: 128 m (one window for the world) or 64 m (512², more detail,
  window joins). Start at 128 m.
- Whether L2 and L3 share one scale-conditioned model. Start with separate small
  models; merge only if training time becomes the bottleneck.
- Step counts per level. Coarse levels may need fewer than 12.
