# Expert review, 2026-10-07

Two read-only reviews made after the owner's look at the 32 km Fujion world at 1 m
(`fujion-painted-chain-7-l4`, export `-v4`: L1 run 4, L2 run 6, L3 run 2, L4 run 6):

- **Machine learning:** Claude Opus 5.5. Measured all `context-v5` and `lidar-v3`
  regions against `fujion-painted-chain-7` on CPU.
- **Geography and geomorphology:** Codex `gpt-6-astra`.

The two reviews agree on almost every point. This report merges them and keeps only
the points that change the plan. File references are to commit 6bc621e. **[V]** marks
findings checked in code or by measurement; **[I]** marks inferences.

## Owner feedback

Biomes still look alike. In particular:

- Canyons have no Grand Canyon-style cliff bands.
- Dunes have no wave-like crest trains.
- Karst has no cone hills.
- The archipelago has no drowned karst.
- The summit crater is invisible, and the north-east cone chain cannot be seen at all.
- Glaciers look like rock.
- Tree and snow lines are too straight.
- Rivers are straight, angular, broken and often stop before the sea.

## Main findings

### 1. Roughness is right now; shape is not [V]

The class gates check residual standard deviation, and that now roughly matches real
terrain at L2 and L3. The landforms fail on statistics that no gate measures. Each
pair below is the real class median against the Fujion value:

| Class | L1 incision | L1 peaks / 100 km² | L3 flat share | L3 coherence |
| --- | --- | --- | --- | --- |
| 1 Karst | 0.29 / 0.01 | 44 / 30 | 0.07 / 0.05 | 0.20 / 0.19 (peaks/km² 25 / **197**) |
| 2 Canyons | 0.26 / 0.03 | 31 / 28 | 0.14 / 0.02 | 0.32 / 0.18 |
| 5 Archipelago | 0.12 / 0.00 | 51 / 5 | — | — |
| 7 Badlands, mesas | 0.07 / 0.00 | 22 / 4 | 0.35 / 0.05 | 0.21 / 0.20 |
| 8 Dunes | 0.13 / 0.00 | 34 / 3 | 0.35 / 0.05 | 0.26 / 0.18 |
| 10 Glaciers | 0.31 / 0.00 | 19 / 0 | 0.02 / 0.00 | 0.29 / 0.18 |
| 11 Cones, lava | 0.09 / 0.01 | 16 / 7 | 0.09 / 0.02 | 0.33 / 0.19 |

The measures are:

- **Incision:** the share of cells more than 50 m below a 1.1 km closing. It counts
  narrow valleys, bowls and craters.
- **Flat share:** the share of cells with slope under 3°.
- **Coherence:** the anisotropy of the structure tensor. Higher means the relief runs
  in one direction.

Every generated class comes out the same way: sloped everywhere, with no flats (mesa
tops, benches, interdune floors), no narrow incision, too few distinct peaks at L1, and
fine texture that has no direction. That shared character is why the biomes look
alike.

### 2. L1 must make most missing landforms, and it is the weakest level [V]

L2 adds only relief smaller than about 200 m (`cascade.py:56,373`). Calderas, cone
fields, dune trains, cone karst, and canyon width and depth all lie between 200 m and
2 km, so they must come from L1. L1 sees the painting only through the 2 km envelope.
It has four problems:

- **The class embedding is a 12-way blend.** Fujion's L1 is a single 256² window, and
  the class embedding is that window's mean class share (`model.py:53-62`). The largest
  share is 0.17, and calderas get 0.002. Training windows hold one or two classes, so
  the model never saw a blend like this in training.
- **It has no AdaGN.** L1 run 4 predates `--scale-shift`.
- **It has little data.** Each region yields about one independent 256² window: about
  120 scenes for 12 classes. The run stopped at 5.7k steps.
- **The painted crater is erased.** The painted summit crater is about 600–800 m wide
  and 190 m deep; its floor is painted at 3,554 m and comes out at about 3,700 m. There
  are two causes:
  - The hold works at σ = 384 m (`cascade_chain.py:40`), which low-passes the crater
    away.
  - The L1 drainage pass gets no dry-basin allowance (`cascade_chain.py:280`), so it
    carves or fills any hollow it does not keep as a lake.

### 3. The labels are diluted and mix landforms of different scales [V]

