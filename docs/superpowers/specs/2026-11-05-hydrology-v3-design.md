# Hydrology v3: one water field at every level

Status: design approved in chat 2026-11-05. Revised after the hydrology expert's review
(`scratch/hydrology-v3-spec-review.md`). Awaiting owner review.
Diagnosis behind it: `scratch/hydrology-v3-expert-diagnosis.md`.

## Why

The finished fujix 1 m world (`data/worlds/fujix-55-l4v2`) shows these defects:
- dam-like steps where rivers leave lakes;
- doubled rivers in the real canyon;
- straight and 45° rivers;
- faceted planes;
- over-carved real terrain.

Each comes from a rule in the current object-based hydrology:
- breach slots cut to the spill minus 30 m at 32 m;
- lips raised at 4 m lake exits;
- 4 m river paths imposed on 1 m ground;
- shortest-path routing (`fine.rs::route_descending`);
- 35° ring-by-ring trench widening;
- frozen-north band seams.

The dams and the straight river at E43966 come from the 32 m and 4 m stages, and the 1 m
diffusion copied them into `elevation-raw.f32`. A 1 m-only change cannot remove them.

## Goal

Plausible game terrain where water behaves like water. Water is one field: every cell has a
ground height, a water surface (or none) and a discharge. Lakes and rivers are labels read off
that field, never objects that shape it. No rule raises banks, cuts slots or imposes paths, and
no rule exempts a check.

Owner decisions (2026-11-05):
- **Terrain change:** water places itself on the diffusion terrain and cuts only light channels
  at 1 m.
- **Coarse guides, fine decides:** the coarser level guides through the terrain it hands on,
  since each diffusion level is conditioned on the level above. The finer level's own water
  model always sets courses and levels. Coarse levels and courses are compared as figures and
  never imposed.
- **Rerun:** regenerate from the 32 m hydrology through the 4 m and 1 m diffusion (about
  35–41 h of GPU).

## The water model (all levels)

One library with one set of rules, parameterised per level.

### Surfaces

- **Ground** is never raised. It is lowered only by erosion (32 m and 4 m), by breaching
  (all levels) and by channel carving (1 m).
- **Routing surface:** the priority-flood fill of the ground. It is used only for routing and
  levels; it never becomes ground. Filling pits into the ground makes flats and facets.

### Fill and depression hierarchy

- Priority-flood gives each depression its spill level and builds the nested hierarchy.
- On tiled grids the exact parallel variant (Barnes 2016) solves the tiles, solves the
  tile-edge graph and finalises. The hierarchy is built per tile and merged along the same
  graph.

### Depression policy: breach or fill

- Each depression is first scored for hybrid least-cost breaching (Lindsay 2016). A narrow,
  deep obstruction is breached when the volume removed is small against the volume it would
  fill, and the breach depth stays under the level's cap.
- **Breach shape:** a graded gorge, never a slot. The bed falls linearly over at least
  20 × the cut depth, is at least 3 cells wide, and its sides are left to the diffusion or
  relaxation.
- **Otherwise:** the depression stays and holds water by the balance rule below.
- Breach cells are recorded with their own figures (count, depth, volume).

### Water balance: closed or spilling

- Each class has a dimensionless lake-to-catchment ratio r ≈ P/E, set by judgement:
  - humid classes: r ≥ 1, so lakes spill;
  - arid classes: r ≈ 0.01–0.1;
  - r = 0: a dry playa.
- **Level:** a depression stands at the level where A_lake(level) = r · A_catchment (including
  overflow from basins upstream). If no level below the spill meets this, it fills to the spill
  and overflows.
- **Solver:** Fill-Spill-Merge (Barnes, Callaghan & Wickert 2020) over the hierarchy, upstream
  first, merged across tiles.
- **Replaces:** the r column in `configs/hydrology/water-v1.toml` replaces `lake_share`,
  `tarn_share` and `dry_share`. Dune hollows and dolines come out dry because of their r.
- **Below sea level:** ground there that is not connected to open sea is a lake at its balance
  level, which may be below 0 m (a lagoon).

### Flow

- **Receivers on dry land:** the fall-line receiver is v2's Horn gradient with a deterministic
  dithered pick between the two neighbours that bracket the gradient. Steepest-neighbour D8
  draws straight axis and 45° rivers on smooth slopes.
