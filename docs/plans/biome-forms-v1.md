# Biome forms v1

Status: in progress (2026-10-07). This plan carries out steps 1–3 of the
[expert review of 2026-10-07](../reports/expert-review-2026-10-07.md), which the owner
chose after looking at Fujion at 1 m. It refines the
[class conditioning plan](class-conditioning-v1.md); the
[terrain cascade plan](terrain-cascade-v1.md) still defines the levels, data and water.

## Goal

Each painted biome should come out as its landform, and look like the place the owner
has in mind. Painted layout classes say "this area is like the Grand Canyon", "that
area is badlands and mesas". Rivers should reach the sea, and tree and snow lines
should follow the terrain.

The owner's constraint: **no new source regions.** The models learn the subclasses
from the regions, lidar areas and 1 m tiles already accepted. The labels change; the
data does not.

## Step 1: fixes without training

Each fix is checked on Fujion with fresh L1–L2 chains (about a minute each) and on 4 m
and 1 m crops.

1. **Rivers.**
   - L4 keeps every stream: the 1 m water mask keeps the drawn channel instead of
     blurring a 4 m line under its 0.5 threshold.
   - After L3, streams whose mouth no longer reaches water are continued downhill to
     the sea, a lake or a dry basin.
   - Stream paths are smoothed inside their valley, so they stop following 8-direction
     grid lines at 32 m.
   - Ground under a stream descends monotonically downstream.