- **Labels are constant per region.** Training fills every window with its region's
  class (`cascade.py:524`). This discards the spatial class raster the context builder
  writes (`crates/topography-core/src/context/mod.rs:392`).
- **Background ground counts as the class.** About 30–40% of the 1 km windows in
  calderas, cones, dunes, badlands and canyons are quiet background.
- **Glacier regions are mostly rock.** Ice covers only part of each lidar glacier
  region: Franz Josef 43%, Tasman 29%, Baker 21%, Glacier Peak 15%, Jefferson 3% and
  Three Sisters 0%.
- **One class covers incompatible landforms:**

  | Class | Mixes |
  | --- | --- |
  | 1 Karst | Cone and tower karst (Puerto Rico, Yangshuo) with doline plains and NZ karst |
  | 2 Canyons | Layered plateau canyons with massive-rock gorges (Taroko, Hells Canyon, Black Canyon) |
  | 4 Calderas | 5–20 km collapse calderas, while Fujion asks for an 800 m summit crater |
  | 5 Archipelago | Drowned karst (Ha Long, Phang Nga) with rias, skerries and NZ/Maine islands (all of the lidar) |
  | 8 Dunes | Crest-train ergs with star dunes, sand sheets and the rocky ground around them |
  | 11 Cones, lava | Big stratovolcanoes at L1/L2 with cinder-cone fields at L4; the NE rift cones are painted as 11 |

  Validation regions hide this. For example, Hells Canyon validates "canyon" and
  Punakaiki validates "karst", but neither shows the look the owner wants.

### 4. Texture has no direction [V code, I effect]

Dihedral augmentation (`cascade.py:539`) leaves the model no orientation input. Each
MultiDiffusion window chooses its own crest or strata direction, and averaging
overlapping windows cancels anisotropic patterns. This hurts dune trains, strata bands
and parallel gullies most.

### 5. Drowned karst cannot appear by construction [V]

- **The painter flattens the sea.** It fades offshore relief to zero for every class
  except 5 (`paint.py:287-290`). Fujion's drowned karst is painted as class 1 at
  −67 to −211 m, so its relief is 0.
- **Training seabeds are smooth.** They are GEBCO joined to the coast.
- **L3 ignores the seabed.** Below −5 m, L3 is replaced by L2.

### 6. Rivers break in post-processing [V]

- **L4 deletes narrow streams.** It enlarges the 4 m water mask, blurs it at σ 3 m and
  thresholds at 0.5 (`cascade_l4.py:156`, `cascade.refine_mask`). A stream one 4 m cell
  wide peaks at 0.49, so the whole channel disappears. This explains most
  "interrupted" streams.
- **Mouths are stranded.** Streams end at L2's shoreline. When L3 moves the coast,
  `cascade_chain.py:365` clips the streams but does not extend stranded mouths.
- **Channels are D8 lines.** Routing uses 8 directions on 32 m cells, and
  `draw_streams` joins cell centres with straight segments (`hydrology.py:78,126`). The
  result looks straight and angular.
- **Nothing keeps the channel downhill.** No step forces the ground under a stream to
  descend after L3.

### 7. Cover lines are elevation contours [V]

- **The lines are contours.** `cover.py:78` uses a hard snowline, which is a
  windward-to-lee plane, and one global treeline. The recent warp only moves these
  contour lines by about 150 m.
- **Bug: substrate is overwritten above the treeline.** `landcover.py:42` turns dunes
  and lava at 1,200 m into grassland when the treeline is 1,000 m.

## Suggestions

### A. Fixes without training (do first)

Each needs only code; L1 and L2 for Fujion take about a minute per chain to test.

1. **Rivers:**
   - Rasterize connected centrelines with widths, so L4 rounds banks without deleting
     channels.
   - Extend stranded mouths to the final L3 coast.
   - Smooth D8 paths inside their valley corridor.
   - Make stream ground descend monotonically downstream.

   About 1–3 days. Existing elevation can be reused for a mask-only repair.
2. **Cover:**
   - Base snow persistence and the treeline on smoothed elevation plus aspect, slope,
     exposure and curvature. Snow then holds in sheltered pole-facing hollows and
     sheds from crests and steep faces, and forest gets ragged edges and fingers up
     gullies.
   - Add modest noise and transition bands.
   - Keep glacier extent separate from the snowline.
   - Share one rule between `cover.py` and `landcover.py`, and fix the substrate bug.
   - Optionally fit a small per-pixel model on the real land-cover and snow layers
     (CPU minutes).

   About ½–1½ days.
