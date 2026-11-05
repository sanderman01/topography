# Hydrology v3: one water field at every level

Status: design approved in chat 2026-11-05; spec awaiting owner review.
Diagnosis behind it: `scratch/hydrology-v3-expert-diagnosis.md` (not committed).

## Why

The finished fujix 1 m world (`data/worlds/fujix-55-l4v2`) shows these defects:
- dam-like steps where rivers leave lakes;
- doubled rivers in the real canyon;
- straight and 45° rivers;
- faceted planes;
- over-carved real terrain.

Each one comes from a rule in the current object-based hydrology:
- breach slots cut to the spill minus 30 m at 32 m;
- lips raised at 4 m lake exits;
- 4 m river paths imposed on 1 m ground;
- shortest-path routing (`fine.rs::route_descending`);
- 35° ring-by-ring trench widening;
- frozen-north band seams.

The dams and the E43966 straight river come from the 32 m and 4 m stages, and the 1 m
diffusion copied them into `elevation-raw.f32`. A 1 m-only change cannot remove them.

## Goal

Plausible game terrain where water behaves like water. Water is a single field: every cell has
a ground height, a water surface (or none) and a discharge. Lakes and rivers are labels read off
that field, never objects that shape it. No rule raises banks, cuts slots or imposes paths, and
no rule exempts a check.

Owner decisions (2026-11-05):
- **Terrain change:** the water places itself on the diffusion terrain and cuts only light
  channels. There is no full erosion simulation at 1 m.
- **Coarse guides, fine decides:** the 4 m world fixes the rough place and size of big rivers
  and lakes. Their exact course and shoreline come from the 1 m terrain. Small streams and ponds
  are 1 m only.
- **Rerun:** regenerate from the 32 m hydrology through the 4 m and 1 m diffusion (about
  35–41 h of GPU).

## The water model (all levels)

One library with one set of rules, parameterised per level (cell size, thresholds).

1. **Fill (exact):** priority-flood gives each depression its spill level. On tiled grids it is
   the exact parallel variant (Barnes 2016: solve tiles, solve the tile-edge graph, finalise).
   It matches a whole-grid fill exactly.
2. **Water balance:**
   - **Lake level:** a depression's level is the lowest level at which evaporation times lake
     area matches inflow (rain times catchment, plus water fed in from the coarser level).
   - **Spill:** if no level below the spill satisfies the balance, the lake fills to its spill
     and overflows.
   - **Defaults:** rain and evaporation come from the class water table that exists now
     (`hydrology.WATER`). Pits shallower than a per-level depth (1 m stage: 2 m) are filled
     into the ground, not kept as ponds.
3. **Flow:** each cell drains to its lowest neighbour on the shown surface (water where wet,
   else ground). Flats and lakes are routed towards their outlet. Accumulation sums discharge
   (Barnes 2017 parallel accumulation on tiles).
4. **Labels:**
   - **Lake:** connected standing water above the depth threshold.
   - **River:** cells whose discharge exceeds the level's threshold.
   - **Sea:** open water at or below sea level connected to the map edge. Sea is ground below
     sea level, not a mask.
   - **Output files:** `bodies.json`, `water-body.u32`, `water-surface.f32` and the flow layer,
     with the same names and formats as today, so `game_layers` and export are unchanged.

Fill and flow are exact graph passes. They stay in Rust on the CPU, where they take minutes. The
iterative passes (erosion, smoothing, channel relaxation) run on the GPU with PyTorch (ROCm, AMD
RX 9060 XT, 16 GB).

## Coarse stages (32 m, 4 m): own the network

- **Erosion:**
  - Stream power (`E = K·A^m·S^n`) plus hillslope diffusion, iterated to a graded state.
  - The task 1 benchmark chooses between an implicit CPU solver (Braun–Willett) and an
    explicit GPU variant.
  - Lake outlets are graded by the erosion itself. There are no breach slots, lips or sill
    raises.
- **Real terrain:** K is scaled per cell from provenance. Interiors of real GLO-30 pieces get a
  small K (the canyon about 0), blend and fill zones between pieces get the full K. The collage
  build records provenance as a new layer.
