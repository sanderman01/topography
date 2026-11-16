# Flaws in exported terrain and mask data

Found by the airship game while drawing exports. Each entry says what is wrong, where it was
seen, how the game works around it, and what the next export should do instead. Map
coordinates are metres east and south of the map's north-west corner. Entries for older exports
(`fujion-v3-chain-14-l4`) and the water data contract they led to are in Git history; the
contract is implemented since `fujix-55-l4v9`.

## Export `fujix-55-l4v9`

Observed on 2026-11-15 while moving the game to `fujix-55-l4v9` (65.5 km at 1 m, schema 2). The
earlier water flaws are fixed in this export: real lake beds, one level per lake at every level,
continuous river mouths, a surface extended past the shore, and every validation check at zero.
What is left:

### `lakes[].outline_m` disagrees with `water_bodies[]` and the tiles

In `description.json`, some `lakes[]` entries have an `outline_m` far from the lake. The
`water_bodies[].bounds_m` of the same body agree with the `water-body` tiles.

- Seen: lake body 688 (level 1060 m). Its `outline_m` is near (47590, 25810), where the ground is
  445 m and there is no water. Its `bounds_m` are x 43059..51005, y 43444..46564, as in the tiles.
  Lakes 417 and 644 show the same mismatch.
- Workaround: the game reads `water_bodies[]` (kind, `level_m`, `bounds_m`) and ignores
  `lakes[].outline_m`.
- Fix: write both from the same body, in export ids, and validate that each outline lies inside
  its body's bounds.

### Class layers are white-noise dithered, and the mode filter does not preserve the dither

At L1 (2 m), `biome` and `land-cover` borders are an independent random draw per texel along a
probability ramp: about 100 m wide for biomes, 45 to 55 m for cover, with isolated texels up to
300 m into the other class. Coarser levels take the mode, which collapses the ramp to an edge of
1 to 4 texels and drops sparse classes.

- Seen: `biome_L1_3_21` against `biome_L3_0_5`; `landcover_L1_50_40`. Over one 2 km square,
  grassland (cover 30) is 6.0% of L1 and 3.5% of L3.
- Effect: a blend drawn from the tiles changes width and class mix with the level of detail, so
  colors and plant density pop when tiles split.
- Workaround: the game filters one-hot class channels itself over a fixed radius in meters.
- Fix: export the blend, not a sample of it. For example the two strongest class ids plus a
  weight per texel, or a fraction per class, area-averaged per level like coverage. If the dither
  stays, state its ramp width in the manifest and build coarse levels from the ramp, not from the
  mode of the dithered texels.

### The sea surface decodes to -0.006 m

`water-surface` shares the height encoding (`h_min_m -498.777`, `step_m 0.0835185`), in which 0 m
is not a code. The sea is code 5972, which decodes to -0.006 m. Lake levels are quantized the same
way, by up to 0.042 m.

- Workaround: the game takes sea and lake levels from `water_bodies[].level_m` by body id.
- Fix: either keep it and say so in the manifest (flat bodies take `level_m`, the tile value is
  approximate), or choose `h_min_m` so that 0 m falls on a code.

### Minor

- Coarse levels have wet texels whose mean ground is above the mean surface
  (`coarse_wet_below_ground` in `report.json`: 156k texels at L1, 151 at L8). Expected from area
  means in narrow channels; the game clamps depth to zero there.
- `moisture` exists only at L7 and L8, and the class and density layers start at L1, normal and
  roughness at L2. The game reads an ancestor tile for finer levels. Fine as is; listed so it is
  a decision, not an accident.
- 79 `ice` bodies are exported as a flat surface at the lake level; the game has no ice material
  yet.
- `normal` and `roughness` have no apron (`apron_texels: 0`), while every other layer has 4. A
  filtered lookup clamps at the file edge, so neighbouring tiles meet with a step of one cell's
  change. The files are BC-compressed, so the game cannot assemble a window across files on the
  CPU. A one-texel apron (padded to the BC block size) would remove the step.

### Found while drawing the water and terrain (same export)

- **Axis-aligned notch at the main river's mouth.** The sea mask at about (43253..43300,
  56800..56850) has a straight north-south edge 40 m long and a straight top, with a round pool
  3 m deep inside. It shows from 600 m up.
- **Rills on steep faces.** 10 977 river bodies; on the face at (18358, 53006) about twenty
  parallel streams 1 to 2 m wide run at slope 2.0. Prune by catchment or discharge, or mark them
  so the game can draw them as wet rock.
- **Steep final reaches.** The main river (body 8437) drops 22.5 m over its last 88 m. Rivers no
  longer reach the coast above sea level, but the grade into the sea is a torrent.
- **River surface in 8 cm steps.** `water-surface` shares the height code (0.0835 m), so calm
  rivers are staircases; the game averages over 6 m. A finer step for this layer, or a float
  layer, removes that.
- **Surface extension without a body.** The 4 texels past the shore have body 0 and step up to
  4.9 m between neighbours. Carry the body id into the extension and extend along the channel.
- **Diagonal one-texel streams** are 8-connected only at L0. Make channels 4-connected or two
  texels wide.
- **`snow_ice` and `tree_cover` are dithers of the snow and vegetation fractions.** The game
  ignores `snow_ice` and draws forest from `vegetation-density`. Say in the legend which classes
  derive from a fraction, or stop dithering them.
- **`vegetation-density` scale.** Closed forest is 0.65, found by measurement. State what 1.0
  means.
- **`roughness`** changes shading by under 2 %; the game no longer reads it. Drop it or encode
  something with range.
- **Flow.** Width, discharge or a falls flag per reach would replace the game's slope thresholds
  for rapids and falls.
- **Class levels and distance.** With mode-filtered class levels the game reads L1 classes up to
  L2 tiles; from L3 (about 3 km) the class mix shifts: 5.7 % mean luminance on dunes and cinder,
  25 % of a border band changes by over 10 %. Class fractions per level remove it.

### Found while tuning biomes and land cover (same export, 2026-11-16)

- **`talus-density` is a slope band, not deposition.** Measured at L4 over the whole map: mean
  density by 10° slope bin is 0.001, 0.004, 0.043, 0.35, 0.205, 0.012, 0.001 (0° to 70°). Within
  the 25° to 40° band, ridges (top 10 % of height minus the 112 m local mean) carry 0.42, valley
  floors 0.39 and mid-slopes 0.20; footslopes at 10° to 25° carry 0.003. In the game this puts
  rubble and boulders along ridge crests and almost none in gullies or at the foot of cliffs.
  Talus should come from a source and a sink: debris sheds from steep or exposed rock (cliffs,
  crests) and collects downslope where the grade drops below the angle of repose, in gullies,
  on fans and on valley floors. Crests and convex ground should be near zero.
- **No vegetation along water in the dry biomes.** `vegetation-density` is exactly 0 in
  `badlands`, `star_dunes`, `crest_dunes` and `grand_canyon`, including within 16 m of rivers and
  lakes (about 86 000 bank texels at L3). Add a narrow riparian band, 10 to 40 m wide, of modest
  density (0.1 to 0.3) around lakes and perennial rivers there, and a `land-cover` class to
  match. The game adds its own band from the water fields for now.
- **No temperature layer, and `moisture` only at L7 and L8 (128 m).** The game picks plant
  species by biome alone, so southern karst and western fjords differ only by biome palette. A
  `temperature` layer (latitude, altitude, coast) and `moisture` down to about L4 would let it
  pick species by climate: tropical in the warm wet south, conifers on cold slopes.