- **Water and flats:** every lake or flat is one graph node with one outlet. Accumulation sums
  into the node and leaves at its outlet, so flats inside water need no resolution. Dry flats
  are resolved with Barnes 2014 flat resolution, through the tile-edge graph.
- **Accumulation:** Barnes 2017 parallel accumulation on tiles. Rain per cell is the 128 m
  moisture field, interpolated bilinearly, times the cell area. Moisture keeps coming from the
  32 m Smith–Barstad stage.
- **Q's unit:** Q is the upstream area weighted by moisture, in km² at mean rain, as v2 has it.
  Hydraulic geometry converts it to m³/s with a stated factor.
- **No injection:** discharge is never fed in from a coarser level. The 1 m accumulation covers
  the whole world exactly.

### Labels and outputs

- **Lake:** connected standing water at least 0.3 m deep (per level).
- **River:** a cell whose Q is over the level's threshold. 1 m starts from v2's 0.05 km²;
  32 m and 4 m keep v2's values.
- **Sea:** open water at or below sea level connected to the map edge. Sea is ground below sea
  level, not a mask.
- **Outputs:** `bodies.json`, `water-body.u32`, `water-surface.f32` and the flow layer, with
  the same names and formats as today, so `game_layers` and export are unchanged.

### Where it runs

- Fill, hierarchy, Fill-Spill-Merge and accumulation are exact graph passes. They stay in Rust
  on the CPU.
- The iterative passes (erosion, relaxation) run on the GPU with PyTorch (ROCm, AMD RX 9060 XT,
  16 GB), using deterministic stencils with no atomics.

## Coarse stages (32 m, 4 m): own the network

### Erosion

- **Model:** stream power (`E = K·A^m·S^n`) plus hillslope diffusion, with a fixed erosion
  budget per level (a dimensionless K·t, or a fixed iteration count with a stated K·dt). With
  no uplift there is no steady state to iterate to.
- **Lowering cap per cell:** for example 30 m at 32 m and 10 m at 4 m. Breach gorges are
  excepted.
- **Routing:** lake-aware (Cordonnier et al. 2019, or Braun–Willett with depressions routed to
  their outlets), so sills incise and outlets grade.
- **Solver:** chosen by benchmark in the plan, an implicit CPU or an explicit GPU solver.

### Provenance and K

- Fujix's whole 32 m surface is a collage of real GLO-30 pieces. The collage build writes a
  provenance layer: piece interior, blend zone, painted feature or exemplar fill.
- **Real interiors:** a low K plus the breach-or-fill policy, so the canyon is barely eroded
  and its GLO-30 pits do not become chains of ponds.
- **Blend zones, painted features and fills:** full K, so lakes do not form behind seams
  between pieces.

### Hand-off to the next diffusion level

- Ground only, as today. The 4 m ground must carry no carved channel narrower than 3 cells
  and no cut steeper than 35°, because L4 copies narrow cuts; that is how v2's slots reached
  1 m.
- Outlets stay smooth valleys several cells wide, so the next level reproduces a clean sill.
- The raw L3 (4 m) diffusion surface is kept from now on, so 4 m hydrology can be redone
  without L3.

## 1 m stage: own local shape only

- **Tiling:** the whole world runs in tiles with halos. There is no frozen north band and
  there are no seam rules.
- **Inputs:** `elevation-raw.f32` and the 128 m moisture field. Nothing is imposed from 4 m.
- **Water:** levels, lakes and rivers all come from the water model above, on 1 m.
- **Channels:**
  - **Size:** width and depth from hydraulic geometry on Q in m³/s (starting values
    `w = 3·Q^0.5`, `d = 0.3·Q^0.4`). Depth is capped at 2 m, which only the largest rivers
    reach.
  - **Carving:** only river cells are lowered. Bumps along the thalweg are breached up to the
    cap, and anything higher ponds behind as a pool from the fill.
  - **Profile:** the water surface never rises downstream. The bed may rise under standing
    water.
  - **Relaxation:** a GPU pass inside the channel corridor only. It limits changed cells to
    about 35–40°, never steepens a cell beyond raw, and smooths. The halo is at least the
    iteration count × the stencil radius, so overlaps come out identical.
- There is no provenance scaling at 1 m. All 1 m detail is diffusion output, and Q alone sets
  the channel.

## Checks (hard per tile and per world; others are figures)