3. **Crater:**
   - Hold crater and caldera cells at about 128 m rather than 384 m.
   - Exempt volcanic, karst and dune hollows from drainage at L1 as well as L2.
   - Keep an ice-filled crater as ice, not a lake.

   Hours.
4. **Drowned karst:**
   - Keep painted relief for drowned-karst cells: paint them as a class exempt from
     the sea fade.
   - Set their painted seabed shallow, about −10 to −40 m, so L1 and L2 raise towers
     above water as they do in the Ha Long and Phang Nga training regions.

   About an hour.
5. **Exemplar hold for hero landforms:** where a caldera or cone field is painted,
   hold a real 128 m residual taken from the GLO-30 releases (Crater Lake, Okmok,
   Aniakchak; cone fields from San Francisco volcanic field or Chaîne des Puys),
   scaled to the painted blob. Large forms then come from real data, the owner still
   paints only a class, and the model blends the seams. This is also the cheapest
   form of the curated-patch fallback. About 1 day.
6. **Dune relief:** lower the default painted dune relief. 150–190 m is far above the
   real dune residual of about 40 m at L1. Minutes.
7. **Class guidance:** try w = 2–3 in the chain, which uses none today. Minutes.
   Expect little until L1 windows are local (B3).

### B. Data and labels

1. **Labels per cell, not per region.** Make training read the spatial class raster.
   Label glacier cells from the release's snow-ice layer and give the rest of those
   regions the alpine class. Relabel each class's quietest windows (about 30%, by
   automatic form measures) as a new background class, "plain / rolling", so quiet
   ground stops counting as canyon or dune. This also gives the painter an explicit
   quiet class. About half a day of code; no new data.
2. **Subclasses where landforms differ in kind or scale.** Keep the 12 parent IDs and
   add subclass IDs to the palette, as one-hot classes:

   | Parent | Subclasses |
   | --- | --- |
   | 1 Karst | 1a cone/tower (Puerto Rico, Yangshuo, Wanfenglin, Chocolate Hills, Cockpit Country); 1b doline plateau |
   | 2 Canyons | 2a layered plateau canyon (Grand Canyon, Canyonlands, Zion, Escalante, Canyon de Chelly, Fish River); 2b gorge (Taroko, Hells, Black Canyon) |
   | 4 Calderas | 4a collapse caldera; 4b summit crater |
   | 5 Archipelago | 5a drowned karst (Ha Long, Phang Nga, Wayag, Palau); 5b ria/skerry |
   | 8 Dunes | 8a crest-train/crescent erg (Algodones, White Sands, St Anthony, Great Sand Dunes); 8b star/megadune; rocky desert and sand sheets leave the class (to background or badlands) |
   | 10 Glaciers | per-cell ice only; later 10a clean ice, 10b debris-covered |
   | 11 Cones, lava | 11a stratovolcano; 11b cinder-cone field (Sunset/SP Crater, Craters of the Moon, Mauna Kea summit, Mono) |

   The other classes keep one ID until they fail. Tags without a model input have no
   effect, so the subclasses are real classes.
3. **Refocus existing sources before adding new ones.** Move gorges, doline plains,
   rocky desert and mostly rocky glacier regions out of the subsets they dilute,
   without deleting or reversing any vote. Add about 30–40 GLO-30 regions for the new
   subclasses (cone fields, layered canyons, crest-train ergs, drowned karst) through
   the existing plans and an hour or two of owner Yes/No votes. ArcticDEM 2 m strips are
   a possible later L3 source for clean glacier surfaces.
4. **Report effective counts.** For each subclass, report feature-bearing windows and
   independent regions, not just the number of regions.

### C. Model changes

1. **Local L1 windows.** Train L1 on 96–128-cell crops with MultiDiffusion at stride ½
   and `--scale-shift`, for 15–20k steps. This gives 5–10× more independent windows per
   region and local class embeddings. A 64 km world needs several L1 windows anyway.
   Add 3–4-class mosaics to the training windows. About 3–5 h of training.