- **Lakes:** levels and outlets come from the water model; there are no share budgets.
- **Hand-off:** ground, water surface, discharge and lake levels condition the next diffusion
  level as today. The current conditioning inputs stay; adding water/discharge channels to the
  models is out of scope.

## 1 m stage: own local shape only

- **Tiling:** the whole world runs in tiles with halos. There is no frozen north band and
  there are no seam rules, because the exact tiled fill and accumulation make tile edges
  invisible.
- **Inputs:** `elevation-raw.f32` (kept), the 4 m lake levels, and the 4 m discharge fed in
  where 4 m rivers enter each 4 m cell's 1 m area, spread over its lowest 1 m cells.
- **Rivers:** they form where the 1 m discharge crosses the threshold, so they run on the 1 m
  valley floor by construction.
- **Lakes:**
  - A lake keeps the 4 m level where the 4 m world has one.
  - Other 1 m depressions take the water-balance level.
  - Pits up to 2 m deep are filled.
- **Channels:**
  - Width and depth come from hydraulic geometry on discharge (`w = a·Q^0.5`,
    `d = c·Q^0.4`), with depth capped at 2 m.
  - The bed never rises downstream.
  - A GPU relaxation pass limits channel walls to about 35–40° and smooths them. Only the
    channel corridor changes.
- **Real terrain:** the provenance K mask also scales the 1 m channel depth.

## Checks (physical, hard, per tile and per world)

A run fails if any of these fails. There are no exemptions, only these thresholds:
- **Drainage:** every land cell reaches the sea or a closed lake along a non-increasing shown
  surface. There are no cycles.
- **Lakes:** a lake is flat, and its level equals its outlet sill within 1 cm, or it is closed
  with its water balance met.
- **Outlet grade (dams):** the water surface falls by at most 2 m within the first 50 m below a
  lake exit. No dry cell within 2 cells of the exit stands more than 0.5 m above the level
  unless the raw ground did.
- **One thalweg:** there is no lower parallel flow path within 30 m of a river. Two distinct
  rivers never run within 20 m of each other for more than 100 m.
- **Straightness:** the share of river length in straight axis or 45° runs longer than 60 m is
  at most the share on the raw surface's own drainage network.
- **Facets:** the share of planar 5×5 windows with slope over 5° does not rise from raw to
  final. Carved walls stay at or below 40°.
- **Change budget:**
  - On real-piece cells: p99 |Δ| ≤ 1 m and max ≤ 3 m.
  - Elsewhere: p99 ≤ 3 m.
- **Tiles:** recomputed overlaps are identical within 1 mm.
- **Review:** hillshade crops at the owner's spots, 512 m each:
  - E25645 N24400, E25307 N24585, E20071 N30501;
  - E27169 N44546, E43966 N21633, E49249 N16381;
  - E47962 N36851, E48148 N36810;
  - E48810 N19452, E51610 N20478, E42581 N56193.

## Rerun and staging

1. Build and check the water model on small test windows at the spots (32 m, 4 m and 1 m).
2. Regenerate fujix 32 m hydrology, then L3 (4 m), then 4 m hydrology. Check a 4 m world
   before any 1 m work.
3. 1 m: a test strip of a few bands at the canyon and the dam spots, then the full L4 diffusion
   and the 1 m water.

Every stage keeps its raw diffusion output and is resumable, so a later water fix never reruns
diffusion. Long jobs follow AGENTS.md: a log, an exit code file, an hourly heartbeat and capped
memory.

## Removed

The object-based hydrology in `crates/topography-core/src/world/hydrology/` is deleted once v3
replaces it:
- lake and river objects;
- breach slots, lips and bank raises;
- coarse path imposition and fine routing;
- band seam rules and their exemptions;
- `widen_trenches`.

Reusable parts (grid I/O, reports, moisture) move to the new module. The band-by-band
`--hydrology-only` driver is replaced by the tiled 1 m stage, which keeps the raw-surface and
resume guarantees.

## Process

A hydrology-expert subagent reviews this spec, the implementation plan and each implementation
task's diff together with its real-terrain output.

## Out of scope

- Shallow-water or pipe-model simulation as the source of truth. It converges too slowly on
  lakes; a cosmetic 4 m pass may come later.
- New model conditioning channels.
- Sediment deposition and deltas.
- Underwater detail.