Hard checks have no exemptions, only these thresholds.

- **Drainage:** every land cell reaches the sea or a closed lake along a non-increasing shown
  surface (water where wet, else ground). There are no cycles.
- **Lakes:** a lake is flat. Its level equals its outlet sill within 1 cm, or it is closed with
  the balance met.
- **Dams:**
  - No water-surface drop of more than 1.5 m between adjacent cells within 50 m below a lake
    exit, unless the raw ground has the same step (a cliff).
  - No dry cell within 2 cells on the exit side stands more than 0.5 m above the level, unless
    the raw ground did.
  - The 50 m fall below each exit is a figure.
- **One thalweg:** for each river cell with Q over the main-river threshold, the lowest raw
  ground within ±30 m perpendicular to the flow is either a river cell of the same body or
  within 1 m of the river surface. The pair test (two rivers within 20 m for over 100 m) is a
  figure.
- **Straightness:** at most 5 % of river length lies in straight axis or diagonal runs longer
  than 60 m.
- **Facets:** among the cells the run changed, the share of 5×5 windows with a plane-fit
  residual under 2 cm and a slope over 5° is no higher than among the same cells in raw.
- **Walls:** changed cells are at most 40°, and never steeper than raw.
- **Change budget:**
  - At 32 m and 4 m, for real-interior cells outside breaches: p99 |Δ| ≤ 1 m, max ≤ 3 m.
    Breaches get their own figures.
  - At 1 m, for all cells: p99 ≤ 1 m, max ≤ channel cap + 0.5 m.
- **Hand-off (4 m):** no carved channel narrower than 3 cells and no cut steeper than 35°.
- **Coarse guide:** these are figures, with a hard failure only on gross mismatch:
  - |level − coarse level| per lake: hard failure over 5 m, or when a lake is gone or doubled;
  - each coarse river with Q over its threshold has a fine river within 100 m.
- **Tiles:** one 16,384² window solved whole and as 16 tiles gives bit-identical labels,
  levels and Q. In world runs, recomputed overlaps are identical within 1 mm.
- **Review:**
  - Hillshade crops at the owner's spots, 512 m each:
    - E25645 N24400, E25307 N24585, E20071 N30501;
    - E27169 N44546, E43966 N21633, E49249 N16381;
    - E47962 N36851, E48148 N36810;
    - E48810 N19452, E51610 N20478, E42581 N56193.
  - An auto-generated contact sheet of every 4 m lake exit.

## Staging (each gate passes before the next starts)

1. Unit-scale: the water model on small test windows at the spots (32 m, 4 m, 1 m). The
   tiled-exactness test passes.
2. The whole 1 m world on the existing `fujix-55-l4v2/elevation-raw.f32`, with no diffusion.
   - **Proves:** tiling, run time and memory, and the fixes for the canyon, E49249 and the
     facets.
   - **Expected:** the dams baked into that raw surface stay.
3. Regenerate fujix 32 m hydrology, then L3 (4 m), keeping its raw output, then 4 m
   hydrology. Gate on:
   - all hard checks on the whole 4 m world;
   - the lake-exit contact sheet;
   - the hand-off checks.
4. An L4 test strip containing the three dam spots and the canyon, then its 1 m water. It
   passes on the hard checks and the owner's Yes.
5. The full L4 diffusion (35–41 h), then the 1 m water, game layers and export.

Every stage keeps its raw output and is resumable. Long jobs follow AGENTS.md: a log, an exit
code file, an hourly heartbeat and capped memory.

## Removed

The object-based hydrology in `crates/topography-core/src/world/hydrology/` is deleted once v3
replaces it:
- lake and river objects and share budgets;
- breach slots, lips and bank raises;
- coarse path imposition and fine routing;
- band seam rules and their exemptions;
- `widen_trenches`.

Reusable parts (grid I/O, reports, moisture) move to the new module. The band-by-band
`--hydrology-only` driver is replaced by the tiled 1 m stage, which keeps the raw-surface and
resume guarantees.

## Process

A hydrology-expert subagent reviews this spec, the implementation plan, and each implementation
task's diff together with its real-terrain output.

## Out of scope

- Shallow-water or pipe-model simulation as the source of truth. It converges too slowly on
  lakes; a cosmetic 4 m pass may come later.
- New model conditioning channels.
- Sediment deposition and deltas.
- Underwater detail.