2. **Form channels (generalising the planned roughness channel), L1–L3.** These are
   per-cell descriptors smoothed over about ¼ window:
   - log residual sd;
   - flat share;
   - peak density;
   - incision or closed-depression share;
   - orientation, as coherence × (cos 2θ, sin 2θ).

   In training they come from the truth, with 20–30% dropout, and the rotation
   augmentation rotates the orientation channels. At generation they come from a
   per-subclass table of real medians plus low-frequency variation; the painted
   roughness layer overrides sd, and the layout's wind direction sets dune
   orientation. Descriptors are learned from every region, are automatic, and do not
   blend across a window. 1–2 days of code, then retraining L1 (3–5 h), L2 (6–8 h) and
   L3 (6 h).

   The geography review cautions that descriptors set statistics but cannot create
   layered cliffs or crest organisation without focused data. Do B before C.
3. **Keep one model per level.** Per-subclass LoRA at L1/L2 is the fallback if form
   channels fail: generation stays in minutes. Per-class models at L3/L4 would make
   L4 about 45 h per 32 km world, so they are ruled out.
4. **Profile training throughput.** 0.5 steps/s at 6 GB VRAM suggests the CPU batch
   build is the limit [I]. A DataLoader with workers under the 7 GB cap could double
   steps per hour.

### D. Evaluation

- **Gate on a form vector, not sd alone.** The vector is flat share, steep share,
  peaks, incision, coherence and the slope histogram, at each level's scale. Section 1
  is the template.
- **Report form on the painted world.** Run an automatic form report of the Fujion
  chain at L1–L2 (seconds) against the real subclass medians.
- **Check landforms at the right level:**
  - L1/L2: crater enclosure, the cone population, canyon organisation, towers above
    water, and dune fields.
  - L3: cliff-band profiles, dune crests, tower spacing and the glacier surface.
  - Water and cover: rivers connected to the sea, narrow channels that survive, and
    irregular cover lines.
- **Owner review stays short:** a class grid and the L2 hillshade, Yes/No, before any
  L3 or L4 hours.

## Suggested order before the 64 km attempt

1. **A1–A4 and A6–A7**, checked on the existing chain-7 world and on fresh L1–L2
   chains. About 3–5 days of code; no training.
2. **A5 exemplar hold** for the summit crater and the NE cone chain. About 1 day.
3. **B1–B4:** relabelling, subclasses, new regions and owner votes. About 2–3 days.
4. **C1, C2 and C4, plus the form gate (D).** About 2–3 days of code.
5. **Retraining, in order:**
   - L1, 3–5 h;
   - L2, 6–8 h, judged on Fujion L1–L2 in minutes;
   - L3, 6 h, judged on 4–8 km crops of each biome, not the whole 32 km.
6. **Per-subclass LoRA at L1/L2,** only if forms still fail.
7. **L4:**
   - retrain with the new labels, 8 h;
   - check 4 km crops per biome, at 15–30 min each;
   - regenerate the 32 km world, about 15 h.
8. **Then 64 km.** L4 scales with area: about 60 h at the current speed. Consider
   fewer L4 steps, or 1 m detail only where it is seen up close.

Expected total: about 1½–2 weeks of work, about 25 GPU h of training and an hour or
two of owner votes before the next full-world L3/L4 run.

## Proposed changes to plans and docs

- **[Class conditioning plan](../plans/class-conditioning-v1.md):**
  - Add these findings.
  - Make form gates replace sd-only gates.
  - Replace fallback 1 (roughness channel) with form channels.
  - Add per-cell labels and subclasses.
  - Update the outdated Data table to `context-v5`, `lidar-v3` and `diffusion-v9`
    counts.
- **[Terrain cascade plan](../plans/terrain-cascade-v1.md):**
  - Give the level responsibilities: L1 owns forms from 200 m to 2 km.
  - Change L1 to local windows.
  - Add the exemplar hold.
  - Add river continuity and profile steps.
  - Specify drowned-karst seabed handling.
- **Label legend:** a `terrain-v3` legend and GIMP palette with the subclasses and the
  background class. The [master plan](../plans/terrain-design-master-plan.md) class
  list should follow it.
- **[World layout spec](../plans/world-layout-spec-v1.md) and paint README:**
  - Paint subclasses.
  - Make drowned karst a shallow painted seabed with relief.
  - Paint a summit crater with the 4b subclass rather than a height dent alone.
- **Cover:** document the terrain-aware snow and tree rules in the surface-cover and
  terrain-export docs.