2. **Cover lines.** Snow and the tree line follow the terrain, not a contour:
   - a transition band with low-frequency noise;
   - aspect: snow lasts longer on sheltered, pole-facing slopes (south-facing in the
     southern hemisphere; set by the layout's `hemisphere`, north by default);
   - slope: steep faces shed snow, and trees thin out on them;
   - curvature: snow and trees run further along gullies, while ridges stay bare.

   Painted glaciers stay ice. `cover.py` and `landcover.py` share one rule.
   `landcover.py` no longer turns dunes or lava above the tree line into grassland.
3. **Craters.**
   - The painted hold works at 128 m (σ 1 cell) on crater and caldera cells, so it
     does not blur a 600–800 m crater away.
   - L1 drainage keeps volcanic, karst and dune hollows as dry basins, as L2 already
     does.
   - An ice-filled crater stays ice, not a lake.
4. **Drowned karst.** Painted cells of the new drowned-karst class keep their painted
   relief offshore, and their seabed is made shallow (−10 to −40 m). L1 and L2 then
   raise towers above the water, as in the Ha Long and Phang Nga regions.
5. **Dune relief.** The default painted relief for dunes drops from 150 m to 60 m (real
   L1 dune residual is about 40 m).
6. **Class guidance.** `cascade_chain --guidance w` uses the class guidance the models
   were trained for (10% class dropout). The chain default stays 1.0, off, until a
   sweep shows a gain.

## Step 2: exemplar hold

Small hero landforms come from real terrain:

- **Where:** wherever a summit crater, caldera, cinder cone or drowned tower karst is
  painted.
- **What:** the 128 m residual of a real example from the GLO-30 releases.
- **How:** L1 holds the example at about 128 m, scaled to the painted blob, and the
  model blends the seams.

The examples are listed in `catalog/exemplars/exemplars-v1.toml`: region, position,
size and kind (crater, caldera, cone, cone field, drowned karst; found automatically
and checked by eye). Picking among them is seeded. This is
also the cheapest form of the curated-patch fallback. The owner still paints only a
class and a height.

## Step 3: subclasses and per-cell labels

### Legend v3

`catalog/labels/terrain-v3.toml` keeps IDs 1–12 and adds:

| ID | Subclass | Parent | Fallback |
| --- | --- | --- | --- |
| 13 | Layered plateau canyons ("Grand Canyon") | 2 | 2 |
| 14 | Cone and tower karst ("mogotes, Yangshuo") | 1 | 1 |
| 15 | Drowned tower karst ("Ha Long Bay") | 5 | 14, 5 |
| 16 | Crest-train dunes ("Algodones, Taklamakan") | 8 | 8 |
| 17 | Cinder cones and lava fields ("Sunset Crater") | 11 | 11 |
| 18 | Plain and rolling ground | 0 | none |

The parents keep the remaining sources: 1 is doline and pinnacle karst, 2 gorges, 5
rias and skerries, 8 star and mountain dunes, 11 stratovolcanoes.

A model is trained on the classes that have data at its level. Every other class maps
to its first fallback that does: for example, drowned karst uses cone karst at 4 m and
1 m, where no drowned-karst lidar exists. The run config records this map, so a
painted subclass always reaches a class the model knows. Old 13-class runs map
subclasses to their parents.

### Assigning regions

Subclasses are assigned per region, lidar area and 1 m tile in
`catalog/labels/subclasses-v1.toml`. That is the owner's "this area is like the Grand
Canyon" applied to the existing data. Highlights:

- **13 layered canyons:**
  - GLO-30: grand-canyon, fish-river-canyon, blyde-river and canyonlands (moved from
    7), zion;
  - lidar: grand-canyon, canyon-de-chelly, escalante, zion and canyonlands;
  - 1 m tiles: Grand Canyon, Zion and Island in the Sky.
- **14 cone/tower karst:**
  - GLO-30: Yangshuo, Wanfenglin, Vang Vieng, Chocolate Hills, Cockpit Country and
    others;
  - lidar and 1 m tiles: the Puerto Rico mogotes.
- **15 drowned tower karst:** GLO-30 ha-long-bay, phang-nga-bay, el-nido and misool.
- **16 crest-train dunes:**
  - GLO-30: taklamakan, skeleton-coast-dunes, liwa and grand-erg-oriental;
  - lidar: algodones, white-sands, st-anthony and coral-pink, and their tiles.
- **17 cinder cones:**
  - lidar: craters-of-the-moon, sunset-crater and mauna-kea;
  - all 1 m class 11 tiles: SP Crater, Sunset, Mono Craters, Craters of the Moon and
    Lava Beds.
  - GLO-30 has only craters-of-the-moon, so L1/L2 fall back to 11 and use the exemplar
    hold.

### Per-cell rules

Applied when training fields are built:

- **Glacier:** cells with ice (snow-ice share above 0.5 after about 100 m of smoothing)
  are class 10 in every region. Ice-free ground in glacier regions becomes class 6.
- **Plain:** land cells whose local relief (range within about 1 km, measured at 32 m)
  is below a third of the region's 90th percentile become class 18. Excluded:
  - braided rivers, whose flat floors are the point;
  - water;
  - ground below sea level.

  Flats next to canyons, mesas or dunes keep their class, because they lie within
  1 km of relief. Distant plateau, sand sheet and rocky desert become plain.

The class of the region still sets class-balanced sampling and mixed-class windows.

### Training

The legend v3 runs have 19 class channels. The cached training fields gain a per-cell
class layer, so `FIELDS_VERSION` is bumped. `train.py` records `num_classes` and the
class map. Retraining (L1, L2, L3, then L4) is the next step and needs the owner's go.
Each run is judged with the existing gates per subclass and on Fujion repainted with
the subclasses.

## Fujion

`data/worlds/fujion-paint-v3/` is the owner's painted layout with the subclasses
filled in from the owner's notes:

- the canyon plateau is 13;
- the karst arm is 14, and the towers in the sea are 15;
- the dunes are 16;
- the NE rift cones are 17;
- the summit crater and its caldera ring use the exemplar hold.

The owner may repaint it. The old paint dir is kept.

## Status (2026-10-07)

Steps 1–3 are implemented. They were checked with the current 13-class models on Fujion
repainted to legend v3 (`data/worlds/fujion-paint-v3`, L0 `data/worlds/fujion-painted-l0-3`,
chain `data/worlds/fujion-v3-chain-6`, export `data/exports/fujion-v3-chain-6`; L3 on the
central 16 km):

- **Streams:** every stream in the L3 square reaches the sea or the square's edge. The
  only inland water bodies left are standalone lakes and tarns. In chain 7, 6 of 17
  water systems in the same square reached the sea or the edge.
- **Summit crater:** at 32 m it is a closed hollow 55 m deep and about 600 m wide, held
  dry (chain 7: 11 m). The exemplar is now scaled to the painted depth (156 m against
  161 m painted). The flank cone has a crater 221 m deep.
- **Drowned karst:** the seabed is shallow (median −56 m, chain 7 −139 m). Ha Long
  exemplars raise 8–9 tower islands up to 311 m. Without the exemplar, the old models
  turned the shallow bay into a flat plain.
- **Cover:** snow and forest edges follow aspect, gullies and slope, with patchy edges.
- **Labels:** per-cell labels split the regions as intended:
  - Grand Canyon: 57% canyon, 43% plateau plain;
  - Erg Chebbi: 42% dunes, 58% rocky plain;
  - Franz Josef: 45% ice, 53% alpine.
- **Trained classes per level:**

  | Level | Not trained, so painted cells use a fallback |
  | --- | --- |
  | L1, L2 | 1 (→14), 17 (→11) |
  | L3 | 2 (→13), 8 (→16), 15 (→14) |
  | L4 | 1 (→14), 11 (→17), 15 (→14), 18 (→0) |

- **Code paths:** a 20-step training smoke at L1 and L2, the chain, the gates and the
  evaluation all ran with a 19-class run.

### Proposed retraining (needs the owner's go)

Each run uses the current recipe of its level on legend v3. Runs that start from scratch
rebuild their cached fields first; L1–L3 caches are already built.

| Run | Release | Recipe | Time | VRAM |
| --- | --- | --- | --- | --- |
| L1 run 5 | `context-v5` | L1 run 4 plus `--scale-shift` | 4 h | ~6 GB |
| L2 run 7 | `context-v5` | L2 run 6 | 6 h | ~6 GB |
| L3 run 3 | `lidar-v3` | L3 run 2 | 6 h | ~6 GB |
| L4 run 7 | `diffusion-v9` | L4 run 6 | 8 h | ~9 GB |

Order:

1. L1 and L2, then gates and a Fujion L1–L2 chain (minutes) for the owner to look at.
2. L3, checked on 4–8 km crops per biome.
3. L4, checked on 4 km crops.
4. Then a full 32 km world.

### Volcano repair (2026-10-07)

The painted volcano was a perfectly radial cone, up to 1,343 m high. It reached into the
fjords, alpine and karst biomes and left a hard arc, together with a round glacier
patch. `paint transplant` replaces it with Fuji's real relief, scaled to the painted
cone (radius 8.4 km, scale 0.70, summit 3,775 m) and faded out at the edge of the volcano
biome. The glaciers are redrawn on the real cone. The current layout is
`data/worlds/fujion-paint-v3b` (L0 `data/worlds/fujion-painted-l0-4`). The south bay's
smooth painted arc was then broken up:

- `paint roughen` adds bays and headlands of a few kilometres;
- the Ha Long exemplar now also reaches the bay's shores, adding towers, coves and
  islets.

The current layout is `data/worlds/fujion-paint-v3c` (L0 `data/worlds/fujion-painted-l0-5`, L1–L2 check
`data/worlds/fujion-v3-chain-9`).

## Not in this plan

The other review suggestions wait for the results of the retraining:

- form channels;
- local L1 windows;
- per-subclass adapters;
- new source regions.

The 64 km world comes after the biomes are accepted.
