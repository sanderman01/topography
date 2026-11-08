# Hydrology v3: one water field at every level

Status: design approved in chat 2026-11-05. Revised after the hydrology expert's review
(`scratch/hydrology-v3-spec-review.md`). Awaiting owner review.
Diagnosis behind it: `scratch/hydrology-v3-expert-diagnosis.md`.

## Plan A departures

Plan A (the 1 m water model, shipped at gate 2 on 2026-11-06) differs from this spec here. The
[workflow doc](../../workflows/terrain-cascade.md#hydrology-v3) has the details.

- **No relaxation.** The GPU bank relaxation (1 m stage, "Where it runs") is dropped. Carving
  alone passed `walls` and `facets` and looked the same in hillshades. It is kept in git at
  `d0c0055` for Plan B. Breach sides are tapered in Rust instead: no off-path cut on raw slopes
  of 39° or more, and a gorge that would leave a wall is refused, so the depression stays
  filled.
- **Straightness and pairs are figures.** By owner ruling on 2026-11-06, straight and parallel
  rivers made by the pre-hydrology terrain are fine. `straight_runs`, `straight_share` and
  `parallel_rivers` are reported but never fail a run.
- **One thalweg (`thalweg_beside`).** The lower cell beside a river fails only where it, or a
  cell within 2 cells, carries a channel (m3 Q at the river threshold) that is not this river's
  own. Dry lower ground on a valley floor passes, and another body's written water is no second
  channel. The spec asked for the lowest raw ground. The cost: a river perched above a dry
  valley floor passes.
- **Export contract checks.** Added as hard checks: `wet_not_above_ground`,
  `wet_without_surface`, `surface_without_body`, `lake_level_json` and `wet_unknown_body`.
  Flowing water over an uncut bed runs 0.005 m deep. Still water at ground level is dry shore.
- **Settle.** After channels, written water is lifted to the final (m3) routing. Labelled lakes
  keep their label's level and extent, and the m3 level gap is a figure.

## Plan B departures

Plan B (the 32 m and 4 m levels, the regeneration and the removal of v2) differs from this spec
here. The [workflow doc](../../workflows/terrain-cascade.md#hydrology-v3) has the details and
the [plan](../plans/2026-11-07-hydrology-v3-b-coarse-and-regeneration.md) the rulings. Measured
numbers from the full run are added at gate 5 (Task 18).

- **Owner rule (2026-11-08, overrides the rest).** The water model changes terrain only
  through erosion where water flows and deposition where it pools; other edits only for
  runtime or band and tile seams. Removed: the erosion wall, narrow and bed-grade rounds, the
  diffusion trough and wall bounds (diffusion is now mass-conserving), the breach refusals for
  walls and `handoff_narrow` and its steep-raw skip, and the channel wall halving, restore,
  bed-grade and pit-restore rounds. `walls`, `handoff_narrow`, `erosion_new_pits`,
  `carve_pits`, `bed_rises`, `facets`, `change_p99`, `interior_p99`, `label_agreement`,
  `thalweg_beside`, `dam_drops`, `dam_steps` and the coarse guide are figures; `dam_rims` is
  gone. This overrides rulings B8, B9, B26, B33, Q2, Q4 and R5. The channels' rim floor
  beside lakes and deep ponds stays until lakes are re-levelled on the carved ground.
- **CPU erosion.** Erosion runs on the CPU: an implicit, global, single-threaded Braun–Willett
  pass. The 16,384² 4 m benchmark (Task 9) took 701 s for its erosion passes; the whole
  level run peaked at 6.2 GB, inside the 45 min and 7 GB limits, so the GPU contingency (Task 9b) was skipped.
- **Erosion budget.** k_dt stays 7 × 10⁻⁴ at 32 m (8.75 × 10⁻⁵ at 4 m). The seam-lake target
  was replaced by `seam_excess` ≤ 1.5 (lakes exiting near region borders against those
  farther away; measured 1.10). Graded K by seam strength was not needed (far blend p99
  1.95 m).
- **Provenance is replayed.** The collage's provenance comes from replaying its surface steps
  and is written as a sidecar (`provenance32.u8`, `provenance32.json`); the collage itself is
  not rebuilt.
- **Hand-off figures at 32 m too.** `handoff_narrow` and the 35° `walls` are counted at 32 m as well
  as 4 m (L3 copies 32 m ground the same way). A cell counts only where the cut itself made a
  trough deeper than raw's.
- **Dams.** `dam_drops` judges labelled lakes' exits only (pond exits are figures) with a
  knickpoint rule: a step over 1.5 m more than 3 × the mean fall after it and before it. At
  32 m and 4 m it counts only where the step exceeds raw's by over 1.5 m; there is no
  steep-source exception. A knickpoint is a dam only with a counter-slope: the ground 2–15 m
  (at least one cell) behind the step top lies over 1 m below the top. Without one it is a
  natural waterfall, counted in the `dam_drops_waterfalls` figure (owner ruling, 2026-11-08).
  `dam_drops` and Plan A's per-cell `dam_steps` are figures (owner rule below).
- **Coarse guide, a figure.** It counts a gross mismatch (`gross_mismatches`) only for coarse lakes of at least
  0.05 km², deeper than the finer level's breach cap + 1 m and at least 1 km from a window
  edge; it never fails a run (owner rule below). A lake that is not full at either level is
  compared by the spills of its depression, not its water level, and a split whose parts
  all keep the coarse level is not doubled (gate 3, lakes 1140, 1141, 1906).
- **L3 exit sills (reverted).** L3 eased each full 32 m lake's exit sill cell to the 32 m ground
  in both directions, so it neither raises nor cuts a spill (gate 3, lakes 1138, 1564).
- **Coarse change budget.** `change_p99` and `interior_p99` are figures; the hard
  budget is `interior_max` and `change_max`, which adds both caps (breach cap + erosion
  cap + 0.5 m).
- **Coastal water table.** Basins below sea level are handled per basin: unless every sub-zero
  cell is in a hot, dry class far from the sea, a lake's level is floored at min(0 m, its
  spill). Where the floor governs, the lake covers the cells below it and is not full.
- **Every full lake starts a river** (the outlet trickle), whatever its discharge.
- **Diffusion only near river cuts.** Diffusion tapers only stream-power cuts. Its footprint
  comes from cuts of at least 0.25 m. Hollows fill (deposition) except channel cells, whose
  supply the river carries off; α is 0.006 at 32 m.
- **Coarse breaches.** Breach discs are at least 2.5 cells wide at erosion levels and cut steep
  raw ground at every level; no gorge is refused for walls or `handoff_narrow`.
- **`thalweg_beside` own water.** These count as the river's own water, not a second channel:
  the sea, a fed lake's outflow within 150 m, the river's own unlabelled continuation, and a dry
  drainage end on a shore.
- **Lake-exit sheets.** The contact sheet crops are max(128 m, 32 cells).
- **Comb-cut pits (Task 13).** The cause was neither of the first two hypotheses: the rim floor
  held cells beside micro-pits. Rims now hold only for lakes and for ponds at least as deep as
  the river. A one-cell pit a cut deepens stays (a scour pool); `carve_pits`, a figure, has
  a clause (b) for such pits.
- **L4 diffuses only.** `cascade_l4` writes the raw 1 m surface; `water.run world` makes the
  water world. Band hydrology, the seam audit and `--hydrology-only` are gone, and the L4 halo
  may again be as wide as a band.
- **Ground plus the existing water conditioning (B32).** L3 keeps its clamp to the 32 m ground
  under lakes and sea, and L4 its zeroed residual under 4 m lakes and sea. Task 15: the L3
  river clamp A/B tied (same hard checks, figures within 1 %, no visible change). Under the
  owner rule rivers are left out by default at both levels: L3 does not clamp them
  (`--l3-river-clamp` opts in) and L4 keeps their residual (`--zero-river-residual` opts out).
- **L4 test strip.** Four squares (dams 8 km, canyon, E43966 and alpine, 4 km each) instead of
  one strip.

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

Hard checks have no exemptions, only these thresholds. Since the owner rule (Plan B
departures) the dam steps, straightness, facets, walls, change p99, hand-off and coarse-guide
checks below are figures.

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
