# Hydrology v3, Plan A: the water model and the 1 m stage — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the level-independent v3 water model (priority-flood hierarchy, breach-or-fill, Fill-Spill-Merge with r, fall-line flow, exact tiling) in Rust, the 1 m stage on top of it (channels, GPU bank relaxation, checks, a resumable world driver), and pass staging gates 1 and 2 of the spec.

**Architecture:** A new module `crates/topography-core/src/world/water/` holds the exact graph passes. Every pass runs tile by tile; a whole window is the same code with one tile, so "whole vs tiled" is a comparison of two tile sizes. Per-tile results are small graphs (drainage ends, spill edges, ports) joined in global steps (Barnes 2016 for the spill graph, Barnes et al. 2020 for the hierarchy and Fill-Spill-Merge, Barnes 2017 for accumulation); depression subtrees that spill below a tile's ring are settled inside the tile and reach the global step as one super-leaf (exact contraction); water nodes are named by canonical keys (their smallest pit cell), and every sum across tiles is an integer, so results are bit-identical at any tiling and with or without contraction. Two CLI commands (`world water`, `world water-finish`) run the passes into a resumable work directory; a Python driver (`topography_training.water.run`) runs the GPU bank relaxation between them and assembles the world. The v2 module `world/hydrology/` is untouched apart from one visibility change and one added method.

**Tech Stack:** Rust 1.85 (edition 2024, std only plus existing workspace crates: serde, serde_json, toml, sha2, tempfile, png), Python 3.11 with numpy, scipy and PyTorch 2.14 on ROCm 7.2 (`data/models/diffusion-env-v1/.venv`), stdlib `unittest` for Python tests (the env has no pytest).

**Spec:** `docs/superpowers/specs/2026-11-05-hydrology-v3-design.md` (binding). Background: `scratch/hydrology-v3-expert-diagnosis.md`, `scratch/hydrology-v3-spec-review.md`. This revision applies the plan review `scratch/hydrology-v3-plan-a-review.md` (items C1–C3, I1–I8, M1–M6) with the coordinator's rulings on it.

## Global Constraints

Copied verbatim from the spec; every task's requirements include them.

- Water is one field: every cell has a ground height, a water surface (or none) and a discharge. Lakes and rivers are labels read off that field, never objects that shape it. No rule raises banks, cuts slots or imposes paths, and no rule exempts a check.
- **Ground** is never raised. It is lowered only by erosion (32 m and 4 m), by breaching (all levels) and by channel carving (1 m).
- **Routing surface:** the priority-flood fill of the ground. It is used only for routing and levels; it never becomes ground. Filling pits into the ground makes flats and facets.
- **Breach shape:** a graded gorge, never a slot. The bed falls linearly over at least 20 × the cut depth, is at least 3 cells wide, and its sides are left to the diffusion or relaxation.
- Breach cells are recorded with their own figures (count, depth, volume).
- **Level:** a depression stands at the level where A_lake(level) = r · A_catchment (including overflow from basins upstream). If no level below the spill meets this, it fills to the spill and overflows.
- **Solver:** Fill-Spill-Merge (Barnes, Callaghan & Wickert 2020) over the hierarchy, upstream first, merged across tiles.
- **Receivers on dry land:** the fall-line receiver is v2's Horn gradient with a deterministic dithered pick between the two neighbours that bracket the gradient.
- **Water and flats:** every lake or flat is one graph node with one outlet. Accumulation sums into the node and leaves at its outlet, so flats inside water need no resolution. Dry flats are resolved with Barnes 2014 flat resolution, through the tile-edge graph.
- **Accumulation:** Barnes 2017 parallel accumulation on tiles. Rain per cell is the 128 m moisture field, interpolated bilinearly, times the cell area. Moisture keeps coming from the 32 m Smith–Barstad stage.
- **Q's unit:** Q is the upstream area weighted by moisture, in km² at mean rain, as v2 has it. Hydraulic geometry converts it to m³/s with a stated factor.
- **No injection:** discharge is never fed in from a coarser level. The 1 m accumulation covers the whole world exactly.
- **Outputs:** `bodies.json`, `water-body.u32`, `water-surface.f32` and the flow layer, with the same names and formats as today, so `game_layers` and export are unchanged.
- Fill, hierarchy, Fill-Spill-Merge and accumulation are exact graph passes. They stay in Rust on the CPU.
- The iterative passes (erosion, relaxation) run on the GPU with PyTorch (ROCm, AMD RX 9060 XT, 16 GB), using deterministic stencils with no atomics.
- **Tiling:** the whole world runs in tiles with halos. There is no frozen north band and there are no seam rules.
- **Inputs:** `elevation-raw.f32` and the 128 m moisture field. Nothing is imposed from 4 m.
- **Size:** width and depth from hydraulic geometry on Q in m³/s (starting values `w = 3·Q^0.5`, `d = 0.3·Q^0.4`). Depth is capped at 2 m, which only the largest rivers reach.
- **Carving:** only river cells are lowered. Bumps along the thalweg are breached up to the cap, and anything higher ponds behind as a pool from the fill.
- **Profile:** the water surface never rises downstream. The bed may rise under standing water.
- **Relaxation:** a GPU pass inside the channel corridor only. It limits changed cells to about 35–40°, never steepens a cell beyond raw, and smooths. The halo is at least the iteration count × the stencil radius, so overlaps come out identical.
- There is no provenance scaling at 1 m. All 1 m detail is diffusion output, and Q alone sets the channel.
- Hard checks have no exemptions, only these thresholds.
- Every stage keeps its raw output and is resumable. Long jobs follow AGENTS.md: a log, an exit code file, an hourly heartbeat and capped memory.
- A hydrology-expert subagent reviews this spec, the implementation plan, and each implementation task's diff together with its real-terrain output.

Project constraints from AGENTS.md that every task also obeys:

- Keep the 16 GB VRAM ceiling; the host has 15 GB RAM: run every real-data job under `systemd-run --user --scope -q -p MemoryMax=<cap>`.
- Before any heavy run (the 16,384² window, the whole world, GPU relaxation of the world), stop and warn the user with the workload, expected memory and duration (stating uncertainty), and wait for their go. Rust builds and downloads need no warning.
- Never overwrite `data/worlds/fujix-55-l4v2`; open its `elevation-raw.f32` read-only.
- Plan A removes no v2 code and must not change the 32 m / 4 m v2 path. Every task ends with the task-finish block below, including `cargo test -p topography-core hydrology`. `bash scripts/check.sh` runs in full from Task 14 on, the first task where the module is wired to the CLI with no dead-code allowance (N5).
- Commits: small Conventional Commits ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Do not commit plan or task files.

---

## Rulings on spec ambiguities

The spec leaves these open; the plan implements the ruling given. The owner may overrule any of them before execution. Rulings R25–R31, and the changes to R2, R5, R6, R7, R9, R14, R16, R17, R19 and R21, apply the plan review (`scratch/hydrology-v3-plan-a-review.md`) as the coordinator ruled on it. R32 and the changes to R6, R22 and R25 apply the pre-flight scan (`.superpowers/sdd/2026-11-05-hydrology-v3-a-water-model/preflight-scan.md`). R21 replaces the spec's "Barnes 2014 flat resolution" for ponds with a priority flood from the outlet, keeping Barnes 2014's mask as the tie-break (review I2). The Global Constraints above keep the spec's wording.

| # | Question | Ruling |
| --- | --- | --- |
| R1 | New `r` column: bump `water-v1.toml` or a new file? | New file `configs/hydrology/water-v2.toml` (`schema_version = 2`, only `r`). v1 is parsed by v2 Rust (`deny_unknown_fields`) and by Python (`Water(**v)`), so adding a column to it would break the v2 path. Plan B deletes v1 with the v2 code. |
| R2 | r per class | Judgement table in Task 1, one value for every legend class 0–19 the fujix world uses (M6): humid classes 1.0–3.0 (fjords, alpine, glaciers 3.0; archipelagos, braided rivers, drowned karst 2.0; plain, unlabelled, calderas 1.0; coastal cliffs 1.5), porous ones below 1 (karst 0.3–0.5, volcanic 0.3–0.5), canyons 0.1 (closed lakes are real there), and **r = 0 (dry playas) for badlands (7) and dunes (8, 16)** (C2.3: dune hollows come out dry). Legend v3 has no separate desert-basin class; desert basins fall under badlands and dunes. |
| R3 | Which r when a catchment spans classes? | Supply = Σ over catchment cells of r(class of the cell). For one class this is exactly r · A_catchment. Overflow from upstream basins adds in the same units. |
| R4 | Bit-identical tiling with float sums | Every sum across cells is an integer: supplies in thousandths of a cell, Q in units of 10⁻¹² km² (u64; 4,300 km² is 4.3·10¹⁵), volumes in mm·cells (i64). Min/max of floats are order-free. Only these are combined across tiles, so the tile split cannot change a result. |
| R5 | Q → m³/s factor | A game setting, `m3s_per_km2` in `water-v2.toml` (I8): **0.56 m³/s per km² at mean rain**, chosen so the largest river (about 177 km², the 4 m world's largest) is about 30 m wide (w = 29.9 m, d = 1.9 m). Typical widths: 0.05 km² → 0.5 m, 1 km² → 2.2 m, 10 km² → 7.1 m, the canyon trunk (≈ 100 km²) → 22 m. The model's literal rain (0.657) is 20 m/yr and not used. |
| R6 | River cells | A river cell is a thalweg cell (Q ≥ the level's threshold, 0.05 km² at 1 m) or a cell of a channel section that ends up under water. A cell within several thalwegs' half-widths belongs to the one with the larger Q, then the nearest (I3b): a tributary thalweg inside a larger river's section joins that river there, and a river cell whose surface lies below that of the river cell it flows into is raised to it for up to 16 cells upstream (backwater, N18; ground untouched, `drainage_rises` catches anything longer). Breach gorges are at least 3 cells wide. A small full pool on a river is labelled as that river (R26). |
| R7 | Standing water below the lake label ("ponds") | A node is a labelled lake only when it is at least 0.3 m deep **and** at least `lake_min_m2` (500 m², a setting in `water-v2.toml`; about 31 cells at 4 m) in area (C2.1). Smaller or shallower nodes are ponds: routed like lakes but unlabelled (`water-surface.f32` NaN, `water-body.u32` 0) except on the river cells that cross them. Full ponds are routed by a priority flood from their outlet over their own ground (I2, R21), so rivers follow the pond's lowest ground at the pond level. The drainage check measures the routing surface D (labelled water at its surface, ponds at their level, else ground); ponds are reported as figures. |
| R8 | Sea and the map edge | Sea = cells whose priority-flood level from the world edge is ≤ 0 m (ground ≤ 0 connected to the edge through ground ≤ 0). World-edge land cells are outlets: water leaves the map there, and the drainage check accepts them. Below-zero ground behind a rim is a lake at its balance level (a lagoon). |
| R9 | Breaching at 1 m | Only depressions a river flows into (node inflow Q ≥ the river threshold), all or nothing: the gorge is cut when its deepest cut stays within `CHANNEL_CAP_M` (2 m) for every river, whatever its size (I1); the channel then cuts only what remains above the raw − 2 m clamp, so breach plus channel never lowers a cell more than 2 m; its length is at least 20 × the cut, and it removes less volume than the depression would fill (Lindsay 2016). Otherwise the pool stays at its spill. Caps at coarser spacings, used only in Plan A's gate-1 windows: 20 m at 32 m, 5 m at 4 m (judgement; Plan B revisits). |
| R10 | Breach path | The receiver path from the spill cell down to its pit (the minimax path) plus the receiver path from the outlet down to the first cell below the pit, not a Dijkstra least-cost search: unique, deterministic, and local. A gorge must lie within 256 cells of its spill cell, so every tiling decides it alike. |
| R11 | "Walls ≤ 40° and never steeper than raw" | A changed cell's Horn slope may not exceed max(40°, its raw slope). Carving on gentle ground needs banks steeper than raw, and real canyon walls stay as steep as they are. The relaxation's limit is max(38°, raw). |
| R12 | Water surface never rises downstream | River surface = routing surface R (ground on dry thalweg cells, the pond level inside ponds). Receivers are strictly lower on dry land, flat inside ponds and step down at outlets, so R never rises along a path by construction; the drainage check proves it on the outputs. |
| R13 | Main-river threshold for the one-thalweg check | 1 km² at mean rain (about 0.66 m³/s, w ≈ 2.4 m). |
| R14 | Lake level vs sill (1 cm) | Checked on the final surfaces (C1): D = the water surface where wet, else the final ground. For each full lake (a) the lowest saddle out of the lake, min over lake cells' non-lake neighbours n of max(level, D(n)), equals the level within 1 cm (`lake_off_sill`); (b) no non-lake neighbour other than the outlet cell has D below the level by more than 1 cm (`lake_leaks`); (c) the outflow's first cell, the cell of the spill edge that carries the saddle, shows the level within 1 cm (`lake_exit_surface`). When the saddle is the outside cell, it is the first river cell and its surface R is its ground, which is the level; when the saddle is the lake's own rim cell, water falls over that rim and the river below starts lower. Channel sections never put a surface below an adjacent lake's level next to the lake (Task 12). |
| R15 | Flow layer | `flow.u8` with v2's bits (low 4 the receiver, 16 thalweg, 32 dry basin, 64 endorheic) and `discharge.f32` are written in the 1 m output directory (`<world>/hydrology/1m`), as v2 wrote them per band; the world's layers stay `elevation.f32`, `water.u8`, `water-surface.f32`, `water-body.u32`, `bodies.json`, so `game_layers` and export need no change. |
| R16 | Body ids and the export limit | Sea 1, lakes next (by node key, R28), then rivers (by head cell). A river body is a main stem: at a confluence it continues the larger-Q branch. Export allows fewer than 65,535 water bodies; the hard check `export_bodies` keeps lakes + rivers **at most 50,000** (well under the limit, C2.4). Dry basins (closed, no water, ≥ 0.01 km²) are listed as kind `dry`, as v2 lists them. |
| R17 | Coarse guide at gate 2 | A figure only at gate 2: it compares with the v2 4 m world, whose slot-lowered lakes and lip rims cause mismatches like the dams; each is listed with its cause in the gate-2 note and nothing withholds `manifest.json`. **Plan B makes it a hard check** (level > 5 m, gone, doubled) once the 4 m water is v3's own (gates 3–4). |
| R18 | The new world | `data/worlds/fujix-55-l4v3-water`; `elevation-raw.f32` is a btrfs reflink (`cp --reflink=always`) of the l4v2 raw surface, not a hard link, so nothing written there can reach l4v2. |
| R19 | Relaxation scope | Only dry corridor cells (within 8 cells of a carved wet section cell) move, only down, never below adjacent water, raw − 2.5 m (tapered to 0 at the corridor edge), or **their receiver + 1 mm** (strict, I7: no flats, no `drainage_stuck`). Beds under water stay as carved. 48 Jacobi iterations, stencil radius 1, halo 64. The stencil is built from shifted slices added in a fixed order (no `conv2d`, whose MIOpen algorithm depends on the shape), with `torch.use_deterministic_algorithms(True)`. |
| R20 | Closed-lake level | The k-th lowest cell of the depression with k = ⌈supply / 1000⌉ (area in whole cells). |
| R21 | Flats | Equal adjacent pit cells form one pit. Lakes are single nodes. Full ponds are routed by a priority flood from their outlet over the pond's own ground, ascending z, ties by the Barnes 2014 mask; each cell's receiver is the cell that flooded it (I2), so paths follow the pond's lowest ground instead of BFS geodesics. Whole pond inside a tile: solved there; else gathered and solved once (the result depends only on the cells, their ground and the outlet). |
| R22 | Change budget "for all cells" at 1 m | As written: p99 of \|Δ\| over all cells ≤ 1 m and max ≤ the level's breach cap + 0.5 m (2.5 m at 1 m); the p99 over changed cells is a figure. River starts and breach caps are per level in `water-v2.toml` (`[levels.*]`, N19), so the 4 m and 32 m gate-1 windows are judged with their own caps (5.5 m and 20.5 m), not the 1 m ones. |
| R23 | Python tests | `unittest` from the standard library, run with the diffusion env's Python (`$P -m unittest`), since the env has no pytest. |
| R24 | Tiles check | The graph passes are exact by construction (Task 9 and the gate-1 16,384² run prove it). "Recomputed overlaps identical within 1 mm" applies to the GPU relaxation, the only stencil pass in Plan A. |
| R25 | Global graph size (C3) | Exact per-tile contraction, always on. A tile settles its local depression trees whose merges all lie strictly below their **frontier**: the lowest saddle by which the tree touches anything outside it, meaning a ring pit, a cell draining out of the tile or to the world edge, or one of its own ring cells. Every frontier is at or above the tile ring's minimum ground, so this includes the coordinator's ring-minimum rule as the special case and is exact by the same argument. It settles at least as much. Each tree reaches the global step as one super-leaf keyed by its smallest pit. The global Fill-Spill-Merge treats it as a leaf and records the pours into it. That is exact for fullness and overflow, because water reaches a depression's own level only when everything below it is full. A tree whose super-leaf holds water without being full needs its inside solved: pass L does it in the tile only when the whole tree lies there, its held-cell count equal to the global one and every pour entering there (B2); otherwise pass L2 gathers the tree's cells and pours from every tile, each mapped to the tree's local leaf by following its path to the tree tile's ring (the ring cells' pre-contraction pits are kept in `Frag`, B1), and solves it at the global step. Leaves, ring cells, pit roots and edges are sorted `Vec`s with binary search, not HashMaps. Task 8b's test shows that every depression and spill survives; Task 9's tests prove every raster and the node table bit-identical with contraction on and off, including an arid closed lake solved in its tile and one whose tree holds cells across a seam (pass L2). |
| R26 | Pools on a river (C2.2) | A full lake node with river inflow (Q ≥ the river start) and area below 20 · w(Q)² is labelled as the river leaving it (the body of its outlet cell), surface at the pool level; it is not listed as a lake. Labels only; routing is unchanged. |
| R27 | Third model run (I3c) | `water-finish` reruns the model on the final ground (tag `m3`). `discharge.f32`, `flow.u8` and the drainage, pond and river checks use it; the hard check `label_agreement` requires the written rivers' thalwegs (m2) and m3's thalwegs to agree within one cell at 0.95 or more (a tolerant IoU: matched cells of both over all cells of both). About 50 % more graph time. |
| R28 | Node identity | A water node (lake, pond, closed basin) is named by its smallest pit cell's world index, which is the same at any tiling and with contraction; `Nodes` is the sorted table of all nodes (global and settled in tiles) with level, low, area, fullness, outlet, fill volume and the lake flag. The `node` raster holds 2 + the node's ordinal in that table. |
| R29 | Parallel rivers (I4) | Hard check `parallel_rivers`: along a river with Q ≥ 1 km², a different flow line with Q ≥ 0.002 km² (below the canyon's 0.0036 km² second channel) running within 10 m and parallel (direction cosine ≥ 0.7) for more than 100 m, unless that line joins the river within 100 m downstream. The 20 m pair length stays a figure. |
| R30 | Straight rivers (I6) | Besides the global 5 % share: hard check `straight_runs`, a river run longer than 150 m in one D8 direction, unless the raw thalweg is straight there too (at 90 % or more of the run's cells the lowest raw cell within ±3 cells across lies on the run). The 20 longest runs are listed with cells and get review crops. |
| R32 | Flats with an outlet (N15) | A depression of zero depth (spill equal to its lowest ground, a flat with an outlet) is never a closed basin: it is full by definition and drains on through flat routing, whatever its class's r. It is a pond node (depth below 0.3 m), so its cells get receivers from the outlet priority flood (R21). |
| R31 | Dams without raw (I5, M5) | Figures `dam_drops_any` (D drops over 1.5 m between adjacent cells within 50 m below each exit, raw ignored, with cells) and `dam_rims_any` (exit-side cells over level + 0.5 m on the final ground). Gate 2 must show the three dam spots and E43966 in `dam_drops_any`, which validates the detector; Plan B makes it hard on the 4 m hand-off and the L4 strip. `dam_rims` stays a hard guard that v3 cannot fail, since ground is never raised. |

## File structure

Rust, new module `crates/topography-core/src/world/water/` (one responsibility per file):

| File | Responsibility |
| --- | --- |
| `mod.rs` | Module docs, judgement constants, `Params` by spacing, `setup` from a grid directory, the staged `run` (model → breach → model → bodies → channels) and `finish` |
| `table.rs` | `water-v2.toml`: r per class in thousandths, and the game settings `lake_min_m2` and `m3s_per_km2` |
| `geom.rs` | World and tile geometry (`World`, `Rect`, `tiles`, D8 offsets), `UnionFind` |
| `layers.rs` | World-sized rasters in memory or in little-endian files (`Layer<T>`, `Classes`), windowed reads and positioned writes |
| `rain.rs` | The 128 m moisture field and per-cell rain in Q units |
| `store.rs` | The work directory: per-tile and global results (`Blob` codec), resume stamps, `for_tiles` |
| `receivers.rs` | Fall-line receivers (port of v2 `flood::fall_line`) |
| `flow.rs` | `Next`, in-tile path ends, local accumulation, the exact port solve across tiles |
| `watershed.rs` | Pass A: pits, drainage ends, r supply by end, lowest spill edge per end pair |
| `hierarchy.rs` | Global step 1: pit unions and seam edges, the depression hierarchy (Kruskal over the spill graph), `Resolver` |
| `balance.rs` | Fill-Spill-Merge with r: capacities, fullness, nodes, levels, closed-lake order statistics |
| `flats.rs` | Receivers across full ponds: priority flood from the outlet over the pond's ground, Barnes 2014 mask as tie-break |
| `contract.rs` | Exact per-tile contraction: local Kruskal below the ring minimum, the key remap, the settled fragments (`Frag`) and their local re-solve |
| `nodes.rs` | The node table (`Node`, `Nodes`): every water node by canonical key, global and settled in tiles |
| `pipeline.rs` | The model run (passes A, B, L, C–F) and its rasters |
| `hydraulic.rs` | Hydraulic geometry: width and depth from discharge with the configured m³/s factor |
| `bodies.rs` | River main stems, body ids, `bodies.json` |
| `breach.rs` | Breach-or-fill decisions and graded gorges |
| `channel.rs` | Hydraulic geometry, channel sections, carving, the relaxation corridor |
| `checks.rs` | Hard checks and figures on the outputs, `report.json` |
| `probe.rs` | Test-only: real-window loader and hillshade PNG for `#[ignore]` real-terrain probes |
| `testing.rs` | Test-only: the synthetic world (sea, lagoon, humid and arid bowls) and its setup and grid directory |

Other Rust changes:

| File | Change |
| --- | --- |
| `crates/topography-core/src/world/mod.rs` | `pub mod water;` |
| `crates/topography-core/src/world/hydrology/grid.rs` | `Stage` becomes `pub(crate)`; new `Stage::copy` (reflink-or-copy a big file into the stage, never a hard link) |
| `crates/topography-cli/src/main.rs` | `world water` and `world water-finish` commands and usage lines |
| `configs/hydrology/water-v2.toml` | New: r per class, `lake_min_m2`, `m3s_per_km2` |
| `scripts/check.sh` | `cargo test … world::water` and a usage-error check |

Python, new subpackage `training/src/topography_training/water/`:

| File | Responsibility |
| --- | --- |
| `__init__.py` | Package docstring |
| `relax.py` | GPU bank relaxation (deterministic Jacobi stencil) |
| `run.py` | Grid building (world and windows), pinned binary, resumable world driver, world assembly |
| `review.py` | Coarse-guide figures against the 4 m world, hillshade crops at the spots |
| `training/tests/test_water_relax.py`, `test_water_run.py`, `test_water_review.py` | `unittest` tests |

Docs: `docs/workflows/terrain-cascade.md` gains a "Hydrology v3 (1 m)" section (Task 18).

## Shared protocols

### Expert review (last step of every task)

Dispatch one subagent with this prompt, filling in the task number, the commit range and the real-terrain output the task names:

```text
You are a hydrology expert reviewing one implementation task of hydrology v3 for a game
terrain tool. Read the spec docs/superpowers/specs/2026-11-05-hydrology-v3-design.md and
the plan docs/superpowers/plans/2026-11-05-hydrology-v3-a-water-model.md (Task <N> and the
Rulings table). Review the diff `git diff <base>..<head>` and the real-terrain output at
<paths>. Judge: (1) does the code do what the spec and rulings say, with no rule that raises
banks, cuts slots, imposes paths or exempts a check; (2) is it exact and deterministic across
tilings where the plan says so; (3) does the real-terrain output look like water behaving like
water (name cells and coordinates for every defect). Report blocking problems first, then
non-blocking ones, each with file:line or world coordinates. Do not edit files.
```

Fix every blocking finding (each fix its own commit), then re-run that task's tests. Record the review verdict in `TASKS.md`.

### Heavy runs

Before a step marked **HEAVY**, stop and tell the user: what runs, the expected peak RAM / VRAM / disk and duration (with the uncertainty stated), and that other applications should be closed. Wait for their go. Then launch as a background Bash job:

```bash
RUN=<run-dir>; mkdir -p "$RUN"
( systemd-run --user --scope -q -p MemoryMax=<cap> <command> > "$RUN/log" 2>&1; echo $? > "$RUN/exit_code" ) &
echo $! > "$RUN/pid"
```

Watch it with a `Monitor` on `grep -E --line-buffered 'water: |relax: |done|Traceback|Error|error|OOM|Killed|nan|FAILED' "$RUN/log"` (re-armed on expiry, at most 30 min each), plus `timeout 3600 tail --pid=$(cat $RUN/pid) -f /dev/null` as an hourly heartbeat for runs over an hour; on each heartbeat report progress, rate and remaining time from the log. A missing PID without `exit_code` is a crash.

### Commands used throughout

```bash
CAP6=(systemd-run --user --scope -q -p MemoryMax=6G)
P=data/models/diffusion-env-v1/.venv/bin/python
"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water   # unit tests
"${CAP6[@]}" cargo clippy --workspace --all-targets --locked --offline -- -D warnings
bash scripts/check.sh
```

Each Rust test step below gives its exact filter. Real-terrain probes are `#[ignore]` tests run with `-- --ignored --nocapture`; they skip (print "no data") when `data/worlds/fujix-55-l4v2` is absent.

### Task-finish block (before every commit, review N5)

```bash
cargo fmt --all
"${CAP6[@]}" cargo clippy -p topography-core -p topography-cli --all-targets --locked --offline -- -D warnings
"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water   # the task's focused tests
"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet hydrology     # v2 stays green
# Python tasks (15–17) instead or in addition:
PYTHONPATH=training/src $P -m unittest discover -s training/tests -p 'test_water_*.py'
# From Task 14 on:
bash scripts/check.sh
```

All must pass; fix the code (never the check) and rerun. Every real-data run in this plan goes through `systemd-run --user --scope -q -p MemoryMax=<cap>` with the cap it names (N13).

---
## Task 1: Water table v2 and the module skeleton

**Files:**
- Create: `configs/hydrology/water-v2.toml`
- Create: `crates/topography-core/src/world/water/mod.rs`
- Create: `crates/topography-core/src/world/water/table.rs`
- Modify: `crates/topography-core/src/world/mod.rs` (add `pub mod water;`)
- Modify: `scripts/check.sh` (after the `cargo test … hydrology` line)

**Interfaces:**
- Consumes: nothing.
- Produces: `water::table::{Table, Level { river_km2: f64, breach_cap_m: f32 }, DEFAULT, MILLI}` with `Table::parse(&str) -> Result<Table>`, `Table::r_milli(u8) -> u32`, `Table::level(spacing_m: f64) -> Level` and the public fields `lake_min_m2: f64`, `m3s_per_km2: f64`; `water::{Params { river_km2, breach_cap_m, breach_radius: u64, channels }, Params::change_max_m() -> f32, params(table: &Table, spacing_m: f64, river_km2: Option<f64>) -> Params}` and the constants `LAKE_DEPTH_M`, `CHANNEL_CAP_M`, `GORGE_RATIO`, `BREACH_RADIUS` (the default `breach_radius`), `MAIN_RIVER_KM2`, `BODY_BUDGET`, `DEFAULT_TILE`. Per-level thresholds live in the config (review N19).

- [ ] **Step 1: Create the branch state check**

Run: `git status --short && git branch --show-current`
Expected: branch `feat/hydrology-v3`; only the untracked plan files and `scratch/`. Leave them alone.

- [ ] **Step 2: Write the water table**

`configs/hydrology/water-v2.toml`:

```toml
# Lake-to-catchment ratio r ≈ P/E per terrain class (legend topography terrain-v3), read by
# `topography world water` (hydrology v3). A depression stands where its lake area equals
# Σ r over its catchment's cells (with upstream overflow); when no level below its spill
# does, it fills and overflows. r >= 1: lakes spill. r = 0.1: arid, mostly closed.
# r = 0: a dry playa (badlands, dunes). Judgment values, 2026-11-05. Classes missing here
# take class 0.
schema_version = 2
kind = "water_table"
legend = "topography terrain-v3"
# Game settings. Standing water is a labelled lake from this area (smaller: an unlabelled
# pond), as well as 0.3 m deep.
lake_min_m2 = 500.0
# Discharge (km² at mean rain) to m³/s for channel width and depth: chosen so the largest
# river (about 177 km²) is about 30 m wide; the model's literal rain would give 0.657.
m3s_per_km2 = 0.56

# Per level (grid spacing in m): river start (km² at mean rain, v2's values) and the deepest
# breach cut (at 1 m the channel cap). `change_max` allows the breach cap + 0.5 m. The 4 m and
# 32 m values serve Plan A's gate-1 windows only; Plan B revisits them.
[levels.1]
river_km2 = 0.05
breach_cap_m = 2.0
[levels.4]
river_km2 = 0.5
breach_cap_m = 5.0
[levels.32]
river_km2 = 10.0
breach_cap_m = 20.0

[classes.0]  # unlabelled
r = 1.0
[classes.1]  # doline, pinnacle and plateau karst (drains underground)
r = 0.3
[classes.2]  # gorges and canyons
r = 0.1
[classes.3]  # fjords and glacial troughs
r = 3.0
[classes.4]  # calderas and craters
r = 1.0
[classes.5]  # archipelagos
r = 2.0
[classes.6]  # alpine ridges and cirques
r = 3.0
[classes.7]  # badlands and mesas (desert basins: playas)
r = 0.0
[classes.8]  # dunes (interdune hollows stay dry)
r = 0.0
[classes.9]  # braided rivers
r = 2.0
[classes.10]  # glaciers and ice fields
r = 3.0
[classes.11]  # volcanic cones and lava fields (porous)
r = 0.5
[classes.12]  # coastal cliffs
r = 1.5
[classes.13]  # layered plateau canyons (2)
r = 0.1
[classes.14]  # cone and tower karst (1)
r = 0.5
[classes.15]  # drowned tower karst (5)
r = 2.0
[classes.16]  # crest-train dunes (8)
r = 0.0
[classes.17]  # cinder cones and lava fields (11)
r = 0.3
[classes.18]  # plain and rolling ground (0)
r = 1.0
[classes.19]  # Grand Canyon (2)
r = 0.1
```

- [ ] **Step 3: Write the failing tests**

`crates/topography-core/src/world/water/table.rs` (tests first, implementation in Step 5):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_table_parses_with_r_in_thousandths() {
        let t = Table::parse(DEFAULT).expect("built-in table");
        assert_eq!(t.r_milli(0), 1000);
        assert_eq!(t.r_milli(8), 0, "dunes are playas");
        assert_eq!(t.r_milli(7), 0);
        assert_eq!(t.r_milli(19), 100);
        assert_eq!(t.r_milli(10), 3000);
        assert_eq!(t.r_milli(200), 1000, "missing classes take class 0");
        assert_eq!(t.lake_min_m2, 500.0);
        assert_eq!(t.m3s_per_km2, 0.56);
        assert_eq!(t.level(1.0), Level { river_km2: 0.05, breach_cap_m: 2.0 });
        assert_eq!(t.level(4.0), Level { river_km2: 0.5, breach_cap_m: 5.0 });
        assert_eq!(t.level(32.0), Level { river_km2: 10.0, breach_cap_m: 20.0 });
        assert_eq!(t.level(2.0).breach_cap_m, 2.0, "the largest level not coarser than the grid");
    }

    #[test]
    fn a_v1_table_or_a_bad_value_is_refused() {
        let levels = "[levels.1]\nriver_km2 = 0.05\nbreach_cap_m = 2.0\n";
        let head = "schema_version = 2\nkind = \"water_table\"\nlegend = \"x\"\nlake_min_m2 = 500.0\nm3s_per_km2 = 0.5\n";
        assert!(Table::parse(&format!("{head}{levels}[classes.0]\nr = 1.0\n")).is_ok());
        let v1 = format!("schema_version = 1\nkind = \"water_table\"\nlegend = \"x\"\nlake_min_m2 = 500.0\nm3s_per_km2 = 0.5\n{levels}[classes.0]\nr = 1.0\n");
        assert!(Table::parse(&v1).is_err());
        assert!(Table::parse(&format!("{head}{levels}[classes.0]\nr = -1.0\n")).is_err());
        assert!(Table::parse(&format!("{head}{levels}[classes.3]\nr = 1.0\n")).is_err(), "no class 0");
        assert!(Table::parse(&format!("{head}{levels}[classes.0]\nlake_share = 0.1\n")).is_err(), "v1 fields are unknown here");
        assert!(Table::parse(&format!("{head}[classes.0]\nr = 1.0\n")).is_err(), "no levels");
        let no_settings = format!("schema_version = 2\nkind = \"water_table\"\nlegend = \"x\"\n{levels}[classes.0]\nr = 1.0\n");
        assert!(Table::parse(&no_settings).is_err());
        let zero = format!("schema_version = 2\nkind = \"water_table\"\nlegend = \"x\"\nlake_min_m2 = 500.0\nm3s_per_km2 = 0.0\n{levels}[classes.0]\nr = 1.0\n");
        assert!(Table::parse(&zero).is_err(), "the m³/s factor must be positive");
    }
}
```

In `mod.rs`, tests for `params`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_follow_the_spacing() {
        let t = table::Table::parse(table::DEFAULT).expect("table");
        let p = |s, r| params(&t, s, r);
        assert_eq!(p(32.0, None), Params { river_km2: 10.0, breach_cap_m: 20.0, breach_radius: BREACH_RADIUS, channels: false });
        assert_eq!(p(4.0, None), Params { river_km2: 0.5, breach_cap_m: 5.0, breach_radius: BREACH_RADIUS, channels: false });
        assert_eq!(p(1.0, None), Params { river_km2: 0.05, breach_cap_m: 2.0, breach_radius: BREACH_RADIUS, channels: true });
        assert_eq!(p(1.0, Some(0.2)).river_km2, 0.2);
        assert_eq!(p(4.0, None).change_max_m(), 5.5, "each level's own breach cap + 0.5 m");
    }
}
```

- [ ] **Step 4: Run the tests to see them fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water`
Expected: compile error (`Table`, `params` not defined).

- [ ] **Step 5: Implement**

`table.rs` (above the tests):

```rust
//! Per-class lake-to-catchment ratio r ≈ P/E (`configs/hydrology/water-v2.toml`), kept in
//! thousandths so supplies sum as integers, and two game settings: the smallest labelled
//! lake and the discharge-to-m³/s factor of the channels.

use crate::acquisition::{AcquisitionError, Result, ensure};
use serde::Deserialize;
use std::collections::BTreeMap;

/// The table built into the binary; `--water FILE` replaces it.
pub(crate) const DEFAULT: &str = include_str!("../../../../../configs/hydrology/water-v2.toml");
/// Supplies are counted in thousandths of a cell.
pub(crate) const MILLI: u64 = 1000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Class {
    r: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    schema_version: u32,
    kind: String,
    #[allow(dead_code)]
    legend: String,
    lake_min_m2: f64,
    m3s_per_km2: f64,
    levels: BTreeMap<String, Level>,
    classes: BTreeMap<String, Class>,
}

/// Per-level thresholds (review N19): river start and breach cap.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Level {
    pub river_km2: f64,
    pub breach_cap_m: f32,
}

/// r in thousandths by class id (0 to 255; ids missing from the file take class 0), the game
/// settings and the per-level thresholds.
pub(crate) struct Table {
    r: Vec<u32>,
    /// Standing water is a labelled lake from this area (and 0.3 m deep).
    pub lake_min_m2: f64,
    /// m³/s per km² at mean rain, for hydraulic geometry.
    pub m3s_per_km2: f64,
    /// (spacing m, thresholds), ascending.
    levels: Vec<(f64, Level)>,
}

impl Table {
    pub fn parse(text: &str) -> Result<Self> {
        let file: File = toml::from_str(text)
            .map_err(|e| AcquisitionError::Invalid(format!("water table: {e}")))?;
        ensure(
            file.schema_version == 2 && file.kind == "water_table",
            "water table must be kind water_table, schema_version 2",
        )?;
        ensure(
            file.lake_min_m2.is_finite() && file.lake_min_m2 >= 0.0,
            "water table lake_min_m2 must be 0 or more",
        )?;
        ensure(
            file.m3s_per_km2.is_finite() && file.m3s_per_km2 > 0.0,
            "water table m3s_per_km2 must be positive",
        )?;
        let mut by_id = BTreeMap::new();
        for (key, class) in file.classes {
            let id: u8 = key
                .parse()
                .map_err(|_| AcquisitionError::Invalid(format!("water table class {key}")))?;
            ensure(
                class.r.is_finite() && (0.0..=100.0).contains(&class.r),
                format!("water table class {id}: r must be from 0 to 100"),
            )?;
            by_id.insert(id, (class.r * MILLI as f64).round() as u32);
        }
        let base = *by_id
            .get(&0)
            .ok_or_else(|| AcquisitionError::Invalid("water table needs class 0".into()))?;
        let mut levels = Vec::new();
        for (key, l) in file.levels {
            let spacing: f64 = key
                .parse()
                .map_err(|_| AcquisitionError::Invalid(format!("water table level {key}")))?;
            ensure(
                spacing > 0.0 && l.river_km2 > 0.0 && l.breach_cap_m >= 0.0,
                format!("water table level {key}: positive spacing and river start, cap 0 or more"),
            )?;
            levels.push((spacing, l));
        }
        levels.sort_by(|a, b| a.0.total_cmp(&b.0));
        ensure(!levels.is_empty(), "water table needs at least one level")?;
        Ok(Self {
            r: (0..=255u8).map(|c| by_id.get(&c).copied().unwrap_or(base)).collect(),
            lake_min_m2: file.lake_min_m2,
            m3s_per_km2: file.m3s_per_km2,
            levels,
        })
    }

    /// r of a class in thousandths.
    pub fn r_milli(&self, class: u8) -> u32 {
        self.r[class as usize]
    }

    /// The thresholds of the largest level not coarser than `spacing_m` (the finest level
    /// for grids finer than every level).
    pub fn level(&self, spacing_m: f64) -> Level {
        self.levels.iter().rev().find(|l| l.0 <= spacing_m).unwrap_or(&self.levels[0]).1
    }
}
```

`mod.rs`:

```rust
//! Hydrology v3: water as one field (ground, water surface, discharge) from one model at every
//! level. Exact graph passes on tiles: the priority-flood spill graph (Barnes 2016), the
//! depression hierarchy and Fill-Spill-Merge (Barnes, Callaghan and Wickert 2020), fall-line
//! receivers, flat ponds (Barnes 2014) and accumulation (Barnes 2017); at 1 m also channels.
//! The GPU bank relaxation runs in Python between `run` and `finish`.
//! See docs/superpowers/specs/2026-11-05-hydrology-v3-design.md.
// Items are wired to the CLI in Task 14, which removes this allow.
#![allow(dead_code)]

mod table;

/// Judgment value (spec): standing water at least this deep is a lake; shallower is a pond.
pub(crate) const LAKE_DEPTH_M: f32 = 0.3;
/// Channel depth cap at 1 m; a 1 m breach plus its channel cuts at most this much too.
pub(crate) const CHANNEL_CAP_M: f32 = 2.0;
/// A gorge's bed falls over at least this many times its deepest cut (spec).
pub(crate) const GORGE_RATIO: f32 = 20.0;
/// Gorge cells lie within this many cells of their spill cell, so every tiling decides alike.
pub(crate) const BREACH_RADIUS: u64 = 256;
/// Judgment value: rivers from this size (km² at mean rain) are main rivers for the
/// one-thalweg check.
pub(crate) const MAIN_RIVER_KM2: f64 = 1.0;
/// Export ids are 16-bit (fewer than 65,535 water bodies); a run must stay well under that
/// (judgment value, review C2).
pub(crate) const BODY_BUDGET: usize = 50_000;
/// Default tile side in cells.
pub const DEFAULT_TILE: u64 = 4096;

/// A run's thresholds: the table's level for the grid spacing (review N19).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Params {
    /// River start, km² at mean rain.
    pub river_km2: f64,
    /// Deepest breach cut (at 1 m the channel cap: breach plus channel stay within it through
    /// the channels' raw − 2 m clamp).
    pub breach_cap_m: f32,
    /// Gorge cells lie within this many cells of their spill cell (tests lower it so a gorge
    /// crosses a tile seam).
    pub breach_radius: u64,
    /// Carve channels (1 m only).
    pub channels: bool,
}

impl Params {
    /// The `change_max` check's bound: this level's breach cap + 0.5 m.
    pub fn change_max_m(&self) -> f32 {
        self.breach_cap_m + 0.5
    }
}

pub(crate) fn params(table: &table::Table, spacing_m: f64, river_km2: Option<f64>) -> Params {
    let level = table.level(spacing_m);
    Params {
        river_km2: river_km2.unwrap_or(level.river_km2),
        breach_cap_m: level.breach_cap_m,
        breach_radius: BREACH_RADIUS,
        channels: spacing_m < 2.0,
    }
}
```

`crates/topography-core/src/world/mod.rs`: add `pub mod water;` after `pub mod hydrology;`.

`scripts/check.sh`: after `cargo test --locked --offline -p topography-core --quiet hydrology` add

```bash
cargo test --locked --offline -p topography-core --quiet world::water
```

- [ ] **Step 6: Run the tests to see them pass**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water`
Expected: 3 tests pass.
Run: `"${CAP6[@]}" cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

Run the task-finish block first.

```bash
git add configs/hydrology/water-v2.toml crates/topography-core/src/world/water crates/topography-core/src/world/mod.rs scripts/check.sh
git commit -m "feat(water): water table v2 with r per class and the v3 module skeleton

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 8: Expert review**

Run the expert review protocol. Real-terrain output: none can exist yet; give the reviewer the class legend (`catalog/labels/terrain-v3.toml` or the comments in `water-v1.toml`) and ask specifically whether the r values are plausible per class (badlands and dunes are playas, r = 0) and whether `lake_min_m2` and `m3s_per_km2` give plausible game water.

---

## Task 2: Geometry, layers, rain and the work store

**Files:**
- Create: `crates/topography-core/src/world/water/geom.rs`
- Create: `crates/topography-core/src/world/water/layers.rs`
- Create: `crates/topography-core/src/world/water/rain.rs`
- Create: `crates/topography-core/src/world/water/store.rs`
- Modify: `crates/topography-core/src/world/water/mod.rs` (add `mod geom; mod layers; mod rain; mod store;`)

**Interfaces:**
- Consumes: `crate::acquisition::{Result, ensure, io_context, AcquisitionError}`, `hydrology::grid::read_window` (already `pub(crate)`).
- Produces:
  - `geom::{OFFSETS: [(i64, i64); 8], step_len(u8) -> f64, World { w: u64, h: u64 }, Rect { x0: u64, y0: u64, w: usize, h: usize }, tiles(World, u64) -> Vec<Rect>, tile_of(World, u64, u64, u64) -> usize, UnionFind}` with `World::{index, xy, is_edge, neighbour}`, `Rect::{cells, contains, local, world_xy, grow, on_ring, whole}`, `UnionFind::{new, find, union}`.
  - `layers::{Cell, Layer<T>, Classes}`: `Layer::{memory, filled, open, open_or_create, read, write, world, to_vec}`, `Classes::{read}`.
  - `rain::{Rain, Q_PER_KM2}`: `Rain::{read, uniform, at, units}`, field `mean_land_mm_h`.
  - `store::{Blob, Enc, Dec, encode, decode, Work, cached, for_tiles}`.

- [ ] **Step 1: Write the failing tests**

`geom.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_cover_the_world_once() {
        let world = World { w: 10, h: 7 };
        let t = tiles(world, 4);
        assert_eq!(t.len(), 3 * 2);
        assert_eq!(t.iter().map(|r| r.cells()).sum::<usize>(), 70);
        assert_eq!(t[2], Rect { x0: 8, y0: 0, w: 2, h: 4 });
        assert_eq!(tile_of(world, 4, 9, 6), 5);
    }

    #[test]
    fn a_grown_rect_is_clipped_and_maps_back() {
        let world = World { w: 10, h: 10 };
        let core = Rect { x0: 0, y0: 4, w: 4, h: 4 };
        let win = core.grow(2, world);
        assert_eq!(win, Rect { x0: 0, y0: 2, w: 6, h: 8 });
        assert_eq!(win.local(1, 3), Some(7));
        assert_eq!(win.world_xy(7), (1, 3));
        assert!(core.on_ring(0, 5) && !core.on_ring(1, 5));
    }

    #[test]
    fn union_find_keeps_the_smaller_root() {
        let mut u = UnionFind::new(5);
        u.union(3, 1);
        u.union(4, 3);
        assert_eq!(u.find(4), 1);
        assert_eq!(u.find(0), 0);
    }
}
```

`layers.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_layer_reads_and_writes_windows() {
        let dir = tempfile::tempdir().expect("temp");
        let world = World { w: 5, h: 4 };
        let layer = Layer::<f32>::open_or_create(world, &dir.path().join("a.f32"), f32::NAN).expect("create");
        let r = Rect { x0: 1, y0: 1, w: 3, h: 2 };
        layer.write(r, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).expect("write");
        let back = layer.read(Rect { x0: 0, y0: 1, w: 5, h: 1 }).expect("read");
        assert!(back[0].is_nan() && back[4].is_nan());
        assert_eq!(&back[1..4], &[1.0, 2.0, 3.0]);
        let again = Layer::<f32>::open_or_create(world, &dir.path().join("a.f32"), 0.0).expect("reopen");
        assert_eq!(again.read(r).expect("read"), vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], "kept");
    }

    #[test]
    fn classes_expand_their_coarse_cells() {
        let coarse = Layer::memory(World { w: 2, h: 2 }, vec![1u8, 2, 3, 4]);
        let classes = Classes { layer: coarse, k: 4 };
        let got = classes.read(Rect { x0: 3, y0: 3, w: 2, h: 2 }).expect("read");
        assert_eq!(got, vec![1, 2, 3, 4]);
    }
}
```

`rain.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rain_is_bilinear_and_in_integer_units() {
        let r = Rain {
            w: 2, h: 1, spacing_m: 128.0, nw_m: [0.0, 128.0], values: vec![0.0, 1.0],
            weight: 2.0, mean_land_mm_h: 2.0, entry: Default::default(),
        };
        assert_eq!(r.at(64.0, 64.0), 0.0);
        assert_eq!(r.at(128.0, 64.0), 0.5);
        assert_eq!(r.at(500.0, 64.0), 1.0, "clamped past the last centre");
        // 1 m² cell at moisture 0.5, weight 2: 1e-6 km² = 1e6 units.
        assert_eq!(r.units(128.0, 64.0, 1e-6), 1_000_000);
    }
}
```

`store.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn blobs_round_trip() {
        let mut m = BTreeMap::new();
        m.insert(3u64, (1.5f32, vec![1u32, 2]));
        let v = (vec![7u8, 9], m, -4i64, [1u64, 2, 3, 4], true);
        let back: (Vec<u8>, BTreeMap<u64, (f32, Vec<u32>)>, i64, [u64; 4], bool) =
            decode(&encode(&v)).expect("decode");
        assert_eq!(back, v);
        assert!(decode::<u64>(&[1, 2]).is_err());
    }

    #[test]
    fn work_caches_and_refuses_other_inputs() {
        let dir = tempfile::tempdir().expect("temp");
        let stamp = serde_json::json!({"input": "a"});
        let calls = AtomicUsize::new(0);
        let work = Work::open(dir.path(), &stamp).expect("open");
        let f = || -> Result<u64> { calls.fetch_add(1, Ordering::Relaxed); Ok(42) };
        assert_eq!(work.cached("m1/a/0000", f).expect("first"), 42);
        assert_eq!(work.cached("m1/a/0000", f).expect("second"), 42);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(Work::open(dir.path(), &serde_json::json!({"input": "b"})).is_err());
    }

    #[test]
    fn for_tiles_keeps_tile_order() {
        let tiles: Vec<Rect> = (0..9).map(|i| Rect { x0: i, y0: 0, w: 1, h: 1 }).collect();
        let got = for_tiles(&tiles, 4, "test", |i, r| Ok(i as u64 * 10 + r.x0)).expect("run");
        assert_eq!(got, (0..9).map(|i| i * 11).collect::<Vec<u64>>());
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water`
Expected: compile errors (types not defined).

- [ ] **Step 3: Implement `geom.rs`**

```rust
//! World and tile geometry. Cells are indexed in the world (u64, rows north to south); a tile
//! is a block of them, a window a block grown by a halo and clipped to the world.

/// D8 offsets (row, column); direction byte = index + 1: 1 N, 2 S, 3 W, 4 E, 5 NW, 6 NE, 7 SW,
/// 8 SE (as v2's `flow.u8`).
pub(crate) const OFFSETS: [(i64, i64); 8] =
    [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (-1, 1), (1, -1), (1, 1)];

/// Step length in cells for a direction byte.
pub(crate) fn step_len(d: u8) -> f64 {
    if d > 4 { std::f64::consts::SQRT_2 } else { 1.0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct World {
    pub w: u64,
    pub h: u64,
}

impl World {
    pub fn index(self, x: u64, y: u64) -> u64 {
        y * self.w + x
    }
    pub fn xy(self, i: u64) -> (u64, u64) {
        (i % self.w, i / self.w)
    }
    pub fn is_edge(self, x: u64, y: u64) -> bool {
        x == 0 || y == 0 || x + 1 == self.w || y + 1 == self.h
    }
    /// Neighbour `k` (an `OFFSETS` index) of (x, y), if inside the world.
    pub fn neighbour(self, x: u64, y: u64, k: usize) -> Option<(u64, u64)> {
        let (dy, dx) = OFFSETS[k];
        let nx = x.checked_add_signed(dx)?;
        let ny = y.checked_add_signed(dy)?;
        (nx < self.w && ny < self.h).then_some((nx, ny))
    }
}

/// A block of cells in world coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rect {
    pub x0: u64,
    pub y0: u64,
    pub w: usize,
    pub h: usize,
}

impl Rect {
    pub fn whole(world: World) -> Self {
        Rect { x0: 0, y0: 0, w: world.w as usize, h: world.h as usize }
    }
    pub fn cells(self) -> usize {
        self.w * self.h
    }
    pub fn contains(self, x: u64, y: u64) -> bool {
        x >= self.x0 && y >= self.y0 && x < self.x0 + self.w as u64 && y < self.y0 + self.h as u64
    }
    /// Index of world cell (x, y) in this block, if inside.
    pub fn local(self, x: u64, y: u64) -> Option<usize> {
        self.contains(x, y)
            .then(|| (y - self.y0) as usize * self.w + (x - self.x0) as usize)
    }
    pub fn world_xy(self, k: usize) -> (u64, u64) {
        (self.x0 + (k % self.w) as u64, self.y0 + (k / self.w) as u64)
    }
    /// This block grown by `halo` cells on every side, clipped to the world.
    pub fn grow(self, halo: u64, world: World) -> Rect {
        let x0 = self.x0.saturating_sub(halo);
        let y0 = self.y0.saturating_sub(halo);
        let x1 = (self.x0 + self.w as u64 + halo).min(world.w);
        let y1 = (self.y0 + self.h as u64 + halo).min(world.h);
        Rect { x0, y0, w: (x1 - x0) as usize, h: (y1 - y0) as usize }
    }
    /// On this block's outer ring of cells.
    pub fn on_ring(self, x: u64, y: u64) -> bool {
        self.contains(x, y)
            && (x == self.x0
                || y == self.y0
                || x + 1 == self.x0 + self.w as u64
                || y + 1 == self.y0 + self.h as u64)
    }
}

/// Tiles of `size` cells covering the world row by row; the last ones may be smaller.
pub(crate) fn tiles(world: World, size: u64) -> Vec<Rect> {
    let mut out = Vec::new();
    for y0 in (0..world.h).step_by(size as usize) {
        for x0 in (0..world.w).step_by(size as usize) {
            out.push(Rect {
                x0,
                y0,
                w: size.min(world.w - x0) as usize,
                h: size.min(world.h - y0) as usize,
            });
        }
    }
    out
}

/// Index (in `tiles` order) of the tile holding (x, y).
pub(crate) fn tile_of(world: World, size: u64, x: u64, y: u64) -> usize {
    (y / size * world.w.div_ceil(size) + x / size) as usize
}

/// Union-find over 0..n whose roots are the smallest member (deterministic).
pub(crate) struct UnionFind(Vec<u32>);

impl UnionFind {
    pub fn new(n: usize) -> Self {
        Self((0..n as u32).collect())
    }
    pub fn find(&mut self, mut a: u32) -> u32 {
        while self.0[a as usize] != a {
            let up = self.0[self.0[a as usize] as usize];
            self.0[a as usize] = up;
            a = up;
        }
        a
    }
    /// Joins the sets of `a` and `b`; returns the new root (the smaller one).
    pub fn union(&mut self, a: u32, b: u32) -> u32 {
        let (ra, rb) = (self.find(a), self.find(b));
        let (lo, hi) = (ra.min(rb), ra.max(rb));
        self.0[hi as usize] = lo;
        lo
    }
}
```

- [ ] **Step 4: Implement `layers.rs`**

```rust
//! World-sized rasters, in memory (tests, windows) or in little-endian files (the world):
//! windowed reads and positioned writes, so tiles never need the whole raster.

use super::geom::{Rect, World};
use crate::acquisition::{Result, ensure, io_context};
use std::{
    fs,
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

/// A raster cell type stored little-endian.
pub(crate) trait Cell: Copy + Send + Sync + PartialEq + 'static {
    const B: usize;
    fn get(b: &[u8]) -> Self;
    fn put(self, out: &mut Vec<u8>);
}

macro_rules! cell {
    ($t:ty) => {
        impl Cell for $t {
            const B: usize = std::mem::size_of::<$t>();
            fn get(b: &[u8]) -> Self {
                <$t>::from_le_bytes(b.try_into().expect("cell width"))
            }
            fn put(self, out: &mut Vec<u8>) {
                out.extend_from_slice(&self.to_le_bytes());
            }
        }
    };
}
cell!(f32);
cell!(u8);
cell!(u32);
cell!(u64);

#[derive(Clone)]
pub(crate) enum Layer<T: Cell> {
    Memory { world: World, values: Arc<RwLock<Vec<T>>> },
    File { world: World, path: PathBuf },
}

impl<T: Cell> Layer<T> {
    pub fn memory(world: World, values: Vec<T>) -> Self {
        assert_eq!(values.len() as u64, world.w * world.h);
        Layer::Memory { world, values: Arc::new(RwLock::new(values)) }
    }
    pub fn filled(world: World, fill: T) -> Self {
        Self::memory(world, vec![fill; (world.w * world.h) as usize])
    }
    /// An existing file of exactly the world's size.
    pub fn open(world: World, path: &Path) -> Result<Self> {
        let size = io_context(format!("inspect {}", path.display()), fs::metadata(path))?.len();
        ensure(
            size == world.w * world.h * T::B as u64,
            format!("{} has {size} bytes, expected {}", path.display(), world.w * world.h * T::B as u64),
        )?;
        Ok(Layer::File { world, path: path.to_path_buf() })
    }
    /// The file at `path`, created full of `fill` when missing (a resumed run keeps it).
    pub fn open_or_create(world: World, path: &Path, fill: T) -> Result<Self> {
        if path.exists() {
            return Self::open(world, path);
        }
        let tmp = path.with_extension("partial");
        let file = io_context(format!("create {}", tmp.display()), fs::File::create(&tmp))?;
        let mut row = Vec::with_capacity(world.w as usize * T::B);
        for _ in 0..world.w {
            fill.put(&mut row);
        }
        for y in 0..world.h {
            io_context("fill layer", file.write_all_at(&row, y * row.len() as u64))?;
        }
        io_context("sync layer", file.sync_all())?;
        io_context("publish layer", fs::rename(&tmp, path))?;
        Self::open(world, path)
    }
    pub fn world(&self) -> World {
        match self {
            Layer::Memory { world, .. } | Layer::File { world, .. } => *world,
        }
    }
    pub fn read(&self, r: Rect) -> Result<Vec<T>> {
        let world = self.world();
        ensure(
            r.x0 + r.w as u64 <= world.w && r.y0 + r.h as u64 <= world.h,
            "raster window out of range",
        )?;
        let mut out = Vec::with_capacity(r.cells());
        match self {
            Layer::Memory { values, .. } => {
                let v = values.read().expect("layer lock");
                for y in r.y0..r.y0 + r.h as u64 {
                    let at = (y * world.w + r.x0) as usize;
                    out.extend_from_slice(&v[at..at + r.w]);
                }
            }
            Layer::File { path, .. } => {
                let file = io_context(format!("open {}", path.display()), fs::File::open(path))?;
                let mut bytes = vec![0u8; r.w * T::B];
                for y in r.y0..r.y0 + r.h as u64 {
                    let at = (y * world.w + r.x0) * T::B as u64;
                    io_context(format!("read {}", path.display()), file.read_exact_at(&mut bytes, at))?;
                    out.extend(bytes.chunks_exact(T::B).map(T::get));
                }
            }
        }
        Ok(out)
    }
    pub fn write(&self, r: Rect, values: &[T]) -> Result<()> {
        let world = self.world();
        ensure(values.len() == r.cells(), "raster write size")?;
        match self {
            Layer::Memory { values: v, .. } => {
                let mut v = v.write().expect("layer lock");
                for (row, y) in (r.y0..r.y0 + r.h as u64).enumerate() {
                    let at = (y * world.w + r.x0) as usize;
                    v[at..at + r.w].copy_from_slice(&values[row * r.w..(row + 1) * r.w]);
                }
            }
            Layer::File { path, .. } => {
                let file = io_context(
                    format!("open {}", path.display()),
                    fs::OpenOptions::new().write(true).open(path),
                )?;
                let mut bytes = Vec::with_capacity(r.w * T::B);
                for (row, y) in (r.y0..r.y0 + r.h as u64).enumerate() {
                    bytes.clear();
                    for &c in &values[row * r.w..(row + 1) * r.w] {
                        c.put(&mut bytes);
                    }
                    let at = (y * world.w + r.x0) * T::B as u64;
                    io_context(format!("write {}", path.display()), file.write_all_at(&bytes, at))?;
                }
            }
        }
        Ok(())
    }
    /// The whole raster (tests and small windows).
    pub fn to_vec(&self) -> Result<Vec<T>> {
        self.read(Rect::whole(self.world()))
    }
}

/// Classes on a coarser grid: `k` grid cells per class cell side.
#[derive(Clone)]
pub(crate) struct Classes {
    pub layer: Layer<u8>,
    pub k: u64,
}

impl Classes {
    pub fn read(&self, r: Rect) -> Result<Vec<u8>> {
        let cw = self.layer.world();
        let (cx0, cy0) = (r.x0 / self.k, r.y0 / self.k);
        let cx1 = ((r.x0 + r.w as u64 - 1) / self.k + 1).min(cw.w);
        let cy1 = ((r.y0 + r.h as u64 - 1) / self.k + 1).min(cw.h);
        let c = Rect { x0: cx0, y0: cy0, w: (cx1 - cx0) as usize, h: (cy1 - cy0) as usize };
        let coarse = self.layer.read(c)?;
        Ok((0..r.cells())
            .map(|k| {
                let (x, y) = r.world_xy(k);
                let (cx, cy) = ((x / self.k).min(cx1 - 1), (y / self.k).min(cy1 - 1));
                coarse[(cy - cy0) as usize * c.w + (cx - cx0) as usize]
            })
            .collect())
    }
}
```

- [ ] **Step 5: Implement `rain.rs`**

```rust
//! The 128 m moisture field from the 32 m Smith–Barstad stage and each cell's rain in integer
//! Q units, so discharge sums exactly across tiles.

use crate::acquisition::{AcquisitionError, Result, ensure, io_context};
use serde_json::{Map, Value};
use std::{fs, path::Path};

/// Q units per km² at mean rain: Q is a u64 count of 1e-12 km² (1e-6 m²).
pub(crate) const Q_PER_KM2: f64 = 1e12;

pub(crate) struct Rain {
    pub(crate) w: usize,
    pub(crate) h: usize,
    pub(crate) spacing_m: f64,
    pub(crate) nw_m: [f64; 2],
    pub(crate) values: Vec<f32>,
    /// Discharge weight per moisture unit: scale_mm_h / mean_land_mm_h (as v2).
    pub(crate) weight: f64,
    pub mean_land_mm_h: f64,
    /// The grid.json `moisture` entry, passed on to the outputs.
    pub entry: Map<String, Value>,
}

impl Rain {
    /// From a grid directory whose grid.json has a `moisture` entry (file, size, spacing_m,
    /// nw_m, scale_mm_h) and `discharge.mean_land_mm_h`.
    pub fn read(dir: &Path, manifest: &Map<String, Value>) -> Result<Self> {
        let entry = manifest
            .get("moisture")
            .and_then(Value::as_object)
            .cloned()
            .ok_or_else(|| AcquisitionError::Invalid("grid.json lacks a moisture entry".into()))?;
        let [w, h]: [usize; 2] = serde_json::from_value(entry["size"].clone())
            .map_err(|_| AcquisitionError::Invalid("moisture size must be [w, h]".into()))?;
        let nw_m: [f64; 2] = serde_json::from_value(entry["nw_m"].clone())
            .map_err(|_| AcquisitionError::Invalid("moisture nw_m must be [x, y]".into()))?;
        let spacing_m = entry["spacing_m"].as_f64().unwrap_or(f64::NAN);
        let scale = entry["scale_mm_h"].as_f64().unwrap_or(f64::NAN);
        let mean = manifest
            .get("discharge")
            .and_then(|d| d["mean_land_mm_h"].as_f64())
            .unwrap_or(f64::NAN);
        ensure(
            spacing_m > 0.0 && scale > 0.0 && mean > 0.0,
            "moisture needs spacing_m, scale_mm_h and discharge.mean_land_mm_h",
        )?;
        let file = entry["file"].as_str().unwrap_or("moisture.f32");
        let bytes = io_context(format!("read {file}"), fs::read(dir.join(file)))?;
        ensure(bytes.len() == w * h * 4, "moisture.f32 does not match its size")?;
        let values = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        Ok(Self { w, h, spacing_m, nw_m, values, weight: scale / mean, mean_land_mm_h: mean, entry })
    }

    /// Moisture 1 and weight 1 everywhere (tests).
    pub fn uniform(mean_land_mm_h: f64) -> Self {
        Self {
            w: 1, h: 1, spacing_m: 1e9, nw_m: [-5e8, 5e8], values: vec![1.0],
            weight: 1.0, mean_land_mm_h, entry: Map::new(),
        }
    }

    /// Bilinear between cell centres at a point of the layout plane (x east, y north), as v2.
    pub fn at(&self, x: f64, y: f64) -> f64 {
        let fx = ((x - self.nw_m[0]) / self.spacing_m - 0.5).clamp(0.0, (self.w - 1) as f64);
        let fy = ((self.nw_m[1] - y) / self.spacing_m - 0.5).clamp(0.0, (self.h - 1) as f64);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (ax, ay) = (fx - x0 as f64, fy - y0 as f64);
        let p = |x: usize, y: usize| f64::from(self.values[y * self.w + x]);
        let top = p(x0, y0) * (1.0 - ax) + p(x1, y0) * ax;
        let bottom = p(x0, y1) * (1.0 - ax) + p(x1, y1) * ax;
        top * (1.0 - ay) + bottom * ay
    }

    /// A cell's rain in Q units (1e-12 km² at mean rain) at its centre (x, y).
    pub fn units(&self, x: f64, y: f64, cell_km2: f64) -> u64 {
        (self.at(x, y) * self.weight * cell_km2 * Q_PER_KM2).round() as u64
    }
}
```

- [ ] **Step 6: Implement `store.rs`**

```rust
//! The work directory: each pass's per-tile results and the global results, so an
//! interrupted run resumes; a small little-endian codec (`Blob`); and `for_tiles`.

use super::geom::Rect;
use crate::acquisition::{AcquisitionError, Result, ensure, io_context};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, atomic::{AtomicUsize, Ordering}},
    time::Instant,
};

pub(crate) struct Enc(pub Vec<u8>);

pub(crate) struct Dec<'a> {
    b: &'a [u8],
    at: usize,
}

impl Dec<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        ensure(self.at + n <= self.b.len(), "work file is truncated")?;
        self.at += n;
        Ok(&self.b[self.at - n..self.at])
    }
}

pub(crate) trait Blob: Sized {
    fn put(&self, e: &mut Enc);
    fn get(d: &mut Dec) -> Result<Self>;
}

macro_rules! blob_num {
    ($t:ty) => {
        impl Blob for $t {
            fn put(&self, e: &mut Enc) {
                e.0.extend_from_slice(&self.to_le_bytes());
            }
            fn get(d: &mut Dec) -> Result<Self> {
                let b = d.take(std::mem::size_of::<$t>())?;
                Ok(<$t>::from_le_bytes(b.try_into().expect("width")))
            }
        }
    };
}
blob_num!(u8);
blob_num!(u32);
blob_num!(u64);
blob_num!(i64);
blob_num!(f32);

impl Blob for bool {
    fn put(&self, e: &mut Enc) {
        u8::from(*self).put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        Ok(u8::get(d)? != 0)
    }
}

impl<T: Blob> Blob for Vec<T> {
    fn put(&self, e: &mut Enc) {
        (self.len() as u64).put(e);
        for v in self {
            v.put(e);
        }
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let n = u64::get(d)? as usize;
        ensure(n <= d.b.len(), "work file length is corrupt")?;
        (0..n).map(|_| T::get(d)).collect()
    }
}

impl<const N: usize> Blob for [u64; N] {
    fn put(&self, e: &mut Enc) {
        for v in self {
            v.put(e);
        }
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let mut out = [0u64; N];
        for v in &mut out {
            *v = u64::get(d)?;
        }
        Ok(out)
    }
}

impl<K: Blob + Ord, V: Blob> Blob for BTreeMap<K, V> {
    fn put(&self, e: &mut Enc) {
        (self.len() as u64).put(e);
        for (k, v) in self {
            k.put(e);
            v.put(e);
        }
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let n = u64::get(d)?;
        (0..n).map(|_| Ok((K::get(d)?, V::get(d)?))).collect()
    }
}

macro_rules! blob_tuple {
    ($($n:ident),+) => {
        impl<$($n: Blob),+> Blob for ($($n,)+) {
            #[allow(non_snake_case)]
            fn put(&self, e: &mut Enc) {
                let ($($n,)+) = self;
                $($n.put(e);)+
            }
            fn get(d: &mut Dec) -> Result<Self> {
                Ok(($($n::get(d)?,)+))
            }
        }
    };
}
blob_tuple!(A, B);
blob_tuple!(A, B, C);
blob_tuple!(A, B, C, D);
blob_tuple!(A, B, C, D, E);

pub(crate) fn encode<T: Blob>(v: &T) -> Vec<u8> {
    let mut e = Enc(Vec::new());
    v.put(&mut e);
    e.0
}

pub(crate) fn decode<T: Blob>(b: &[u8]) -> Result<T> {
    let mut d = Dec { b, at: 0 };
    let v = T::get(&mut d)?;
    ensure(d.at == b.len(), "work file has trailing bytes")?;
    Ok(v)
}

pub(crate) struct Work {
    dir: PathBuf,
}

impl Work {
    /// Opens (creating) a work directory for one set of inputs. `stamp` (input digests, tile
    /// size, tool version) must equal what an earlier run wrote, so a resume never mixes runs.
    pub fn open(dir: &Path, stamp: &Value) -> Result<Self> {
        io_context(format!("create {}", dir.display()), fs::create_dir_all(dir))?;
        let path = dir.join("stamp.json");
        if path.exists() {
            let old: Value = serde_json::from_slice(&io_context("read stamp", fs::read(&path))?)?;
            ensure(
                &old == stamp,
                format!("{} belongs to other inputs or settings; remove it to start again", dir.display()),
            )?;
        } else {
            io_context("write stamp", fs::write(&path, serde_json::to_vec_pretty(stamp)?))?;
        }
        Ok(Self { dir: dir.to_path_buf() })
    }
    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
    /// A result read back when present, else computed and written (atomically).
    pub fn cached<T: Blob>(&self, name: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let path = self.dir.join(format!("{name}.bin"));
        if path.exists() {
            return decode(&io_context(format!("read {}", path.display()), fs::read(&path))?);
        }
        let v = f()?;
        if let Some(parent) = path.parent() {
            io_context("create work dir", fs::create_dir_all(parent))?;
        }
        let tmp = path.with_extension("partial");
        io_context("write work file", fs::write(&tmp, encode(&v)))?;
        io_context("publish work file", fs::rename(&tmp, &path))?;
        Ok(v)
    }
    pub fn is_done(&self, name: &str) -> bool {
        self.dir.join(format!("{name}.done")).exists()
    }
    pub fn mark_done(&self, name: &str) -> Result<()> {
        let path = self.dir.join(format!("{name}.done"));
        if let Some(parent) = path.parent() {
            io_context("create work dir", fs::create_dir_all(parent))?;
        }
        io_context("mark done", fs::write(path, b""))
    }
}

/// `Work::cached` when there is a work directory; in-memory runs compute every time.
pub(crate) fn cached<T: Blob>(work: Option<&Work>, name: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    match work {
        Some(w) => w.cached(name, f),
        None => f(),
    }
}

/// Runs `f` on every tile, `threads` at a time; results in tile order. Progress goes to
/// stderr as `water: <label> <done>/<tiles>` lines.
pub(crate) fn for_tiles<T: Send>(
    tiles: &[Rect],
    threads: usize,
    label: &str,
    f: impl Fn(usize, Rect) -> Result<T> + Sync,
) -> Result<Vec<T>> {
    let started = Instant::now();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<Result<T>>>> = tiles.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= tiles.len() {
                    break;
                }
                let r = f(i, tiles[i]);
                *slots[i].lock().expect("slot") = Some(r);
                let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                if tiles.len() > 16 && d % 16 == 0 {
                    eprintln!("water: {label} {d}/{} ({:.0} s)", tiles.len(), started.elapsed().as_secs_f64());
                }
            });
        }
    });
    eprintln!("water: {label} done in {:.1} s", started.elapsed().as_secs_f64());
    slots
        .into_iter()
        .map(|m| {
            m.into_inner()
                .expect("slot")
                .unwrap_or_else(|| Err(AcquisitionError::Invalid("a tile did not run".into())))
        })
        .collect()
}
```

Add to `mod.rs`: `mod geom; mod layers; mod rain; mod store;`.

- [ ] **Step 7: Run the tests to see them pass**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water`
Expected: all tests pass (3 from Task 1, 9 new).
Run clippy as in the shared commands. Expected: clean.

- [ ] **Step 8: Commit**

Run the task-finish block first.

```bash
git add crates/topography-core/src/world/water
git commit -m "feat(water): tile geometry, windowed rasters, rain units and a resumable work store

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 9: Expert review**

Expert review protocol. Real-terrain output: none yet; give the reviewer `data/worlds/fujix-55-chain/hydrology/4m/grid.json` (its `moisture` and `discharge` entries) and ask them to confirm that `units()` reproduces v2's discharge weighting (moisture × scale / mean × cell area) and that the integer unit (1e-12 km²) cannot overflow a world (4,300 km² = 4.3·10¹⁵ < 1.8·10¹⁹).

---

## Task 3: Fall-line receivers and real-window probes

**Files:**
- Create: `crates/topography-core/src/world/water/receivers.rs`
- Create: `crates/topography-core/src/world/water/probe.rs`
- Modify: `crates/topography-core/src/world/water/mod.rs` (`mod receivers;` and `#[cfg(test)] mod probe;`)

**Interfaces:**
- Consumes: `geom::{World, Rect, OFFSETS, step_len}`, `layers::Layer`.
- Produces: `receivers::receivers(world: World, win: Rect, z: &[f32], core: Rect) -> Vec<u8>` (direction byte per core cell, 0 none; `win` is `core.grow(1, world)` or larger and `z` covers `win`). Test-only `probe::{WORLD, window(east_m: u64, north_m: u64, size: usize) -> Option<(Rect, Vec<f32>)>, shade_png(name: &str, z: &[f32], w: usize, wet: &[bool])}`.

- [ ] **Step 1: Write the failing tests**

In `receivers.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn plane(w: usize, h: usize, deg: f32) -> Vec<f32> {
        let a = deg.to_radians();
        (0..w * h)
            .map(|i| 100.0 - (i % w) as f32 * a.cos() - (i / w) as f32 * a.sin())
            .collect()
    }

    #[test]
    fn paths_follow_the_fall_line_on_a_plane() {
        // A plane falling 20° south of east: paths mix E and SE steps, never a straight line.
        let world = World { w: 60, h: 60 };
        let all = Rect::whole(world);
        let z = plane(60, 60, 20.0);
        let dir = receivers(world, all, &z, all);
        let (mut x, mut y, mut steps) = (1i64, 10i64, 0);
        loop {
            let d = dir[(y * 60 + x) as usize];
            if d == 0 {
                break;
            }
            let (dy, dx) = OFFSETS[d as usize - 1];
            (x, y, steps) = (x + dx, y + dy, steps + 1);
        }
        let slope = (y - 10) as f32 / (x - 1) as f32;
        assert!(steps > 20);
        assert!((slope - 20f32.to_radians().tan()).abs() < 0.1, "path slope {slope}");
    }

    #[test]
    fn receivers_are_strictly_lower_and_edges_have_none() {
        let world = World { w: 30, h: 30 };
        let all = Rect::whole(world);
        let z: Vec<f32> = (0..900)
            .map(|i| ((i * 7919) % 101) as f32 * 0.1 + (i % 30) as f32 * 0.05)
            .collect();
        let dir = receivers(world, all, &z, all);
        for k in 0..900 {
            let (x, y) = all.world_xy(k);
            if world.is_edge(x, y) {
                assert_eq!(dir[k], 0);
            } else if dir[k] != 0 {
                let (nx, ny) = world.neighbour(x, y, dir[k] as usize - 1).expect("inside");
                assert!(z[all.local(nx, ny).expect("in")] < z[k]);
            }
        }
    }

    #[test]
    fn tiles_pick_the_same_receivers_as_the_whole() {
        let world = World { w: 40, h: 40 };
        let all = Rect::whole(world);
        let z: Vec<f32> = (0..1600)
            .map(|i| 100.0 - (i % 40) as f32 * 0.9 - (i / 40) as f32 * 0.4 + ((i * 13) % 5) as f32 * 0.05)
            .collect();
        let whole = receivers(world, all, &z, all);
        for core in tiles(world, 16) {
            let win = core.grow(1, world);
            let zw: Vec<f32> = (0..win.cells())
                .map(|k| { let (x, y) = win.world_xy(k); z[all.local(x, y).expect("in")] })
                .collect();
            let part = receivers(world, win, &zw, core);
            for (k, &p) in part.iter().enumerate() {
                let (x, y) = core.world_xy(k);
                assert_eq!(p, whole[all.local(x, y).expect("in")]);
            }
        }
    }
}
```

(`tiles` comes from `super::super::geom::tiles`; add it to the test's `use`.)

- [ ] **Step 2: Run the tests to see them fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::receivers`
Expected: compile error (`receivers` not defined).

- [ ] **Step 3: Implement `receivers.rs`** (port of v2 `flood::fall_line` and the steepest fallback, on world coordinates)

```rust
//! Receivers on dry land: each cell takes the neighbour on its fall line (Horn's 3 x 3
//! gradient), dithered between the two directions that bracket the gradient by an R2
//! low-discrepancy threshold of its world position, when that neighbour is strictly lower;
//! else its steepest strictly lower neighbour; else none (a pit). Plain steepest descent
//! draws straight axis and 45° rivers on smooth slopes (v2's `flood::fall_line`).

use super::geom::{OFFSETS, Rect, World, step_len};

/// Directions at 0°, 45°, … from east turning south (as `OFFSETS` indices).
const AROUND: [usize; 8] = [3, 7, 1, 6, 2, 4, 0, 5];

/// Direction bytes (0 none) of the cells of `core`; `z` covers `win`, which holds `core`
/// grown by at least one cell. World-edge cells get none: water leaves the map there.
pub(crate) fn receivers(world: World, win: Rect, z: &[f32], core: Rect) -> Vec<u8> {
    let mut out = vec![0u8; core.cells()];
    for (k, o) in out.iter_mut().enumerate() {
        let (x, y) = core.world_xy(k);
        if world.is_edge(x, y) {
            continue;
        }
        let at = |dy: i64, dx: i64| {
            let (nx, ny) = ((x as i64 + dx) as u64, (y as i64 + dy) as u64);
            z[win.local(nx, ny).expect("receivers need a one-cell halo")]
        };
        let zc = at(0, 0);
        let mut best: Option<(f32, u8)> = None;
        for (j, &(dy, dx)) in OFFSETS.iter().enumerate() {
            let drop = zc - at(dy, dx);
            if drop > 0.0 {
                let s = drop / step_len(j as u8 + 1) as f32;
                if best.is_none_or(|(b, _)| s > b) {
                    best = Some((s, j as u8 + 1));
                }
            }
        }
        let Some((_, steepest)) = best else { continue };
        *o = steepest;
        let f = |dy: i64, dx: i64| f64::from(at(dy, dx));
        // Horn: east and south rises; the water goes the other way.
        let east = (f(-1, 1) + 2.0 * f(0, 1) + f(1, 1)) - (f(-1, -1) + 2.0 * f(0, -1) + f(1, -1));
        let south = (f(1, -1) + 2.0 * f(1, 0) + f(1, 1)) - (f(-1, -1) + 2.0 * f(-1, 0) + f(-1, 1));
        if east == 0.0 && south == 0.0 {
            continue;
        }
        let turns = (-south).atan2(-east).rem_euclid(std::f64::consts::TAU)
            / std::f64::consts::FRAC_PI_4;
        let sector = turns.floor() as usize % 8;
        // R2 low-discrepancy threshold over the world position: alike in every tiling.
        let t = (x as f64 * 0.754_877_666_2 + y as f64 * 0.569_840_290_9).rem_euclid(1.0);
        let pick = AROUND[if turns.fract() > t { (sector + 1) % 8 } else { sector }];
        let (dy, dx) = OFFSETS[pick];
        if at(dy, dx) < zc {
            *o = pick as u8 + 1;
        }
    }
    out
}
```

- [ ] **Step 4: Implement `probe.rs`** (test-only helpers for real-terrain probes)

```rust
//! Test-only: windows of the l4v2 raw 1 m surface and hillshade PNGs, for the `#[ignore]`
//! real-terrain probes each task's expert review reads.

use super::geom::{Rect, World};
use super::layers::Layer;
use std::path::Path;

pub(crate) const RAW: &str = "../../data/worlds/fujix-55-l4v2/elevation-raw.f32";
pub(crate) const OUT: &str = "../../scratch/hydrology-v3/probes";
pub(crate) const WORLD: World = World { w: 65536, h: 65536 };

/// A `size`² window of the raw surface centred on (east, north) metres; None without data.
pub(crate) fn window(east: u64, north: u64, size: usize) -> Option<(Rect, Vec<f32>)> {
    if !Path::new(RAW).exists() {
        eprintln!("no data: {RAW}");
        return None;
    }
    let half = size as u64 / 2;
    let r = Rect { x0: east - half, y0: WORLD.h - north - half, w: size, h: size };
    let z = Layer::<f32>::open(WORLD, Path::new(RAW)).ok()?.read(r).ok()?;
    Some((r, z))
}

/// A hillshade (north-west sun) of `z`, `w` cells wide, with `wet` cells tinted blue, written
/// to scratch/hydrology-v3/probes/<name>.png.
pub(crate) fn shade_png(name: &str, z: &[f32], w: usize, wet: &[bool]) {
    let h = z.len() / w;
    let at = |x: usize, y: usize| f64::from(z[y.min(h - 1) * w + x.min(w - 1)]);
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let gx = (at(x + 1, y) - at(x.saturating_sub(1), y)) / 2.0;
            let gy = (at(x, y + 1) - at(x, y.saturating_sub(1))) / 2.0;
            let (slope, aspect) = (gx.hypot(gy).atan(), (-gx).atan2(gy));
            let (az, alt) = (315f64.to_radians(), 45f64.to_radians());
            let s = alt.sin() * slope.cos() + alt.cos() * slope.sin() * (az - aspect).cos();
            let g = (s.clamp(0.0, 1.0) * 255.0) as u8;
            if wet[y * w + x] {
                rgb.extend_from_slice(&[g / 3, g / 2, g]);
            } else {
                rgb.extend_from_slice(&[g, g, g]);
            }
        }
    }
    std::fs::create_dir_all(OUT).expect("probe dir");
    let file = std::fs::File::create(Path::new(OUT).join(format!("{name}.png"))).expect("png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().expect("png header").write_image_data(&rgb).expect("png data");
    eprintln!("wrote {OUT}/{name}.png");
}
```

Add a real-terrain probe to `receivers.rs` tests:

```rust
    /// Straight-run share of steepest-descent vs fall-line paths at E49249 N16381 (the
    /// straight river of the diagnosis): cargo test … receivers -- --ignored --nocapture
    #[test]
    #[ignore]
    fn probe_straight_runs_at_e49249() {
        use super::super::probe;
        let Some((win, z)) = probe::window(49249, 16381, 1024) else { return };
        let core = Rect { x0: win.x0 + 1, y0: win.y0 + 1, w: win.w - 2, h: win.h - 2 };
        let world = probe::WORLD;
        let fall = receivers(world, win, &z, core);
        // Steepest descent alone, for comparison.
        let steep: Vec<u8> = (0..core.cells())
            .map(|k| {
                let (x, y) = core.world_xy(k);
                let zc = z[win.local(x, y).expect("in")];
                (0..8)
                    .filter_map(|j| {
                        let (nx, ny) = world.neighbour(x, y, j)?;
                        let d = (zc - z[win.local(nx, ny)?]) / step_len(j as u8 + 1) as f32;
                        (d > 0.0).then_some((d, j as u8 + 1))
                    })
                    .max_by(|a, b| a.0.total_cmp(&b.0))
                    .map_or(0, |b| b.1)
            })
            .collect();
        let share = |dir: &[u8]| {
            // Cells in same-direction runs of 60 or more steps along their receivers.
            let (mut straight, mut total) = (0usize, 0usize);
            for k in 0..core.cells() {
                if dir[k] == 0 { continue; }
                total += 1;
                let (mut run, mut c) = (0, k);
                while run < 60 {
                    let (x, y) = core.world_xy(c);
                    let (dy, dx) = OFFSETS[dir[k] as usize - 1];
                    let Some(n) = core.local((x as i64 + dx) as u64, (y as i64 + dy) as u64) else { break };
                    if dir[n] != dir[k] { break; }
                    (run, c) = (run + 1, n);
                }
                if run >= 60 { straight += 1; }
            }
            straight as f64 / total.max(1) as f64
        };
        eprintln!("straight-run share: steepest {:.3}, fall line {:.3}", share(&steep), share(&fall));
        probe::shade_png("t3-e49249-raw", &z, win.w, &vec![false; z.len()]);
    }
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::receivers`
Expected: 3 pass, 1 ignored.
Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core world::water::receivers::tests::probe_straight_runs_at_e49249 -- --ignored --nocapture`
Expected: prints two shares, the fall-line one lower; writes `scratch/hydrology-v3/probes/t3-e49249-raw.png`.
Run clippy. Expected: clean.

- [ ] **Step 6: Commit**

Run the task-finish block first.

```bash
git add crates/topography-core/src/world/water
git commit -m "feat(water): fall-line receivers on world coordinates and real-window probes

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 7: Expert review**

Expert review protocol. Real-terrain output: the probe's printed shares and `scratch/hydrology-v3/probes/t3-e49249-raw.png`.

---
## Task 4: In-tile paths, local sums and the exact port solve

**Files:**
- Create: `crates/topography-core/src/world/water/flow.rs`
- Modify: `crates/topography-core/src/world/water/mod.rs` (`mod flow;`)

**Interfaces:**
- Consumes: `store::{Blob, Enc, Dec}`.
- Produces:
  - `flow::Next { Cell(u32), Out(u64), Node(u64), End }` (derives `Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord`; implements `Blob`).
  - `flow::ends(n: usize, next: impl Fn(usize) -> Next) -> (Vec<Next>, u64)`: where each cell's in-tile path ends (never `Cell`) and the count of cells on cycles.
  - `flow::accumulate(n, next, rain: impl Fn(usize) -> u64) -> Local` with `Local { q: Vec<u64>, out: BTreeMap<u64, u64>, node_in: BTreeMap<u64, u64>, cycles: u64 }`.
  - `flow::Report { out, node_in, port_end: BTreeMap<u64, Next> }` (`Blob`), `flow::solve(&[Report], outlet: &BTreeMap<u64, Option<u64>>) -> Result<Solved>` with `Solved { inflow: HashMap<u64, u64>, node_total: BTreeMap<u64, u64> }`, `flow::add_inflows(q: &mut [u64], ports: &[(usize, u64)], next)`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::geom::{Rect, World, tiles};

    /// A slope falling east with noise; cells of the block x 5..9, y 5..9 are node 7, which
    /// leaves by (10, 7). Receivers: the lowest lower neighbour; edge cells end.
    fn case(world: World) -> (Vec<f32>, impl Fn(u64, u64) -> Option<Next>) {
        let z: Vec<f32> = (0..world.w * world.h)
            .map(|i| 100.0 - (i % world.w) as f32 + ((i * 7919) % 13) as f32 * 0.01)
            .collect();
        let zz = z.clone();
        let next = move |x: u64, y: u64| -> Option<Next> {
            if (5..10).contains(&x) && (5..10).contains(&y) {
                return Some(Next::Node(7));
            }
            if world.is_edge(x, y) {
                return Some(Next::End);
            }
            let here = zz[world.index(x, y) as usize];
            let mut best: Option<(f32, u64)> = None;
            for k in 0..8 {
                let (nx, ny) = world.neighbour(x, y, k)?;
                let v = zz[world.index(nx, ny) as usize];
                if v < here && best.is_none_or(|b| v < b.0) {
                    best = Some((v, world.index(nx, ny)));
                }
            }
            Some(best.map_or(Next::End, |b| Next::Out(b.1)))
        };
        (z, next)
    }

    /// Discharge on every cell from tiles of `size` (whole world when size >= its side).
    fn discharge(world: World, size: u64) -> Vec<u64> {
        let (_, global) = case(world);
        let outlet = BTreeMap::from([(7u64, Some(world.index(10, 7)))]);
        let rain = |_: usize| 3u64;
        let tiles = tiles(world, size);
        let local_next = |core: Rect| {
            let g = &global;
            move |k: usize| {
                let (x, y) = core.world_xy(k);
                match g(x, y).expect("cell") {
                    Next::Out(c) => {
                        let (cx, cy) = world.xy(c);
                        core.local(cx, cy).map_or(Next::Out(c), |j| Next::Cell(j as u32))
                    }
                    other => other,
                }
            }
        };
        let mut reports = Vec::new();
        for &core in &tiles {
            let next = local_next(core);
            let local = accumulate(core.cells(), &next, rain);
            let (end, cycles) = ends(core.cells(), &next);
            assert_eq!(cycles + local.cycles, 0);
            let mut port_end = BTreeMap::new();
            for (k, &e) in end.iter().enumerate() {
                let (x, y) = core.world_xy(k);
                if core.on_ring(x, y) || world.index(x, y) == world.index(10, 7) {
                    port_end.insert(world.index(x, y), e);
                }
            }
            reports.push(Report { out: local.out, node_in: local.node_in, port_end });
        }
        let solved = solve(&reports, &outlet).expect("solve");
        let mut q = vec![0u64; (world.w * world.h) as usize];
        for &core in &tiles {
            let next = local_next(core);
            let mut local = accumulate(core.cells(), &next, rain).q;
            let ports: Vec<(usize, u64)> = (0..core.cells())
                .filter_map(|k| {
                    let (x, y) = core.world_xy(k);
                    solved.inflow.get(&world.index(x, y)).map(|&a| (k, a))
                })
                .collect();
            add_inflows(&mut local, &ports, &next);
            for (k, &lq) in local.iter().enumerate() {
                let (x, y) = core.world_xy(k);
                q[world.index(x, y) as usize] = match next(k) {
                    Next::Node(n) => solved.node_total[&n],
                    _ => lq,
                };
            }
        }
        q
    }

    #[test]
    fn tiled_sums_equal_the_whole() {
        let world = World { w: 32, h: 24 };
        let whole = discharge(world, 64);
        assert_eq!(discharge(world, 8), whole);
        assert_eq!(discharge(world, 5), whole);
        // Mass: every cell's water ends in a cell whose next is End (the edge or a dry pit), so
        // their discharges sum to all the rain (3 per cell) (N9).
        let (_, global) = case(world);
        let arriving: u64 = (0..world.w * world.h)
            .filter(|&i| global(i % world.w, i / world.w) == Some(Next::End))
            .map(|i| whole[i as usize])
            .sum();
        assert_eq!(arriving, 3 * world.w * world.h);
        assert!(whole.iter().any(|&q| q > 3 * 50), "the node's water flows on");
    }

    #[test]
    fn ends_report_cycles() {
        let next = |k: usize| match k {
            0 => Next::Cell(1),
            1 => Next::Cell(0),
            2 => Next::Cell(0),
            _ => Next::Out(99),
        };
        let (end, cycles) = ends(4, next);
        assert_eq!(cycles, 2);
        assert_eq!(end[3], Next::Out(99));
        assert_eq!(end[2], Next::End);
    }

    #[test]
    fn a_cycle_between_tiles_is_an_error() {
        let a = Report { port_end: BTreeMap::from([(1, Next::Out(2))]), ..Default::default() };
        let b = Report { port_end: BTreeMap::from([(2, Next::Out(1))]), ..Default::default() };
        assert!(solve(&[a, b], &BTreeMap::new()).is_err());
    }
}
```


- [ ] **Step 2: Run the tests to see them fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::flow`
Expected: compile errors.

- [ ] **Step 3: Implement `flow.rs`**

```rust
//! Flow inside a tile and across tiles (Barnes 2017): where in-tile paths end, upstream sums
//! along a forest of next cells, and the exact solve over the tiles' ports (ring cells and
//! node outlets). Amounts are u64, so no result depends on the tiling.

use super::store::{Blob, Dec, Enc};
use crate::acquisition::{AcquisitionError, Result, ensure};
use std::collections::{BTreeMap, HashMap};

/// Where flow goes from a cell: a cell of this tile, the first cell outside the tile (world
/// index), a node (a lake, a pit or the world edge, by key), or nowhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Next {
    Cell(u32),
    Out(u64),
    Node(u64),
    End,
}

impl Blob for Next {
    fn put(&self, e: &mut Enc) {
        let (tag, v) = match *self {
            Next::Cell(c) => (0u8, u64::from(c)),
            Next::Out(c) => (1, c),
            Next::Node(k) => (2, k),
            Next::End => (3, 0),
        };
        tag.put(e);
        v.put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (tag, v) = (u8::get(d)?, u64::get(d)?);
        Ok(match tag {
            0 => Next::Cell(v as u32),
            1 => Next::Out(v),
            2 => Next::Node(v),
            3 => Next::End,
            _ => return Err(AcquisitionError::Invalid("work file: bad Next".into())),
        })
    }
}

/// For every cell, where its in-tile path ends (`Out`, `Node` or `End`, never `Cell`), and
/// how many cells lie on cycles (their end is `End`).
pub(crate) fn ends(n: usize, next: impl Fn(usize) -> Next) -> (Vec<Next>, u64) {
    const UNSEEN: u8 = 0;
    const OPEN: u8 = 1;
    const DONE: u8 = 2;
    let mut state = vec![UNSEEN; n];
    let mut end = vec![Next::End; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut cycles = 0u64;
    for start in 0..n {
        if state[start] != UNSEEN {
            continue;
        }
        let mut k = start;
        let found = loop {
            state[k] = OPEN;
            stack.push(k);
            match next(k) {
                Next::Cell(j) => {
                    let j = j as usize;
                    match state[j] {
                        UNSEEN => k = j,
                        DONE => break end[j],
                        _ => {
                            let from = stack.iter().rposition(|&c| c == j).unwrap_or(0);
                            cycles += (stack.len() - from) as u64;
                            break Next::End;
                        }
                    }
                }
                other => break other,
            }
        };
        for c in stack.drain(..) {
            end[c] = found;
            state[c] = DONE;
        }
    }
    (end, cycles)
}

/// Upstream sums of a tile.
pub(crate) struct Local {
    /// Each cell's rain plus everything upstream of it inside the tile.
    pub q: Vec<u64>,
    /// What leaves the tile, by the outside cell it enters.
    pub out: BTreeMap<u64, u64>,
    /// What enters nodes, by node key.
    pub node_in: BTreeMap<u64, u64>,
    /// Cells never reached in topological order (on or below a cycle).
    pub cycles: u64,
}

/// Sums `rain` along `next` in topological (Kahn) order, upstream first.
pub(crate) fn accumulate(n: usize, next: impl Fn(usize) -> Next, rain: impl Fn(usize) -> u64) -> Local {
    let mut indeg = vec![0u8; n];
    for k in 0..n {
        if let Next::Cell(j) = next(k) {
            indeg[j as usize] += 1;
        }
    }
    let mut q: Vec<u64> = (0..n).map(&rain).collect();
    let mut order: Vec<u32> = (0..n as u32).filter(|&k| indeg[k as usize] == 0).collect();
    let (mut out, mut node_in) = (BTreeMap::new(), BTreeMap::new());
    let mut head = 0;
    while head < order.len() {
        let k = order[head] as usize;
        head += 1;
        match next(k) {
            Next::Cell(j) => {
                let j = j as usize;
                q[j] += q[k];
                indeg[j] -= 1;
                if indeg[j] == 0 {
                    order.push(j as u32);
                }
            }
            Next::Out(c) => *out.entry(c).or_insert(0) += q[k],
            Next::Node(key) => *node_in.entry(key).or_insert(0) += q[k],
            Next::End => {}
        }
    }
    Local { q, out, node_in, cycles: (n - order.len()) as u64 }
}

/// A tile's flow report for the solve across tiles.
#[derive(Default, Debug, Clone, PartialEq)]
pub(crate) struct Report {
    pub out: BTreeMap<u64, u64>,
    pub node_in: BTreeMap<u64, u64>,
    /// Where the in-tile path from each port (ring cells, node outlets in the tile) ends.
    pub port_end: BTreeMap<u64, Next>,
}

impl Blob for Report {
    fn put(&self, e: &mut Enc) {
        (self.out.clone(), self.node_in.clone(), self.port_end.clone()).put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (out, node_in, port_end) = Blob::get(d)?;
        Ok(Self { out, node_in, port_end })
    }
}

#[derive(Default)]
pub(crate) struct Solved {
    /// What enters each port from beyond its own tile's cells.
    pub inflow: HashMap<u64, u64>,
    /// Each node's whole inflow.
    pub node_total: BTreeMap<u64, u64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum V {
    Port(u64),
    Node(u64),
}

/// The exact totals across tiles. `outlet` gives the cell a node's water leaves by (None: the
/// node keeps it). Errors on a cycle or on flow into a cell no tile reported.
pub(crate) fn solve(reports: &[Report], outlet: &BTreeMap<u64, Option<u64>>) -> Result<Solved> {
    let mut port_end: HashMap<u64, Next> = HashMap::new();
    let mut inflow: HashMap<u64, u64> = HashMap::new();
    let mut node_total: BTreeMap<u64, u64> = outlet.keys().map(|&k| (k, 0)).collect();
    for r in reports {
        port_end.extend(r.port_end.iter().map(|(&c, &e)| (c, e)));
        for (&c, &q) in &r.out {
            *inflow.entry(c).or_insert(0) += q;
        }
        for (&k, &q) in &r.node_in {
            *node_total.entry(k).or_insert(0) += q;
        }
    }
    for e in port_end.values() {
        if let Next::Node(k) = e {
            node_total.entry(*k).or_insert(0);
        }
    }
    let target = |v: V| -> Option<V> {
        match v {
            V::Port(c) => match port_end.get(&c) {
                Some(Next::Out(o)) => Some(V::Port(*o)),
                Some(Next::Node(k)) => Some(V::Node(*k)),
                _ => None,
            },
            V::Node(k) => outlet.get(&k).copied().flatten().map(V::Port),
        }
    };
    let verts: Vec<V> = port_end
        .keys()
        .map(|&c| V::Port(c))
        .chain(node_total.keys().map(|&k| V::Node(k)))
        .collect();
    let mut indeg: HashMap<V, u32> = verts.iter().map(|&v| (v, 0)).collect();
    for &v in &verts {
        if let Some(t) = target(v) {
            *indeg.get_mut(&t).ok_or_else(|| {
                AcquisitionError::Invalid("flow leaves into a cell no tile reported".into())
            })? += 1;
        }
    }
    let mut queue: Vec<V> = verts.iter().copied().filter(|v| indeg[v] == 0).collect();
    let mut done = 0usize;
    while let Some(v) = queue.pop() {
        done += 1;
        let amount = match v {
            V::Port(c) => inflow.get(&c).copied().unwrap_or(0),
            V::Node(k) => node_total[&k],
        };
        if let Some(t) = target(v) {
            match t {
                V::Port(c) => *inflow.entry(c).or_insert(0) += amount,
                V::Node(k) => *node_total.get_mut(&k).expect("node vertex") += amount,
            }
            let d = indeg.get_mut(&t).expect("vertex");
            *d -= 1;
            if *d == 0 {
                queue.push(t);
            }
        }
    }
    ensure(done == verts.len(), "flow between tiles has a cycle")?;
    Ok(Solved { inflow, node_total })
}

/// Adds what enters each port (local index, amount) to every cell on its in-tile path.
pub(crate) fn add_inflows(q: &mut [u64], ports: &[(usize, u64)], next: impl Fn(usize) -> Next) {
    for &(k, a) in ports {
        if a == 0 {
            continue;
        }
        let mut c = k;
        for _ in 0..q.len() {
            q[c] += a;
            match next(c) {
                Next::Cell(j) => c = j as usize,
                _ => break,
            }
        }
    }
}
```

Add `mod flow;` to `mod.rs`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::flow`
Expected: 3 pass. Clippy clean.

- [ ] **Step 5: Commit**

Run the task-finish block first.

```bash
git add crates/topography-core/src/world/water
git commit -m "feat(water): in-tile path ends, local sums and the exact port solve across tiles

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Expert review**

Expert review protocol. Real-terrain output: none yet (pure graph code); ask the reviewer to check the solve against Barnes 2017's perimeter graph (no double counting of in-tile flow, node totals passed to outlets exactly once).

---

## Task 5: Watersheds and spill edges per tile (pass A)

**Files:**
- Create: `crates/topography-core/src/world/water/watershed.rs`
- Modify: `crates/topography-core/src/world/water/mod.rs` (`mod watershed;`)

**Interfaces:**
- Consumes: `receivers::receivers`, `flow::{Next, ends}`, `geom::{World, Rect, UnionFind}`, `store::Blob`.
- Produces:
  - `watershed::EDGE_KEY: u64` (node key of the world edge), `watershed::ordered(f32) -> u32`.
  - `watershed::Drain { dir: Vec<u8>, pit: HashMap<u32, u64>, end: Vec<Next>, cycles: u64 }`, `drain(world, win, z: &[f32], core) -> Drain` (`pit` maps a pit cell's local index to its pit key, the smallest world index of its equal pit region in the tile).
  - `watershed::Edge { a: Next, b: Next, saddle: f32, ca: u64, cb: u64 }` with `rank() -> (u32, u64, u64)`.
  - `watershed::Ring { cell: u64, z: f32, end: Next, pit: bool }`, `TileGraph { ring: Vec<Ring>, pits: Vec<(u64, f32)>, supply: BTreeMap<Next, u64>, edges: Vec<Edge>, cycles: u64 }` (`Blob`), `tile_graph(world, core, zc: &[f32], d: &Drain, supply: impl Fn(usize) -> u64) -> TileGraph`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 7 x 5: an edge of 9 with an outlet of 4 at (6, 2); inside, pit A (1 at x 1) and pit B
    /// (2 at x 3) separated by a 5, B draining east over a 7 to the outlet.
    pub(crate) fn two_pits() -> (World, Vec<f32>) {
        let rows = [
            [9., 9., 9., 9., 9., 9., 9.],
            [9., 8., 8., 8., 8., 8., 9.],
            [9., 1., 5., 2., 6., 7., 4.],
            [9., 8., 8., 8., 8., 8., 9.],
            [9., 9., 9., 9., 9., 9., 9.],
        ];
        (World { w: 7, h: 5 }, rows.iter().flatten().copied().collect())
    }

    #[test]
    fn cells_end_at_pits_or_the_edge() {
        let (world, z) = two_pits();
        let all = Rect::whole(world);
        let d = drain(world, all, &z, all);
        assert_eq!(d.cycles, 0);
        assert_eq!(d.end[2 * 7 + 1], Next::Node(15), "pit A is its own key");
        assert_eq!(d.end[2 * 7 + 3], Next::Node(17));
        assert_eq!(d.end[2 * 7 + 4], Next::Node(17), "the 6 drains to B");
        assert_eq!(d.end[0], Next::Node(EDGE_KEY));
    }

    #[test]
    fn the_lowest_edge_per_pair_is_kept() {
        let (world, z) = two_pits();
        let all = Rect::whole(world);
        let d = drain(world, all, &z, all);
        let g = tile_graph(world, all, &z, &d, |_| 1000);
        let between = |a: u64, b: u64| {
            g.edges.iter().find(|e| {
                (e.a, e.b) == (Next::Node(a), Next::Node(b)) || (e.a, e.b) == (Next::Node(b), Next::Node(a))
            })
        };
        assert_eq!(between(15, 17).expect("A-B").saddle, 5.0);
        assert_eq!(between(17, EDGE_KEY).expect("B-edge").saddle, 7.0);
        assert_eq!(g.supply.values().sum::<u64>(), 35 * 1000);
        assert_eq!(g.pits.len(), 2);
    }

    #[test]
    fn equal_neighbouring_pits_are_one_pit_and_exits_name_the_outside_cell() {
        let world = World { w: 6, h: 4 };
        let mut z = vec![5.0f32; 24];
        (z[8], z[9]) = (1.0, 1.0); // (2, 1) and (3, 1): one flat pit
        let core = Rect { x0: 0, y0: 0, w: 3, h: 4 };
        let win = core.grow(1, world);
        let zw: Vec<f32> = (0..win.cells()).map(|k| { let (x, y) = win.world_xy(k); z[(y * 6 + x) as usize] }).collect();
        let d = drain(world, win, &zw, core);
        assert_eq!(d.pit.values().copied().collect::<Vec<_>>(), vec![8], "only (2, 1) is in this tile");
        let left = drain(world, Rect::whole(world), &z, Rect::whole(world));
        assert_eq!(left.end[9], Next::Node(8), "(3, 1) joins the pit named 8");
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::watershed`
Expected: compile errors.

- [ ] **Step 3: Implement `watershed.rs`**

```rust
//! Pass A: in each tile, the pits (equal neighbouring pit cells are one pit), where every
//! cell's flow ends inside the tile (the world edge, a pit, or the first cell outside), the r
//! supply by end, and the lowest spill edge between each pair of ends: Barnes 2016's label
//! graph, which the hierarchy (`hierarchy.rs`) joins across tiles.

use super::flow::{Next, ends};
use super::geom::{Rect, UnionFind, World};
use super::receivers::receivers;
use super::store::{Blob, Dec, Enc};
use crate::acquisition::Result;
use std::collections::{BTreeMap, HashMap};

/// Node key of the world edge: the ocean of the hierarchy.
pub(crate) const EDGE_KEY: u64 = u64::MAX;

/// An f32 as an unsigned key in total order.
pub(crate) fn ordered(z: f32) -> u32 {
    let b = z.to_bits();
    if b & 0x8000_0000 != 0 { !b } else { b | 0x8000_0000 }
}

/// A spill edge: cell `ca` drains to end `a`, its neighbour `cb` to `b`; the water crosses at
/// the higher of the two.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Edge {
    pub a: Next,
    pub b: Next,
    pub saddle: f32,
    pub ca: u64,
    pub cb: u64,
}

impl Edge {
    /// Orders edges alike in every tiling: lowest saddle, then the cell pair.
    pub fn rank(&self) -> (u32, u64, u64) {
        (ordered(self.saddle), self.ca.min(self.cb), self.ca.max(self.cb))
    }
}

/// A tile's drainage on the ground.
pub(crate) struct Drain {
    pub dir: Vec<u8>,
    /// Pit cells (local index) and their pit key.
    pub pit: HashMap<u32, u64>,
    pub end: Vec<Next>,
    pub cycles: u64,
}

pub(crate) fn drain(world: World, win: Rect, z: &[f32], core: Rect) -> Drain {
    let dir = receivers(world, win, z, core);
    let n = core.cells();
    let zc = |k: usize| {
        let (x, y) = core.world_xy(k);
        z[win.local(x, y).expect("core inside window")]
    };
    let is_pit = |k: usize| {
        let (x, y) = core.world_xy(k);
        dir[k] == 0 && !world.is_edge(x, y)
    };
    let pits: Vec<u32> = (0..n as u32).filter(|&k| is_pit(k as usize)).collect();
    let slot: HashMap<u32, u32> = pits.iter().enumerate().map(|(i, &k)| (k, i as u32)).collect();
    let mut uf = UnionFind::new(pits.len());
    for (i, &k) in pits.iter().enumerate() {
        let (x, y) = core.world_xy(k as usize);
        for j in 0..8 {
            let Some((nx, ny)) = world.neighbour(x, y, j) else { continue };
            let Some(m) = core.local(nx, ny) else { continue };
            if let Some(&s) = slot.get(&(m as u32)) {
                if zc(m) == zc(k as usize) {
                    uf.union(i as u32, s);
                }
            }
        }
    }
    let mut name: HashMap<u32, u64> = HashMap::new();
    for (i, &k) in pits.iter().enumerate() {
        let (x, y) = core.world_xy(k as usize);
        let e = name.entry(uf.find(i as u32)).or_insert(u64::MAX);
        *e = (*e).min(world.index(x, y));
    }
    let pit: HashMap<u32, u64> = pits
        .iter()
        .enumerate()
        .map(|(i, &k)| (k, name[&uf.find(i as u32)]))
        .collect();
    let next = |k: usize| {
        let (x, y) = core.world_xy(k);
        if world.is_edge(x, y) {
            return Next::Node(EDGE_KEY);
        }
        match dir[k] {
            0 => Next::Node(pit[&(k as u32)]),
            d => {
                let (nx, ny) = world.neighbour(x, y, d as usize - 1).expect("receiver inside");
                match core.local(nx, ny) {
                    Some(j) => Next::Cell(j as u32),
                    None => Next::Out(world.index(nx, ny)),
                }
            }
        }
    };
    let (end, cycles) = ends(n, next);
    Drain { dir, pit, end, cycles }
}

/// A cell of the tile's outer ring, for joins across seams.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Ring {
    pub cell: u64,
    pub z: f32,
    pub end: Next,
    pub pit: bool,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct TileGraph {
    pub ring: Vec<Ring>,
    /// Pit keys and their ground.
    pub pits: Vec<(u64, f32)>,
    /// r supply (thousandths of a cell) by end.
    pub supply: BTreeMap<Next, u64>,
    /// The lowest edge per pair of ends inside the tile.
    pub edges: Vec<Edge>,
    pub cycles: u64,
}

/// Pass A's result for one tile; `zc` holds the core's ground, `supply` each core cell's r.
pub(crate) fn tile_graph(world: World, core: Rect, zc: &[f32], d: &Drain, supply: impl Fn(usize) -> u64) -> TileGraph {
    let n = core.cells();
    let mut sup = BTreeMap::new();
    for k in 0..n {
        *sup.entry(d.end[k]).or_insert(0) += supply(k);
    }
    let mut best: HashMap<(Next, Next), Edge> = HashMap::new();
    for k in 0..n {
        let (x, y) = core.world_xy(k);
        // Forward neighbours (S, E, SW, SE) see each pair once.
        for j in [1usize, 3, 6, 7] {
            let Some((nx, ny)) = world.neighbour(x, y, j) else { continue };
            let Some(m) = core.local(nx, ny) else { continue };
            let (ea, eb) = (d.end[k], d.end[m]);
            if ea == eb {
                continue;
            }
            let (ck, cm, saddle) = (world.index(x, y), world.index(nx, ny), zc[k].max(zc[m]));
            let e = if ea < eb {
                Edge { a: ea, b: eb, saddle, ca: ck, cb: cm }
            } else {
                Edge { a: eb, b: ea, saddle, ca: cm, cb: ck }
            };
            best.entry((e.a, e.b))
                .and_modify(|o| if e.rank() < o.rank() { *o = e })
                .or_insert(e);
        }
    }
    let mut edges: Vec<Edge> = best.into_values().collect();
    edges.sort_by_key(|e| (e.a, e.b));
    let ring = (0..n)
        .filter_map(|k| {
            let (x, y) = core.world_xy(k);
            core.on_ring(x, y).then(|| Ring {
                cell: world.index(x, y),
                z: zc[k],
                end: d.end[k],
                pit: d.pit.contains_key(&(k as u32)),
            })
        })
        .collect();
    let mut pits: BTreeMap<u64, f32> = BTreeMap::new();
    for (&k, &key) in &d.pit {
        pits.insert(key, zc[k as usize]);
    }
    TileGraph { ring, pits: pits.into_iter().collect(), supply: sup, edges, cycles: d.cycles }
}

impl Blob for TileGraph {
    fn put(&self, e: &mut Enc) {
        let ring: Vec<(u64, f32, Next, bool)> = self.ring.iter().map(|r| (r.cell, r.z, r.end, r.pit)).collect();
        let edges: Vec<(Next, Next, f32, u64, u64)> =
            self.edges.iter().map(|x| (x.a, x.b, x.saddle, x.ca, x.cb)).collect();
        (ring, self.pits.clone(), self.supply.clone(), edges, self.cycles).put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (ring, pits, supply, edges, cycles): (
            Vec<(u64, f32, Next, bool)>,
            Vec<(u64, f32)>,
            BTreeMap<Next, u64>,
            Vec<(Next, Next, f32, u64, u64)>,
            u64,
        ) = Blob::get(d)?;
        Ok(Self {
            ring: ring.into_iter().map(|(cell, z, end, pit)| Ring { cell, z, end, pit }).collect(),
            pits,
            supply,
            edges: edges.into_iter().map(|(a, b, saddle, ca, cb)| Edge { a, b, saddle, ca, cb }).collect(),
            cycles,
        })
    }
}
```

Add `mod watershed;`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::watershed`
Expected: 3 pass. Clippy clean.

- [ ] **Step 5: Commit**

Run the task-finish block first.

```bash
git add crates/topography-core/src/world/water
git commit -m "feat(water): per-tile pits, drainage ends, r supply and spill edges

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Expert review**

Expert review protocol. Real-terrain output: add and run this probe in `watershed.rs` tests, then give its output to the reviewer:

```rust
    #[test]
    #[ignore]
    fn probe_pits_in_the_canyon() {
        use crate::world::water::probe;
        let Some((win, z)) = probe::window(27169, 44546, 2048) else { return };
        let core = Rect { x0: win.x0 + 1, y0: win.y0 + 1, w: win.w - 2, h: win.h - 2 };
        let d = drain(probe::WORLD, win, &z, core);
        let zc: Vec<f32> = (0..core.cells()).map(|k| { let (x, y) = core.world_xy(k); z[win.local(x, y).expect("in")] }).collect();
        let g = tile_graph(probe::WORLD, core, &zc, &d, |_| 1000);
        eprintln!("pits {} (one per {} cells), edges {}, cycles {}", g.pits.len(), core.cells() / g.pits.len().max(1), g.edges.len(), g.cycles);
    }
```

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core world::water::watershed::tests::probe_pits_in_the_canyon -- --ignored --nocapture`. The pit density feeds the memory estimate for the world's global graph (Task 18).

---

## Task 6: The depression hierarchy (global step 1)

**Files:**
- Create: `crates/topography-core/src/world/water/hierarchy.rs`
- Modify: `crates/topography-core/src/world/water/mod.rs` (`mod hierarchy;`)

**Interfaces:**
- Consumes: `watershed::{TileGraph, Edge, EDGE_KEY}`, `flow::Next`, `geom::{World, UnionFind, tile_of}`, `store::Blob`.
- Produces:
  - `hierarchy::NONE: u32`.
  - `hierarchy::Hierarchy { leaves: usize, leaf_pit: Vec<u64>, low: Vec<f32>, spill: Vec<f32>, parent: Vec<u32>, children: Vec<[u32; 2]>, outlet: Vec<[u64; 2]>, geolink: Vec<u32>, ocean_order: Vec<u32>, supply: Vec<u64> }` (`Blob`, `PartialEq`). Depressions `0..leaves` are leaves; metas follow in creation order, so children always have lower indices than their parent. `outlet[d] = [cell inside d, cell outside]`; `geolink[d]` is the leaf beyond the outlet (`NONE`: the world edge). `ocean_order` lists top-level depressions in the order they joined the ocean.
  - `hierarchy::Resolver { ring: Vec<(u64, Next)>, root: Vec<(u64, u64)>, leaf_pit: Vec<u64> }` (all sorted; `Blob`, `PartialEq`) with `ring_end(u64) -> Option<Next>`, `root_of(u64) -> u64`, `leaf_index(u64) -> u32` and `leaf_of(Next) -> u32` (`NONE` for the world edge). Sorted `Vec`s instead of HashMaps (review C3). Leaves are pit regions or the super-leaves of Task 8b's contraction; `build` does not need to know which.
  - `hierarchy::build(world, tile: u64, graphs: &[TileGraph]) -> Result<(Hierarchy, Resolver)>`.
  - Test helper `hierarchy::tests::graph_for(world, z: &[f32], tile: u64) -> (Hierarchy, Resolver)` (`pub(crate)` inside `#[cfg(test)]`, reused by Tasks 7 and 9).

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::world::water::geom::{Rect, tiles};
    use crate::world::water::watershed::{drain, tile_graph};

    /// Pass A on tiles of `tile` cells, then the hierarchy (r = 1 everywhere).
    pub(crate) fn graph_for(world: World, z: &[f32], tile: u64) -> (Hierarchy, Resolver) {
        let all = Rect::whole(world);
        let graphs: Vec<TileGraph> = tiles(world, tile)
            .into_iter()
            .map(|core| {
                let win = core.grow(1, world);
                let zw: Vec<f32> = (0..win.cells()).map(|k| { let (x, y) = win.world_xy(k); z[all.local(x, y).expect("in")] }).collect();
                let zc: Vec<f32> = (0..core.cells()).map(|k| { let (x, y) = core.world_xy(k); z[all.local(x, y).expect("in")] }).collect();
                let d = drain(world, win, &zw, core);
                tile_graph(world, core, &zc, &d, |_| 1000)
            })
            .collect();
        build(world, tile, &graphs).expect("hierarchy")
    }

    /// Rough terrain with many pits (deterministic).
    pub(crate) fn rough(world: World) -> Vec<f32> {
        (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                20.0 + (x * 0.7).sin() * 3.0 + (y * 0.5).cos() * 3.0 + ((i * 7919) % 17) as f32 * 0.2 - x * 0.1
            })
            .collect()
    }

    #[test]
    fn two_pits_merge_below_their_way_out() {
        let (world, z) = crate::world::water::watershed::tests::two_pits();
        let (h, _) = graph_for(world, &z, 64);
        assert_eq!(h.leaves, 2);
        assert_eq!(h.leaf_pit, vec![15, 17]);
        assert_eq!(h.spill, vec![5.0, 5.0, 7.0]);
        assert_eq!(h.parent, vec![2, 2, NONE]);
        assert_eq!(h.children[2], [0, 1]);
        assert_eq!(h.low[2], 1.0);
        assert_eq!(h.geolink[0], 1);
        assert_eq!(h.geolink[2], NONE);
        assert_eq!(h.ocean_order, vec![2]);
        let [inside, outside] = h.outlet[2];
        assert_eq!(z[inside as usize].max(z[outside as usize]), 7.0);
        assert_eq!(h.supply.len(), 2);
        assert!(h.supply.iter().all(|&s| s >= 1000), "each pit drains at least itself");
    }

    #[test]
    fn tiles_build_the_same_hierarchy() {
        let world = World { w: 48, h: 40 };
        let z = rough(world);
        let (whole, _) = graph_for(world, &z, 64);
        assert!(whole.leaves > 5, "the case has pits");
        for size in [16, 7] {
            let (tiled, _) = graph_for(world, &z, size);
            assert_eq!(tiled, whole, "tile size {size}");
        }
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::hierarchy`
Expected: compile errors.

- [ ] **Step 3: Implement `hierarchy.rs`**

```rust
//! Global step 1: join the tiles' label graphs (pits that meet across a seam, edges across
//! seams) and build the depression hierarchy (Barnes, Callaghan and Wickert 2020) by merging
//! along spill edges from the lowest (Kruskal; equal saddles ordered by cell pair, so every
//! tiling builds the same tree). Leaves are pits; a meta-depression forms where two spill into
//! each other; a depression whose lowest edge leads to the ocean (the world edge, or anything
//! already joined to it) is top-level.

use super::flow::Next;
use super::geom::{UnionFind, World, tile_of};
use super::store::{Blob, Dec, Enc};
use super::watershed::{EDGE_KEY, Edge, TileGraph};
use crate::acquisition::{Result, ensure};

pub(crate) const NONE: u32 = u32::MAX;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Hierarchy {
    pub leaves: usize,
    pub leaf_pit: Vec<u64>,
    pub low: Vec<f32>,
    pub spill: Vec<f32>,
    pub parent: Vec<u32>,
    pub children: Vec<[u32; 2]>,
    pub outlet: Vec<[u64; 2]>,
    pub geolink: Vec<u32>,
    pub ocean_order: Vec<u32>,
    pub supply: Vec<u64>,
}

impl Hierarchy {
    pub fn len(&self) -> usize {
        self.spill.len()
    }
    pub fn is_leaf(&self, d: u32) -> bool {
        (d as usize) < self.leaves
    }
}

impl Blob for Hierarchy {
    fn put(&self, e: &mut Enc) {
        (self.leaves as u64, self.leaf_pit.clone(), self.low.clone(), self.spill.clone(), self.parent.clone()).put(e);
        let children: Vec<(u32, u32)> = self.children.iter().map(|c| (c[0], c[1])).collect();
        let outlet: Vec<(u64, u64)> = self.outlet.iter().map(|o| (o[0], o[1])).collect();
        (children, outlet, self.geolink.clone(), self.ocean_order.clone(), self.supply.clone()).put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (leaves, leaf_pit, low, spill, parent): (u64, _, _, _, _) = Blob::get(d)?;
        let (children, outlet, geolink, ocean_order, supply): (Vec<(u32, u32)>, Vec<(u64, u64)>, _, _, _) =
            Blob::get(d)?;
        Ok(Self {
            leaves: leaves as usize,
            leaf_pit,
            low,
            spill,
            parent,
            children: children.into_iter().map(|(a, b)| [a, b]).collect(),
            outlet: outlet.into_iter().map(|(a, b)| [a, b]).collect(),
            geolink,
            ocean_order,
            supply,
        })
    }
}

/// Maps a tile's drainage ends to leaves. Sorted `Vec`s with binary search (review C3): about
/// a third of the memory of HashMaps at 10⁷ leaves.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Resolver {
    /// Ring cells' in-tile ends, sorted by world index.
    pub ring: Vec<(u64, Next)>,
    /// Pit key → the key of the pit region it belongs to across seams, sorted by pit key.
    pub root: Vec<(u64, u64)>,
    /// Leaf keys (pit regions and contracted super-leaves), sorted: a leaf is its position.
    pub leaf_pit: Vec<u64>,
}

impl Resolver {
    pub fn ring_end(&self, c: u64) -> Option<Next> {
        self.ring.binary_search_by_key(&c, |e| e.0).ok().map(|i| self.ring[i].1)
    }
    pub fn root_of(&self, p: u64) -> u64 {
        self.root.binary_search_by_key(&p, |e| e.0).map_or(p, |i| self.root[i].1)
    }
    pub fn leaf_index(&self, key: u64) -> u32 {
        self.leaf_pit.binary_search(&key).map_or(NONE, |i| i as u32)
    }
    /// The leaf an in-tile end drains to; NONE for the world edge.
    pub fn leaf_of(&self, mut t: Next) -> u32 {
        loop {
            match t {
                Next::Out(c) => t = self.ring_end(c).expect("an exit enters a ring cell"),
                Next::Node(EDGE_KEY) | Next::End | Next::Cell(_) => return NONE,
                Next::Node(p) => return self.leaf_index(self.root_of(p)),
            }
        }
    }
}

impl Blob for Resolver {
    fn put(&self, e: &mut Enc) {
        (self.ring.clone(), self.root.clone(), self.leaf_pit.clone()).put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (ring, root, leaf_pit) = Blob::get(d)?;
        Ok(Self { ring, root, leaf_pit })
    }
}

pub(crate) fn build(world: World, tile: u64, graphs: &[TileGraph]) -> Result<(Hierarchy, Resolver)> {
    // Ring cells: (cell, end, ground, pit), sorted by cell.
    let mut ring: Vec<(u64, Next, f32, bool)> =
        graphs.iter().flat_map(|g| g.ring.iter().map(|r| (r.cell, r.end, r.z, r.pit))).collect();
    ring.sort_unstable_by_key(|r| r.0);
    let ring_at = |c: u64| ring.binary_search_by_key(&c, |r| r.0).ok().map(|i| ring[i]);
    // Pit regions that meet across a seam are one pit, named by the smallest key.
    let mut keys: Vec<u64> = graphs.iter().flat_map(|g| g.pits.iter().map(|p| p.0)).collect();
    keys.sort_unstable();
    keys.dedup();
    let slot = |k: u64| keys.binary_search(&k).expect("a pit key") as u32;
    let mut uf = UnionFind::new(keys.len());
    for r in ring.iter().filter(|r| r.3) {
        let (x, y) = world.xy(r.0);
        for j in 0..8 {
            let Some((nx, ny)) = world.neighbour(x, y, j) else { continue };
            if let (Some((_, Next::Node(q), z, true)), Next::Node(p)) = (ring_at(world.index(nx, ny)), r.1) {
                if z == r.2 {
                    uf.union(slot(p), slot(q));
                }
            }
        }
    }
    let root: Vec<(u64, u64)> =
        keys.iter().enumerate().map(|(i, &k)| (k, keys[uf.find(i as u32) as usize])).collect();
    let mut leaf_pit: Vec<u64> = root.iter().map(|r| r.1).collect();
    leaf_pit.sort_unstable();
    leaf_pit.dedup();
    let resolver = Resolver { ring: ring.iter().map(|r| (r.0, r.1)).collect(), root, leaf_pit: leaf_pit.clone() };
    let nl = leaf_pit.len();

    let mut low = vec![f32::INFINITY; nl];
    let mut supply = vec![0u64; nl];
    for g in graphs {
        for &(p, z) in &g.pits {
            let l = resolver.leaf_index(resolver.root_of(p)) as usize;
            low[l] = low[l].min(z);
        }
        for (&t, &s) in &g.supply {
            let l = resolver.leaf_of(t);
            if l != NONE {
                supply[l as usize] += s;
            }
        }
    }

    // Candidate edges between leaves (NONE: the world edge), `ca` in the first leaf; then the
    // lowest per pair by sorting (no HashMap).
    let mut cand: Vec<(u32, u32, Edge)> = Vec::new();
    let mut add = |la: u32, lb: u32, saddle: f32, ca: u64, cb: u64| {
        if la == lb {
            return;
        }
        let e = |ca, cb| Edge { a: Next::End, b: Next::End, saddle, ca, cb };
        cand.push(if la < lb { (la, lb, e(ca, cb)) } else { (lb, la, e(cb, ca)) });
    };
    for g in graphs {
        for e in &g.edges {
            add(resolver.leaf_of(e.a), resolver.leaf_of(e.b), e.saddle, e.ca, e.cb);
        }
    }
    for &(c, end_c, zc, _) in &ring {
        let (x, y) = world.xy(c);
        for j in 0..8 {
            let Some((nx, ny)) = world.neighbour(x, y, j) else { continue };
            let o = world.index(nx, ny);
            if o < c || tile_of(world, tile, x, y) == tile_of(world, tile, nx, ny) {
                continue;
            }
            if let Some((_, end_o, zo, _)) = ring_at(o) {
                add(resolver.leaf_of(end_c), resolver.leaf_of(end_o), zc.max(zo), c, o);
            }
        }
    }
    cand.sort_unstable_by(|x, y| (x.0, x.1, x.2.rank()).cmp(&(y.0, y.1, y.2.rank())));
    cand.dedup_by_key(|c| (c.0, c.1));
    let mut edges: Vec<((u32, u32), Edge)> = cand.into_iter().map(|(a, b, e)| ((a, b), e)).collect();
    edges.sort_by_key(|(_, e)| e.rank());

    // Kruskal from the lowest saddle; union-find slot `nl` is the ocean.
    let ocean = nl as u32;
    let mut uf = UnionFind::new(nl + 1);
    let mut top: Vec<u32> = (0..nl as u32).chain([NONE]).collect();
    let mut h = Hierarchy {
        leaves: nl,
        leaf_pit,
        low,
        spill: vec![f32::INFINITY; nl],
        parent: vec![NONE; nl],
        children: vec![[NONE, NONE]; nl],
        outlet: vec![[0, 0]; nl],
        geolink: vec![NONE; nl],
        ocean_order: Vec::new(),
        supply,
    };
    for ((la, lb), e) in edges {
        let ia = if la == NONE { ocean } else { la };
        let ib = if lb == NONE { ocean } else { lb };
        let (ra, rb) = (uf.find(ia), uf.find(ib));
        if ra == rb {
            continue;
        }
        let ro = uf.find(ocean);
        if ra == ro || rb == ro {
            let (r, inside, outside, beyond) = if rb == ro { (ra, e.ca, e.cb, lb) } else { (rb, e.cb, e.ca, la) };
            let d = top[r as usize] as usize;
            h.spill[d] = e.saddle;
            h.outlet[d] = [inside, outside];
            h.geolink[d] = beyond;
            h.ocean_order.push(d as u32);
            let joined = uf.union(r, ro);
            top[joined as usize] = NONE;
        } else {
            let (da, db) = (top[ra as usize], top[rb as usize]);
            let m = h.len() as u32;
            let low = h.low[da as usize].min(h.low[db as usize]);
            h.low.push(low);
            h.spill.push(f32::INFINITY);
            h.parent.push(NONE);
            h.children.push([da, db]);
            h.outlet.push([0, 0]);
            h.geolink.push(NONE);
            for (d, inside, outside, beyond) in [(da, e.ca, e.cb, lb), (db, e.cb, e.ca, la)] {
                h.spill[d as usize] = e.saddle;
                h.outlet[d as usize] = [inside, outside];
                h.geolink[d as usize] = beyond;
                h.parent[d as usize] = m;
            }
            let joined = uf.union(ra, rb);
            top[joined as usize] = m;
        }
    }
    let ro = uf.find(ocean);
    ensure((0..nl as u32).all(|l| uf.find(l) == ro), "a basin has no way to the world edge")?;
    Ok((h, resolver))
}
```

Add `mod hierarchy;`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::hierarchy`
Expected: 2 pass. Clippy clean.

- [ ] **Step 5: Commit**

Run the task-finish block first.

```bash
git add crates/topography-core/src/world/water
git commit -m "feat(water): depression hierarchy from the joined spill graph, alike at every tiling

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Expert review**

Expert review protocol. Real-terrain output: add this probe (in `hierarchy.rs` tests), run it, and pass the printout:

```rust
    #[test]
    #[ignore]
    fn probe_hierarchy_at_the_dam_spot() {
        use crate::world::water::probe;
        let Some((win, z)) = probe::window(25645, 24400, 1024) else { return };
        let world = World { w: win.w as u64, h: win.h as u64 };
        let (h, _) = graph_for(world, &z, 256);
        let deepest = (0..h.len()).filter(|&d| h.spill[d].is_finite()).max_by(|&a, &b| (h.spill[a] - h.low[a]).total_cmp(&(h.spill[b] - h.low[b]))).expect("depressions");
        eprintln!("leaves {}, metas {}, top-level {}; deepest depression {} m (low {} spill {})",
            h.leaves, h.len() - h.leaves, h.ocean_order.len(), h.spill[deepest] - h.low[deepest], h.low[deepest], h.spill[deepest]);
    }
```

---
## Task 7: Water balance by Fill-Spill-Merge with r

**Files:**
- Create: `crates/topography-core/src/world/water/balance.rs`
- Modify: `crates/topography-core/src/world/water/mod.rs` (`mod balance;`)

**Interfaces:**
- Consumes: `hierarchy::{Hierarchy, NONE}`, `table::MILLI`, `store::Blob`, `LAKE_DEPTH_M`.
- Produces:
  - `balance::mm(f32) -> i64`, `balance::top_spill(&Hierarchy) -> Vec<f32>` (per leaf), `balance::holding(&Hierarchy, top: &[f32], leaf: u32, z: f32) -> u32` (lowest depression on the leaf's chain whose spill is ≥ z; `NONE` above all).
  - `balance::TileCounts = BTreeMap<u32, (u64, i64, [u64; 4])>` (cells, Σ ground in mm, bbox `[x0, y0, x1, y1]` by holding depression) and `balance::count(&mut TileCounts, d: u32, z: f32, x: u64, y: u64)`.
  - `balance::Counts { cells, sum_mm, bbox }`, `Counts::dense(nd: usize, tiles: &[TileCounts]) -> Counts`.
  - `balance::Balance { sea, n, vfill_mm, bbox, full, own, node, level, need, to_sea, pours: Vec<(u32, u64, u64)> }` (`Blob`, `PartialEq`; `pours` = (leaf, entry cell, amount) of every overflow poured into a leaf from outside it, sorted), `balance::solve(&Hierarchy, &Counts, sea_at_zero: bool) -> Balance`, `Balance::set_closed_levels(&mut self, gathered: &[(u32, f32)]) -> Result<()>`, `Balance::area_cells(&self, h, node: u32) -> u64`, `Balance::is_lake(&self, h: &Hierarchy, node: u32, min_cells: u64) -> bool` (≥ 0.3 m deep and ≥ `min_cells`, review C2).

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Leaves 0 and 1 merge at 5 m into meta 2, which spills to the ocean at 7 m. Cells at or
    /// below each spill (by first holding depression): leaf 0: 3, leaf 1: 2, meta 2: 4 more.
    fn handmade(supply: [u64; 2], ocean_spill: f32) -> (Hierarchy, Counts) {
        let h = Hierarchy {
            leaves: 2,
            leaf_pit: vec![10, 20],
            low: vec![1.0, 2.0, 1.0],
            spill: vec![5.0, 5.0, ocean_spill],
            parent: vec![2, 2, NONE],
            children: vec![[NONE, NONE], [NONE, NONE], [0, 1]],
            outlet: vec![[11, 12], [12, 11], [13, 14]],
            geolink: vec![1, 0, NONE],
            ocean_order: vec![2],
            supply: supply.to_vec(),
        };
        let c = Counts { cells: vec![3, 2, 4], sum_mm: vec![6000, 5000, 24000], bbox: vec![[0, 0, 1, 1]; 3] };
        (h, c)
    }

    #[test]
    fn a_humid_pair_fills_and_spills_to_the_sea() {
        let (h, c) = handmade([10_000, 10_000], 7.0);
        let b = solve(&h, &c, true);
        assert_eq!(b.n, vec![3, 2, 9]);
        assert_eq!(b.full, vec![true, true, true]);
        assert_eq!(b.node, vec![2, 2, 2]);
        assert_eq!(b.level[2], 7.0);
        assert_eq!(b.to_sea, 20_000 - 9_000);
        assert_eq!(b.vfill_mm[0], 5000 * 3 - 6000);
    }

    #[test]
    fn overflow_runs_into_the_sibling_first() {
        // Leaf 0 holds 3 cells; its extra 2 cells' worth fills leaf 1 exactly: one lake at 5 m.
        let (h, c) = handmade([5_000, 0], 7.0);
        let b = solve(&h, &c, true);
        assert_eq!(b.full, vec![true, true, false]);
        assert_eq!(b.node, vec![2, 2, 2], "both full: one lake at their saddle");
        assert_eq!(b.level[2], 5.0);
        assert_eq!(b.to_sea, 0);
    }

    #[test]
    fn an_arid_basin_stands_where_its_area_meets_its_supply() {
        let (h, c) = handmade([2_500, 0], 7.0);
        let mut b = solve(&h, &c, true);
        assert_eq!(b.full, vec![false, false, false]);
        assert_eq!(b.node, vec![0, NONE, NONE]);
        assert_eq!(b.need[0], 3, "2.5 cells of supply cover 3 cells");
        assert!(b.level[0].is_nan());
        b.set_closed_levels(&[(0, 4.0), (0, 1.0), (0, 2.5)]).expect("levels");
        assert_eq!(b.level[0], 4.0);
        assert!(!b.is_lake(&h, 1, 1));
        assert_eq!(b.area_cells(&h, 0), 3);
        assert!(b.is_lake(&h, 0, 3), "3 m deep over 3 cells");
        assert!(!b.is_lake(&h, 0, 4), "smaller than the lake minimum: a pond");
    }

    #[test]
    fn a_flat_with_an_outlet_is_never_a_closed_basin() {
        // Leaf 1 is a flat at 5 m whose spill is its own level: zero depth, no supply (r = 0).
        let (mut h, c) = handmade([0, 0], 7.0);
        h.low[1] = 5.0;
        let b = solve(&h, &c, true);
        assert!(b.full[1], "zero depth: full by definition (R32)");
        assert_eq!(b.node[1], 1, "a node with an outlet, so flow crosses it");
        assert_eq!(b.level[1], 5.0);
        assert!(!b.full[0], "leaf 0 has depth and no water: dry");
    }

    #[test]
    fn overflow_into_a_leaf_is_recorded_with_its_entry_cell() {
        let (h, c) = handmade([5_000, 0], 7.0);
        let b = solve(&h, &c, true);
        assert_eq!(b.pours, vec![(1, 12, 2_000)], "leaf 0's extra entered leaf 1 by cell 12");
    }

    #[test]
    fn no_supply_leaves_a_dry_basin_and_sea_trees_take_everything() {
        let (h, c) = handmade([0, 0], 7.0);
        assert_eq!(solve(&h, &c, true).node, vec![NONE; 3]);
        let (h, c) = handmade([10_000, 1], -1.0);
        let b = solve(&h, &c, true);
        assert_eq!(b.sea, vec![true; 3]);
        assert_eq!(b.node, vec![NONE; 3]);
        assert_eq!(b.to_sea, 10_001);
    }

    #[test]
    fn overflow_crosses_into_a_tree_that_joined_the_ocean_earlier() {
        // Leaf 1 joined the ocean first (spill 2 m); leaf 0 spills at 3 m into leaf 1.
        let h = Hierarchy {
            leaves: 2,
            leaf_pit: vec![10, 20],
            low: vec![1.0, 1.0],
            spill: vec![3.0, 2.0],
            parent: vec![NONE, NONE],
            children: vec![[NONE, NONE]; 2],
            outlet: vec![[11, 21], [22, 23]],
            geolink: vec![1, NONE],
            ocean_order: vec![1, 0],
            supply: vec![5_000, 0],
        };
        let c = Counts { cells: vec![2, 4], sum_mm: vec![0, 0], bbox: vec![[0, 0, 0, 0]; 2] };
        let b = solve(&h, &c, true);
        assert!(b.full[0]);
        assert!(!b.full[1], "3 cells of overflow do not fill 4");
        assert_eq!(b.need[1], 3);
        assert_eq!(b.to_sea, 0);
    }

    #[test]
    fn holding_finds_the_first_spill_above() {
        let (h, _) = handmade([0, 0], 7.0);
        let top = top_spill(&h);
        assert_eq!(top, vec![7.0, 7.0]);
        assert_eq!(holding(&h, &top, 0, 4.0), 0);
        assert_eq!(holding(&h, &top, 0, 6.0), 2);
        assert_eq!(holding(&h, &top, 0, 7.5), NONE);
        assert_eq!(holding(&h, &top, NONE, 0.0), NONE);
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::balance`
Expected: compile errors.

- [ ] **Step 3: Implement `balance.rs`**

```rust
//! Fill-Spill-Merge (Barnes, Callaghan and Wickert 2020) with a water balance by area. A
//! depression's catchment supplies Σ r over its cells (thousandths of a cell); a lake of that
//! many cells evaporates it. A depression that cannot hold its supply below its spill fills,
//! and the rest overflows: into its sibling, into its parent when both are full, or (top-level)
//! across its outlet into the tree beyond. Trees are taken in reverse of the order they joined
//! the ocean, so overflow always runs into a tree not yet taken. All amounts are integers.

use super::LAKE_DEPTH_M;
use super::hierarchy::{Hierarchy, NONE};
use super::store::{Blob, Dec, Enc};
use super::table::MILLI;
use crate::acquisition::{Result, ensure};
use std::collections::BTreeMap;

/// Metres as integer millimetres.
pub(crate) fn mm(z: f32) -> i64 {
    (f64::from(z) * 1000.0).round() as i64
}

/// The spill of each leaf's top-level ancestor (cells above it are in no depression).
pub(crate) fn top_spill(h: &Hierarchy) -> Vec<f32> {
    let mut top = vec![0.0f32; h.len()];
    for d in (0..h.len()).rev() {
        let p = h.parent[d];
        top[d] = if p == NONE { h.spill[d] } else { top[p as usize] };
    }
    top.truncate(h.leaves);
    top
}

/// The lowest depression on `leaf`'s chain whose spill is at or above `z`; NONE above all.
pub(crate) fn holding(h: &Hierarchy, top: &[f32], leaf: u32, z: f32) -> u32 {
    if leaf == NONE || z > top[leaf as usize] {
        return NONE;
    }
    let mut d = leaf;
    while h.spill[d as usize] < z {
        d = h.parent[d as usize];
    }
    d
}

/// A tile's cells by holding depression: count, Σ ground (mm), bbox [x0, y0, x1, y1].
pub(crate) type TileCounts = BTreeMap<u32, (u64, i64, [u64; 4])>;

pub(crate) fn count(t: &mut TileCounts, d: u32, z: f32, x: u64, y: u64) {
    let e = t.entry(d).or_insert((0, 0, [u64::MAX, u64::MAX, 0, 0]));
    e.0 += 1;
    e.1 += mm(z);
    e.2 = [e.2[0].min(x), e.2[1].min(y), e.2[2].max(x), e.2[3].max(y)];
}

fn union(a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Counts {
    pub cells: Vec<u64>,
    pub sum_mm: Vec<i64>,
    pub bbox: Vec<[u64; 4]>,
}

impl Counts {
    pub fn dense(nd: usize, tiles: &[TileCounts]) -> Self {
        let mut c = Counts { cells: vec![0; nd], sum_mm: vec![0; nd], bbox: vec![[u64::MAX, u64::MAX, 0, 0]; nd] };
        for t in tiles {
            for (&d, &(n, s, b)) in t {
                let d = d as usize;
                c.cells[d] += n;
                c.sum_mm[d] += s;
                c.bbox[d] = union(c.bbox[d], b);
            }
        }
        c
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Balance {
    /// In a tree that joins the ocean at or below 0 m.
    pub sea: Vec<bool>,
    /// Cells at or below the spill, over the subtree.
    pub n: Vec<u64>,
    /// Volume to the spill, mm × cells.
    pub vfill_mm: Vec<i64>,
    pub bbox: Vec<[u64; 4]>,
    pub full: Vec<bool>,
    /// Water held at this depression's own level (beyond its children's capacities).
    pub own: Vec<u64>,
    /// The depression whose water covers this one's cells (itself or its top wet ancestor).
    pub node: Vec<u32>,
    /// Water level of node depressions (NaN elsewhere, and for closed nodes until set).
    pub level: Vec<f32>,
    /// Closed wet nodes: how many cells their level covers.
    pub need: Vec<u64>,
    /// Supply that reached the sea or left the map.
    pub to_sea: u64,
    /// Overflow poured into a leaf from outside it: (leaf, the outlet cell it entered by,
    /// amount), sorted. A contracted super-leaf (Task 8b) re-solves its inside from these.
    pub pours: Vec<(u32, u64, u64)>,
}

struct Fsm<'a> {
    h: &'a Hierarchy,
    cap: Vec<u64>,
    sea: Vec<bool>,
    top_of: Vec<u32>,
    taken: Vec<bool>,
    full: Vec<bool>,
    own: Vec<u64>,
    pending: Vec<u64>,
    to_sea: u64,
    pours: Vec<(u32, u64, u64)>,
}

impl Fsm<'_> {
    fn pour(&mut self, mut d: u32, mut w: u64) {
        let h = self.h;
        while w > 0 {
            let du = d as usize;
            if !self.full[du] {
                let kids: u64 = if h.is_leaf(d) { 0 } else { h.children[du].iter().map(|&c| self.cap[c as usize]).sum() };
                let room = self.cap[du] - kids - self.own[du];
                if w < room {
                    self.own[du] += w;
                    return;
                }
                self.own[du] += room;
                w -= room;
                self.full[du] = true;
            }
            let p = h.parent[du];
            if p == NONE {
                let g = h.geolink[du];
                if g == NONE || self.sea[self.top_of[g as usize] as usize] {
                    self.to_sea += w;
                } else {
                    debug_assert!(!self.taken[self.top_of[g as usize] as usize], "overflow into a taken tree");
                    self.pending[g as usize] += w;
                    self.pours.push((g, h.outlet[du][1], w));
                }
                return;
            }
            let [a, b] = h.children[p as usize];
            let sibling = if a == d { b } else { a };
            d = if self.full[sibling as usize] {
                p
            } else {
                self.pours.push((h.geolink[du], h.outlet[du][1], w));
                h.geolink[du]
            };
        }
    }
}

/// `sea_at_zero`: top-level trees spilling at or below 0 m are sea (the world's hierarchy);
/// false for a tile's settled fragment, whose tops are not top-level in the world (Task 8b).
pub(crate) fn solve(h: &Hierarchy, c: &Counts, sea_at_zero: bool) -> Balance {
    let nd = h.len();
    let (mut n, mut sum, mut bbox) = (c.cells.clone(), c.sum_mm.clone(), c.bbox.clone());
    // Children come before their parents.
    for d in 0..nd {
        let p = h.parent[d];
        if p != NONE {
            let p = p as usize;
            n[p] += n[d];
            sum[p] += sum[d];
            bbox[p] = union(bbox[p], bbox[d]);
        }
    }
    let vfill_mm = (0..nd).map(|d| mm(h.spill[d]) * n[d] as i64 - sum[d]).collect();
    let mut sea = vec![false; nd];
    let mut top_of = vec![0u32; nd];
    for d in (0..nd).rev() {
        let p = h.parent[d];
        (sea[d], top_of[d]) =
            if p == NONE { (sea_at_zero && h.spill[d] <= 0.0, d as u32) } else { (sea[p as usize], top_of[p as usize]) };
    }
    let mut by_top: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for l in 0..h.leaves as u32 {
        by_top.entry(top_of[l as usize]).or_default().push(l);
    }
    let mut f = Fsm {
        h,
        cap: n.iter().map(|&k| MILLI * k).collect(),
        sea: sea.clone(),
        top_of,
        taken: vec![false; nd],
        // A depression of zero depth (a flat with an outlet) holds no water: it is full from
        // the start and drains on, whatever its r (R32, review N15).
        full: (0..nd).map(|d| !sea[d] && h.spill[d] <= h.low[d]).collect(),
        own: vec![0; nd],
        pending: h.supply.clone(),
        to_sea: 0,
        pours: Vec::new(),
    };
    for &t in h.ocean_order.iter().rev() {
        f.taken[t as usize] = true;
        for l in by_top.remove(&t).unwrap_or_default() {
            let w = std::mem::take(&mut f.pending[l as usize]);
            if sea[t as usize] {
                f.to_sea += w;
            } else {
                f.pour(l, w);
            }
        }
    }
    let both_full = |d: usize| !h.is_leaf(d as u32) && h.children[d].iter().all(|&k| f.full[k as usize]);
    let wet = |d: usize| !sea[d] && (f.full[d] || f.own[d] > 0 || both_full(d));
    let mut node = vec![NONE; nd];
    for d in (0..nd).rev() {
        let p = h.parent[d];
        let up = if p == NONE { NONE } else { node[p as usize] };
        node[d] = if up != NONE { up } else if wet(d) { d as u32 } else { NONE };
    }
    let mut level = vec![f32::NAN; nd];
    let mut need = vec![0u64; nd];
    for d in 0..nd {
        if node[d] != d as u32 {
            continue;
        }
        if f.full[d] {
            level[d] = h.spill[d];
            continue;
        }
        let kids = if h.is_leaf(d as u32) { 0 } else { h.children[d].iter().map(|&k| n[k as usize]).sum() };
        match f.own[d].div_ceil(MILLI) {
            0 => level[d] = h.spill[h.children[d][0] as usize],
            more => need[d] = kids + more,
        }
    }
    let mut pours = f.pours;
    pours.sort_unstable();
    Balance { sea, n, vfill_mm, bbox, full: f.full, own: f.own, node, level, need, to_sea: f.to_sea, pours }
}

impl Balance {
    /// Levels of closed wet nodes from their cells' ground (gathered from the tiles): the
    /// `need`-th lowest.
    pub fn set_closed_levels(&mut self, gathered: &[(u32, f32)]) -> Result<()> {
        let mut by: BTreeMap<u32, Vec<f32>> = BTreeMap::new();
        for &(d, z) in gathered {
            by.entry(d).or_default().push(z);
        }
        for (d, mut zs) in by {
            let k = self.need[d as usize] as usize;
            ensure(k >= 1 && k <= zs.len(), format!("closed basin {d} has {} cells, needs {k}", zs.len()))?;
            zs.sort_unstable_by(f32::total_cmp);
            self.level[d as usize] = zs[k - 1];
        }
        ensure(
            self.need.iter().zip(&self.level).all(|(&k, l)| k == 0 || l.is_finite()),
            "a closed basin got no level",
        )
    }

    /// Cells a node's water covers: all held cells when full, the needed cells when closed,
    /// both children's when it stands exactly at their merge saddle.
    pub fn area_cells(&self, h: &Hierarchy, d: u32) -> u64 {
        let d = d as usize;
        if self.full[d] {
            self.n[d]
        } else if self.need[d] > 0 {
            self.need[d]
        } else if !h.is_leaf(d as u32) {
            h.children[d].iter().map(|&k| self.n[k as usize]).sum()
        } else {
            0
        }
    }

    /// A node labelled a lake: at least 0.3 m deep (spec) and at least `min_cells` in area
    /// (`lake_min_m2`, review C2); other nodes are ponds.
    pub fn is_lake(&self, h: &Hierarchy, d: u32, min_cells: u64) -> bool {
        self.level[d as usize] - h.low[d as usize] >= LAKE_DEPTH_M && self.area_cells(h, d) >= min_cells
    }
}

impl Blob for Balance {
    fn put(&self, e: &mut Enc) {
        let bbox: Vec<[u64; 4]> = self.bbox.clone();
        (self.sea.clone(), self.n.clone(), self.vfill_mm.clone(), bbox, self.full.clone()).put(e);
        (self.own.clone(), self.node.clone(), self.level.clone(), self.need.clone(), self.to_sea).put(e);
        self.pours.put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (sea, n, vfill_mm, bbox, full) = Blob::get(d)?;
        let (own, node, level, need, to_sea) = Blob::get(d)?;
        let pours = Blob::get(d)?;
        Ok(Self { sea, n, vfill_mm, bbox, full, own, node, level, need, to_sea, pours })
    }
}
```

Add `mod balance;`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::balance`
Expected: 8 pass. Clippy clean.

- [ ] **Step 5: Commit**

Run the task-finish block first.

```bash
git add crates/topography-core/src/world/water
git commit -m "feat(water): Fill-Spill-Merge with a per-class lake-to-catchment ratio

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Expert review**

Expert review protocol. Real-terrain output: none until Task 9 wires the passes; give the reviewer the six tests and ask whether the area balance (R3, R20) and the overflow order match Barnes et al. 2020 and the spec's "including overflow from basins upstream".

---

## Task 8: Routing across full ponds (priority flood from the outlet)

**Files:**
- Create: `crates/topography-core/src/world/water/flats.rs`
- Modify: `crates/topography-core/src/world/water/mod.rs` (`mod flats;`)

**Interfaces:**
- Consumes: `geom::{World, OFFSETS}`, `watershed::ordered`.
- Produces: `flats::resolve(world: World, cells: &[(u64, f32)], outlet: u64) -> Vec<(u64, u8)>`: a direction byte for every pond cell (cells = (world index, ground), sorted by index; same order out). Receivers come from a priority flood from the outlet over the pond's own ground (ascending z, ties by the Barnes 2014 mask, then index): each cell drains to the cell that flooded it, so paths follow the pond's lowest ground (review I2). The routing surface is flat at the pond level, so drainage still never rises.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::geom::OFFSETS;

    fn follow(world: World, dirs: &std::collections::HashMap<u64, u8>, mut c: u64, outlet: u64) -> Vec<u64> {
        let mut path = vec![c];
        while c != outlet {
            let (dy, dx) = OFFSETS[dirs[&c] as usize - 1];
            let (x, y) = world.xy(c);
            c = world.index((x as i64 + dx) as u64, (y as i64 + dy) as u64);
            path.push(c);
            assert!(path.len() <= 200, "a loop");
        }
        path
    }

    #[test]
    fn a_pond_drains_along_its_lowest_ground_to_the_outlet() {
        // A 7 x 3 pond (x 1..8, y 1..4) in a 10 x 5 world, outlet (8, 2); the middle row is
        // 10 cm lower than the rows beside it.
        let world = World { w: 10, h: 5 };
        let cells: Vec<(u64, f32)> =
            (1..4).flat_map(|y| (1..8).map(move |x| (y * 10 + x, if y == 2 { 9.9 } else { 10.0 }))).collect();
        let outlet = 28;
        let dirs: std::collections::HashMap<u64, u8> = resolve(world, &cells, outlet).into_iter().collect();
        assert_eq!(dirs.len(), 21);
        for &(start, _) in &cells {
            let (mut c, mut steps) = (start, 0);
            while c != outlet {
                let d = dirs[&c];
                assert_ne!(d, 0, "cell {c} has no receiver");
                let (dy, dx) = OFFSETS[d as usize - 1];
                let (x, y) = world.xy(c);
                c = world.index((x as i64 + dx) as u64, (y as i64 + dy) as u64);
                steps += 1;
                assert!(steps <= 21, "a loop from {start}");
            }
        }
        // From the far end of the low middle row the path keeps to it.
        for c in follow(world, &dirs, 21, outlet) {
            assert!(c == outlet || c / 10 == 2, "left the low row at {c}");
        }
    }

    #[test]
    fn a_path_follows_a_bent_low_line_not_a_straight_one() {
        // A 12 x 12 pond with a low L-shaped line: down column 2, then along row 10 to the
        // outlet at (12, 10). A BFS geodesic would cut the corner diagonally.
        let world = World { w: 14, h: 14 };
        let low = |x: u64, y: u64| (x == 2 && y <= 10) || (y == 10 && x >= 2);
        let cells: Vec<(u64, f32)> = (1..13)
            .flat_map(|y| (1..12).map(move |x| (y * 14 + x, if low(x, y) { 4.0 } else { 5.0 })))
            .collect();
        let outlet = 10 * 14 + 12;
        let dirs: std::collections::HashMap<u64, u8> = resolve(world, &cells, outlet).into_iter().collect();
        let path = follow(world, &dirs, 14 + 2, outlet);
        for c in &path[..path.len() - 1] {
            assert!(low(c % 14, c / 14), "left the low line at {c}");
        }
    }
}
```

- [ ] **Step 2: Run the test to see it fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::flats`
Expected: compile error.

- [ ] **Step 3: Implement `flats.rs`**

```rust
//! Receivers across a full pond. A pond's cells stand under one level (the routing surface is
//! flat there) and drain to one outlet cell. A priority flood from the outlet over the pond's
//! own ground (lowest first; ties by the Barnes, Lehman and Mulla 2014 mask, 2 · steps from
//! the outlet side + reversed steps from the banks, then by cell index) gives each cell the
//! cell that flooded it as receiver: paths follow the pond's lowest ground, its natural
//! thalweg, instead of BFS geodesics that run straight or diagonal (review I2). The result
//! depends only on the cells, their ground and the outlet, so a pond solved in its tile or
//! gathered across tiles comes out alike.

use super::geom::{OFFSETS, World};
use super::watershed::ordered;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

pub(crate) fn resolve(world: World, cells: &[(u64, f32)], outlet: u64) -> Vec<(u64, u8)> {
    let n = cells.len();
    let index = |c: u64| cells.binary_search_by_key(&c, |p| p.0).ok();
    let around = |i: usize| {
        let (x, y) = world.xy(cells[i].0);
        (0..8).filter_map(move |k| world.neighbour(x, y, k).map(|(nx, ny)| (k, world.index(nx, ny))))
    };
    let bfs = |seed: &dyn Fn(usize) -> bool| {
        let mut dist = vec![0u32; n];
        let mut queue = VecDeque::new();
        for (i, d) in dist.iter_mut().enumerate() {
            if seed(i) {
                *d = 1;
                queue.push_back(i);
            }
        }
        while let Some(i) = queue.pop_front() {
            for (_, c) in around(i) {
                if let Some(j) = index(c) {
                    if dist[j] == 0 {
                        dist[j] = dist[i] + 1;
                        queue.push_back(j);
                    }
                }
            }
        }
        dist
    };
    let toward = bfs(&|i| around(i).any(|(_, c)| c == outlet));
    let away = bfs(&|i| around(i).any(|(_, c)| c != outlet && index(c).is_none()));
    let high = away.iter().copied().max().unwrap_or(0);
    let mask: Vec<u32> = (0..n)
        .map(|i| 2 * toward[i] + if away[i] > 0 { high + 1 - away[i] } else { 0 })
        .collect();
    // Direction byte from cell i to the neighbour (world index) `to`.
    let towards = |i: usize, to: u64| {
        let ((x, y), (tx, ty)) = (world.xy(cells[i].0), world.xy(to));
        let off = (ty as i64 - y as i64, tx as i64 - x as i64);
        OFFSETS.iter().position(|&o| o == off).map_or(0, |k| k as u8 + 1)
    };
    let mut dir = vec![0u8; n];
    let mut heap = BinaryHeap::new();
    for i in 0..n {
        if around(i).any(|(_, c)| c == outlet) {
            dir[i] = towards(i, outlet);
            heap.push(Reverse((ordered(cells[i].1), mask[i], cells[i].0, i)));
        }
    }
    while let Some(Reverse((_, _, _, i))) = heap.pop() {
        for (_, c) in around(i) {
            if let Some(j) = index(c) {
                if dir[j] == 0 {
                    dir[j] = towards(j, cells[i].0);
                    heap.push(Reverse((ordered(cells[j].1), mask[j], cells[j].0, j)));
                }
            }
        }
    }
    cells.iter().zip(dir).map(|(&(c, _), d)| (c, d)).collect()
}
```

Add `mod flats;`.

- [ ] **Step 4: Run the test to see it pass**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::flats`
Expected: 2 pass. Clippy clean.

- [ ] **Step 5: Commit**

Run the task-finish block first.

```bash
git add crates/topography-core/src/world/water
git commit -m "feat(water): route full ponds by a priority flood from their outlet

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Expert review**

Expert review protocol. Real-terrain output: none yet (ponds appear with Task 9's probe); ask the reviewer to confirm that rivers crossing ponds follow the pond's low ground and do not run straight or diagonal.

---
## Task 8b: Exact per-tile contraction and the node table

Review C3, ruling R25. Pass A settles a tile's depression trees whose merges all lie below what
the tile can see of the rest of the world, and ships each as one super-leaf; the global graph
then holds only what crosses tiles. Ruling R28 names every water node by its smallest pit, so
nothing downstream depends on how the hierarchy was split.

The coordinator's rule is "settled when its spill is below the tile ring's minimum ground".
The plan implements the per-tree form of the same argument: a tree is settled up to its
**frontier**, the lowest saddle by which it touches anything not settled in the tile. That is
the lowest edge to a ring pit, a cell draining out of the tile or to the world edge, or one of
its own ring cells. Every frontier is at or above the ring minimum: cells draining out descend
to a ring cell, and seam edges sit on ring cells. So the ring-minimum rule is the special case
that replaces every frontier by that bound. The per-tree rule settles at least as much and is
exact by the same proof.

**Files:**
- Create: `crates/topography-core/src/world/water/contract.rs`
- Create: `crates/topography-core/src/world/water/nodes.rs`
- Modify: `crates/topography-core/src/world/water/mod.rs` (`mod contract; mod nodes;`)
- Modify: `crates/topography-core/src/world/water/watershed.rs` (Step 6 extends its canyon probe)

**Interfaces:**
- Consumes: `watershed::{TileGraph, Edge, EDGE_KEY, drain, tile_graph}`, `hierarchy::{Hierarchy, NONE, build}`, `balance::{Balance, Counts, solve}`, `geom::UnionFind`, `store::Blob`.
- Produces:
  - `contract::Frag { h: Hierarchy, tree: Vec<u64>, size: Vec<u32>, ring_pit: Vec<(u64, u64)> }` (`Blob`, `PartialEq`) with `leaf(pit: u64) -> Option<u32>` and `ring_leaf(cell: u64) -> Option<u32>`. `ring_pit` keeps each ring cell's pre-contraction pit, so a path entering the tile reaches the right local leaf (B1). Its leaves are the tile's candidate pits (not ring pits), with keys in `h.leaf_pit`; its metas are their settled merges. Each local depression knows its tree's key (the tree's smallest pit) and the number of pits in the tree. A tree's top has `parent == NONE` and spill +∞ until the global step sets it.
  - `contract::contract(g: TileGraph) -> (TileGraph, Frag)`: the remapped graph (super-leaf keys in place of their pits; supplies summed, edges re-minimised, ring ends remapped) and the fragment.
  - `contract::min_pits(h: &Hierarchy) -> Vec<u64>` (the smallest pit key under each depression: its node key, R28).
  - `contract::Tops = BTreeMap<u64, (f32, [u64; 2])>` (tree key → its super-leaf's global spill and spill edge), `contract::with_spills(frag: &Frag, tops: &Tops) -> Hierarchy` (listed tops get the global spill and outlet, N7; unlisted ones −∞), `contract::solve_local(frag: &Frag, tops: &Tops, extra: &[(u32, u64)], counts: &Counts) -> Balance`: the inside of the listed trees, with `extra` water poured into local leaves. `counts` must hold every cell the listed trees hold, from every tile (B2; Task 9 decides where).
  - `nodes::Node { key: u64, level: f32, low: f32, full: bool, lake: bool, outlet: [u64; 2], cells: u64, vfill_mm: i64, need: u64, bbox: [u64; 4] }` and `nodes::Nodes(pub Vec<Node>)` (sorted by key; `Blob`, `PartialEq`) with `ordinal(key) -> Option<u32>`, `get(key) -> Option<&Node>`, `at(ordinal) -> &Node`.

- [ ] **Step 1: Write the failing tests**

`contract.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::geom::{Rect, World, tiles};
    use crate::world::water::hierarchy::build;
    use crate::world::water::hierarchy::tests::{graph_for, rough};
    use crate::world::water::watershed::{drain, ordered, tile_graph};

    /// Pass A with contraction on tiles of `tile` cells: the fragments and the global build.
    fn contracted_for(world: World, z: &[f32], tile: u64) -> (Vec<Frag>, Hierarchy) {
        let all = Rect::whole(world);
        let (graphs, frags): (Vec<TileGraph>, Vec<Frag>) = tiles(world, tile)
            .into_iter()
            .map(|core| {
                let win = core.grow(1, world);
                let zw: Vec<f32> = (0..win.cells()).map(|k| { let (x, y) = win.world_xy(k); z[all.local(x, y).expect("in")] }).collect();
                let zc: Vec<f32> = (0..core.cells()).map(|k| { let (x, y) = core.world_xy(k); z[all.local(x, y).expect("in")] }).collect();
                let d = drain(world, win, &zw, core);
                contract(tile_graph(world, core, &zc, &d, |_| 1000))
            })
            .unzip();
        let (h, _) = build(world, tile, &graphs).expect("hierarchy");
        (frags, h)
    }

    /// (smallest pit, spill bits, low bits) of every depression, sorted.
    fn signature(h: &Hierarchy, skip_tops: bool) -> Vec<(u64, u32, u32)> {
        let keys = min_pits(h);
        let mut v: Vec<(u64, u32, u32)> = (0..h.len())
            .filter(|&d| !(skip_tops && h.parent[d] == NONE))
            .map(|d| (keys[d], ordered(h.spill[d]), ordered(h.low[d])))
            .collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn contraction_keeps_every_depression_and_spill() {
        let world = World { w: 48, h: 40 };
        let z = rough(world);
        let (full, _) = graph_for(world, &z, 64);
        for size in [64, 16, 7] {
            let (frags, part) = contracted_for(world, &z, size);
            // The global part plus every fragment's settled (non-top) depressions is the whole
            // hierarchy: same smallest pits, spills and lows.
            let mut got = signature(&part, false);
            for f in &frags {
                got.extend(signature(&f.h, true));
            }
            got.sort_unstable();
            assert_eq!(got, signature(&full, false), "tile size {size}");
            assert!(part.leaves <= full.leaves);
        }
    }

    #[test]
    fn two_pits_below_their_frontiers_settle_into_one_tree() {
        // 7 x 5, pits A (1 m) and B (2 m) behind a 5 m saddle, B leaves east over a 7 m rim.
        let (world, z) = crate::world::water::watershed::tests::two_pits();
        let all = Rect::whole(world);
        let d = drain(world, all, &z, all);
        let (g, frag) = contract(tile_graph(world, all, &z, &d, |_| 1000));
        // A and B merge at 5 m, below both frontiers (A's 8 m rim, B's 7 m exit): one tree.
        assert_eq!(frag.h.leaves, 2);
        assert_eq!(frag.h.len(), 3);
        assert_eq!(frag.tree, vec![15, 15, 15]);
        assert_eq!(g.pits, vec![(15, 1.0)], "one super-leaf named by the smaller pit");
        assert_eq!(g.supply[&Next::Node(15)], frag.h.supply.iter().sum::<u64>());
        assert!(g.edges.iter().all(|e| e.a != e.b));
    }
}
```

`nodes.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_are_found_by_key_and_ordinal() {
        let n = |key| Node { key, ..Default::default() };
        let t = Nodes(vec![n(3), n(10), n(42)]);
        assert_eq!(t.ordinal(10), Some(1));
        assert_eq!(t.ordinal(11), None);
        assert_eq!(t.at(2).key, 42);
        assert_eq!(t.get(3).map(|x| x.key), Some(3));
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::contract` and `… world::water::nodes`
Expected: compile errors.

- [ ] **Step 3: Implement `contract.rs`**

```rust
//! Exact per-tile contraction (review C3). A tile's candidate pits are its pits off the ring
//! (ring pits may join pits across a seam). Each candidate's frontier is the lowest saddle by
//! which its watershed touches anything else: a non-candidate end (a ring pit, a cell draining
//! out of the tile or to the world edge) or one of its own ring cells (any seam edge sits
//! there). All are at or above the ring's lowest ground. A local Kruskal merges two components
//! only below both frontiers; a component that meets an edge it may not take is frozen. Every
//! merge it makes is then one the world's Kruskal makes in the same way, because no edge of
//! lower rank touches either component outside the tile. Settled trees go to the global step
//! as one super-leaf named by their smallest pit; `solve_local` solves their inside later.

use super::balance::{Balance, Counts, solve};
use super::flow::Next;
use super::geom::UnionFind;
use super::hierarchy::{Hierarchy, NONE};
use super::store::{Blob, Dec, Enc};
use super::watershed::{EDGE_KEY, Edge, TileGraph};
use crate::acquisition::Result;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Frag {
    pub h: Hierarchy,
    pub tree: Vec<u64>,
    pub size: Vec<u32>,
    /// Ring cells draining to a candidate pit, with that pit as it was before the remap
    /// (sorted by cell): a path entering the tile there reaches that pit's leaf (B1).
    pub ring_pit: Vec<(u64, u64)>,
}

impl Blob for Frag {
    fn put(&self, e: &mut Enc) {
        (self.h.clone(), self.tree.clone(), self.size.clone(), self.ring_pit.clone()).put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (h, tree, size, ring_pit) = Blob::get(d)?;
        Ok(Self { h, tree, size, ring_pit })
    }
}

impl Frag {
    /// The local leaf of a pit key, if it is a candidate of this tile.
    pub fn leaf(&self, pit: u64) -> Option<u32> {
        self.h.leaf_pit.binary_search(&pit).ok().map(|i| i as u32)
    }
    /// The local leaf a ring cell of this tile drains to, if a candidate pit.
    pub fn ring_leaf(&self, cell: u64) -> Option<u32> {
        let i = self.ring_pit.binary_search_by_key(&cell, |r| r.0).ok()?;
        self.leaf(self.ring_pit[i].1)
    }
}

/// The smallest pit key under each depression (children come before parents).
pub(crate) fn min_pits(h: &Hierarchy) -> Vec<u64> {
    let mut out = vec![u64::MAX; h.len()];
    for d in 0..h.len() {
        if h.is_leaf(d as u32) {
            out[d] = h.leaf_pit[d];
        } else {
            out[d] = h.children[d].iter().map(|&c| out[c as usize]).min().unwrap_or(u64::MAX);
        }
    }
    out
}

pub(crate) fn contract(g: TileGraph) -> (TileGraph, Frag) {
    let ring_pits: BTreeSet<u64> = g
        .ring
        .iter()
        .filter_map(|r| match (r.pit, r.end) {
            (true, Next::Node(p)) => Some(p),
            _ => None,
        })
        .collect();
    let pits: Vec<(u64, f32)> = g.pits.iter().copied().filter(|p| !ring_pits.contains(&p.0)).collect();
    let n = pits.len();
    let slot = |t: Next| match t {
        Next::Node(p) if p != EDGE_KEY => pits.binary_search_by_key(&p, |q| q.0).ok(),
        _ => None,
    };
    let mut front = vec![f32::INFINITY; n];
    for r in &g.ring {
        if let Some(i) = slot(r.end) {
            front[i] = front[i].min(r.z);
        }
    }
    let mut inner: Vec<(usize, usize, Edge)> = Vec::new();
    for e in &g.edges {
        match (slot(e.a), slot(e.b)) {
            (Some(i), Some(j)) => inner.push((i, j, *e)),
            (Some(i), None) => front[i] = front[i].min(e.saddle),
            (None, Some(j)) => front[j] = front[j].min(e.saddle),
            (None, None) => {}
        }
    }
    inner.sort_by_key(|x| x.2.rank());

    let mut h = Hierarchy {
        leaves: n,
        leaf_pit: pits.iter().map(|p| p.0).collect(),
        low: pits.iter().map(|p| p.1).collect(),
        spill: vec![f32::INFINITY; n],
        parent: vec![NONE; n],
        children: vec![[NONE, NONE]; n],
        outlet: vec![[0, 0]; n],
        geolink: vec![NONE; n],
        ocean_order: Vec::new(),
        supply: pits.iter().map(|p| g.supply.get(&Next::Node(p.0)).copied().unwrap_or(0)).collect(),
    };
    let mut uf = UnionFind::new(n);
    let (mut top, mut frontier, mut frozen): (Vec<u32>, Vec<f32>, Vec<bool>) =
        ((0..n as u32).collect(), front, vec![false; n]);
    for (i, j, e) in inner {
        let (ri, rj) = (uf.find(i as u32) as usize, uf.find(j as u32) as usize);
        if ri == rj {
            continue;
        }
        if frozen[ri] || frozen[rj] || e.saddle >= frontier[ri] || e.saddle >= frontier[rj] {
            (frozen[ri], frozen[rj]) = (true, true);
            continue;
        }
        let (da, db) = (top[ri], top[rj]);
        let m = h.len() as u32;
        let low = h.low[da as usize].min(h.low[db as usize]);
        h.low.push(low);
        h.spill.push(f32::INFINITY);
        h.parent.push(NONE);
        h.children.push([da, db]);
        h.outlet.push([0, 0]);
        h.geolink.push(NONE);
        // `e.ca` drains to pit i, `e.cb` to pit j.
        for (d, inside, outside, beyond) in [(da, e.ca, e.cb, j as u32), (db, e.cb, e.ca, i as u32)] {
            h.spill[d as usize] = e.saddle;
            h.outlet[d as usize] = [inside, outside];
            h.geolink[d as usize] = beyond;
            h.parent[d as usize] = m;
        }
        let r = uf.union(ri as u32, rj as u32) as usize;
        (top[r], frontier[r], frozen[r]) = (m, frontier[ri].min(frontier[rj]), false);
    }
    // Tree keys: the root's pit (the union-find root is the smallest index, pits are sorted).
    let root_key: Vec<u64> = (0..n).map(|i| pits[uf.find(i as u32) as usize].0).collect();
    let mut members: BTreeMap<u64, u32> = BTreeMap::new();
    for &k in &root_key {
        *members.entry(k).or_insert(0) += 1;
    }
    let mut tree = vec![0u64; h.len()];
    for d in 0..h.len() {
        tree[d] = if d < n { root_key[d] } else { tree[h.children[d][0] as usize] };
    }
    let size = tree.iter().map(|k| members[k]).collect();

    // The remapped graph: each tree is one leaf named by its key.
    let remap = |t: Next| match slot(t) {
        Some(i) => Next::Node(root_key[i]),
        None => t,
    };
    let mut supply = BTreeMap::new();
    for (&t, &s) in &g.supply {
        *supply.entry(remap(t)).or_insert(0) += s;
    }
    let mut edges: Vec<Edge> = g
        .edges
        .iter()
        .filter_map(|e| {
            let (a, b) = (remap(e.a), remap(e.b));
            (a != b).then(|| if a < b { Edge { a, b, ..*e } } else { Edge { a: b, b: a, ca: e.cb, cb: e.ca, ..*e } })
        })
        .collect();
    edges.sort_unstable_by(|x, y| (x.a, x.b, x.rank()).cmp(&(y.a, y.b, y.rank())));
    edges.dedup_by_key(|e| (e.a, e.b));
    let ring = g.ring.iter().map(|r| super::watershed::Ring { end: remap(r.end), ..*r }).collect();
    let mut low: BTreeMap<u64, f32> = BTreeMap::new();
    for &(p, z) in &g.pits {
        let k = match slot(Next::Node(p)) { Some(i) => root_key[i], None => p };
        let e = low.entry(k).or_insert(z);
        *e = e.min(z);
    }
    let ring_pit: Vec<(u64, u64)> = g
        .ring
        .iter()
        .filter_map(|r| match r.end {
            Next::Node(p) if slot(r.end).is_some() => Some((r.cell, p)),
            _ => None,
        })
        .collect::<BTreeMap<u64, u64>>()
        .into_iter()
        .collect();
    let graph = TileGraph { ring, pits: low.into_iter().collect(), supply, edges, cycles: g.cycles };
    (graph, Frag { h, tree, size, ring_pit })
}

/// A tree's global facts, by tree key: its super-leaf's spill and spill edge.
pub(crate) type Tops = BTreeMap<u64, (f32, [u64; 2])>;

/// The fragment's hierarchy with each listed tree's top given its super-leaf's global spill
/// and outlet (N7: a full top's node then matches the uncontracted run's), unlisted tops −∞ so
/// `holding` finds nothing in them.
pub(crate) fn with_spills(frag: &Frag, tops: &Tops) -> Hierarchy {
    let mut h = frag.h.clone();
    for d in 0..h.len() {
        if h.parent[d] == NONE {
            match tops.get(&frag.tree[d]) {
                Some(&(spill, outlet)) => (h.spill[d], h.outlet[d]) = (spill, outlet),
                None => h.spill[d] = f32::NEG_INFINITY,
            }
        }
    }
    h
}

/// Solves the inside of the trees in `tops`. Their leaves get their own supply plus `extra`
/// (local leaf, water poured in from outside); other trees are left dry. `counts` holds every
/// cell the listed trees hold (from every tile) by local holding depression.
pub(crate) fn solve_local(frag: &Frag, tops: &Tops, extra: &[(u32, u64)], counts: &Counts) -> Balance {
    let mut h = with_spills(frag, tops);
    for d in 0..h.len() {
        if h.parent[d] == NONE && tops.contains_key(&frag.tree[d]) {
            h.ocean_order.push(d as u32);
        }
    }
    for l in 0..h.leaves {
        if !tops.contains_key(&frag.tree[l]) {
            h.supply[l] = 0;
        }
    }
    for &(l, w) in extra {
        h.supply[l as usize] += w;
    }
    solve(&h, counts, false)
}
```

`nodes.rs`:

```rust
//! Every water node of a run (a depression whose water covers the cells under it: a lake, a
//! pond or a closed basin), from the global hierarchy and from the trees settled in tiles,
//! named by its smallest pit cell (R28): the same name at any tiling and with contraction.

use super::store::{Blob, Dec, Enc};
use crate::acquisition::Result;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Node {
    pub key: u64,
    pub level: f32,
    pub low: f32,
    pub full: bool,
    pub lake: bool,
    /// [cell inside, cell outside] of its spill edge (meaningful when full).
    pub outlet: [u64; 2],
    /// Cells its water covers.
    pub cells: u64,
    pub vfill_mm: i64,
    /// Closed nodes: cells the balance needs.
    pub need: u64,
    pub bbox: [u64; 4],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Nodes(pub Vec<Node>);

impl Nodes {
    pub fn ordinal(&self, key: u64) -> Option<u32> {
        self.0.binary_search_by_key(&key, |n| n.key).ok().map(|i| i as u32)
    }
    pub fn get(&self, key: u64) -> Option<&Node> {
        self.ordinal(key).map(|o| &self.0[o as usize])
    }
    pub fn at(&self, ordinal: u32) -> &Node {
        &self.0[ordinal as usize]
    }
}

impl Blob for Nodes {
    fn put(&self, e: &mut Enc) {
        let rows: Vec<((u64, f32, f32, bool, bool), (u64, u64, u64, i64, u64), [u64; 4])> = self
            .0
            .iter()
            .map(|n| ((n.key, n.level, n.low, n.full, n.lake), (n.outlet[0], n.outlet[1], n.cells, n.vfill_mm, n.need), n.bbox))
            .collect();
        rows.put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let rows: Vec<((u64, f32, f32, bool, bool), (u64, u64, u64, i64, u64), [u64; 4])> = Blob::get(d)?;
        Ok(Self(
            rows.into_iter()
                .map(|((key, level, low, full, lake), (o0, o1, cells, vfill_mm, need), bbox)| Node {
                    key, level, low, full, lake, outlet: [o0, o1], cells, vfill_mm, need, bbox,
                })
                .collect(),
        ))
    }
}
```

`Ring` needs `Clone, Copy` (it derives them in Task 5). `Hierarchy` is `Default` (Task 6).

- [ ] **Step 4: Run the tests to see them pass**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::contract` and `… world::water::nodes`
Expected: 2 + 1 pass. Clippy clean.

- [ ] **Step 5: Commit**

Run the task-finish block first.

```bash
git add crates/topography-core/src/world/water
git commit -m "feat(water): exact per-tile contraction of settled depression trees and the node table

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Expert review**

Expert review protocol. Real-terrain output: extend Task 5's canyon probe (`probe_pits_in_the_canyon`) to run `contract` on the 2,048² window cut into 512² tiles and print pits, super-leaves and the share settled. This is the number that decides gate 2's memory, so the extrapolation to the world goes into `TASKS.md`. Ask the reviewer to check the frontier proof, including on ties.

---

## Task 9: The model run over tiles, and tiled exactness

**Files:**
- Create: `crates/topography-core/src/world/water/pipeline.rs`
- Create: `crates/topography-core/src/world/water/testing.rs` (`#[cfg(test)]` synthetic terrain and setup)
- Modify: `crates/topography-core/src/world/water/mod.rs` (`mod pipeline;` and `#[cfg(test)] mod testing;`)
- Modify: `crates/topography-core/src/world/water/flow.rs` (`Blob` for `Solved`)

**Interfaces:**
- Consumes: everything from Tasks 2–8b.
- Produces:
  - `pipeline::Setup { world, nw_m: [f64; 2], spacing_m, tile: u64, threads: usize, classes: Option<Classes>, table: Table, rain: Rain, params: Params, contract: bool }` with `tiles()`, `cell_km2()`, `lake_min_cells() -> u64`, `centre(x, y) -> (f64, f64)`, `rain_units(core: Rect) -> Vec<u64>`, `supply(core: Rect) -> Result<Vec<u64>>`, `river_km2() -> f32`. `contract` is on in every real run; off only in the test that proves it changes nothing.
  - `pipeline::Wet { Dry, Sea, Lake(u32), Pond(u32) }` (the `u32` is the node's ordinal in `Nodes`), `pipeline::Water { h: Hierarchy, resolver: Resolver, top: Vec<f32>, bal: Balance, minpit: Vec<u64>, nodes: Nodes, solved: Vec<u64>, remote: Vec<Local>, remote_of: BTreeMap<u64, u32> }`, `pipeline::Local { frag: Frag, h: Hierarchy, top: Vec<f32>, bal: Balance, minpit: Vec<u64>, solved: Tops }` (`Blob`; a tile's settled trees and the inside of those solved apart) with `remap(Next) -> Next`, `solved_leaf(Next) -> Option<u32>` and `leaf_of_end(&Resolver, Next) -> Option<u32>` (follows an `Out` path to the tree's ring entry, B1), `pipeline::Model { water: Water, ponds: HashMap<u64, u8>, flow: Solved, cycles: u64, locals: Vec<Local> }` (`locals` filled only without a work directory; `Model::local(work, tag, i) -> Result<Local>` reads either, decoding the cached `(Local, Nodes, Vec<u64>)`, N6), `pipeline::crop<T: Copy>(win, values, core) -> Vec<T>` (shared, N16). Pass L solves a tree in its tile only when the tree's held cell count there equals the global one and every pour into it enters there; pass L2 solves the rest from cells and pours gathered in every tile (B2).
  - `pipeline::Rasters { level: Layer<f32>, node: Layer<u32>, discharge: Layer<f32>, flow: Layer<u8> }` with `Rasters::memory(World)` and `Rasters::files(World, dir: &Path, tag: &str) -> Result<Rasters>`. Encodings: `level` the water level where wet (sea 0, ponds their level), NaN dry; `node` 0 dry, 1 sea, `2 + ordinal` of the node in `Water::nodes` (R28; the same at any tiling); `discharge` km² at mean rain (f32); `flow` v2 bits (low 4 the receiver, 16 thalweg, 32 dry basin, 64 endorheic).
  - `pipeline::model(s: &Setup, ground: &Layer<f32>, work: Option<&Work>, tag: &str, out: Option<&Rasters>) -> Result<Model>` with passes A (label graphs and contraction), B (global counts), L (settled trees solved in their tile when wholly there), L2 (the other settled trees, from every tile's cells), C (closed levels), D (ponds across tiles), E (flow reports), F (rasters).
  - `pipeline::state(s, w: &Water, local: &Local, core, win, zw: &[f32]) -> TileState` and `TileState { core, z, dir, wet: Vec<Wet>, held: Vec<bool>, level: Vec<f32> }` (used again by breach, Task 11).
  - Test-only `testing::{terrain(World) -> Vec<f32>, classes(World) -> Vec<u8>, setup(World, tile: u64, river_km2: f64) -> Setup}`.

- [ ] **Step 1: Write the synthetic terrain and the failing tests**

`testing.rs`:

```rust
//! Test-only: a synthetic grid with sea, a lagoon, a humid basin, an arid basin and rivers.

use super::geom::World;
use super::layers::{Classes, Layer};
use super::pipeline::Setup;
use super::rain::Rain;
use super::table::{DEFAULT, Table};
use super::params;

/// Falls east into the sea; a humid bowl at (16, 16), an arid bowl at (16, 46) (dune class),
/// a lagoon behind a rim at (54, 32), bumps and noise.
pub(crate) fn terrain(world: World) -> Vec<f32> {
    let bowl = |x: f32, y: f32, cx: f32, cy: f32, r: f32, depth: f32| {
        let d2 = ((x - cx).powi(2) + (y - cy).powi(2)) / (r * r);
        if d2 < 1.0 { -depth * (1.0 - d2) } else { 0.0 }
    };
    (0..world.w * world.h)
        .map(|i| {
            let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
            let mut z = 30.0 - 0.5 * x + 3.0 * (0.3 * x).sin() * (0.25 * y).cos()
                + ((i * 7919) % 23) as f32 * 0.05;
            z += bowl(x, y, 16.0, 16.0, 7.0, 9.0) + bowl(x, y, 16.0, 46.0, 7.0, 9.0);
            let r = ((x - 54.0).powi(2) + (y - 32.0).powi(2)).sqrt();
            if r < 3.0 {
                z = -2.0 + r * 0.3;
            } else if r < 5.0 {
                z = z.max(2.5);
            }
            z
        })
        .collect()
}

/// Dunes (8, r = 0: playas) in the south-west quarter, plain (18, r = 1) elsewhere.
pub(crate) fn classes(world: World) -> Vec<u8> {
    (0..world.w * world.h)
        .map(|i| if i % world.w < world.w / 2 && i / world.w >= world.h / 2 { 8 } else { 18 })
        .collect()
}

pub(crate) fn setup(world: World, tile: u64, river_km2: f64) -> Setup {
    let mut table = Table::parse(DEFAULT).expect("table");
    table.lake_min_m2 = 4.0;
    let params = params(&table, 1.0, Some(river_km2));
    Setup {
        world,
        nw_m: [0.0, world.h as f64],
        spacing_m: 1.0,
        tile,
        threads: 3,
        classes: Some(Classes { layer: Layer::memory(world, classes(world)), k: 1 }),
        table,
        rain: Rain::uniform(2.0),
        params,
        contract: true,
    }
}
```

The synthetic world is small, so the test setup lowers the lake minimum to 4 cells
(`s.table.lake_min_m2 = 4.0` in `setup`, after `Table::parse`); real runs use the table's
500 m².

`pipeline.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::testing::{setup, terrain};

    fn run_with(world: World, tile: u64, contract: bool) -> (Model, Vec<f32>, Vec<u32>, Vec<f32>, Vec<u8>) {
        let mut s = setup(world, tile, 1e-5);
        s.contract = contract;
        let ground = Layer::memory(world, terrain(world));
        let r = Rasters::memory(world);
        let m = model(&s, &ground, None, "m", Some(&r)).expect("model");
        (m, r.level.to_vec().unwrap(), r.node.to_vec().unwrap(), r.discharge.to_vec().unwrap(), r.flow.to_vec().unwrap())
    }

    fn run(world: World, tile: u64) -> (Model, Vec<f32>, Vec<u32>, Vec<f32>, Vec<u8>) {
        run_with(world, tile, true)
    }

    /// Whole vs tiles, and contraction on vs off (review C3): every raster and the node table
    /// bit-identical.
    #[test]
    fn tilings_and_contraction_give_bit_identical_results() {
        let world = World { w: 64, h: 64 };
        let (wm, wl, wn, wq, wf) = run_with(world, 64, false);
        assert_eq!(wm.cycles, 0);
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        for (tile, contract) in [(64, true), (16, false), (16, true), (13, true)] {
            let (m, l, n, q, f) = run_with(world, tile, contract);
            let at = format!("tile {tile}, contraction {contract}");
            assert_eq!(bits(&l), bits(&wl), "levels, {at}");
            assert_eq!(n, wn, "labels, {at}");
            assert_eq!(bits(&q), bits(&wq), "discharge, {at}");
            assert_eq!(f, wf, "flow, {at}");
            assert_eq!(m.water.nodes, wm.water.nodes, "nodes, {at}");
            assert_eq!(m.cycles, 0);
        }
    }

    /// An arid (r = 0.1) closed basin with two pits inside tile x 32..48, y 16..32 of a 64 x 48
    /// world, spilling east through a notch. `elongated`: its floor ramps west across the seam
    /// at x = 32, so cells in the neighbouring tile are held by the settled tree (pass L2);
    /// otherwise every held cell lies in the tile (pass L).
    fn basin(elongated: bool) -> (Setup, Layer<f32>) {
        let world = World { w: 64, h: 48 };
        let z: Vec<f32> = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                let mut z = 25.0 + 0.01 * (50.0 - x) + ((i * 7919) % 13) as f32 * 0.001;
                if x > 50.0 {
                    z -= 2.0 * (x - 50.0);
                }
                let rx = if x < 40.0 && elongated { 14.0 } else { 6.0 };
                let d2 = ((x - 40.0) / rx).powi(2) + ((y - 24.0) / 5.0).powi(2);
                if d2 < 1.0 {
                    z -= 5.0 * (1.0 - d2);
                    for px in [38.0, 42.0] {
                        z -= (-((x - px).powi(2) + (y - 24.0).powi(2)) / 2.0).exp();
                    }
                }
                if y == 24.0 && x >= 44.0 {
                    z = z.min(23.0 - 0.1 * (x - 44.0));
                }
                z
            })
            .collect();
        let mut s = setup(world, 16, 1e-5);
        s.classes = Some(Classes { layer: Layer::memory(world, vec![2; (world.w * world.h) as usize]), k: 1 });
        (s, Layer::memory(world, z))
    }

    /// Pass L and L2 against the uncontracted whole (B2): a closed lake inside a settled tree,
    /// once wholly in its tile and once holding cells across a seam.
    #[test]
    fn settled_trees_solved_in_their_tile_or_across_seams_match_the_whole() {
        for elongated in [false, true] {
            let run = |tile: u64, contract: bool| {
                let (mut s, ground) = basin(elongated);
                (s.tile, s.contract) = (tile, contract);
                let r = Rasters::memory(s.world);
                let m = model(&s, &ground, None, "m", Some(&r)).expect("model");
                let bits = |v: Vec<f32>| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
                (m, bits(r.level.to_vec().unwrap()), r.node.to_vec().unwrap(), bits(r.discharge.to_vec().unwrap()), r.flow.to_vec().unwrap())
            };
            let whole = run(64, false);
            let tiled = run(16, true);
            assert_eq!(tiled.1, whole.1, "levels, elongated {elongated}");
            assert_eq!(tiled.2, whole.2, "labels, elongated {elongated}");
            assert_eq!(tiled.3, whole.3, "discharge, elongated {elongated}");
            assert_eq!(tiled.4, whole.4, "flow, elongated {elongated}");
            assert_eq!(tiled.0.water.nodes, whole.0.water.nodes, "nodes (outlets included, N7)");
            assert!(whole.0.water.nodes.0.iter().any(|n| !n.full && n.need > 0), "the basin holds a closed lake");
            if elongated {
                assert!(!tiled.0.water.remote.is_empty(), "the tree holds cells across the seam: pass L2");
            } else {
                assert!(tiled.0.water.remote.is_empty() && !tiled.0.water.solved.is_empty(), "solved in its tile");
            }
        }
    }

    #[test]
    fn the_synthetic_world_has_sea_a_lagoon_lakes_and_a_closed_basin() {
        let world = World { w: 64, h: 64 };
        let (m, level, node, q, flow) = run(world, 16);
        let at = |x: usize, y: usize| y * 64 + x;
        assert_eq!(node[at(63, 10)], 1, "the east edge is sea");
        assert!(node[at(54, 32)] >= 2, "the lagoon is a lake, not sea");
        assert!(level[at(54, 32)] > -2.0, "it stands above its floor");
        let humid = node[at(16, 16)];
        assert!(humid >= 2, "the humid bowl holds water");
        assert!(m.water.nodes.at(humid - 2).full, "r = 1 fills the humid bowl to its spill");
        assert_eq!(node[at(16, 46)], 0, "r = 0 leaves the dune bowl a dry playa");
        assert!(flow.iter().any(|f| f & 16 != 0), "rivers");
        // Discharge never falls along a receiver on dry land.
        for k in 0..64 * 64 {
            let d = flow[k] & 15;
            if d == 0 || node[k] != 0 {
                continue;
            }
            let (dy, dx) = crate::world::water::geom::OFFSETS[d as usize - 1];
            let j = ((k / 64) as i64 + dy) as usize * 64 + ((k % 64) as i64 + dx) as usize;
            assert!(q[j] >= q[k], "discharge falls from {k} to {j}");
        }
    }

    /// cargo test … pipeline -- --ignored --nocapture
    #[test]
    #[ignore]
    fn probe_model_at_the_dam_spot() {
        use crate::world::water::probe;
        let Some((win, z)) = probe::window(25645, 24400, 1024) else { return };
        let world = World { w: win.w as u64, h: win.h as u64 };
        let mut s = setup(world, 256, 0.05);
        s.classes = None;
        s.table.lake_min_m2 = 500.0;
        s.nw_m = [win.x0 as f64, (65536 - win.y0) as f64];
        let r = Rasters::memory(world);
        let m = model(&s, &Layer::memory(world, z.clone()), None, "m", Some(&r)).expect("model");
        let node = r.node.to_vec().unwrap();
        let flow = r.flow.to_vec().unwrap();
        let nodes = &m.water.nodes.0;
        let lakes = nodes.iter().filter(|n| n.lake).count();
        eprintln!("leaves {} nodes {} lakes {lakes} ponds {} lake cells {} river cells {} cycles {}",
            m.water.h.leaves, nodes.len(), nodes.len() - lakes,
            node.iter().filter(|&&n| n >= 2 && m.water.nodes.at(n - 2).lake).count(),
            flow.iter().filter(|&&f| f & 16 != 0).count(), m.cycles);
        // Node area and depth histograms (review C2.4): log2 bins of m², 10 cm bins of depth.
        let mut area = [0u64; 24];
        let mut depth = [0u64; 32];
        for n in nodes {
            area[((n.cells as f64 * 1.0).max(1.0).log2() as usize).min(23)] += 1;
            depth[(((n.level - n.low) * 10.0) as usize).min(31)] += 1;
        }
        eprintln!("node area (log2 m² bins): {area:?}\nnode depth (0.1 m bins): {depth:?}");
        let wet: Vec<bool> = (0..z.len()).map(|k| node[k] >= 2 && m.water.nodes.at(node[k] - 2).lake || flow[k] & 16 != 0).collect();
        probe::shade_png("t9-dam-spot-water", &z, win.w, &wet);
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::pipeline`
Expected: compile errors.

- [ ] **Step 3: Add `Blob` for `Solved` in `flow.rs`**

```rust
impl Blob for Solved {
    fn put(&self, e: &mut Enc) {
        let mut inflow: Vec<(u64, u64)> = self.inflow.iter().map(|(&k, &v)| (k, v)).collect();
        inflow.sort_unstable();
        (inflow, self.node_total.clone()).put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (inflow, node_total): (Vec<(u64, u64)>, BTreeMap<u64, u64>) = Blob::get(d)?;
        Ok(Self { inflow: inflow.into_iter().collect(), node_total })
    }
}
```

- [ ] **Step 4: Implement `pipeline.rs`**

```rust
//! One run of the water model over tiles: pass A (label graphs, settled trees contracted) →
//! global hierarchy → pass B (cells by holding depression) → Fill-Spill-Merge → pass L (the
//! inside of settled trees, solved in their tile) → pass C (ground of closed basins) → levels
//! and the node table → pass D (ponds that cross tiles) → their receivers → pass E (flow
//! reports) → the exact port solve → pass F (write the rasters). A window is the same run with
//! one tile; water nodes are named by their smallest pit (R28), so nothing depends on the split.

use super::balance::{Balance, Counts, TileCounts, count, holding, solve, top_spill};
use super::contract::{Frag, Tops, contract, min_pits, solve_local, with_spills};
use super::flats;
use super::flow::{Next, Report, Solved, accumulate, add_inflows, ends, solve as solve_flow};
use super::geom::{Rect, World, tiles};
use super::hierarchy::{Hierarchy, NONE, Resolver, build};
use super::layers::{Classes, Layer};
use super::nodes::{Node, Nodes};
use super::rain::{Q_PER_KM2, Rain};
use super::store::{Blob, Dec, Enc, Work, cached, for_tiles};
use super::table::Table;
use super::watershed::{TileGraph, drain, tile_graph};
use super::Params;
use crate::acquisition::{AcquisitionError, Result};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

pub(crate) struct Setup {
    pub world: World,
    /// North-west corner of the grid in the layout plane (x east, y north).
    pub nw_m: [f64; 2],
    pub spacing_m: f64,
    pub tile: u64,
    pub threads: usize,
    pub classes: Option<Classes>,
    pub table: Table,
    pub rain: Rain,
    pub params: Params,
    /// Settle depression trees inside tiles (review C3); exact, so off only in its test.
    pub contract: bool,
}

impl Setup {
    pub fn tiles(&self) -> Vec<Rect> {
        tiles(self.world, self.tile)
    }
    pub fn cell_km2(&self) -> f64 {
        self.spacing_m * self.spacing_m / 1e6
    }
    /// The smallest labelled lake in cells (`lake_min_m2`).
    pub fn lake_min_cells(&self) -> u64 {
        (self.table.lake_min_m2 / (self.spacing_m * self.spacing_m)).ceil() as u64
    }
    pub fn centre(&self, x: u64, y: u64) -> (f64, f64) {
        (self.nw_m[0] + (x as f64 + 0.5) * self.spacing_m, self.nw_m[1] - (y as f64 + 0.5) * self.spacing_m)
    }
    pub fn rain_units(&self, core: Rect) -> Vec<u64> {
        (0..core.cells())
            .map(|k| {
                let (x, y) = core.world_xy(k);
                let (cx, cy) = self.centre(x, y);
                self.rain.units(cx, cy, self.cell_km2())
            })
            .collect()
    }
    /// Each cell's r in thousandths (class 0 without classes).
    pub fn supply(&self, core: Rect) -> Result<Vec<u64>> {
        Ok(match &self.classes {
            Some(c) => c.read(core)?.into_iter().map(|k| u64::from(self.table.r_milli(k))).collect(),
            None => vec![u64::from(self.table.r_milli(0)); core.cells()],
        })
    }
    /// River start in km² as f32, compared with `discharge` everywhere.
    pub fn river_km2(&self) -> f32 {
        self.params.river_km2 as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Wet {
    Dry,
    Sea,
    Lake(u32),
    Pond(u32),
}

/// The water of a run (global).
pub(crate) struct Water {
    pub h: Hierarchy,
    pub resolver: Resolver,
    pub top: Vec<f32>,
    pub bal: Balance,
    /// Each global depression's smallest pit: its node key.
    pub minpit: Vec<u64>,
    /// Every node, global and settled in tiles, by key.
    pub nodes: Nodes,
    /// Keys of super-leaves whose inside was solved apart, in their tile or in pass L2
    /// (sorted); they are not dry basins.
    pub solved: Vec<u64>,
    /// Trees solved in pass L2 from cells of every tile (B2), and their index by tree key.
    pub remote: Vec<Local>,
    pub remote_of: BTreeMap<u64, u32>,
}

/// A tile's settled trees (pass A) and the inside of those it solves (pass L).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Local {
    pub frag: Frag,
    /// `frag.h` with the solved trees' top spills set (others −∞).
    pub h: Hierarchy,
    pub top: Vec<f32>,
    pub bal: Balance,
    pub minpit: Vec<u64>,
    /// Trees solved in this `Local`: tree key → the super-leaf's global spill and spill edge.
    pub solved: Tops,
}

impl Local {
    /// A drainage end with settled pits replaced by their tree's key (what the resolver knows).
    pub fn remap(&self, t: Next) -> Next {
        match t {
            Next::Node(p) => self.frag.leaf(p).map_or(t, |l| Next::Node(self.frag.tree[l as usize])),
            other => other,
        }
    }
    /// The local leaf of an in-tile end inside a tree solved here.
    pub fn solved_leaf(&self, t: Next) -> Option<u32> {
        let Next::Node(p) = t else { return None };
        let l = self.frag.leaf(p)?;
        self.solved.contains_key(&self.frag.tree[l as usize]).then_some(l)
    }
    /// The local leaf a cell's flow reaches in this tile's fragment, from the cell's own
    /// in-tile end (pre-contraction), in any tile: an `Out` path is followed through the
    /// resolver's ring ends until it enters this tile at a ring cell, whose pre-contraction pit
    /// gives the leaf (B1). None when the path ends elsewhere.
    pub fn leaf_of_end(&self, resolver: &Resolver, end: Next) -> Option<u32> {
        let mut t = match end {
            Next::Node(p) => return self.frag.leaf(p),
            Next::Out(c) => c,
            _ => return None,
        };
        loop {
            if let Some(l) = self.frag.ring_leaf(t) {
                return Some(l);
            }
            match resolver.ring_end(t)? {
                Next::Out(c) => t = c,
                _ => return None,
            }
        }
    }
    /// (held, node key and level) of a cell at `z` on local leaf `l` of a tree solved here.
    fn water_at(&self, l: u32, z: f32) -> (bool, Option<(u64, f32)>) {
        water_in(&self.h, &self.top, &self.bal, &self.minpit, l, z)
    }
}

/// (held, node key and level) of a cell at `z` on `leaf` of a hierarchy (N16: one helper for
/// the global, in-tile and remote cases).
fn water_in(h: &Hierarchy, top: &[f32], bal: &Balance, minpit: &[u64], leaf: u32, z: f32) -> (bool, Option<(u64, f32)>) {
    let h0 = holding(h, top, leaf, z);
    let nd = if h0 == NONE { NONE } else { bal.node[h0 as usize] };
    (h0 != NONE, (nd != NONE).then(|| (minpit[nd as usize], bal.level[nd as usize])))
}

impl Blob for Local {
    fn put(&self, e: &mut Enc) {
        let solved: Vec<(u64, f32, u64, u64)> = self.solved.iter().map(|(&k, &(s, o))| (k, s, o[0], o[1])).collect();
        (self.frag.clone(), self.h.clone(), self.top.clone(), self.bal.clone(), (self.minpit.clone(), solved)).put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        #[allow(clippy::type_complexity)]
        let (frag, h, top, bal, (minpit, solved)): (Frag, Hierarchy, Vec<f32>, Balance, (Vec<u64>, Vec<(u64, f32, u64, u64)>)) =
            Blob::get(d)?;
        let solved = solved.into_iter().map(|(k, s, o0, o1)| (k, (s, [o0, o1]))).collect();
        Ok(Self { frag, h, top, bal, minpit, solved })
    }
}

pub(crate) struct Model {
    pub water: Water,
    /// Receivers of full ponds that cross tiles, by world cell.
    pub ponds: HashMap<u64, u8>,
    pub flow: Solved,
    pub cycles: u64,
    /// Each tile's `Local` (runs without a work directory; with one, they are read back).
    pub locals: Vec<Local>,
}

impl Model {
    pub fn local(&self, work: Option<&Work>, tag: &str, i: usize) -> Result<Local> {
        match work {
            Some(w) => Ok(w
                .cached::<(Local, Nodes, Vec<u64>)>(&format!("{tag}/l/{i:05}"), || {
                    Err(AcquisitionError::Invalid("pass L result missing from the work directory".into()))
                })?
                .0),
            None => Ok(self.locals[i].clone()),
        }
    }
}

pub(crate) struct Rasters {
    pub level: Layer<f32>,
    pub node: Layer<u32>,
    pub discharge: Layer<f32>,
    pub flow: Layer<u8>,
}

impl Rasters {
    pub fn memory(world: World) -> Self {
        Self {
            level: Layer::filled(world, f32::NAN),
            node: Layer::filled(world, 0),
            discharge: Layer::filled(world, 0.0),
            flow: Layer::filled(world, 0),
        }
    }
    pub fn files(world: World, dir: &Path, tag: &str) -> Result<Self> {
        Ok(Self {
            level: Layer::open_or_create(world, &dir.join(format!("{tag}-level.f32")), f32::NAN)?,
            node: Layer::open_or_create(world, &dir.join(format!("{tag}-node.u32")), 0)?,
            discharge: Layer::open_or_create(world, &dir.join(format!("{tag}-discharge.f32")), 0.0)?,
            flow: Layer::open_or_create(world, &dir.join(format!("{tag}-flow.u8")), 0)?,
        })
    }
}

pub(crate) struct TileState {
    pub core: Rect,
    pub z: Vec<f32>,
    pub dir: Vec<u8>,
    pub wet: Vec<Wet>,
    /// Inside a depression (held), wet or not.
    pub held: Vec<bool>,
    pub level: Vec<f32>,
}

/// The `core` part of a window's values (shared by every pass that reads a halo, N16).
pub(crate) fn crop<T: Copy>(win: Rect, zw: &[T], core: Rect) -> Vec<T> {
    (0..core.cells())
        .map(|k| {
            let (x, y) = core.world_xy(k);
            zw[win.local(x, y).expect("core in window")]
        })
        .collect()
}

fn inside(b: [u64; 4], core: Rect) -> bool {
    core.contains(b[0], b[1]) && core.contains(b[2], b[3])
}

/// Every core cell's water: sea (fill level at or below 0 m from the world edge), a node's
/// water (lake or pond) where the ground is at or below its level, else dry. Cells of a tree
/// solved apart (in its tile or in pass L2) use that tree's local hierarchy; all others the
/// global one.
pub(crate) fn state(s: &Setup, w: &Water, local: &Local, core: Rect, win: Rect, zw: &[f32]) -> TileState {
    let d = drain(s.world, win, zw, core);
    let z = crop(win, zw, core);
    let n = core.cells();
    let (mut wet, mut held, mut level) = (vec![Wet::Dry; n], vec![false; n], vec![f32::NAN; n]);
    for k in 0..n {
        let leaf = w.resolver.leaf_of(local.remap(d.end[k]));
        if z[k] <= 0.0 && (leaf == NONE || w.top[leaf as usize] <= 0.0) {
            (wet[k], level[k]) = (Wet::Sea, 0.0);
            continue;
        }
        // (held, node key and level) of the water over this cell: a tree solved in pass L2 (any
        // tile), a tree solved in this tile, else the global hierarchy.
        let remote = (leaf != NONE)
            .then(|| w.remote_of.get(&w.resolver.leaf_pit[leaf as usize]))
            .flatten()
            .map(|&r| &w.remote[r as usize]);
        let (is_held, water) = match (remote, local.solved_leaf(d.end[k])) {
            (Some(r), _) => match r.leaf_of_end(&w.resolver, d.end[k]) {
                Some(l) => r.water_at(l, z[k]),
                None => (false, None),
            },
            (None, Some(l)) => local.water_at(l, z[k]),
            (None, None) => water_in(&w.h, &w.top, &w.bal, &w.minpit, leaf, z[k]),
        };
        held[k] = is_held;
        if let Some((key, lv)) = water {
            if z[k] <= lv {
                let o = w.nodes.ordinal(key).expect("every node is in the table");
                level[k] = lv;
                wet[k] = if w.nodes.at(o).lake { Wet::Lake(o) } else { Wet::Pond(o) };
            }
        }
    }
    TileState { core, z, dir: d.dir, wet, held, level }
}

/// Receivers (local index → direction byte) of the full ponds in a tile: from the gathered
/// ones, else solved here when the whole pond lies in the tile.
fn flats_for(s: &Setup, w: &Water, st: &TileState, gathered: &HashMap<u64, u8>) -> HashMap<u32, u8> {
    let mut out = HashMap::new();
    let mut local: BTreeMap<u32, Vec<(u64, f32)>> = BTreeMap::new();
    for k in 0..st.core.cells() {
        let Wet::Pond(o) = st.wet[k] else { continue };
        let node = w.nodes.at(o);
        if !node.full {
            continue;
        }
        let (x, y) = st.core.world_xy(k);
        let gi = s.world.index(x, y);
        if let Some(&d) = gathered.get(&gi) {
            out.insert(k as u32, d);
        } else if inside(node.bbox, st.core) {
            local.entry(o).or_default().push((gi, st.z[k]));
        }
    }
    for (o, cells) in local {
        for (c, d) in flats::resolve(s.world, &cells, w.nodes.at(o).outlet[1]) {
            let (x, y) = s.world.xy(c);
            out.insert(st.core.local(x, y).expect("pond in tile") as u32, d);
        }
    }
    out
}

/// The receiver byte a cell's flow takes (0 where it ends in a node, the sea or a pit).
fn byte_of(w: &Water, st: &TileState, flat: &HashMap<u32, u8>, k: usize) -> u8 {
    match st.wet[k] {
        Wet::Dry => st.dir[k],
        Wet::Pond(o) if w.nodes.at(o).full => flat.get(&(k as u32)).copied().unwrap_or(0),
        _ => 0,
    }
}

/// Solves the listed trees of one tile's fragment from the cells they hold (local leaf, z, x,
/// y; from every tile) and the water poured in (local leaf, amount); returns the `Local` and
/// its nodes. Shared by the in-tile and the L2 cases.
fn solve_trees(
    frag: Frag,
    tops: Tops,
    cells: &[(u32, f32, u64, u64)],
    extra: &[(u32, u64)],
    min_cells: u64,
) -> Result<(Local, Nodes)> {
    let lh = with_spills(&frag, &tops);
    let ltop = top_spill(&lh);
    let mut t = TileCounts::new();
    let mut held = Vec::new();
    for &(l, z, x, y) in cells {
        let h0 = holding(&lh, &ltop, l, z);
        if h0 != NONE {
            count(&mut t, h0, z, x, y);
            held.push((h0, z));
        }
    }
    let mut lbal = solve_local(&frag, &tops, extra, &Counts::dense(lh.len(), &[t]));
    let gathered: Vec<(u32, f32)> = held
        .iter()
        .filter_map(|&(h0, z)| {
            let nd = lbal.node[h0 as usize];
            (nd != NONE && lbal.need[nd as usize] > 0).then_some((nd, z))
        })
        .collect();
    lbal.set_closed_levels(&gathered)?;
    let minpit = min_pits(&lh);
    let nodes = (0..lh.len() as u32)
        .filter(|&d| lbal.node[d as usize] == d)
        .map(|d| node_of(&lh, &lbal, &minpit, d, min_cells))
        .collect();
    Ok((Local { frag, h: lh, top: ltop, bal: lbal, minpit, solved: tops }, Nodes(nodes)))
}

/// A node depression's row in the node table.
fn node_of(h: &Hierarchy, b: &Balance, minpit: &[u64], d: u32, min_cells: u64) -> Node {
    let du = d as usize;
    Node {
        key: minpit[du],
        level: b.level[du],
        low: h.low[du],
        full: b.full[du],
        lake: b.is_lake(h, d, min_cells),
        outlet: h.outlet[du],
        cells: b.area_cells(h, d),
        vfill_mm: b.vfill_mm[du],
        need: b.need[du],
        bbox: b.bbox[du],
    }
}

fn next_of(s: &Setup, w: &Water, st: &TileState, flat: &HashMap<u32, u8>, k: usize) -> Next {
    let (x, y) = st.core.world_xy(k);
    match (byte_of(w, st, flat, k), st.wet[k]) {
        (0, Wet::Lake(o) | Wet::Pond(o)) => Next::Node(w.nodes.at(o).key),
        (0, _) => Next::End,
        (d, _) => {
            let (nx, ny) = s.world.neighbour(x, y, d as usize - 1).expect("receiver inside");
            match st.core.local(nx, ny) {
                Some(j) => Next::Cell(j as u32),
                None => Next::Out(s.world.index(nx, ny)),
            }
        }
    }
}

pub(crate) fn model(s: &Setup, ground: &Layer<f32>, work: Option<&Work>, tag: &str, out: Option<&Rasters>) -> Result<Model> {
    let tiles = s.tiles();
    let world = s.world;
    let read = |core: Rect| -> Result<(Rect, Vec<f32>)> {
        let win = core.grow(1, world);
        Ok((win, ground.read(win)?))
    };

    let missing = |what: &str| AcquisitionError::Invalid(format!("{what} missing from the work directory"));

    // A: label graphs; trees settled in the tile become super-leaves (Task 8b).
    let parts: Vec<(TileGraph, Frag)> = for_tiles(&tiles, s.threads, &format!("{tag} A"), |i, core| {
        let (g, frag) = cached(work, &format!("{tag}/a/{i:05}"), || {
            let (win, zw) = read(core)?;
            let supply = s.supply(core)?;
            let d = drain(world, win, &zw, core);
            let g = tile_graph(world, core, &crop(win, &zw, core), &d, |k| supply[k]);
            Ok(if s.contract { contract(g) } else { (g, Frag::default()) })
        })?;
        // With a work directory fragments are read back when needed; keep memory small.
        Ok((g, if work.is_some() { Frag::default() } else { frag }))
    })?;
    let mut cycles: u64 = parts.iter().map(|p| p.0.cycles).sum();
    let (graphs, frags): (Vec<TileGraph>, Vec<Frag>) = parts.into_iter().unzip();
    let (h, resolver) = cached(work, &format!("{tag}/g1"), || build(world, s.tile, &graphs))?;
    drop(graphs);
    let top = top_spill(&h);
    let frag_at = |i: usize| -> Result<Frag> {
        match work {
            Some(wk) => Ok(wk.cached::<(TileGraph, Frag)>(&format!("{tag}/a/{i:05}"), || Err(missing("pass A")))?.1),
            None => Ok(frags[i].clone()),
        }
    };

    // B: cells by their holding depression in the global hierarchy.
    let counts: Vec<TileCounts> = for_tiles(&tiles, s.threads, &format!("{tag} B"), |i, core| {
        cached(work, &format!("{tag}/b/{i:05}"), || {
            let (win, zw) = read(core)?;
            let d = drain(world, win, &zw, core);
            let local = Local { frag: frag_at(i)?, ..Default::default() };
            let mut t = TileCounts::new();
            for k in 0..core.cells() {
                let (x, y) = core.world_xy(k);
                let z = zw[win.local(x, y).expect("in")];
                let h0 = holding(&h, &top, resolver.leaf_of(local.remap(d.end[k])), z);
                if h0 != NONE {
                    count(&mut t, h0, z, x, y);
                }
            }
            Ok(t)
        })
    })?;
    let mut bal = cached(work, &format!("{tag}/g2"), || Ok(solve(&h, &Counts::dense(h.len(), &counts), true)))?;
    drop(counts);

    // L: a settled tree whose super-leaf holds water without being full keeps that water
    // inside, and its inside must be solved apart (globally it was one leaf). A tile solves
    // such a tree itself only when every cell the tree holds lies in the tile (its count equals
    // the global one) and every pour into it enters there; every other such tree goes to pass
    // L2, which gathers its cells and pours from all tiles (B2). Exact either way.
    let min_cells = s.lake_min_cells();
    let pours_into = |sl: u32| {
        let from = bal.pours.partition_point(|p| p.0 < sl);
        bal.pours[from..].iter().take_while(move |p| p.0 == sl)
    };
    let wanted = |frag: &Frag| -> Tops {
        (0..frag.h.len())
            .filter(|&d| frag.h.parent[d] == NONE && frag.size[d] >= 2)
            .filter_map(|d| {
                let sl = resolver.leaf_index(frag.tree[d]) as usize;
                (bal.node[sl] == sl as u32 && !bal.full[sl]).then_some((frag.tree[d], (h.spill[sl], h.outlet[sl])))
            })
            .collect()
    };
    #[allow(clippy::type_complexity)]
    let part_l: Vec<(Vec<u64>, Vec<u64>, Vec<Node>, Local)> = for_tiles(&tiles, s.threads, &format!("{tag} L"), |i, core| {
        let (local, nodes, remote) = cached(work, &format!("{tag}/l/{i:05}"), || {
            let frag = frag_at(i)?;
            let want = wanted(&frag);
            if want.is_empty() {
                return Ok((Local { frag, ..Default::default() }, Nodes::default(), Vec::new()));
            }
            let (win, zw) = read(core)?;
            let d = drain(world, win, &zw, core);
            let probe = Local { frag: frag.clone(), solved: want.clone(), ..Default::default() };
            let mut cells: BTreeMap<u64, Vec<(u32, f32, u64, u64)>> = BTreeMap::new();
            for k in 0..core.cells() {
                let Some(l) = probe.solved_leaf(d.end[k]) else { continue };
                let key = frag.tree[l as usize];
                let (x, y) = core.world_xy(k);
                let z = zw[win.local(x, y).expect("in")];
                if z <= want[&key].0 {
                    cells.entry(key).or_default().push((l, z, x, y));
                }
            }
            let (mut here, mut remote, mut extra) = (Tops::new(), Vec::new(), Vec::new());
            for (&key, &top_info) in &want {
                let sl = resolver.leaf_index(key);
                let mut inside = cells.get(&key).map_or(0, Vec::len) as u64 == bal.n[sl as usize];
                let mut entries = Vec::new();
                for &(_, cell, amount) in pours_into(sl) {
                    let (cx, cy) = world.xy(cell);
                    match core.local(cx, cy).and_then(|k| probe.leaf_of_end(&resolver, d.end[k])) {
                        Some(l) => entries.push((l, amount)),
                        None => inside = false,
                    }
                }
                if inside {
                    here.insert(key, top_info);
                    extra.extend(entries);
                } else {
                    remote.push(key);
                }
            }
            if here.is_empty() {
                return Ok((Local { frag, ..Default::default() }, Nodes::default(), remote));
            }
            let mine: Vec<(u32, f32, u64, u64)> = here.keys().flat_map(|k| cells.remove(k).unwrap_or_default()).collect();
            let (local, nodes) = solve_trees(frag, here, &mine, &extra, min_cells)?;
            Ok((local, nodes, remote))
        })?;
        let keys = local.solved.keys().copied().collect();
        Ok((keys, remote, nodes.0, if work.is_some() { Local::default() } else { local }))
    })?;

    // L2: the remaining trees, solved from the cells and pours of every tile.
    let mut by_tile: BTreeMap<usize, Tops> = BTreeMap::new();
    for (i, p) in part_l.iter().enumerate() {
        for &key in &p.1 {
            let sl = resolver.leaf_index(key) as usize;
            by_tile.entry(i).or_default().insert(key, (h.spill[sl], h.outlet[sl]));
        }
    }
    let mut remote: Vec<Local> = Vec::new();
    let mut remote_of: BTreeMap<u64, u32> = BTreeMap::new();
    let mut remote_nodes: Vec<Node> = Vec::new();
    if !by_tile.is_empty() {
        let probes: Vec<Local> = by_tile
            .iter()
            .map(|(&i, tops)| Ok(Local { frag: frag_at(i)?, solved: tops.clone(), ..Default::default() }))
            .collect::<Result<_>>()?;
        for (r, p) in probes.iter().enumerate() {
            for &key in p.solved.keys() {
                remote_of.insert(key, r as u32);
            }
        }
        #[allow(clippy::type_complexity)]
        let gathered: Vec<(Vec<(u64, u32, f32, u64, u64)>, Vec<(u64, u32, u64)>)> =
            for_tiles(&tiles, s.threads, &format!("{tag} L2"), |i, core| {
                cached(work, &format!("{tag}/l2/{i:05}"), || {
                    let (win, zw) = read(core)?;
                    let d = drain(world, win, &zw, core);
                    let own = Local { frag: frag_at(i)?, ..Default::default() };
                    let lost = || AcquisitionError::Invalid("a cell of a remote tree does not reach it".into());
                    let mut cells = Vec::new();
                    for k in 0..core.cells() {
                        let sl = resolver.leaf_of(own.remap(d.end[k]));
                        if sl == NONE {
                            continue;
                        }
                        let key = resolver.leaf_pit[sl as usize];
                        let Some(&r) = remote_of.get(&key) else { continue };
                        let (x, y) = core.world_xy(k);
                        let z = zw[win.local(x, y).expect("in")];
                        if z <= h.spill[sl as usize] {
                            let l = probes[r as usize].leaf_of_end(&resolver, d.end[k]).ok_or_else(lost)?;
                            cells.push((key, l, z, x, y));
                        }
                    }
                    let mut pours = Vec::new();
                    for (&key, &r) in &remote_of {
                        for &(_, cell, amount) in pours_into(resolver.leaf_index(key)) {
                            let (cx, cy) = world.xy(cell);
                            if let Some(k) = core.local(cx, cy) {
                                let l = probes[r as usize].leaf_of_end(&resolver, d.end[k]).ok_or_else(lost)?;
                                pours.push((key, l, amount));
                            }
                        }
                    }
                    Ok((cells, pours))
                })
            })?;
        for p in probes {
            let mine: Vec<(u32, f32, u64, u64)> = gathered
                .iter()
                .flat_map(|g| g.0.iter())
                .filter(|c| p.solved.contains_key(&c.0))
                .map(|c| (c.1, c.2, c.3, c.4))
                .collect();
            let extra: Vec<(u32, u64)> =
                gathered.iter().flat_map(|g| g.1.iter()).filter(|c| p.solved.contains_key(&c.0)).map(|c| (c.1, c.2)).collect();
            let (local, nodes) = solve_trees(p.frag, p.solved, &mine, &extra, min_cells)?;
            remote_nodes.extend(nodes.0);
            remote.push(local);
        }
    }

    // A super-leaf solved apart is no global node: its water is its local nodes'.
    let mut solved: Vec<u64> = part_l.iter().flat_map(|p| p.0.iter().chain(&p.1).copied()).collect();
    solved.sort_unstable();
    for &k in &solved {
        let sl = resolver.leaf_index(k) as usize;
        (bal.node[sl], bal.need[sl]) = (NONE, 0);
    }
    let mut table: Vec<Node> = remote_nodes;
    let mut mem_locals: Vec<Local> = Vec::new();
    for (_, _, nodes, local) in part_l {
        table.extend(nodes);
        mem_locals.push(local);
    }
    let local_of = |i: usize| -> Result<Local> {
        match work {
            Some(wk) => Ok(wk.cached::<(Local, Nodes, Vec<u64>)>(&format!("{tag}/l/{i:05}"), || Err(missing("pass L")))?.0),
            None => Ok(mem_locals[i].clone()),
        }
    };

    // C: the ground of closed wet basins of the global hierarchy, for their levels.
    if bal.need.iter().any(|&k| k > 0) {
        let gathered: Vec<Vec<(u32, f32)>> = for_tiles(&tiles, s.threads, &format!("{tag} C"), |i, core| {
            cached(work, &format!("{tag}/c/{i:05}"), || {
                let (win, zw) = read(core)?;
                let d = drain(world, win, &zw, core);
                let local = local_of(i)?;
                let mut v = Vec::new();
                for k in 0..core.cells() {
                    if local.solved_leaf(d.end[k]).is_some() {
                        continue;
                    }
                    let (x, y) = core.world_xy(k);
                    let z = zw[win.local(x, y).expect("in")];
                    let h0 = holding(&h, &top, resolver.leaf_of(local.remap(d.end[k])), z);
                    if h0 != NONE {
                        let nd = bal.node[h0 as usize];
                        if nd != NONE && bal.need[nd as usize] > 0 {
                            v.push((nd, z));
                        }
                    }
                }
                Ok(v)
            })
        })?;
        bal = cached(work, &format!("{tag}/g3"), || {
            let mut b = bal.clone();
            b.set_closed_levels(&gathered.concat())?;
            Ok(b)
        })?;
    }
    // The node table: global nodes and every tile's local ones, by key (R28).
    let minpit = min_pits(&h);
    table.extend((0..h.len() as u32).filter(|&d| bal.node[d as usize] == d).map(|d| node_of(&h, &bal, &minpit, d, min_cells)));
    table.sort_unstable_by_key(|n| n.key);
    let water = Water { h, resolver, top, bal, minpit, nodes: Nodes(table), solved, remote, remote_of };
    let w = &water;

    // D: full ponds that cross tiles, solved once.
    let spanning: Vec<Vec<(u32, u64, f32)>> = for_tiles(&tiles, s.threads, &format!("{tag} D"), |i, core| {
        cached(work, &format!("{tag}/d/{i:05}"), || {
            let (win, zw) = read(core)?;
            let st = state(s, w, &local_of(i)?, core, win, &zw);
            Ok((0..core.cells())
                .filter_map(|k| match st.wet[k] {
                    Wet::Pond(o) if w.nodes.at(o).full && !inside(w.nodes.at(o).bbox, core) => {
                        let (x, y) = core.world_xy(k);
                        Some((o, world.index(x, y), st.z[k]))
                    }
                    _ => None,
                })
                .collect())
        })
    })?;
    let ponds: Vec<(u64, u8)> = cached(work, &format!("{tag}/gp"), || {
        let mut by: BTreeMap<u32, Vec<(u64, f32)>> = BTreeMap::new();
        for (o, c, z) in spanning.into_iter().flatten() {
            by.entry(o).or_default().push((c, z));
        }
        Ok(by
            .into_iter()
            .flat_map(|(o, mut cells)| {
                cells.sort_unstable_by_key(|c| c.0);
                flats::resolve(world, &cells, w.nodes.at(o).outlet[1])
            })
            .collect())
    })?;
    let ponds: HashMap<u64, u8> = ponds.into_iter().collect();

    // E: flow reports; full nodes pass their water on at their outlet.
    let outlets: BTreeMap<u64, Option<u64>> =
        w.nodes.0.iter().map(|n| (n.key, n.full.then_some(n.outlet[1]))).collect();
    let outlet_cells: HashSet<u64> = outlets.values().flatten().copied().collect();
    let reports: Vec<(Report, u64)> = for_tiles(&tiles, s.threads, &format!("{tag} E"), |i, core| {
        cached(work, &format!("{tag}/e/{i:05}"), || {
            let (win, zw) = read(core)?;
            let st = state(s, w, &local_of(i)?, core, win, &zw);
            let flat = flats_for(s, w, &st, &ponds);
            let rain = s.rain_units(core);
            let next = |k: usize| next_of(s, w, &st, &flat, k);
            let local = accumulate(core.cells(), next, |k| rain[k]);
            let (end, looped) = ends(core.cells(), next);
            let mut port_end = BTreeMap::new();
            for (k, &e) in end.iter().enumerate() {
                let (x, y) = core.world_xy(k);
                let gi = world.index(x, y);
                if core.on_ring(x, y) || outlet_cells.contains(&gi) {
                    port_end.insert(gi, e);
                }
            }
            Ok((Report { out: local.out, node_in: local.node_in, port_end }, local.cycles + looped))
        })
    })?;
    cycles += reports.iter().map(|r| r.1).sum::<u64>();
    let flow = cached(work, &format!("{tag}/g4"), || {
        solve_flow(&reports.iter().map(|r| r.0.clone()).collect::<Vec<_>>(), &outlets)
    })?;
    drop(reports);

    // F: the rasters.
    if let Some(r) = out {
        let thr = s.river_km2();
        for_tiles(&tiles, s.threads, &format!("{tag} F"), |i, core| {
            let name = format!("{tag}/f/{i:05}");
            if work.is_some_and(|wk| wk.is_done(&name)) {
                return Ok(());
            }
            let (win, zw) = read(core)?;
            let st = state(s, w, &local_of(i)?, core, win, &zw);
            let flat = flats_for(s, w, &st, &ponds);
            let rain = s.rain_units(core);
            let next = |k: usize| next_of(s, w, &st, &flat, k);
            let mut q = accumulate(core.cells(), next, |k| rain[k]).q;
            let ports: Vec<(usize, u64)> = (0..core.cells())
                .filter_map(|k| {
                    let (x, y) = core.world_xy(k);
                    flow.inflow.get(&world.index(x, y)).map(|&a| (k, a))
                })
                .collect();
            add_inflows(&mut q, &ports, next);
            let n = core.cells();
            let (mut node, mut disch, mut fl) = (vec![0u32; n], vec![0f32; n], vec![0u8; n]);
            for k in 0..n {
                let qk = match next(k) {
                    Next::Node(nd) => flow.node_total.get(&nd).copied().unwrap_or(q[k]),
                    _ => q[k],
                };
                disch[k] = (qk as f64 / Q_PER_KM2) as f32;
                node[k] = match st.wet[k] {
                    Wet::Dry => 0,
                    Wet::Sea => 1,
                    Wet::Lake(o) | Wet::Pond(o) => o + 2,
                };
                let b = byte_of(w, &st, &flat, k);
                let mut f = b;
                if b != 0 && disch[k] >= thr {
                    f |= 16;
                }
                match st.wet[k] {
                    Wet::Dry if st.held[k] => f |= 32 | 64,
                    Wet::Lake(o) | Wet::Pond(o) if !w.nodes.at(o).full => f |= 64,
                    _ => {}
                }
                fl[k] = f;
            }
            r.level.write(core, &st.level)?;
            r.node.write(core, &node)?;
            r.discharge.write(core, &disch)?;
            r.flow.write(core, &fl)?;
            match work {
                Some(wk) => wk.mark_done(&name),
                None => Ok(()),
            }
        })?;
    }
    Ok(Model { water, ponds, flow, cycles, locals: mem_locals })
}
```

Add to `mod.rs`: `mod pipeline;` and `#[cfg(test)] mod testing;`.

Note: results are cached as tuples, `(Hierarchy, Resolver)`, `(TileGraph, Frag)` and `(Local, Nodes)`; the tuple impl from Task 2 covers them. `Vec<(u32, u64, f32)>`, `Vec<(u64, u8)>` and `(Report, u64)` are covered by the generic impls. Pass L runs on every tile but reads ground only where a settled tree holds water without being full, which is rare.

- [ ] **Step 5: Run the tests to see them pass**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::pipeline`
Expected: 2 pass, 1 ignored. If the semantic test fails on the synthetic features (not on exactness), print `level`/`node` around the named cells and adjust the terrain function so the feature exists; never relax the exactness test. The exactness test covers contraction on and off (review C3), and `settled_trees_solved_in_their_tile_or_across_seams_match_the_whole` makes pass L and pass L2 run on an arid closed basin (B2). If either fails only with contraction on, the fault is in Task 8b's frontier or in passes L/L2, not in the test. If the basin case does not reach the pass it names (the `remote`/`solved` asserts), adjust `basin` (ridge, notch, r) until it does; never weaken the equality asserts.
Run clippy. Expected: clean.
Run the probe: `"${CAP6[@]}" cargo test --locked --offline -p topography-core world::water::pipeline::tests::probe_model_at_the_dam_spot -- --ignored --nocapture`
Expected: counts printed, `scratch/hydrology-v3/probes/t9-dam-spot-water.png` written, cycles 0.

- [ ] **Step 6: Commit**

Run the task-finish block first.

```bash
git add crates/topography-core/src/world/water
git commit -m "feat(water): the tiled model run (hierarchy, balance, ponds, exact discharge) with bit-identical tilings

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 7: Expert review**

Expert review protocol. Real-terrain output: the probe printout and `t9-dam-spot-water.png` (E25645 N24400, 1,024 m, uniform rain). Ask: do the lakes sit where the raw surface holds water, do rivers follow the valley floors, and is the dam's box from the raw surface visible as expected (it stays at gate 2).

---
## Task 10: River main stems, body ids and `bodies.json`

**Files:**
- Create: `crates/topography-core/src/world/water/bodies.rs`
- Modify: `crates/topography-core/src/world/water/mod.rs` (`mod bodies;`)

**Interfaces:**
- Consumes: `pipeline::{Setup, Water, Rasters}`, `flow::{Next, ends}`, `store::{Work, cached, for_tiles}`, `geom::{OFFSETS, Rect, World}`.
- Produces:
  - `bodies::main_up(world, win: Rect, flow: &[u8], q: &[f32], x, y) -> Option<u64>`: the thalweg neighbour draining into (x, y) with the largest discharge (ties: smaller world index).
  - `bodies::label(s: &Setup, w: &Water, r: &Rasters, body: &Layer<u32>, pools: &BTreeMap<u64, u64>, work: Option<&Work>, tag: &str) -> Result<Value>`: writes `body` (0 none, 1 sea, lake ids, river ids on thalweg cells, and on the cells of the pools in `pools` (node key → outlet cell, R26) the id of the river leaving them) and returns the `bodies.json` value. Lake ids follow node keys (R28), so they are the same at any tiling. `bodies::pools` (which nodes are river pools) comes in Task 11, which has the hydraulic geometry; Task 10's tests pass an empty map.
  - JSON shape (v2 compatible, `kind: "water_bodies"`, `schema_version: 1`): sea `{id: 1, kind: "sea", level_m: 0, parent: 0, area_km2}`; lakes `{id, kind: "lake", node, level_m, area_km2, max_depth_m, exit_cell: [col, row] | null, endorheic, parent}` (`node` = the node key, read by `water-finish`; export ignores it); rivers `{id, kind: "river", length_km, drainage_km2, mouth_cell: [col, row], parent}`; dry basins `{id, kind: "dry", level_m, area_km2, endorheic: true, parent: 0}` (area ≥ 0.01 km², not in the raster).

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::geom::World;
    use crate::world::water::layers::Layer;
    use crate::world::water::pipeline::{Rasters, model};
    use crate::world::water::testing::{setup, terrain};

    fn run(tile: u64) -> (Vec<u32>, Value, Vec<u8>) {
        let world = World { w: 64, h: 64 };
        let s = setup(world, tile, 1e-5);
        let r = Rasters::memory(world);
        let m = model(&s, &Layer::memory(world, terrain(world)), None, "m", Some(&r)).expect("model");
        let body = Layer::filled(world, 0u32);
        let json = label(&s, &m.water, &r, &body, &BTreeMap::new(), None, "m").expect("bodies");
        (body.to_vec().unwrap(), json, r.flow.to_vec().unwrap())
    }

    #[test]
    fn tiles_label_alike() {
        let (body, json, _) = run(64);
        let (b16, j16, _) = run(16);
        assert_eq!(b16, body);
        assert_eq!(j16, json);
    }

    #[test]
    fn every_river_has_one_mouth_and_known_parents() {
        let (body, json, flow) = run(16);
        let list = json["bodies"].as_array().expect("bodies");
        let ids: std::collections::HashSet<u64> = list.iter().map(|b| b["id"].as_u64().unwrap()).collect();
        let rivers: Vec<&Value> = list.iter().filter(|b| b["kind"] == "river").collect();
        assert!(!rivers.is_empty());
        for r in &rivers {
            let p = r["parent"].as_u64().unwrap();
            assert!(p == 0 || ids.contains(&p), "parent {p}");
        }
        // Mouths: thalweg cells whose receiver is not a thalweg cell of the same body.
        let mut mouths = std::collections::HashMap::<u32, u32>::new();
        for k in 0..64 * 64 {
            if flow[k] & 16 == 0 {
                continue;
            }
            let (dy, dx) = OFFSETS[(flow[k] & 15) as usize - 1];
            let j = ((k / 64) as i64 + dy) as usize * 64 + ((k % 64) as i64 + dx) as usize;
            if flow[j] & 16 == 0 || body[j] != body[k] {
                *mouths.entry(body[k]).or_insert(0) += 1;
            }
        }
        assert!(mouths.values().all(|&m| m == 1), "{mouths:?}");
        assert_eq!(mouths.len(), rivers.len());
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::bodies`
Expected: compile errors.

- [ ] **Step 3: Implement `bodies.rs`**

```rust
//! Water bodies: the sea (id 1), lakes (labelled nodes by key, R28; river pools take their
//! river's id, R26), and rivers as
//! main stems: at a confluence the branch with the larger discharge keeps its body (ties: the
//! smaller cell index), so a river body runs from its head to its mouth. Writes the ids of the
//! sea, lakes and thalweg cells and returns the body list (v2's `bodies.json` fields).

use super::flow::{Next, ends};
use super::geom::{OFFSETS, Rect, World};
use super::hierarchy::NONE;
use super::layers::Layer;
use super::pipeline::{Rasters, Setup, Water};
use super::store::{Work, cached, for_tiles};
use crate::acquisition::Result;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

/// Dry basins listed from this area (as v2's world lists).
const DRY_MIN_KM2: f64 = 0.01;

/// The thalweg neighbour draining into (x, y) with the most discharge.
pub(crate) fn main_up(world: World, win: Rect, flow: &[u8], q: &[f32], x: u64, y: u64) -> Option<u64> {
    let mut best: Option<(f32, u64)> = None;
    for k in 0..8 {
        let Some((nx, ny)) = world.neighbour(x, y, k) else { continue };
        let Some(j) = win.local(nx, ny) else { continue };
        let (f, d) = (flow[j], flow[j] & 15);
        if f & 16 == 0 || d == 0 {
            continue;
        }
        let (dy, dx) = OFFSETS[d as usize - 1];
        if (nx as i64 + dx, ny as i64 + dy) != (x as i64, y as i64) {
            continue;
        }
        let c = (q[j], world.index(nx, ny));
        if best.is_none_or(|b| c.0 > b.0 || (c.0 == b.0 && c.1 < b.1)) {
            best = Some(c);
        }
    }
    best.map(|b| b.1)
}

/// A tile's river attributes: by river id, axial and diagonal steps, and the mouth (cell,
/// discharge, parent id).
type RiverAttrs = BTreeMap<u32, (u64, u64, Vec<(u64, f32, u32)>)>;
/// By lake id: cells, and the parent seen at its outlet.
type LakeAttrs = BTreeMap<u32, (u64, Vec<u32>)>;

#[allow(clippy::too_many_arguments)]
pub(crate) fn label(
    s: &Setup,
    w: &Water,
    r: &Rasters,
    body: &Layer<u32>,
    pools: &BTreeMap<u64, u64>,
    work: Option<&Work>,
    tag: &str,
) -> Result<Value> {
    let world = s.world;
    let tiles = s.tiles();
    // Pass G: each thalweg cell's chain up its main stem, to a head or out of the tile; and
    // the chain end of each river pool's outlet cell (the river that leaves the pool).
    #[allow(clippy::type_complexity)]
    let chased: Vec<(Vec<u64>, Vec<(u64, Next)>, Vec<(u64, Next)>)> = for_tiles(&tiles, s.threads, &format!("{tag} G"), |i, core| {
        cached(work, &format!("{tag}/bg/{i:05}"), || {
            let win = core.grow(2, world);
            let (flow, q) = (r.flow.read(win)?, r.discharge.read(win)?);
            let thal = |k: usize| {
                let (x, y) = core.world_xy(k);
                flow[win.local(x, y).expect("in")] & 16 != 0
            };
            let next = |k: usize| {
                if !thal(k) {
                    return Next::End;
                }
                let (x, y) = core.world_xy(k);
                match main_up(world, win, &flow, &q, x, y) {
                    Some(u) => {
                        let (ux, uy) = world.xy(u);
                        core.local(ux, uy).map_or(Next::Out(u), |j| Next::Cell(j as u32))
                    }
                    None => Next::Node(world.index(x, y)),
                }
            };
            let (end, _) = ends(core.cells(), next);
            let (mut heads, mut ring) = (Vec::new(), Vec::new());
            for (k, &e) in end.iter().enumerate() {
                let (x, y) = core.world_xy(k);
                if next(k) == Next::Node(world.index(x, y)) {
                    heads.push(world.index(x, y));
                }
                if thal(k) && core.on_ring(x, y) {
                    ring.push((world.index(x, y), e));
                }
            }
            let pool_ends = pools
                .iter()
                .filter_map(|(&key, &b)| {
                    let (bx, by) = world.xy(b);
                    let k = core.local(bx, by)?;
                    thal(k).then_some((key, end[k]))
                })
                .collect();
            Ok((heads, ring, pool_ends))
        })
    })?;
    let ring: HashMap<u64, Next> = chased.iter().flat_map(|c| c.1.iter().copied()).collect();
    let head_of = |mut t: Next| loop {
        match t {
            Next::Out(c) => t = *ring.get(&c).expect("a main stem enters a ring cell"),
            Next::Node(h) => return h,
            _ => return u64::MAX,
        }
    };
    // Ids: sea 1, lakes by node key (pools are not lakes), rivers by head cell, then dry basins.
    let mut lake_id: BTreeMap<u32, u32> = BTreeMap::new();
    let mut next_id = 2u32;
    for (o, n) in w.nodes.0.iter().enumerate() {
        if n.lake && !pools.contains_key(&n.key) {
            lake_id.insert(o as u32, next_id);
            next_id += 1;
        }
    }
    let mut heads: Vec<u64> = chased.iter().flat_map(|c| c.0.iter().copied()).collect();
    heads.sort_unstable();
    let head_id: HashMap<u64, u32> = heads.iter().enumerate().map(|(i, &h)| (h, next_id + i as u32)).collect();
    next_id += heads.len() as u32;
    // A pool takes the id of the river leaving it.
    let pool_id: HashMap<u64, u32> = chased
        .iter()
        .flat_map(|c| c.2.iter().map(|&(key, e)| (key, head_id.get(&head_of(e)).copied().unwrap_or(0))))
        .collect();
    drop(chased);

    // Pass H: write ids and gather attributes.
    let outlet_of: HashMap<u64, u32> = lake_id
        .keys()
        .filter(|&&o| w.nodes.at(o).full)
        .map(|&o| (w.nodes.at(o).outlet[1], o))
        .collect();
    let parts: Vec<(RiverAttrs, LakeAttrs, u64)> = for_tiles(&tiles, s.threads, &format!("{tag} H"), |_, core| {
        let win = core.grow(2, world);
        let (flow, q, node) = (r.flow.read(win)?, r.discharge.read(win)?, r.node.read(win)?);
        let at = |x: u64, y: u64| win.local(x, y).expect("in window");
        let thal = |j: usize| flow[j] & 16 != 0;
        // A thalweg cell's river id: its main stem's head, chased in this tile or across seams.
        let river_of = |x: u64, y: u64| -> u32 {
            let mut c = world.index(x, y);
            for _ in 0..core.cells() + 1 {
                let (cx, cy) = world.xy(c);
                if !core.contains(cx, cy) {
                    return head_id.get(&head_of(Next::Out(c))).copied().unwrap_or(0);
                }
                match main_up(world, win, &flow, &q, cx, cy) {
                    Some(u) => c = u,
                    None => return head_id.get(&c).copied().unwrap_or(0),
                }
            }
            0
        };
        let body_at = |x: u64, y: u64| -> u32 {
            let j = at(x, y);
            match node[j] {
                1 => 1,
                n if n >= 2 => match pool_id.get(&w.nodes.at(n - 2).key) {
                    Some(&river) => river,
                    None => lake_id.get(&(n - 2)).copied().unwrap_or(if thal(j) { river_of(x, y) } else { 0 }),
                },
                _ if thal(j) => river_of(x, y),
                _ => 0,
            }
        };
        let (mut rivers, mut lakes, mut sea) = (RiverAttrs::new(), LakeAttrs::new(), 0u64);
        let mut ids = vec![0u32; core.cells()];
        for (k, id) in ids.iter_mut().enumerate() {
            let (x, y) = core.world_xy(k);
            let j = at(x, y);
            *id = body_at(x, y);
            if node[j] == 1 {
                sea += 1;
            } else if node[j] >= 2 && lake_id.contains_key(&(node[j] - 2)) {
                lakes.entry(*id).or_default().0 += 1;
            }
            if let Some(&o) = outlet_of.get(&world.index(x, y)) {
                lakes.entry(lake_id[&o]).or_default().1.push(body_at(x, y));
            }
            if !thal(j) || *id == 0 {
                continue;
            }
            let dir = flow[j] & 15;
            let e = rivers.entry(*id).or_default();
            if dir > 4 { e.1 += 1 } else { e.0 += 1 }
            let (rx, ry) = world.neighbour(x, y, dir as usize - 1).expect("receiver inside");
            let rj = at(rx, ry);
            let continues = thal(rj) && main_up(world, win, &flow, &q, rx, ry) == Some(world.index(x, y));
            if !continues {
                e.2.push((world.index(x, y), q[j], body_at(rx, ry)));
            }
        }
        body.write(core, &ids)?;
        Ok((rivers, lakes, sea))
    })?;

    // Merge.
    let km2 = s.cell_km2();
    let (mut rivers, mut lakes, mut sea) = (RiverAttrs::new(), LakeAttrs::new(), 0u64);
    for (rv, lk, se) in parts {
        sea += se;
        for (id, (a, d, m)) in rv {
            let e = rivers.entry(id).or_default();
            (e.0, e.1) = (e.0 + a, e.1 + d);
            e.2.extend(m);
        }
        for (id, (n, p)) in lk {
            let e = lakes.entry(id).or_default();
            e.0 += n;
            e.1.extend(p);
        }
    }
    let cell = |c: u64| {
        let (x, y) = world.xy(c);
        json!([x, y])
    };
    let mut list = vec![json!({"id": 1, "kind": "sea", "level_m": 0.0, "parent": 0, "area_km2": sea as f64 * km2})];
    for (&o, &id) in &lake_id {
        let n = w.nodes.at(o);
        let (cells, parents) = lakes.get(&id).cloned().unwrap_or_default();
        list.push(json!({
            "id": id, "kind": "lake", "node": n.key, "level_m": n.level, "area_km2": cells as f64 * km2,
            "max_depth_m": n.level - n.low,
            "exit_cell": if n.full { cell(n.outlet[1]) } else { Value::Null },
            "endorheic": !n.full, "parent": parents.first().copied().unwrap_or(0),
        }));
    }
    for h in &heads {
        let id = head_id[h];
        let (axial, diagonal, mouths) = rivers.get(&id).cloned().unwrap_or_default();
        let (mouth, q, parent) = mouths.first().copied().unwrap_or((0, 0.0, 0));
        list.push(json!({
            "id": id, "kind": "river",
            "length_km": (axial as f64 + diagonal as f64 * std::f64::consts::SQRT_2) * s.spacing_m / 1000.0,
            "drainage_km2": q, "mouth_cell": cell(mouth), "parent": parent,
        }));
    }
    // Dry basins: top-level depressions without water (not super-leaves solved in a tile),
    // by node key.
    let mut dry: Vec<(u64, usize)> = (0..w.h.len())
        .filter(|&d| {
            w.h.parent[d] == NONE
                && w.bal.node[d] == NONE
                && !w.bal.sea[d]
                && w.bal.n[d] as f64 * km2 >= DRY_MIN_KM2
                && w.solved.binary_search(&w.minpit[d]).is_err()
        })
        .map(|d| (w.minpit[d], d))
        .collect();
    dry.sort_unstable();
    for (_, d) in dry {
        list.push(json!({
            "id": next_id, "kind": "dry", "level_m": w.h.spill[d], "area_km2": w.bal.n[d] as f64 * km2,
            "endorheic": true, "parent": 0,
        }));
        next_id += 1;
    }
    Ok(json!({
        "schema_version": 1, "kind": "water_bodies",
        "note": "hydrology v3: lakes by node (smallest pit), rivers as main stems (larger discharge keeps the body), river pools labelled as their river",
        "dry_min_km2": DRY_MIN_KM2, "bodies": list,
    }))
}
```

Add `mod bodies;`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::bodies`
Expected: 2 pass. Clippy clean.

- [ ] **Step 5: Commit**

Run the task-finish block first.

```bash
git add crates/topography-core/src/world/water
git commit -m "feat(water): main-stem river bodies, body ids and the v2-shaped body list

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Expert review**

Expert review protocol. Real-terrain output: extend the Task 9 probe (copy it as `probe_bodies_in_the_canyon` in `bodies.rs`, window E27169 N44546, 2,048 m, river start 0.05 km²) to call `label` and write `t10-canyon-bodies.png` with river cells tinted; print the number of river bodies, their total length, and any river body pair whose cells lie within 20 m of each other for over 100 m. The canyon's double river is the defect to look for.

---

## Task 11: Hydraulic geometry and breach-or-fill gorges

**Files:**
- Create: `crates/topography-core/src/world/water/hydraulic.rs`
- Create: `crates/topography-core/src/world/water/breach.rs`
- Modify: `crates/topography-core/src/world/water/mod.rs` (`mod hydraulic; mod breach;`)
- Modify: `crates/topography-core/src/world/water/bodies.rs` (add `pools`)
- Modify: `crates/topography-core/src/world/water/store.rs` (`blob_num!(f64)`)

**Interfaces:**
- Consumes: `pipeline::{Setup, Model}`, `nodes::Nodes`, `receivers::receivers`, `balance::mm`, `store`, `pipeline::crop`, `Params::{breach_cap_m, breach_radius}`, `GORGE_RATIO`, `CHANNEL_CAP_M`, `Table::m3s_per_km2`.
- Produces:
  - `hydraulic::Geometry { m3s_per_km2: f64 }` with `new(m3s_per_km2: f64)` (the table's game setting, R5: 0.56), `width_m(q_km2: f64) -> f64` (3·Q^0.5), `depth_m(q_km2: f64) -> f32` (0.3·Q^0.4, at most 2 m).
  - `bodies::pools(s: &Setup, m: &Model, geo: &Geometry) -> BTreeMap<u64, u64>`: river pools (R26), node key → outlet cell: full lake nodes with inflow ≥ the river start and area below 20 · w(Q)².
  - `breach::Candidate { key: u64, inside: u64, outside: u64, vfill_mm: i64, half: u64, cap: f32, radius: u64 }` (`Copy`), `breach::candidates(s: &Setup, m: &Model, geo: &Geometry) -> Vec<Candidate>`. The cap is `breach_cap_m` (2 m at 1 m) for every river (review I1).
  - `breach::Gorge { key: u64, cut: Vec<(u64, f32)>, depth: f32, volume_mm: i64, length_m: f64 }`, `breach::gorge(world, win, zw: &[f32], c: &Candidate, spacing_m: f64) -> Option<Gorge>`. The cross-section is a Euclidean disc of radius `half + 1` cells around each path cell with a parabolic bed: the line at the centre, rising to the ground at the edge (review M1).
  - `breach::breach(s, m, ground: &Layer<f32>, out: &Layer<f32>, geo: &Geometry, work: Option<&Work>, tag: &str) -> Result<Value>` (writes the breached ground, returns the breach figures: count, deepest cut, total volume in m³, the 20 deepest with their cells).

- [ ] **Step 1: Write the failing tests**

`hydraulic.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_game_factor_gives_the_widths_of_ruling_r5() {
        let g = Geometry::new(0.56);
        assert!((g.width_m(177.0) - 29.9).abs() < 0.2, "the largest river is about 30 m wide");
        assert!((g.depth_m(177.0) - 1.89).abs() < 0.02, "and stays under the 2 m cap");
        assert!((g.width_m(100.0) - 22.4).abs() < 0.2, "the canyon trunk");
        assert!((g.width_m(1.0) - 2.24).abs() < 0.02);
        assert!((g.width_m(0.05) - 0.50).abs() < 0.01);
        assert!((g.depth_m(0.05) - 0.072).abs() < 0.003);
        assert_eq!(Geometry::new(10.0).depth_m(177.0), 2.0, "capped");
    }
}
```

`breach.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::geom::World;
    use crate::world::water::layers::Layer;
    use crate::world::water::pipeline::{Rasters, model};
    use crate::world::water::testing::{setup, terrain};

    /// A V valley falling east 5 cm a cell, dammed at x = 40 by a one-cell wall `dam` m high.
    /// A V valley `w` cells long falling east 5 cm a cell, walled at its west end (so the pond
    /// can only leave over the dam, N8) and dammed at `at` by a one-cell wall `dam` m high.
    fn dammed_at(w: u64, at: f32, dam: f32) -> (World, Vec<f32>) {
        let world = World { w, h: 21 };
        let z = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                let v = 10.0 - 0.05 * x + 0.5 * (y - 10.0).abs() + if x <= 1.0 { 30.0 } else { 0.0 };
                if x == at && (y - 10.0).abs() <= 6.0 { 10.0 - 0.05 * (at - 1.0) + dam } else { v }
            })
            .collect();
        (world, z)
    }

    fn dammed(dam: f32) -> (World, Vec<f32>) {
        dammed_at(100, 40.0, dam)
    }

    fn the_candidate(world: World, z: &[f32], cap: f32) -> (Candidate, Rect) {
        let mut s = setup(world, 128, 1e-5);
        s.classes = None;
        s.params.breach_cap_m = cap;
        let m = model(&s, &Layer::memory(world, z.to_vec()), None, "m", None).expect("model");
        let geo = Geometry::new(2.0);
        let c = candidates(&s, &m, &geo).into_iter().max_by_key(|c| c.vfill_mm).expect("the pond behind the dam");
        (c, Rect::whole(world))
    }

    #[test]
    fn a_narrow_dam_is_cut_as_a_graded_gorge() {
        let (world, z) = dammed(1.0);
        let (c, all) = the_candidate(world, &z, 2.0);
        let g = gorge(world, all, &z, &c, 1.0).expect("breached");
        assert!(g.depth > 0.9 && g.depth <= 2.0, "depth {}", g.depth);
        assert!(g.length_m >= f64::from(GORGE_RATIO * g.depth), "length {} for depth {}", g.length_m, g.depth);
        // At least 3 neighbouring dam cells are cut (the gorge is 3 cells wide, not a slot).
        let cut_dam: Vec<u64> = g.cut.iter().filter(|&&(c, v)| c % 100 == 40 && v < z[c as usize] - 0.5).map(|p| p.0 / 100).collect();
        assert!(cut_dam.len() >= 3, "{cut_dam:?}");
        assert!(cut_dam.windows(2).all(|p| p[1] == p[0] + 1), "adjacent rows: {cut_dam:?}");
    }

    #[test]
    fn a_dam_above_the_cap_or_worth_less_than_its_cut_stays() {
        let (world, z) = dammed(2.5);
        let (c, all) = the_candidate(world, &z, 2.0);
        // The pond leaves over the dam, not off the map: the cap is what refuses it.
        let (ix, _) = world.xy(c.inside);
        let (ox, _) = world.xy(c.outside);
        assert!((39..=41).contains(&ix) && (39..=41).contains(&ox), "outlet at the dam: {ix} {ox}");
        assert!(gorge(world, all, &z, &c, 1.0).is_none(), "a 2.55 m cut, 2 m cap");
        let mut wide = c;
        wide.cap = 3.0;
        assert!(gorge(world, all, &z, &wide, 1.0).is_some(), "the same dam is cut under a 3 m cap");
        let (world, z) = dammed(1.0);
        let (mut c, all) = the_candidate(world, &z, 2.0);
        c.vfill_mm = 1;
        assert!(gorge(world, all, &z, &c, 1.0).is_none(), "the cut would remove more than it drains");
    }

    #[test]
    fn breaching_is_alike_at_every_tiling() {
        // A dam on the tile seam at x = 64; a breach radius of 24 keeps the windows (tile + 27)
        // smaller than the world, so the gorge is cut across tiles (N9).
        let (world, z) = dammed_at(128, 64.0, 1.0);
        let ground = Layer::memory(world, z.clone());
        let mut out = Vec::new();
        for tile in [128, 32] {
            let mut s = setup(world, tile, 1e-5);
            s.classes = None;
            s.params.breach_radius = 24;
            let m = model(&s, &ground, None, "m", Some(&Rasters::memory(world))).expect("model");
            let breached = Layer::filled(world, 0.0f32);
            let figures = breach(&s, &m, &ground, &breached, &Geometry::new(2.0), None, "b").expect("breach");
            assert_eq!(figures["breaches"], 1, "tile {tile}");
            out.push(breached.to_vec().unwrap());
        }
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&out[0]), bits(&out[1]));
        let cut: Vec<u64> = (0..z.len()).filter(|&k| out[1][k] < z[k]).map(|k| k as u64 % 128).collect();
        assert!(cut.iter().any(|&x| x < 64) && cut.iter().any(|&x| x >= 64), "the gorge crosses the seam");
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::hydraulic world::water::breach`
(Run the two filters as two commands if cargo rejects two filters.)
Expected: compile errors.

- [ ] **Step 3: Implement `hydraulic.rs`**

```rust
//! Hydraulic geometry: channel width and depth from discharge, with the spec's starting values
//! w = 3·Q^0.5 and d = 0.3·Q^0.4 (Q in m³/s), depth capped at 2 m.

use super::CHANNEL_CAP_M;

pub(crate) struct Geometry {
    /// m³/s per km² at mean rain: a game setting (`m3s_per_km2` in water-v2.toml, R5), tuned
    /// so the largest river is about 30 m wide.
    pub m3s_per_km2: f64,
}

impl Geometry {
    pub fn new(m3s_per_km2: f64) -> Self {
        Self { m3s_per_km2 }
    }
    pub fn width_m(&self, q_km2: f64) -> f64 {
        3.0 * (q_km2 * self.m3s_per_km2).sqrt()
    }
    pub fn depth_m(&self, q_km2: f64) -> f32 {
        ((0.3 * (q_km2 * self.m3s_per_km2).powf(0.4)) as f32).min(CHANNEL_CAP_M)
    }
}
```

- [ ] **Step 4: Implement `breach.rs`**

```rust
//! Breach or fill (Lindsay 2016's hybrid). A full depression is cut through when a graded
//! gorge drains it for less volume than it would hold: the gorge runs from the depression's
//! pit up its receivers to the spill and down the outlet's receivers to the first cell below
//! the pit; its bed falls linearly over at least GORGE_RATIO × its deepest cut, it is at least
//! 3 cells wide, and its deepest cut stays within the level's breach cap (at 1 m the 2 m
//! channel cap for every river, R9; the channels' raw − 2 m clamp keeps breach plus channel
//! within it; and only depressions a river flows into). Otherwise the depression keeps its
//! water. Gorges lie within `Params::breach_radius` cells of their spill, so every tiling
//! decides alike.

use super::GORGE_RATIO;
use super::balance::mm;
use super::geom::{Rect, World, step_len};
use super::hydraulic::Geometry;
use super::layers::Layer;
use super::pipeline::{Model, Setup, crop};
use super::rain::Q_PER_KM2;
use super::receivers::receivers;
use super::store::{Work, for_tiles};
use crate::acquisition::Result;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Candidate {
    /// The node's key (R28).
    pub key: u64,
    pub inside: u64,
    pub outside: u64,
    pub vfill_mm: i64,
    /// Gorge half-width in cells (at least 1: 3 cells wide).
    pub half: u64,
    pub cap: f32,
    /// Gorge cells lie within this many cells of `inside`.
    pub radius: u64,
}

/// Full nodes (never sea: sea is no node); at 1 m only those a river flows into. The cap is
/// the level's breach cap for every river (I1): at 1 m the channels' raw − 2 m clamp keeps
/// breach plus channel within 2 m.
pub(crate) fn candidates(s: &Setup, m: &Model, geo: &Geometry) -> Vec<Candidate> {
    let thr = (s.params.river_km2 * Q_PER_KM2) as u64;
    m.water
        .nodes
        .0
        .iter()
        .filter(|n| n.full)
        .filter_map(|n| {
            let inflow = m.flow.node_total.get(&n.key).copied().unwrap_or(0);
            let half = if s.params.channels {
                if inflow < thr {
                    return None;
                }
                (geo.width_m(inflow as f64 / Q_PER_KM2) / 2.0 / s.spacing_m).ceil().max(1.0) as u64
            } else {
                1
            };
            Some(Candidate {
                key: n.key,
                inside: n.outlet[0],
                outside: n.outlet[1],
                vfill_mm: n.vfill_mm,
                half,
                cap: s.params.breach_cap_m,
                radius: s.params.breach_radius,
            })
        })
        .collect()
}

#[derive(Clone, Debug)]
pub(crate) struct Gorge {
    pub key: u64,
    pub cut: Vec<(u64, f32)>,
    pub depth: f32,
    pub volume_mm: i64,
    pub length_m: f64,
}

pub(crate) fn gorge(world: World, win: Rect, zw: &[f32], c: &Candidate, spacing_m: f64) -> Option<Gorge> {
    let (ix, iy) = world.xy(c.inside);
    // Within the radius, with the 3 x 3 block receivers need inside the window.
    let near = |cell: u64| {
        let (x, y) = world.xy(cell);
        x.abs_diff(ix).max(y.abs_diff(iy)) <= c.radius
            && win.contains(x.saturating_sub(1), y.saturating_sub(1))
            && win.contains((x + 1).min(world.w - 1), (y + 1).min(world.h - 1))
    };
    let z = |cell: u64| {
        let (x, y) = world.xy(cell);
        zw[win.local(x, y).expect("near cells are in the window")]
    };
    let receiver = |cell: u64| -> Option<u64> {
        let (x, y) = world.xy(cell);
        let d = receivers(world, win, zw, Rect { x0: x, y0: y, w: 1, h: 1 })[0];
        (d != 0).then(|| {
            let (nx, ny) = world.neighbour(x, y, d as usize - 1).expect("inside");
            world.index(nx, ny)
        })
    };
    if !near(c.inside) || !near(c.outside) {
        return None;
    }
    // Up: from the spill's inside cell down to its pit; the gorge starts at the pit.
    let mut path = vec![c.inside];
    while let Some(next) = receiver(*path.last()?) {
        if !near(next) {
            return None;
        }
        path.push(next);
    }
    path.reverse();
    let zp = z(path[0]);
    // Down: from the outlet to the first cell below the pit.
    let mut cur = c.outside;
    path.push(cur);
    while z(cur) >= zp {
        cur = receiver(cur).filter(|&n| near(n))?;
        path.push(cur);
    }
    // Lengthen downstream until the bed falls over GORGE_RATIO × the deepest cut.
    loop {
        let mut s = vec![0.0f64; path.len()];
        for i in 1..path.len() {
            let (a, b) = (world.xy(path[i - 1]), world.xy(path[i]));
            let diag = a.0 != b.0 && a.1 != b.1;
            s[i] = s[i - 1] + step_len(if diag { 5 } else { 1 }) * spacing_m;
        }
        let (len, z1) = (s[path.len() - 1], z(*path.last()?));
        let line = |i: usize| zp + ((z1 - zp) as f64 * s[i] / len) as f32;
        let depth = (0..path.len()).map(|i| z(path[i]) - line(i)).fold(0.0f32, f32::max);
        if depth <= 0.0 || depth > c.cap {
            return None;
        }
        if len < f64::from(GORGE_RATIO * depth) {
            path.push(receiver(*path.last()?).filter(|&n| near(n))?);
            continue;
        }
        // The section: a Euclidean disc of radius half + 1 cells around each path cell with a
        // parabolic bed, the line at the centre rising to the ground at the edge (M1), so the
        // gorge is at least 3 cells wide and has no square, stair-stepped walls; its sides stay
        // for the relaxation.
        let mut new: HashMap<u64, f32> = HashMap::new();
        let radius = c.half as f32 + 1.0;
        let h = c.half as i64 + 1;
        for (i, &p) in path.iter().enumerate() {
            let (px, py) = world.xy(p);
            for dy in -h..=h {
                for dx in -h..=h {
                    let r = ((dx * dx + dy * dy) as f32).sqrt();
                    if r >= radius {
                        continue;
                    }
                    let (Some(x), Some(y)) = (px.checked_add_signed(dx), py.checked_add_signed(dy)) else { continue };
                    if x >= world.w || y >= world.h {
                        continue;
                    }
                    let cell = world.index(x, y);
                    if !near(cell) {
                        return None;
                    }
                    let (ground, centre) = (z(cell), line(i));
                    let bed = centre + (ground - centre).max(0.0) * (r / radius).powi(2);
                    if bed < ground {
                        let e = new.entry(cell).or_insert(bed);
                        *e = e.min(bed);
                    }
                }
            }
        }
        let volume_mm: i64 = new.iter().map(|(&cell, &v)| mm(z(cell)) - mm(v)).sum();
        if volume_mm >= c.vfill_mm {
            return None;
        }
        let mut cut: Vec<(u64, f32)> = new.into_iter().collect();
        cut.sort_unstable_by_key(|p| p.0);
        return Some(Gorge { key: c.key, cut, depth, volume_mm, length_m: len });
    }
}

pub(crate) fn breach(s: &Setup, m: &Model, ground: &Layer<f32>, out: &Layer<f32>, geo: &Geometry, work: Option<&Work>, tag: &str) -> Result<Value> {
    let world = s.world;
    let cands = candidates(s, m, geo);
    let reach = s.params.breach_radius + cands.iter().map(|c| c.half).max().unwrap_or(1) + 2;
    let halo = reach + 1;
    let per_tile: Vec<Vec<(u64, f32, i64, f64, u64)>> = for_tiles(&s.tiles(), s.threads, &format!("{tag} breach"), |i, core| {
        let name = format!("{tag}/breach/{i:05}");
        if let Some(wk) = work {
            if wk.is_done(&name) {
                return wk.cached(&format!("{name}-figures"), || Ok(Vec::new()));
            }
        }
        let win = core.grow(halo, world);
        let zw = ground.read(win)?;
        let mut new: Vec<f32> = crop(win, &zw, core);
        let reach_rect = core.grow(reach, world);
        let mut figures = Vec::new();
        for c in &cands {
            let (x, y) = world.xy(c.inside);
            if !reach_rect.contains(x, y) {
                continue;
            }
            let Some(g) = gorge(world, win, &zw, c, s.spacing_m) else { continue };
            for &(cell, v) in &g.cut {
                let (cx, cy) = world.xy(cell);
                if let Some(k) = core.local(cx, cy) {
                    new[k] = new[k].min(v);
                }
            }
            if core.contains(x, y) {
                figures.push((g.key, g.depth, g.volume_mm, g.length_m, c.inside));
            }
        }
        out.write(core, &new)?;
        if let Some(wk) = work {
            wk.cached(&format!("{name}-figures"), || Ok(figures.clone()))?;
            wk.mark_done(&name)?;
        }
        Ok(figures)
    })?;
    let mut all: Vec<(u64, f32, i64, f64, u64)> = per_tile.into_iter().flatten().collect();
    all.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let cell_m2 = s.spacing_m * s.spacing_m;
    Ok(json!({
        "breaches": all.len(),
        "deepest_cut_m": all.first().map_or(0.0, |b| b.1),
        "volume_m3": all.iter().map(|b| b.2 as f64 / 1000.0 * cell_m2).sum::<f64>().round(),
        "deepest": all.iter().take(20).map(|b| {
            let (x, y) = world.xy(b.4);
            json!({"node": b.0, "cut_m": b.1, "volume_m3": (b.2 as f64 / 1000.0 * cell_m2).round(), "length_m": b.3, "spill_cell": [x, y]})
        }).collect::<Vec<_>>(),
    }))
}
```

`Vec<(u64, f32, i64, f64, u64)>` needs a `Blob` for `f64`: add `blob_num!(f64);` to `store.rs`.

Add the river pools to `bodies.rs` (R26, review C2.2):

```rust
/// River pools: full lake nodes a river flows into (inflow ≥ the river start) whose area is
/// below 20 · w(Q)² of that river; they are labelled as the river leaving them. Node key →
/// outlet cell.
pub(crate) fn pools(s: &Setup, m: &Model, geo: &Geometry) -> BTreeMap<u64, u64> {
    let thr = s.params.river_km2;
    m.water
        .nodes
        .0
        .iter()
        .filter(|n| n.lake && n.full)
        .filter_map(|n| {
            let q = m.flow.node_total.get(&n.key).copied().unwrap_or(0) as f64 / Q_PER_KM2;
            let w = geo.width_m(q);
            (q >= thr && (n.cells as f64) * s.spacing_m * s.spacing_m < 20.0 * w * w).then_some((n.key, n.outlet[1]))
        })
        .collect()
}
```

with `use super::{hydraulic::Geometry, pipeline::Model, rain::Q_PER_KM2};` added to its imports, and a test in `bodies.rs`:

```rust
    #[test]
    fn a_small_pool_on_a_river_is_labelled_as_that_river() {
        let world = World { w: 64, h: 64 };
        let s = setup(world, 16, 1e-5);
        let r = Rasters::memory(world);
        let m = model(&s, &Layer::memory(world, terrain(world)), None, "m", Some(&r)).expect("model");
        // A huge factor makes every lake with river inflow a pool.
        let p = pools(&s, &m, &crate::world::water::hydraulic::Geometry::new(1e9));
        assert!(!p.is_empty(), "the humid bowl takes a river");
        let body = Layer::filled(world, 0u32);
        let json = label(&s, &m.water, &r, &body, &p, None, "m").expect("bodies");
        let lakes = json["bodies"].as_array().unwrap().iter().filter(|b| b["kind"] == "lake").count();
        let all_lakes = m.water.nodes.0.iter().filter(|n| n.lake).count();
        assert_eq!(lakes, all_lakes - p.len(), "pools are not listed as lakes");
        let ids = body.to_vec().unwrap();
        let node = r.node.to_vec().unwrap();
        for k in 0..ids.len() {
            if node[k] >= 2 && p.contains_key(&m.water.nodes.at(node[k] - 2).key) {
                assert!(ids[k] > 1, "a pool cell carries its river's id");
            }
        }
    }
```

Add `mod hydraulic; mod breach;`.

- [ ] **Step 5: Run the tests to see them pass**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::hydraulic` then `… world::water::breach`
Expected: 1 + 3 pass. If `the_candidate` finds no candidate, print `candidates()` and the depression at (39, 10); the dam must hold a pond (r = 1, uniform rain). Clippy clean.

- [ ] **Step 6: Commit**

Run the task-finish block first.

```bash
git add crates/topography-core/src/world/water
git commit -m "feat(water): hydraulic geometry and breach-or-fill with graded gorges

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 7: Expert review**

Expert review protocol. Real-terrain output: an `#[ignore]` probe `probe_breach_at_e43966` in `breach.rs` (copy of Task 9's probe at E43966 N21633, 1,024 m, river start 0.05 km²): run `model`, `breach`, then `model` again on the breached ground; print the breach figures and write `t11-e43966-before.png` and `t11-e43966-after.png` (river and lake cells tinted). The reviewer judges whether gorges look graded and whether any cut is a slot.

---
## Task 12: 1 m channels and the staged run

**Files:**
- Create: `crates/topography-core/src/world/water/channel.rs`
- Modify: `crates/topography-core/src/world/water/mod.rs` (`mod channel;`, `Options`, `setup`, `run`)

**Interfaces:**
- Consumes: `pipeline::{Setup, Water, Rasters, model}`, `nodes::Nodes`, `bodies::{label, pools}`, `breach::breach`, `hydraulic::Geometry`, `hydrology::grid::shape`, `acquisition::read_small`.
- Produces:
  - `channel::{HALO = 64, CORRIDOR = 8, BACKWATER = 16}` (a tributary's surface is raised to the river it flows into for up to BACKWATER cells upstream, N18), `channel::Layers { carved: Layer<f32>, surface: Layer<f32>, body: Layer<u32>, corridor: Layer<u8> }` with `memory(World)` and `files(World, dir)`, `channel::channels(s, w: &Water, raw: &Layer<f32>, ground: &Layer<f32>, r: &Rasters, body: &Layer<u32>, out: &Layers, geo: &Geometry, work, tag) -> Result<Value>`. `corridor`: 0 outside, 1 dry corridor cell (the relaxation may lower it), 2 lowered river cell.
  - `water::Options { grid: PathBuf, work: PathBuf, tile: u64, threads: usize, water: Option<PathBuf>, river_km2: Option<f64> }` and `water::run(&Options) -> Result<String>`: the stages model → breach → model → pools and bodies → channels into the work directory (hydraulic geometry from the table's `m3s_per_km2`). `stages.json` also holds the node area and depth histograms and the leaf, settled-tree and node counts (review C2.4, C3). Work files: `breached.f32`, `m2-level.f32`, `m2-node.u32`, `m2-discharge.f32`, `m2-flow.u8`, `body.u32`, `carved.f32`, `water-surface.f32`, `water-body.u32`, `corridor.u8`, `bodies.json`, `stages.json` (figures), `stamp.json`.
  - `water::setup(grid: &Path, tile, threads, water: Option<&Path>, river_km2) -> Result<(Setup, Layer<f32>, Value)>` (the setup, the raw ground and the resume stamp).

- [ ] **Step 1: Write the failing tests**

`channel.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::bodies::label;
    use crate::world::water::geom::{OFFSETS, World};
    use std::collections::BTreeMap;
    use crate::world::water::pipeline::{Rasters, model};
    use crate::world::water::testing::{setup, terrain};

    /// The synthetic world at 256²: with 32-cell tiles a window (tile + 2 · 88) is smaller than
    /// the world, so window edges fall among the rivers (N9).
    fn run(tile: u64) -> (Vec<f32>, Vec<f32>, Vec<f32>, Vec<u32>, Vec<u8>, Vec<u8>) {
        let world = World { w: 256, h: 256 };
        run_on(world, terrain(world), tile)
    }

    fn run_on(world: World, z: Vec<f32>, tile: u64) -> (Vec<f32>, Vec<f32>, Vec<f32>, Vec<u32>, Vec<u8>, Vec<u8>) {
        let s = setup(world, tile, 1e-5);
        let raw = Layer::memory(world, z);
        let r = Rasters::memory(world);
        let m = model(&s, &raw, None, "m", Some(&r)).expect("model");
        let body = Layer::filled(world, 0u32);
        label(&s, &m.water, &r, &body, &BTreeMap::new(), None, "m").expect("bodies");
        let out = Layers::memory(world);
        // Wide channels on a small grid: 3.6e4 m³/s per km² (radii up to about 25 cells, inside
        // HALO with BACKWATER and CORRIDOR).
        let geo = Geometry::new(3.6e4);
        channels(&s, &m.water, &raw, &raw, &r, &body, &out, &geo, None, "c").expect("channels");
        (raw.to_vec().unwrap(), out.carved.to_vec().unwrap(), out.surface.to_vec().unwrap(),
         out.body.to_vec().unwrap(), out.corridor.to_vec().unwrap(), r.flow.to_vec().unwrap())
    }

    #[test]
    fn only_river_cells_are_lowered_and_never_past_the_cap() {
        let (raw, carved, surface, body, corridor, _) = run(32);
        let mut lowered = 0;
        for k in 0..raw.len() {
            assert!(carved[k] <= raw[k], "raised at {k}");
            assert!(carved[k] >= raw[k] - CHANNEL_CAP_M - 1e-5, "cut past the cap at {k}");
            if carved[k] < raw[k] {
                lowered += 1;
                assert!(surface[k].is_finite() && body[k] > 1, "a dry or unlabelled cell was lowered at {k}");
                assert_eq!(corridor[k], 2);
            }
        }
        assert!(lowered > 20, "channels were cut");
        assert!(corridor.iter().any(|&c| c == 1));
    }

    /// Every wet cell with a receiver: the receiver's surface is not higher.
    fn assert_no_rise(w: usize, surface: &[f32], flow: &[u8]) {
        for (k, &f) in flow.iter().enumerate() {
            if f & 15 == 0 || !surface[k].is_finite() {
                continue;
            }
            let (dy, dx) = OFFSETS[(f & 15) as usize - 1];
            let j = ((k / w) as i64 + dy) as usize * w + ((k % w) as i64 + dx) as usize;
            if surface[j].is_finite() {
                assert!(surface[j] <= surface[k] + 1e-5, "rises from {k} to {j}");
            }
        }
    }

    #[test]
    fn river_surfaces_never_rise_downstream() {
        let (_, _, surface, _, _, flow) = run(32);
        assert_no_rise(256, &surface, &flow);
    }

    #[test]
    fn a_tributary_meets_a_larger_river_at_its_surface() {
        // A trunk valley along row 40 falling east, a tributary valley down column 80 from the
        // north, joining it; the trunk's section is wide (3.6e4 m³/s per km²), so the tributary's
        // last cells lie inside it (N18).
        let world = World { w: 256, h: 80 };
        let z: Vec<f32> = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                let trunk = 50.0 - 0.1 * x + 0.3 * (y - 40.0).abs() + ((i * 7919) % 7) as f32 * 0.002;
                let trib = if y < 40.0 { 50.0 - 0.1 * 80.0 + 0.05 * (40.0 - y) + 0.3 * (x - 80.0).abs() } else { f32::INFINITY };
                trunk.min(trib)
            })
            .collect();
        let (_, _, surface, body, _, flow) = run_on(world, z, 32);
        assert_no_rise(256, &surface, &flow);
        // The tributary's cell next to the trunk thalweg carries the trunk's body (I3b).
        let at = |x: usize, y: usize| y * 256 + x;
        assert!(flow[at(80, 30)] & 16 != 0, "the tributary is a river");
        assert_eq!(body[at(80, 39)], body[at(100, 40)], "it joins the trunk inside its section");
    }

    #[test]
    fn channels_are_alike_at_every_tiling() {
        let a = run(256);
        let b = run(32);
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&a.1), bits(&b.1));
        assert_eq!(bits(&a.2), bits(&b.2));
        assert_eq!(a.3, b.3);
        assert_eq!(a.4, b.4);
    }
}
```

Add a grid-directory writer to `testing.rs` (64², `grid.json` with a `moisture` entry, a 1 x 1 `moisture.f32`):

```rust
/// A grid directory holding `terrain(world)` at 1 m with uniform moisture.
pub(crate) fn grid_dir(dir: &std::path::Path, world: World) -> std::path::PathBuf {
    use std::fs;
    let grid = dir.join("grid");
    fs::create_dir_all(&grid).expect("grid dir");
    let z: Vec<u8> = terrain(world).iter().flat_map(|v| v.to_le_bytes()).collect();
    fs::write(grid.join("elevation.f32"), z).expect("elevation");
    fs::write(grid.join("moisture.f32"), 1.0f32.to_le_bytes()).expect("moisture");
    let manifest = serde_json::json!({
        "size": [world.w, world.h], "spacing_m": 1.0, "origin": [0.0, world.h as f64],
        "moisture": {"file": "moisture.f32", "size": [1, 1], "spacing_m": 1000.0,
                     "nw_m": [-500.0, world.h as f64 + 500.0], "scale_mm_h": 2.0},
        "discharge": {"mean_land_mm_h": 2.0},
    });
    fs::write(grid.join("grid.json"), serde_json::to_vec(&manifest).expect("json")).expect("grid.json");
    // The synthetic bowls are small: a table whose lake minimum is 4 m² (pass it as `water`).
    let table = super::table::DEFAULT.replace("lake_min_m2 = 500.0", "lake_min_m2 = 4.0");
    fs::write(grid.join("water.toml"), table).expect("water.toml");
    grid
}
```

In `mod.rs` tests, a test of `run`:

```rust
    #[test]
    fn run_resumes_from_its_work_directory() {
        use std::fs;
        let dir = tempfile::tempdir().expect("temp");
        let grid = testing::grid_dir(dir.path(), geom::World { w: 64, h: 64 });
        let o = Options { grid: grid.clone(), work: dir.path().join("work"), tile: 16, threads: 2, water: Some(grid.join("water.toml")), river_km2: Some(1e-5) };
        let first = run(&o).expect("run");
        let carved = fs::read(dir.path().join("work/carved.f32")).unwrap();
        let again = run(&o).expect("resume");
        assert_eq!(first, again);
        assert_eq!(fs::read(dir.path().join("work/carved.f32")).unwrap(), carved);
        assert!(run(&Options { tile: 32, ..o }).is_err(), "another tile size needs a fresh work dir");
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::channel` and `… world::water::tests`
Expected: compile errors.

- [ ] **Step 3: Implement `channel.rs`**

```rust
//! 1 m channels. Each thalweg cell's section (width and depth from its discharge, hydraulic
//! geometry) is cut below the river's surface, which is the routing surface R: the ground on
//! dry thalweg cells, the pond level inside ponds. R never rises along a path, so neither does
//! the river. A cell within several thalwegs' half-widths belongs to the one with the larger
//! discharge, then the nearest, then the smaller index (review I3b): a tributary thalweg inside
//! a larger river's section joins that river there instead of running beside it. A cell is
//! lowered only when it ends up under its river's surface, and never more than CHANNEL_CAP_M
//! below raw; a section cell next to a lake whose level stands above the river's surface is
//! left as it is (no water below a lake beside it, R14). Lakes, pools and the sea keep their
//! level. Dry cells within CORRIDOR cells of a lowered river cell form the corridor the GPU
//! relaxation may touch. At coarser spacings (no channels) rivers are their thalweg cells at R
//! and nothing is cut.

use super::CHANNEL_CAP_M;
use super::geom::World;
use super::hydraulic::Geometry;
use super::layers::Layer;
use super::pipeline::{Rasters, Setup, Water};
use super::store::{Work, for_tiles};
use crate::acquisition::Result;
use serde_json::{Value, json};
use std::path::Path;

/// Window halo: the widest section's radius plus the corridor fits well inside it.
pub(crate) const HALO: u64 = 64;
pub(crate) const CORRIDOR: u64 = 8;
/// A river cell's surface is raised to that of the river cell it flows into over at most
/// this many cells upstream (N18). HALO covers the widest radius + BACKWATER + CORRIDOR.
pub(crate) const BACKWATER: u64 = 16;

pub(crate) struct Layers {
    pub carved: Layer<f32>,
    pub surface: Layer<f32>,
    pub body: Layer<u32>,
    pub corridor: Layer<u8>,
}

impl Layers {
    pub fn memory(world: World) -> Self {
        Self {
            carved: Layer::filled(world, 0.0),
            surface: Layer::filled(world, f32::NAN),
            body: Layer::filled(world, 0),
            corridor: Layer::filled(world, 0),
        }
    }
    pub fn files(world: World, dir: &Path) -> Result<Self> {
        Ok(Self {
            carved: Layer::open_or_create(world, &dir.join("carved.f32"), 0.0)?,
            surface: Layer::open_or_create(world, &dir.join("water-surface.f32"), f32::NAN)?,
            body: Layer::open_or_create(world, &dir.join("water-body.u32"), 0)?,
            corridor: Layer::open_or_create(world, &dir.join("corridor.u8"), 0)?,
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn channels(
    s: &Setup,
    w: &Water,
    raw: &Layer<f32>,
    ground: &Layer<f32>,
    r: &Rasters,
    body: &Layer<u32>,
    out: &Layers,
    geo: &Geometry,
    work: Option<&Work>,
    tag: &str,
) -> Result<Value> {
    let world = s.world;
    let cells_per_m = 1.0 / s.spacing_m;
    let parts: Vec<(u64, f32, f64, u64)> = for_tiles(&s.tiles(), s.threads, &format!("{tag} channels"), |i, core| {
        let name = format!("{tag}/ch/{i:05}");
        if let Some(wk) = work {
            if wk.is_done(&name) {
                return wk.cached(&format!("{name}-figures"), || Ok((0, 0.0, 0.0, 0)));
            }
        }
        let win = core.grow(HALO + CORRIDOR, world);
        let area = core.grow(CORRIDOR + BACKWATER, world);
        let (g, rw) = (ground.read(win)?, raw.read(win)?);
        let (level, node, flow, q, bw) = (r.level.read(win)?, r.node.read(win)?, r.flow.read(win)?, r.discharge.read(win)?, body.read(win)?);
        // The thalweg every cell of `area` belongs to: (-discharge, distance², thalweg), least
        // first, so the larger river dominates (I3b).
        let mut best: Vec<Option<(f32, f64, usize)>> = vec![None; area.cells()];
        for t in 0..win.cells() {
            if flow[t] & 16 == 0 {
                continue;
            }
            let (tx, ty) = win.world_xy(t);
            let radius = if s.params.channels { geo.width_m(f64::from(q[t])) / 2.0 * cells_per_m } else { 0.0 };
            let reach = radius.ceil() as i64;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let (Some(x), Some(y)) = (tx.checked_add_signed(dx), ty.checked_add_signed(dy)) else { continue };
                    let Some(a) = area.local(x, y) else { continue };
                    let d2 = (dx * dx + dy * dy) as f64;
                    if d2 > radius * radius && (dx, dy) != (0, 0) {
                        continue;
                    }
                    let cand = (-q[t], d2, t);
                    let better = match best[a] {
                        None => true,
                        Some(b) => cand.0 < b.0 || (cand.0 == b.0 && (cand.1 < b.1 || (cand.1 == b.1 && cand.2 < b.2))),
                    };
                    if better {
                        best[a] = Some(cand);
                    }
                }
            }
        }
        let is_lake = |j: usize| node[j] >= 2 && w.nodes.at(node[j] - 2).lake;
        let n = area.cells();
        let (mut carved, mut surface, mut ids, mut lowered) = (vec![0f32; n], vec![f32::NAN; n], vec![0u32; n], vec![false; n]);
        let mut is_river = vec![false; n];
        for a in 0..n {
            let (x, y) = area.world_xy(a);
            let j = win.local(x, y).expect("area in window");
            carved[a] = g[j];
            ids[a] = bw[j];
            if node[j] == 1 || is_lake(j) {
                surface[a] = level[j];
                continue;
            }
            let Some((_, d2, t)) = best[a] else { continue };
            let river = if level[t].is_finite() { level[t] } else { g[t] };
            let below_a_lake = (0..8).any(|k| {
                world
                    .neighbour(x, y, k)
                    .and_then(|(nx, ny)| win.local(nx, ny))
                    .is_some_and(|j2| is_lake(j2) && level[j2] > river)
            });
            if below_a_lake && t != j {
                continue;
            }
            let depth = if s.params.channels {
                let radius = geo.width_m(f64::from(q[t])) / 2.0 * cells_per_m;
                let full = geo.depth_m(f64::from(q[t]));
                if t == j || radius <= 0.0 { full } else { full * (1.0 - (d2 / (radius * radius)) as f32) }
            } else {
                0.0
            };
            let cut = g[j].min((river - depth).max(rw[j] - CHANNEL_CAP_M));
            if cut < river || t == j {
                lowered[a] = cut < g[j];
                carved[a] = cut;
                surface[a] = river;
                ids[a] = bw[t];
                is_river[a] = true;
            }
        }
        // Backwater (N18): where a tributary meets a larger river, a river cell whose surface
        // lies below that of the river cell it flows into is raised to it, BACKWATER Jacobi
        // steps upstream (water deepens; ground is not touched). Every core cell's chain lies
        // in `area`, so every tiling agrees; anything longer is left for `drainage_rises`.
        for _ in 0..BACKWATER {
            let before = surface.clone();
            for a in 0..n {
                if !is_river[a] {
                    continue;
                }
                let (x, y) = area.world_xy(a);
                let d = flow[win.local(x, y).expect("area in window")] & 15;
                let below = (d != 0)
                    .then(|| world.neighbour(x, y, d as usize - 1))
                    .flatten()
                    .and_then(|(nx, ny)| area.local(nx, ny))
                    .map(|b| before[b])
                    .filter(|s| s.is_finite());
                if let Some(s) = below.filter(|&s| s > before[a]) {
                    surface[a] = s;
                }
            }
        }
        // The corridor: dry cells within CORRIDOR cells of a lowered river cell.
        let mut corridor = vec![0u8; core.cells()];
        let c = CORRIDOR as i64;
        for (k, v) in corridor.iter_mut().enumerate() {
            let (x, y) = core.world_xy(k);
            let a = area.local(x, y).expect("core in area");
            if lowered[a] {
                *v = 2;
                continue;
            }
            if surface[a].is_finite() {
                continue;
            }
            let near = (-c..=c).any(|dy| {
                (-c..=c).any(|dx| {
                    let (Some(nx), Some(ny)) = (x.checked_add_signed(dx), y.checked_add_signed(dy)) else { return false };
                    area.local(nx, ny).is_some_and(|b| lowered[b])
                })
            });
            if near {
                *v = 1;
            }
        }
        let crop = |v: &[f32]| -> Vec<f32> { (0..core.cells()).map(|k| { let (x, y) = core.world_xy(k); v[area.local(x, y).expect("in")] }).collect() };
        let core_ids: Vec<u32> = (0..core.cells()).map(|k| { let (x, y) = core.world_xy(k); ids[area.local(x, y).expect("in")] }).collect();
        let (cc, cs) = (crop(&carved), crop(&surface));
        out.carved.write(core, &cc)?;
        out.surface.write(core, &cs)?;
        out.body.write(core, &core_ids)?;
        out.corridor.write(core, &corridor)?;
        let mut fig = (0u64, 0f32, 0f64, 0u64);
        for k in 0..core.cells() {
            let (x, y) = core.world_xy(k);
            let j = win.local(x, y).expect("in");
            let drop = g[j] - cc[k];
            if drop > 0.0 {
                fig.0 += 1;
                fig.1 = fig.1.max(drop);
                fig.2 += f64::from(drop) * s.spacing_m * s.spacing_m;
            }
            if cs[k].is_finite() && node[j] != 1 && !is_lake(j) {
                fig.3 += 1;
            }
        }
        if let Some(wk) = work {
            wk.cached(&format!("{name}-figures"), || Ok(fig))?;
            wk.mark_done(&name)?;
        }
        Ok(fig)
    })?;
    Ok(json!({
        "lowered_cells": parts.iter().map(|p| p.0).sum::<u64>(),
        "max_lowering_m": parts.iter().map(|p| p.1).fold(0.0f32, f32::max),
        "volume_m3": parts.iter().map(|p| p.2).sum::<f64>().round(),
        "river_cells": parts.iter().map(|p| p.3).sum::<u64>(),
        "width": "3 * Q^0.5 m, Q in m³/s", "depth": "min(0.3 * Q^0.4, 2) m",
        "m3s_per_km2": geo.m3s_per_km2,
    }))
}
```

Note `(u64, f32, f64, u64)` is a `Blob` through the tuple impls (with `f64` from Task 11).

- [ ] **Step 4: Implement `setup` and `run` in `mod.rs`**

```rust
mod bodies;
mod breach;
mod channel;
// (with the other mod lines)

use crate::acquisition::{Result, ensure, io_context, read_small};
use serde_json::{Map, Value, json};
use std::{fs, path::{Path, PathBuf}};

pub struct Options {
    pub grid: PathBuf,
    pub work: PathBuf,
    pub tile: u64,
    pub threads: usize,
    /// Water table TOML; the built-in configs/hydrology/water-v2.toml when absent.
    pub water: Option<PathBuf>,
    /// River start (km² at mean rain); by spacing when absent.
    pub river_km2: Option<f64>,
}

/// The run's setup from a grid directory (grid.json with a `moisture` entry, elevation.f32,
/// optional classes.u8 with `classes_cell_m`), the raw ground, and the stamp a work directory
/// must match to be resumed.
pub(crate) fn setup(
    grid: &Path,
    tile: u64,
    threads: usize,
    water: Option<&Path>,
    river_km2: Option<f64>,
) -> Result<(pipeline::Setup, layers::Layer<f32>, Value)> {
    ensure(tile >= 16, "--tile must be at least 16 cells")?;
    ensure(river_km2.is_none_or(|a| a.is_finite() && a > 0.0), "--river-km2 must be positive")?;
    let bytes = read_small(&grid.join("grid.json"))?;
    let manifest: Map<String, Value> = serde_json::from_slice(&bytes)?;
    let g = crate::world::hydrology::grid::shape(&manifest)?;
    let world = geom::World { w: g.w as u64, h: g.h as u64 };
    ensure(world.w >= 3 && world.h >= 3 && world.w * world.h < u32::MAX as u64 * 2, "grid size out of range")?;
    let raw = layers::Layer::open(world, &grid.join("elevation.f32"))?;
    let classes = if grid.join("classes.u8").exists() {
        let cell_m = manifest.get("classes_cell_m").and_then(Value::as_f64).unwrap_or(g.spacing_m);
        let k = cell_m / g.spacing_m;
        ensure(k >= 1.0 && k.fract() == 0.0, "classes_cell_m must be a whole multiple of spacing_m")?;
        let k = k as u64;
        let cw = geom::World { w: world.w.div_ceil(k), h: world.h.div_ceil(k) };
        Some(layers::Classes { layer: layers::Layer::open(cw, &grid.join("classes.u8"))?, k })
    } else {
        None
    };
    let (text, source) = match water {
        Some(p) => (io_context(format!("read {}", p.display()), fs::read_to_string(p))?, p.display().to_string()),
        None => (table::DEFAULT.to_string(), "built-in water-v2.toml".into()),
    };
    let rain = rain::Rain::read(grid, &manifest)?;
    let elevation = io_context("inspect elevation.f32", fs::metadata(grid.join("elevation.f32")))?;
    let stamp = json!({
        "grid.json": crate::acquisition::digest(&bytes),
        "elevation.f32": {"bytes": elevation.len(), "modified": format!("{:?}", elevation.modified().ok())},
        "tile": tile,
        "water_table": {"source": source, "sha256": crate::acquisition::digest(text.as_bytes())},
        "river_km2": river_km2,
        "version": env!("CARGO_PKG_VERSION"),
    });
    let table = table::Table::parse(&text)?;
    let params = params(&table, g.spacing_m, river_km2);
    let s = pipeline::Setup {
        world,
        nw_m: g.nw_m,
        spacing_m: g.spacing_m,
        tile,
        threads: threads.max(1),
        classes,
        table,
        rain,
        params,
        contract: true,
    };
    Ok((s, raw, stamp))
}

/// Peak resident memory where the OS reports it (Linux), for the logs.
fn peak() -> String {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmHWM:")).map(|l| format!(", peak {}", l[6..].trim())))
        .unwrap_or_default()
}

/// Runs model → breach → model → bodies → channels into `work` (resumable). Returns a
/// one-line summary; the figures go to `work/stages.json`.
pub fn run(o: &Options) -> Result<String> {
    let (s, raw, stamp) = setup(&o.grid, o.tile, o.threads, o.water.as_deref(), o.river_km2)?;
    let work = store::Work::open(&o.work, &stamp)?;
    let wk = Some(&work);
    let geo = hydraulic::Geometry::new(s.table.m3s_per_km2);
    let m1 = pipeline::model(&s, &raw, wk, "m1", None)?;
    let breached = layers::Layer::open_or_create(s.world, &work.path("breached.f32"), 0.0)?;
    let breaches = breach::breach(&s, &m1, &raw, &breached, &geo, wk, "b")?;
    drop(m1);
    let rasters = pipeline::Rasters::files(s.world, &work.path(""), "m2")?;
    let m2 = pipeline::model(&s, &breached, wk, "m2", Some(&rasters))?;
    let body = layers::Layer::open_or_create(s.world, &work.path("body.u32"), 0)?;
    let pools = bodies::pools(&s, &m2, &geo);
    let bodies = match work.is_done("bodies") {
        true => serde_json::from_slice(&io_context("read bodies.json", fs::read(work.path("bodies.json")))?)?,
        false => {
            let v = bodies::label(&s, &m2.water, &rasters, &body, &pools, wk, "m2")?;
            io_context("write bodies.json", fs::write(work.path("bodies.json"), serde_json::to_vec_pretty(&v)?))?;
            work.mark_done("bodies")?;
            v
        }
    };
    let out = channel::Layers::files(s.world, &work.path(""))?;
    let channels = channel::channels(&s, &m2.water, &raw, &breached, &rasters, &body, &out, &geo, wk, "c")?;
    let n = |kind: &str| bodies["bodies"].as_array().map_or(0, |b| b.iter().filter(|e| e["kind"] == kind).count());
    // Node area (log2 m² bins) and depth (0.1 m bins) histograms, for the lake-count probe
    // before gate 2 (review C2.4).
    let nodes = &m2.water.nodes.0;
    let cell_m2 = s.spacing_m * s.spacing_m;
    let (mut area, mut depth) = (vec![0u64; 32], vec![0u64; 40]);
    for nd in nodes {
        area[((nd.cells as f64 * cell_m2).max(1.0).log2() as usize).min(31)] += 1;
        depth[(((nd.level - nd.low) * 10.0).max(0.0) as usize).min(39)] += 1;
    }
    let stages = json!({
        "breach": breaches,
        "channels": channels,
        "cycles": m2.cycles,
        "leaves": m2.water.h.leaves, "depressions": m2.water.h.len(),
        "settled_trees_solved_in_tiles": m2.water.solved.len(),
        "nodes": nodes.len(),
        "lakes": n("lake"), "rivers": n("river"), "dry_basins": n("dry"), "river_pools": pools.len(),
        "ponds": nodes.iter().filter(|nd| !nd.lake).count(),
        "node_area_log2_m2_hist": area, "node_depth_dm_hist": depth,
        "to_sea_cells": m2.water.bal.to_sea as f64 / table::MILLI as f64,
    });
    io_context("write stages.json", fs::write(work.path("stages.json"), serde_json::to_vec_pretty(&stages)?))?;
    ensure(m2.cycles == 0, format!("{} cells on flow cycles", m2.cycles))?;
    eprintln!("water: done{}", peak());
    Ok(format!(
        "{}: {} lakes, {} rivers, {} breaches, {} river cells; next: relax, then water-finish",
        o.work.display(), n("lake"), n("river"), stages["breach"]["breaches"], stages["channels"]["river_cells"],
    ))
}
```

Task 13 adds `AcquisitionError` to this `use` line when `finish` needs it.

- [ ] **Step 5: Run the tests to see them pass**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water`
Expected: all pass. Clippy clean.

- [ ] **Step 6: Commit**

Run the task-finish block first.

```bash
git add crates/topography-core/src/world/water
git commit -m "feat(water): 1 m channel sections, the relaxation corridor and the resumable staged run

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 7: Expert review**

Expert review protocol. Real-terrain output: build a 2,048² grid at the canyon and run the stages (no relaxation yet; the Python grid builder comes in Task 16, so write this grid with a small `#[ignore]` test `probe_stages_in_the_canyon` in `mod.rs` that copies the window of `probe::RAW` into `scratch/hydrology-v3/probes/t12-canyon-grid` (kept for Tasks 13 and 14) with `grid.json`, `moisture.f32` (copied from `data/worlds/fujix-55-l4v2/moisture.f32`, entry from that world's manifest, `discharge.mean_land_mm_h` from `data/worlds/fujix-55-chain/hydrology/4m/grid.json`) and runs `run` into `scratch/hydrology-v3/probes/t12-canyon-work`). Write `t12-canyon-carved.png` (hillshade of `carved.f32`, river and lake cells tinted) and print `stages.json`. The reviewer looks for one river on the canyon floor, sections that follow the valley, and cuts no deeper than needed.

---

## Task 13: Checks, figures and `water-finish`

**Files:**
- Create: `crates/topography-core/src/world/water/checks.rs`
- Modify: `crates/topography-core/src/world/water/mod.rs` (`mod checks;`, `FinishOptions`, `Outcome`, `finish`)
- Modify: `crates/topography-core/src/world/hydrology/grid.rs` (`Stage` → `pub(crate)`, add `Stage::copy`)

**Interfaces:**
- Consumes: the work directory of Task 12, `hydrology::grid::Stage`.
- Produces:
  - `checks::Tile` (per-tile counts, examples, histogram, facet and straightness tallies, longest runs, dam falls, raw-independent dam drops and rims, lowest lake saddles, label-agreement counts) and `checks::check_tile(s: &Setup, l: &CheckLayers, core: Rect, lakes: &LakeInfo) -> Result<Tile>`; `checks::report(tiles: Vec<Tile>, extra: CheckExtra, figures: Map<String, Value>) -> (Value, bool)`; helpers `planar`, `horn_deg`, `p99_m`, `BINS`.
  - `checks::CheckLayers { raw, ground, surface, body, level, node, flow, discharge, flow_written }` (all `Layer`s; `level`, `node`, `flow`, `discharge` are the third model run's on the final ground, R27; `flow_written` is m2's flow, the written rivers), `checks::LakeInfo = BTreeMap<u32, (f32 level, bool full, [u64; 2] outlet, u64 need)>` (by lake id), `checks::CheckExtra { cycles: u64, bodies: usize, lake_short: u64, relax: Option<Value>, lakes: LakeInfo }`. `change_max` uses `s.params.change_max_m()` (N19).
  - `water::FinishOptions { grid, work, ground: PathBuf, out: PathBuf, tile, threads, water, river_km2, relax: Option<PathBuf> }`, `water::Outcome { out: PathBuf, passed: bool, summary: String }`, `water::finish(&FinishOptions) -> Result<Outcome>`: reruns the model on the final ground (tag `m3`, R27, cached in the work directory), runs the checks and publishes `out` with `elevation.f32`, `water-surface.f32`, `water-body.u32`, `discharge.f32` and `flow.u8` (both from m3), `bodies.json`, `report.json`, `grid.json`. Outputs are published even when a check fails (`passed` false), as v2.
  - `hydrology::grid::Stage::copy(&mut self, name: &str, src: &Path) -> Result<()>` (`cp --reflink=auto`: a reflink on btrfs, a copy on other file systems such as the tmpfs the unit tests use; never a hard link, N17; plus its SHA-256).

Check definitions (hard: every count must be 0; thresholds from the spec, rulings R7, R11, R14, R16, R22, R27, R29, R30, R31). D is the water surface where wet, the pond level in ponds (m3), else the final ground:

| Check | Fails a cell (or lake) when |
| --- | --- |
| `drainage_rises` | a non-sea cell with a receiver (m3) has D(receiver) > D(cell) + 1e-4 m |
| `drainage_stuck` | a dry cell has no receiver, is not on the world edge and is not in a closed basin (flow bit 32) |
| `drainage_cycles` | the third model run reported cells on cycles |
| `lake_not_flat` | a lake cell's surface differs from its lake's level |
| `lake_off_sill` | a full lake's lowest saddle on the final surfaces, min over lake cells' non-lake neighbours n of max(level, D(n)), differs from its level by more than 0.01 m (C1) |
| `lake_leaks` | a non-lake neighbour of a lake cell, other than the lake's outlet cell, has D more than 0.01 m below the level (C1) |
| `lake_exit_surface` | the cell of a full lake's spill edge that carries the saddle (the outflow's first cell) shows a surface or ground more than 0.01 m off the level (C1) |
| `lake_balance` | a closed lake covers fewer cells than its supply needs |
| `dam_steps` | within 50 m below a lake exit, D drops more than 1.5 m between adjacent path cells and raw does not drop at least as much (less 1 cm) |
| `dam_rims` | a dry cell within 2 cells of the exit, on its outflow side, stands more than 0.5 m above the level and raw did not. A guard: v3 never raises ground, so it cannot fail (M5) |
| `thalweg_beside` | a thalweg cell with Q ≥ 1 km²: the lowest raw cell within ±30 m across the flow is neither its body's river cell nor within 1 m of its surface |
| `parallel_rivers` | along a river with Q ≥ 1 km², another flow line with Q ≥ 0.002 km² runs within 10 m and parallel (direction cosine ≥ 0.7) for more than 100 m without joining the river within 100 m downstream (I4, R29) |
| `straight_share` | more than 5 % of river length lies in same-direction runs longer than 60 m (global; reported as 0 or 1) |
| `straight_runs` | a river run longer than 150 m in one D8 direction, unless at 90 % or more of its cells the lowest raw cell within ±3 cells across lies on the run (a straight gorge) (I6, R30) |
| `facets` | among changed cells (\|Δ\| > 1 cm), more 5×5 windows are planar (fit residual < 2 cm, slope > 5°) in the result than in raw (global; 0 or 1) |
| `walls` | a changed cell's Horn slope exceeds max(40°, its raw slope) + 0.01° |
| `change_p99` | p99 of \|Δ\| over all cells > 1 m (global; 0 or 1) |
| `change_max` | a cell's \|Δ\| > the level's breach cap + 0.5 m (2.5 m at 1 m, 5.5 m at 4 m, 20.5 m at 32 m; N19) |
| `label_agreement` | the written rivers' thalweg cells (m2 thalweg under a written surface) and the final routing's thalweg cells (m3) agree less than 0.95 within one cell: (written cells with an m3 thalweg within one cell + m3 cells with a written thalweg within one cell) / (all of both) (I3c, R27; a tolerant IoU, since a river is one thalweg cell wide in both; global) |
| `export_bodies` | lakes + rivers > 50,000 (well under export's 65,535; C2.4, R16; global) |
| `relax_overlap` | the relaxation's recomputed overlaps differ by more than 1 mm (from `--relax`; global) |

Figures: lakes, ponds (count, km²), rivers (count, km), sea km², max discharge, breach figures and the node area and depth histograms (from `stages.json`), channel figures, the fall over 50 m below each exit (max and the 10 largest with cells), **`dam_drops_any` and `dam_rims_any`** (raw ignored, with cells, R31: Plan B's baseline; gate 2 must list the three dam spots and E43966 there), straight share, **the 20 longest same-direction runs with their cells** (review crops in Task 17), facet shares, p99 over changed cells, river length beside another river within 20 m, **rivers with Q ≥ 1 km² that end in a closed node** (M2: intended in arid classes, a table error elsewhere), label IoU, examples (up to 5 cells per failed check). The check window is the tile grown by 160 cells (a 150 m run at 1 m must fit).

- [ ] **Step 1: Write the failing tests**

`checks.rs` unit tests on small hand-built layers (one per detector), plus an end-to-end test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plane_fit_finds_facets_only_on_steep_planes() {
        let steep: Vec<f32> = (0..25).map(|i| (i % 5) as f32 * 0.2).collect();
        assert!(planar(&steep, 1.0), "a 11° plane");
        let flat: Vec<f32> = vec![1.0; 25];
        assert!(!planar(&flat, 1.0), "flat is not a facet");
        let rough: Vec<f32> = (0..25).map(|i| (i % 5) as f32 * 0.2 + ((i * 7) % 3) as f32 * 0.1).collect();
        assert!(!planar(&rough, 1.0));
    }

    #[test]
    fn horn_slope_matches_a_plane() {
        let z: Vec<f32> = (0..9).map(|i| (i % 3) as f32).collect();
        assert!((horn_deg(&z, 3, 1, 1, 1.0) - 45.0).abs() < 1e-3);
    }

    /// A straight river along row 10 of a plane tilting south, and optionally a second flow
    /// line along row `beside` that never joins it.
    fn river_case(beside: Option<u64>) -> Tile {
        use crate::world::water::geom::World;
        use crate::world::water::testing::setup;
        let world = World { w: 260, h: 30 };
        let n = (world.w * world.h) as usize;
        let z: Vec<f32> = (0..n).map(|i| 100.0 - 0.01 * (i % 260) as f32 - 0.1 * (i / 260) as f32).collect();
        let (mut flow, mut q, mut body, mut surface) = (vec![0u8; n], vec![0f32; n], vec![0u32; n], vec![f32::NAN; n]);
        for x in 10..240 {
            let k = 10 * 260 + x;
            (flow[k], q[k], body[k], surface[k]) = (4 | 16, 2.0, 5, z[k]);
            if let Some(row) = beside {
                let k = (row as usize) * 260 + x;
                (flow[k], q[k]) = (4, 0.01);
            }
        }
        let l = CheckLayers {
            raw: Layer::memory(world, z.clone()),
            ground: Layer::memory(world, z),
            surface: Layer::memory(world, surface),
            body: Layer::memory(world, body),
            level: Layer::filled(world, f32::NAN),
            node: Layer::filled(world, 0),
            flow: Layer::memory(world, flow.clone()),
            discharge: Layer::memory(world, q),
            flow_written: Layer::memory(world, flow),
        };
        check_tile(&setup(world, 512, 0.05), &l, Rect::whole(world), &LakeInfo::new()).expect("check")
    }

    #[test]
    fn a_long_straight_river_and_a_parallel_channel_fail() {
        let alone = river_case(None);
        assert!(alone.fails.get("straight_runs").copied().unwrap_or(0) > 0, "a 230 m run on a tilted plane");
        assert_eq!(alone.fails.get("parallel_rivers").copied().unwrap_or(0), 0);
        let doubled = river_case(Some(15));
        assert!(doubled.fails.get("parallel_rivers").copied().unwrap_or(0) > 0, "a second channel 5 m away");
        assert_eq!(river_case(Some(25)).fails.get("parallel_rivers").copied().unwrap_or(0), 0, "15 m away is fine");
    }

    #[test]
    fn p99_reads_the_histogram() {
        let mut hist = vec![0u64; BINS];
        hist[0] = 990;
        hist[150] = 10;
        assert_eq!(p99_m(&hist), 0.0);
        hist[150] = 11;
        assert!((p99_m(&hist) - 1.5).abs() < 0.011);
    }

    #[test]
    fn the_synthetic_world_passes_its_checks() {
        use crate::world::water::{FinishOptions, Options, finish, geom::World, run, testing};
        let dir = tempfile::tempdir().expect("temp");
        let grid = testing::grid_dir(dir.path(), World { w: 64, h: 64 });
        let o = Options { grid: grid.clone(), work: dir.path().join("work"), tile: 16, threads: 2, water: Some(grid.join("water.toml")), river_km2: Some(1e-5) };
        run(&o).expect("run");
        let f = FinishOptions {
            grid: grid.clone(), work: o.work.clone(), ground: o.work.join("carved.f32"), out: dir.path().join("out"),
            tile: 16, threads: 2, water: Some(grid.join("water.toml")), river_km2: Some(1e-5), relax: None,
        };
        let outcome = finish(&f).expect("finish");
        let report: Value = serde_json::from_slice(&std::fs::read(dir.path().join("out/report.json")).unwrap()).unwrap();
        assert!(outcome.passed, "{}", serde_json::to_string_pretty(&report["checks"]).unwrap());
        for name in ["elevation.f32", "water-surface.f32", "water-body.u32", "discharge.f32", "flow.u8", "bodies.json", "grid.json"] {
            assert!(dir.path().join("out").join(name).exists(), "{name}");
        }
    }

    #[test]
    fn a_raised_rim_below_a_lake_fails_the_dam_check() {
        use crate::world::water::{FinishOptions, Options, finish, geom::World, run, testing};
        let dir = tempfile::tempdir().expect("temp");
        let grid = testing::grid_dir(dir.path(), World { w: 64, h: 64 });
        let o = Options { grid: grid.clone(), work: dir.path().join("work"), tile: 16, threads: 2, water: Some(grid.join("water.toml")), river_km2: Some(1e-5) };
        run(&o).expect("run");
        // Raise the dry cells beside the humid lake's exit by 2 m in a copy of the ground.
        let bodies: Value = serde_json::from_slice(&std::fs::read(o.work.join("bodies.json")).unwrap()).unwrap();
        let lake = bodies["bodies"].as_array().unwrap().iter().find(|b| b["kind"] == "lake" && !b["exit_cell"].is_null()).expect("a full lake");
        let (ex, ey) = (lake["exit_cell"][0].as_u64().unwrap(), lake["exit_cell"][1].as_u64().unwrap());
        let mut z: Vec<f32> = std::fs::read(o.work.join("carved.f32")).unwrap().chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
        let surface: Vec<f32> = std::fs::read(o.work.join("water-surface.f32")).unwrap().chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
        for y in ey.saturating_sub(2)..=(ey + 2).min(63) {
            for x in ex.saturating_sub(2)..=(ex + 2).min(63) {
                let k = (y * 64 + x) as usize;
                if surface[k].is_nan() {
                    z[k] = lake["level_m"].as_f64().unwrap() as f32 + 2.0;
                }
            }
        }
        let bumped = dir.path().join("bumped.f32");
        std::fs::write(&bumped, z.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        let f = FinishOptions { grid: grid.clone(), work: o.work.clone(), ground: bumped, out: dir.path().join("out"), tile: 16, threads: 2, water: Some(grid.join("water.toml")), river_km2: Some(1e-5), relax: None };
        let outcome = finish(&f).expect("finish");
        assert!(!outcome.passed);
        let report: Value = serde_json::from_slice(&std::fs::read(dir.path().join("out/report.json")).unwrap()).unwrap();
        assert!(report["checks"]["dam_rims"].as_u64().unwrap() > 0, "{}", report["checks"]);
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::checks`
Expected: compile errors.

- [ ] **Step 3: Add `Stage::copy` to `hydrology/grid.rs`**

Change `pub(super) struct Stage` to `pub(crate) struct Stage` and add:

```rust
    /// A finished file (e.g. a 17 GB raster from a work directory) copied into the stage as a
    /// reflink where the file system has them (btrfs: no extra space), a plain copy elsewhere,
    /// never a hard link: work rasters are written in place, and a rerun must not change a
    /// published output behind its digest (N17). Then digested.
    pub fn copy(&mut self, name: &str, src: &Path) -> Result<()> {
        let dst = self.dir.path().join(name);
        let done = std::process::Command::new("cp")
            .arg("--reflink=auto")
            .arg(src)
            .arg(&dst)
            .status();
        ensure(
            io_context(format!("copy {}", src.display()), done)?.success(),
            format!("copy {} failed", src.display()),
        )?;
        let mut file = io_context(format!("open {name}"), fs::File::open(&dst))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 22];
        loop {
            let n = io_context(format!("read {name}"), file.read(&mut buf))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        self.digests.insert(name.into(), hex(&hasher.finalize()).into());
        Ok(())
    }
```

- [ ] **Step 4: Implement `checks.rs`**

```rust
//! Hard checks (each a count that must be 0; spec "Checks", rulings R7, R11, R14, R16, R22,
//! R27, R29, R30, R31) and figures on a finished run, tile by tile with a 160-cell halo (a
//! 150 m straight run at 1 m must fit).

use super::bodies::main_up;
use super::geom::{OFFSETS, Rect, step_len};
use super::layers::Layer;
use super::pipeline::Setup;
use super::{BODY_BUDGET, MAIN_RIVER_KM2};
use crate::acquisition::Result;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

const HALO: u64 = 160;
/// Parallel rivers (R29): another flow line from this size, this close, this parallel, this long.
const PARALLEL_KM2: f32 = 0.002;
const PARALLEL_M: f64 = 10.0;
const PARALLEL_COS: f64 = 0.7;
const PARALLEL_RUN_M: f64 = 100.0;
/// Straight runs (R30): longer than this fail unless the raw thalweg is straight there too.
const STRAIGHT_RUN_M: f64 = 150.0;
const GORGE_ACROSS: i64 = 3;
const GORGE_SHARE: f64 = 0.9;
/// Label agreement (R27).
const AGREEMENT: f64 = 0.95;
/// |Δ| histogram: 1 cm bins to 10 m, the last bin holds the rest.
pub(crate) const BINS: usize = 1001;
const RISE_M: f32 = 1e-4;
const SILL_M: f32 = 0.01;
const DAM_STEP_M: f32 = 1.5;
const DAM_RIM_M: f32 = 0.5;
const DAM_REACH_M: f64 = 50.0;
const ACROSS_M: f64 = 30.0;
const THALWEG_M: f32 = 1.0;
const STRAIGHT_M: f64 = 60.0;
const STRAIGHT_SHARE: f64 = 0.05;
const PLANE_RMS_M: f64 = 0.02;
const PLANE_DEG: f64 = 5.0;
const WALL_DEG: f64 = 40.0;
const CHANGED_M: f32 = 0.01;

pub(crate) struct CheckLayers {
    pub raw: Layer<f32>,
    pub ground: Layer<f32>,
    pub surface: Layer<f32>,
    pub body: Layer<u32>,
    pub level: Layer<f32>,
    pub node: Layer<u32>,
    pub flow: Layer<u8>,
    pub discharge: Layer<f32>,
    /// The written rivers' flow layer (m2), for the label agreement with m3's `flow`.
    pub flow_written: Layer<u8>,
}

/// Lakes by id: level, full, outlet [inside, outside], cells needed (closed).
pub(crate) type LakeInfo = BTreeMap<u32, (f32, bool, [u64; 2], u64)>;

#[derive(Default)]
pub(crate) struct Tile {
    pub fails: BTreeMap<&'static str, u64>,
    pub examples: BTreeMap<&'static str, Vec<[u64; 2]>>,
    pub hist: Vec<u64>,
    pub max_change: f32,
    pub planar: [u64; 2],
    pub straight_m: f64,
    pub river_m: f64,
    pub beside_m: f64,
    pub pond_cells: u64,
    pub falls: Vec<(u32, f32)>,
    /// Lowest saddle out of each lake seen in this tile (lake id → min of max(level, D)).
    pub saddle: BTreeMap<u32, f32>,
    /// Same-direction runs over 60 m: (length m, start cell, end cell), the longest kept.
    pub runs: Vec<(f64, [u64; 2], [u64; 2])>,
    /// Raw-independent dam figures (R31): (lake, cell, drop or height m).
    pub drops_any: Vec<(u32, [u64; 2], f32)>,
    pub rims_any: Vec<(u32, [u64; 2], f32)>,
    /// Label agreement (R27): written thalweg cells, of them matched within one cell, m3
    /// thalweg cells, of them matched.
    pub agree: [u64; 4],
    /// Rivers with Q ≥ 1 km² ending in a closed node (M2).
    pub closed_ends: u64,
}

impl Tile {
    fn fail(&mut self, name: &'static str, x: u64, y: u64) {
        *self.fails.entry(name).or_insert(0) += 1;
        let e = self.examples.entry(name).or_default();
        if e.len() < 5 {
            e.push([x, y]);
        }
    }
}

/// Planar 5 x 5 window: least-squares plane residual under 2 cm and slope over 5°.
pub(crate) fn planar(z: &[f32], spacing_m: f64) -> bool {
    let mean = z.iter().map(|&v| f64::from(v)).sum::<f64>() / 25.0;
    let (mut sx, mut sy) = (0.0, 0.0);
    for (i, &v) in z.iter().enumerate() {
        let (x, y) = ((i % 5) as f64 - 2.0, (i / 5) as f64 - 2.0);
        sx += x * f64::from(v);
        sy += y * f64::from(v);
    }
    let (a, b) = (sx / 50.0, sy / 50.0);
    let rms = (z.iter().enumerate().map(|(i, &v)| {
        let (x, y) = ((i % 5) as f64 - 2.0, (i / 5) as f64 - 2.0);
        (f64::from(v) - mean - a * x - b * y).powi(2)
    }).sum::<f64>() / 25.0).sqrt();
    rms < PLANE_RMS_M && (a.hypot(b) / spacing_m).atan().to_degrees() > PLANE_DEG
}

/// Horn slope in degrees at (x, y) of a `w`-wide block.
pub(crate) fn horn_deg(z: &[f32], w: usize, x: usize, y: usize, spacing_m: f64) -> f64 {
    let at = |dx: i64, dy: i64| f64::from(z[(y as i64 + dy) as usize * w + (x as i64 + dx) as usize]);
    let gx = (at(1, -1) + 2.0 * at(1, 0) + at(1, 1) - at(-1, -1) - 2.0 * at(-1, 0) - at(-1, 1)) / (8.0 * spacing_m);
    let gy = (at(-1, 1) + 2.0 * at(0, 1) + at(1, 1) - at(-1, -1) - 2.0 * at(0, -1) - at(1, -1)) / (8.0 * spacing_m);
    gx.hypot(gy).atan().to_degrees()
}

pub(crate) fn p99_m(hist: &[u64]) -> f64 {
    let total: u64 = hist.iter().sum();
    let want = (total as f64 * 0.99).ceil() as u64;
    let mut seen = 0;
    for (b, &n) in hist.iter().enumerate() {
        seen += n;
        if seen >= want {
            return b as f64 / 100.0;
        }
    }
    (BINS - 1) as f64 / 100.0
}

pub(crate) fn check_tile(s: &Setup, l: &CheckLayers, core: Rect, lakes: &LakeInfo) -> Result<Tile> {
    let world = s.world;
    let win = core.grow(HALO, world);
    let (raw, z, surf, body, level, node, flow, q) = (
        l.raw.read(win)?, l.ground.read(win)?, l.surface.read(win)?, l.body.read(win)?,
        l.level.read(win)?, l.node.read(win)?, l.flow.read(win)?, l.discharge.read(win)?,
    );
    let fw = l.flow_written.read(win)?;
    let at = |x: u64, y: u64| win.local(x, y);
    let gi = |j: usize| {
        let (x, y) = win.world_xy(j);
        world.index(x, y)
    };
    let around = |j: usize| {
        let (x, y) = win.world_xy(j);
        (0..8).filter_map(move |k| world.neighbour(x, y, k).and_then(|(nx, ny)| win.local(nx, ny)))
    };
    // Another flow line beside a main river (R29), seen from river cell j with unit direction v.
    let parallel_at = |j: usize, v: (f64, f64)| -> bool {
        let (x, y) = win.world_xy(j);
        let reach = (PARALLEL_M / s.spacing_m).round() as i64;
        (-reach..=reach).filter(|&i| i != 0).any(|i| {
            let (px, py) = ((x as f64 - v.1 * i as f64).round(), (y as f64 + v.0 * i as f64).round());
            if px < 0.0 || py < 0.0 {
                return false;
            }
            let Some(p) = at(px as u64, py as u64) else { return false };
            let d = flow[p] & 15;
            if q[p] < PARALLEL_KM2 || d == 0 || (surf[p].is_finite() && body[p] == body[j]) {
                return false;
            }
            let (dy, dx) = OFFSETS[d as usize - 1];
            let len = ((dx * dx + dy * dy) as f64).sqrt();
            if (dx as f64 * v.0 + dy as f64 * v.1) / len < PARALLEL_COS {
                return false;
            }
            // It joins the river within 100 m downstream: a tributary, not a second channel.
            let (mut c, mut walked) = (p, 0.0);
            while walked <= PARALLEL_RUN_M {
                if surf[c].is_finite() && body[c] == body[j] {
                    return false;
                }
                let dc = flow[c] & 15;
                let (cx, cy) = win.world_xy(c);
                let Some(n) = (dc != 0).then(|| world.neighbour(cx, cy, dc as usize - 1)).flatten().and_then(|(nx, ny)| at(nx, ny)) else { break };
                walked += step_len(dc) * s.spacing_m;
                c = n;
            }
            true
        })
    };
    // Unit flow direction over 5 steps downstream of window cell j.
    let direction = |j: usize| -> Option<(f64, f64)> {
        let (x, y) = win.world_xy(j);
        let (mut cx, mut cy) = (x, y);
        for _ in 0..5 {
            let c = at(cx, cy)?;
            let d = flow[c] & 15;
            if d == 0 {
                break;
            }
            let Some(n) = world.neighbour(cx, cy, d as usize - 1) else { break };
            (cx, cy) = n;
        }
        let (vx, vy) = (cx as f64 - x as f64, cy as f64 - y as f64);
        let len = vx.hypot(vy);
        (len > 0.0).then(|| (vx / len, vy / len))
    };
    let main_river = |j: usize| flow[j] & 16 != 0 && f64::from(q[j]) >= MAIN_RIVER_KM2;
    let flagged = |j: usize| main_river(j) && direction(j).is_some_and(|v| parallel_at(j, v));
    // What drains: labelled water at its surface, ponds at their level, else ground.
    let dsurf = |j: usize| if surf[j].is_finite() { surf[j] } else if level[j].is_finite() { level[j] } else { z[j] };
    let shown = |j: usize| if surf[j].is_finite() { surf[j] } else { z[j] };
    let receiver = |x: u64, y: u64, j: usize| -> Option<(u64, u64)> {
        let d = flow[j] & 15;
        (d != 0).then(|| world.neighbour(x, y, d as usize - 1)).flatten()
    };
    let lake_of = |id: u32| lakes.get(&id);
    let mut t = Tile { hist: vec![0; BINS], ..Default::default() };
    let thr = s.river_km2();
    for k in 0..core.cells() {
        let (x, y) = core.world_xy(k);
        let j = at(x, y).expect("core in window");
        // Change budget.
        let delta = (z[j] - raw[j]).abs();
        t.hist[((delta * 100.0).round() as usize).min(BINS - 1)] += 1;
        t.max_change = t.max_change.max(delta);
        if delta > s.params.change_max_m() {
            t.fail("change_max", x, y);
        }
        if node[j] >= 2 && surf[j].is_nan() {
            t.pond_cells += 1;
        }
        // Drainage.
        if node[j] != 1 {
            match receiver(x, y, j) {
                Some((rx, ry)) => {
                    let r = at(rx, ry).expect("receiver in window");
                    if dsurf(r) > dsurf(j) + RISE_M {
                        t.fail("drainage_rises", x, y);
                    }
                }
                None => {
                    if node[j] == 0 && !world.is_edge(x, y) && flow[j] & 32 == 0 {
                        t.fail("drainage_stuck", x, y);
                    }
                }
            }
        }
        // Lakes are flat, stand at their lowest saddle and do not leak (C1).
        if let Some(&(lv, full, outlet, _)) = lake_of(body[j]).filter(|_| surf[j].is_finite()) {
            if surf[j] != lv {
                t.fail("lake_not_flat", x, y);
            }
            for n in around(j) {
                if surf[n].is_finite() && body[n] == body[j] {
                    continue;
                }
                let dn = dsurf(n);
                let e = t.saddle.entry(body[j]).or_insert(f32::INFINITY);
                *e = e.min(lv.max(dn));
                if full && dn < lv - SILL_M && gi(n) != outlet[1] {
                    let (nx, ny) = win.world_xy(n);
                    t.fail("lake_leaks", nx, ny);
                }
            }
        }
        // Label agreement (R27): written thalwegs (m2) vs the final routing's (m3), within one cell.
        let thal_m2 = fw[j] & 16 != 0 && surf[j].is_finite();
        let thal_m3 = flow[j] & 16 != 0;
        if thal_m2 {
            t.agree[0] += 1;
            t.agree[1] += u64::from(thal_m3 || around(j).any(|n| flow[n] & 16 != 0));
        }
        if thal_m3 {
            t.agree[2] += 1;
            t.agree[3] += u64::from(thal_m2 || around(j).any(|n| fw[n] & 16 != 0 && surf[n].is_finite()));
        }
        // Walls and facets on changed cells (away from the window edge).
        let inner = x >= win.x0 + 2 && y >= win.y0 + 2 && x + 2 < win.x0 + win.w as u64 && y + 2 < win.y0 + win.h as u64;
        if delta > CHANGED_M && inner {
            let (lx, ly) = ((x - win.x0) as usize, (y - win.y0) as usize);
            let (fs, rs) = (horn_deg(&z, win.w, lx, ly, s.spacing_m), horn_deg(&raw, win.w, lx, ly, s.spacing_m));
            if fs > WALL_DEG.max(rs) + 0.01 {
                t.fail("walls", x, y);
            }
            let block = |v: &[f32]| -> Vec<f32> { (0..25).map(|i| v[(ly + i / 5 - 2) * win.w + lx + i % 5 - 2]).collect() };
            t.planar[0] += u64::from(planar(&block(&raw), s.spacing_m));
            t.planar[1] += u64::from(planar(&block(&z), s.spacing_m));
        }
        // Rivers: straightness, one thalweg, rivers beside rivers.
        if flow[j] & 16 != 0 && q[j] >= thr {
            let d = flow[j] & 15;
            let step = step_len(d) * s.spacing_m;
            t.river_m += step;
            let (dy, dx) = OFFSETS[d as usize - 1];
            let same = |cx: u64, cy: u64| at(cx, cy).is_some_and(|c| flow[c] & 16 != 0 && flow[c] & 15 == d && body[c] == body[j]);
            let mut run = 1;
            for sign in [-1i64, 1] {
                let (mut cx, mut cy) = (x as i64, y as i64);
                while (run as f64) * step <= STRAIGHT_M {
                    (cx, cy) = (cx + sign * dx, cy + sign * dy);
                    if cx < 0 || cy < 0 || !same(cx as u64, cy as u64) {
                        break;
                    }
                    run += 1;
                }
            }
            if run as f64 * step > STRAIGHT_M {
                t.straight_m += step;
            }
            // Runs, counted once from their upstream end (R30).
            let (ux, uy) = (x as i64 - dx, y as i64 - dy);
            if ux < 0 || uy < 0 || !same(ux as u64, uy as u64) {
                let (mut cells, mut cx, mut cy) = (vec![(x, y)], x as i64, y as i64);
                while ((cells.len() - 1) as f64) * step <= STRAIGHT_RUN_M + 2.0 * step {
                    (cx, cy) = (cx + dx, cy + dy);
                    if cx < 0 || cy < 0 || !same(cx as u64, cy as u64) {
                        break;
                    }
                    cells.push((cx as u64, cy as u64));
                }
                let len = (cells.len() - 1) as f64 * step;
                if len > STRAIGHT_M {
                    let end = cells[cells.len() - 1];
                    t.runs.push((len, [x, y], [end.0, end.1]));
                }
                if len > STRAIGHT_RUN_M {
                    // A straight gorge: the lowest raw cell across (±3) lies on the run.
                    let (px, py) = (-dy, dx);
                    let on = cells
                        .iter()
                        .filter(|&&(cx, cy)| {
                            let here = at(cx, cy).map_or(f32::INFINITY, |c| raw[c]);
                            (-GORGE_ACROSS..=GORGE_ACROSS).filter(|&i| i != 0).all(|i| {
                                let (ax, ay) = (cx as i64 + px * i, cy as i64 + py * i);
                                ax < 0 || ay < 0 || at(ax as u64, ay as u64).is_none_or(|c| raw[c] >= here)
                            })
                        })
                        .count();
                    if (on as f64) < GORGE_SHARE * cells.len() as f64 {
                        t.fail("straight_runs", x, y);
                    }
                }
            }
            // A big river ending in a closed node (M2).
            if f64::from(q[j]) >= MAIN_RIVER_KM2 {
                if let Some(r) = receiver(x, y, j).and_then(|(rx, ry)| at(rx, ry)) {
                    if node[r] >= 2 && flow[r] & 64 != 0 {
                        t.closed_ends += 1;
                    }
                }
            }
            // Parallel rivers (R29): a run of flagged main-river cells over 100 m, counted from
            // its upstream end (the main stem above is not flagged).
            if flagged(j) && main_up(world, win, &flow, &q, x, y).and_then(|u| { let (ux, uy) = world.xy(u); at(ux, uy) }).is_none_or(|u| !flagged(u)) {
                let (mut c, mut walked) = (j, 0.0);
                while walked <= PARALLEL_RUN_M && flagged(c) {
                    let dc = flow[c] & 15;
                    let (cx, cy) = win.world_xy(c);
                    let Some(n) = world.neighbour(cx, cy, dc as usize - 1).and_then(|(nx, ny)| at(nx, ny)) else { break };
                    walked += step_len(dc) * s.spacing_m;
                    c = n;
                }
                if walked > PARALLEL_RUN_M {
                    t.fail("parallel_rivers", x, y);
                }
            }
            if f64::from(q[j]) >= MAIN_RIVER_KM2 {
                // Across the flow direction over 5 steps downstream (the shared helper, N16).
                if let Some((vx, vy)) = direction(j) {
                    let (nx, ny) = (-vy, vx);
                    let reach = (ACROSS_M / s.spacing_m).round() as i64;
                    let mut low: Option<(f32, i64, usize)> = None;
                    for i in -reach..=reach {
                        let (px, py) = ((x as f64 + nx * i as f64).round(), (y as f64 + ny * i as f64).round());
                        if px < 0.0 || py < 0.0 {
                            continue;
                        }
                        let Some(p) = at(px as u64, py as u64) else { continue };
                        let c = (raw[p], i.abs(), p);
                        if low.is_none_or(|b| c.0 < b.0 || (c.0 == b.0 && c.1 < b.1)) {
                            low = Some(c);
                        }
                    }
                    if let Some((zl, _, p)) = low {
                        if body[p] != body[j] && zl < surf[j] - THALWEG_M {
                            t.fail("thalweg_beside", x, y);
                        }
                    }
                }
            }
            let near = (20.0 / s.spacing_m).ceil() as i64;
            let beside = (-near..=near).any(|oy| (-near..=near).any(|ox| {
                let (px, py) = (x as i64 + ox, y as i64 + oy);
                px >= 0 && py >= 0 && ((ox * ox + oy * oy) as f64).sqrt() * s.spacing_m <= 20.0
                    && at(px as u64, py as u64).is_some_and(|c| flow[c] & 16 != 0 && body[c] != body[j] && body[c] > 1)
            }));
            if beside {
                t.beside_m += step;
            }
        }
    }
    // Lake exits in this tile: the outflow's first cell, steps and rims.
    for (&id, &(lv, full, [inside, outside], _)) in lakes {
        if !full {
            continue;
        }
        let (bx, by) = world.xy(outside);
        if !core.contains(bx, by) {
            continue;
        }
        let b = at(bx, by).expect("in");
        let (ix, iy) = world.xy(inside);
        // The cell of the spill edge that carries the saddle shows the level (C1): the inside
        // cell is lake at the level, so the outside one may only stand above it by the 1 cm.
        let first = at(ix, iy).map_or(shown(b), |a| shown(a).max(shown(b)));
        if (first - lv).abs() > SILL_M {
            t.fail("lake_exit_surface", bx, by);
        }
        let (mut prev, mut prev_raw) = (lv, at(ix, iy).map_or(raw[b], |a| raw[a]));
        let (mut cx, mut cy, mut walked) = (bx, by, 0.0f64);
        let mut fall_end = lv;
        while walked <= DAM_REACH_M {
            let Some(c) = at(cx, cy) else { break };
            let drop = prev - dsurf(c);
            if drop > DAM_STEP_M {
                t.drops_any.push((id, [cx, cy], drop));
                if prev_raw - raw[c] < drop - SILL_M {
                    t.fail("dam_steps", cx, cy);
                }
            }
            (prev, prev_raw, fall_end) = (dsurf(c), raw[c], dsurf(c));
            let Some(n) = receiver(cx, cy, c) else { break };
            walked += if n.0 != cx && n.1 != cy { std::f64::consts::SQRT_2 } else { 1.0 } * s.spacing_m;
            (cx, cy) = n;
        }
        t.falls.push((id, lv - fall_end));
        // Rim: dry cells within 2 cells of the exit on its outflow side.
        let out_dir = receiver(bx, by, b).map(|(rx, ry)| (rx as i64 - bx as i64, ry as i64 - by as i64)).unwrap_or((bx as i64 - ix as i64, by as i64 - iy as i64));
        for oy in -2i64..=2 {
            for ox in -2i64..=2 {
                if ox * out_dir.0 + oy * out_dir.1 < 0 {
                    continue;
                }
                let (px, py) = (bx as i64 + ox, by as i64 + oy);
                if px < 0 || py < 0 {
                    continue;
                }
                let Some(p) = at(px as u64, py as u64) else { continue };
                if surf[p].is_nan() && z[p] > lv + DAM_RIM_M {
                    t.rims_any.push((id, [px as u64, py as u64], z[p] - lv));
                    if raw[p] <= lv + DAM_RIM_M {
                        t.fail("dam_rims", px as u64, py as u64);
                    }
                }
            }
        }
    }
    t.runs.sort_by(|a, b| b.0.total_cmp(&a.0));
    t.runs.truncate(20);
    Ok(t)
}

pub(crate) struct CheckExtra {
    pub cycles: u64,
    pub bodies: usize,
    /// Lake cells by lake id (from the body list areas) against their need.
    pub lake_short: u64,
    pub relax: Option<Value>,
    /// The lakes, for the lowest-saddle check over all tiles.
    pub lakes: LakeInfo,
}

/// The report: checks (all must be 0) and figures; `passed`.
pub(crate) fn report(tiles: Vec<Tile>, extra: CheckExtra, mut figures: Map<String, Value>) -> (Value, bool) {
    let mut fails: BTreeMap<&str, u64> = [
        "drainage_rises", "drainage_stuck", "lake_not_flat", "lake_leaks", "lake_exit_surface", "dam_steps",
        "dam_rims", "thalweg_beside", "parallel_rivers", "straight_runs", "walls", "change_max",
    ].into_iter().map(|k| (k, 0)).collect();
    let mut examples: BTreeMap<&str, Vec<[u64; 2]>> = BTreeMap::new();
    let mut hist = vec![0u64; BINS];
    let (mut planar, mut straight, mut river, mut beside, mut ponds, mut max_change) = ([0u64; 2], 0.0, 0.0, 0.0, 0u64, 0.0f32);
    let mut falls = Vec::new();
    let mut saddle: BTreeMap<u32, f32> = BTreeMap::new();
    let (mut runs, mut drops_any, mut rims_any, mut agree, mut closed_ends) = (Vec::new(), Vec::new(), Vec::new(), [0u64; 4], 0u64);
    for t in tiles {
        for (id, s) in &t.saddle {
            let e = saddle.entry(*id).or_insert(f32::INFINITY);
            *e = e.min(*s);
        }
        runs.extend(t.runs.iter().copied());
        drops_any.extend(t.drops_any.iter().copied());
        rims_any.extend(t.rims_any.iter().copied());
        for (a, b) in agree.iter_mut().zip(t.agree) {
            *a += b;
        }
        closed_ends += t.closed_ends;
        for (k, v) in t.fails {
            *fails.entry(k).or_insert(0) += v;
        }
        for (k, v) in t.examples {
            let e = examples.entry(k).or_default();
            e.extend(v.into_iter().take(5 - e.len().min(5)));
        }
        for (h, v) in hist.iter_mut().zip(t.hist) {
            *h += v;
        }
        (planar[0], planar[1]) = (planar[0] + t.planar[0], planar[1] + t.planar[1]);
        (straight, river, beside, ponds) = (straight + t.straight_m, river + t.river_m, beside + t.beside_m, ponds + t.pond_cells);
        max_change = max_change.max(t.max_change);
        falls.extend(t.falls);
    }
    let p99 = p99_m(&hist);
    fails.insert("drainage_cycles", extra.cycles);
    fails.insert("lake_balance", extra.lake_short);
    fails.insert("straight_share", u64::from(river > 0.0 && straight / river > STRAIGHT_SHARE));
    fails.insert("facets", u64::from(planar[1] > planar[0]));
    fails.insert("change_p99", u64::from(p99 > 1.0));
    fails.insert("export_bodies", u64::from(extra.bodies > BODY_BUDGET));
    let overlap = extra.relax.as_ref().and_then(|r| r["max_overlap_diff_m"].as_f64());
    fails.insert("relax_overlap", u64::from(overlap.is_some_and(|d| d > 0.001)));
    // A full lake stands at its lowest saddle (C1).
    let off_sill: Vec<u32> = extra
        .lakes
        .iter()
        .filter(|(id, l)| l.1 && saddle.get(id).is_none_or(|&s| (s - l.0).abs() > SILL_M))
        .map(|(&id, _)| id)
        .collect();
    fails.insert("lake_off_sill", off_sill.len() as u64);
    let agreement = if agree[0] + agree[2] == 0 { 1.0 } else { (agree[1] + agree[3]) as f64 / (agree[0] + agree[2]) as f64 };
    fails.insert("label_agreement", u64::from(agreement < AGREEMENT));
    falls.sort_by(|a, b| b.1.total_cmp(&a.1));
    runs.sort_by(|a: &(f64, [u64; 2], [u64; 2]), b| b.0.total_cmp(&a.0));
    drops_any.sort_by(|a: &(u32, [u64; 2], f32), b| b.2.total_cmp(&a.2));
    rims_any.sort_by(|a: &(u32, [u64; 2], f32), b| b.2.total_cmp(&a.2));
    figures.insert("lakes_off_sill".into(), json!(off_sill.iter().take(20).collect::<Vec<_>>()));
    figures.insert("label_agreement".into(), json!(agreement));
    figures.insert("longest_straight_runs".into(), json!(runs.iter().take(20).map(|r| json!({"length_m": r.0, "from": r.1, "to": r.2})).collect::<Vec<_>>()));
    figures.insert("dam_drops_any".into(), json!({"count": drops_any.len(), "largest": drops_any.iter().take(50).map(|d| json!({"lake": d.0, "cell": d.1, "drop_m": d.2})).collect::<Vec<_>>()}));
    figures.insert("dam_rims_any".into(), json!({"count": rims_any.len(), "highest": rims_any.iter().take(50).map(|d| json!({"lake": d.0, "cell": d.1, "above_level_m": d.2})).collect::<Vec<_>>()}));
    figures.insert("big_rivers_into_closed_nodes".into(), json!(closed_ends));
    figures.insert("straight_share".into(), json!(if river > 0.0 { straight / river } else { 0.0 }));
    figures.insert("river_km_checked".into(), json!(river / 1000.0));
    figures.insert("river_km_beside_another_within_20m".into(), json!(beside / 1000.0));
    figures.insert("planar_windows_raw_vs_result".into(), json!(planar));
    figures.insert("change_p99_m".into(), json!(p99));
    figures.insert("change_max_m".into(), json!(max_change));
    figures.insert("pond_cells".into(), json!(ponds));
    figures.insert("exit_fall_50m_largest".into(), json!(falls.iter().take(10).map(|f| json!({"lake": f.0, "fall_m": f.1})).collect::<Vec<_>>()));
    figures.insert("relax".into(), extra.relax.unwrap_or(Value::Null));
    let failures: Vec<&str> = fails.iter().filter(|(_, &v)| v > 0).map(|(k, _)| *k).collect();
    let passed = failures.is_empty();
    (json!({
        "schema_version": 1, "kind": "water_report", "passed": passed, "failures": failures,
        "checks": fails, "examples": examples, "figures": figures,
    }), passed)
}
```

- [ ] **Step 5: Implement `finish` in `mod.rs`**

Change the `use crate::acquisition::…` line to `use crate::acquisition::{AcquisitionError, Result, ensure, io_context, read_small};`, then add:

```rust
pub struct FinishOptions {
    pub grid: PathBuf,
    pub work: PathBuf,
    /// The final ground: `work/relaxed.f32` after the relaxation (`carved.f32` without it).
    pub ground: PathBuf,
    pub out: PathBuf,
    pub tile: u64,
    pub threads: usize,
    pub water: Option<PathBuf>,
    pub river_km2: Option<f64>,
    /// The relaxation's report (`max_overlap_diff_m`, iterations, halo), if it ran.
    pub relax: Option<PathBuf>,
}

pub struct Outcome {
    pub out: PathBuf,
    pub passed: bool,
    pub summary: String,
}

/// Checks the finished run on the final ground and publishes `out` (also when a check fails,
/// so the report can be read).
pub fn finish(o: &FinishOptions) -> Result<Outcome> {
    ensure(!o.out.exists(), format!("{} already exists", o.out.display()))?;
    let (s, raw, stamp) = setup(&o.grid, o.tile, o.threads, o.water.as_deref(), o.river_km2)?;
    let work = store::Work::open(&o.work, &stamp)?;
    ensure(work.is_done("bodies") && work.path("stages.json").exists(), "run `world water` to the end first")?;
    let rasters = pipeline::Rasters::files(s.world, &work.path(""), "m2")?;
    let breached = layers::Layer::open(s.world, &work.path("breached.f32"))?;
    let m2 = pipeline::model(&s, &breached, Some(&work), "m2", None)?;
    // The third model run, on the final ground (R27); its tag names the ground file, so a
    // finish on another ground never reads stale passes.
    let ground = layers::Layer::open(s.world, &o.ground)?;
    let meta = io_context("inspect the final ground", fs::metadata(&o.ground))?;
    let m3_tag = format!(
        "m3-{}",
        &crate::acquisition::digest(format!("{}:{}:{:?}", o.ground.display(), meta.len(), meta.modified().ok()).as_bytes())[..12]
    );
    let final_rasters = pipeline::Rasters::files(s.world, &work.path(""), &m3_tag)?;
    let m3 = pipeline::model(&s, &ground, Some(&work), &m3_tag, Some(&final_rasters))?;
    let bodies: Value = serde_json::from_slice(&io_context("read bodies.json", fs::read(work.path("bodies.json")))?)?;
    let list = bodies["bodies"].as_array().cloned().unwrap_or_default();
    // Lakes by id from the body list, each with its node (by key) from the model.
    let mut lakes = checks::LakeInfo::new();
    let mut lake_short = 0u64;
    for b in list.iter().filter(|b| b["kind"] == "lake") {
        let id = b["id"].as_u64().unwrap_or(0) as u32;
        let key = b["node"].as_u64().unwrap_or(u64::MAX);
        let n = m2.water.nodes.get(key).ok_or_else(|| AcquisitionError::Invalid(format!("lake {id}: node {key} not in the model")))?;
        lakes.insert(id, (n.level, n.full, n.outlet, n.need));
        let area = b["area_km2"].as_f64().unwrap_or(0.0);
        if n.need > 0 && ((area / s.cell_km2()).round() as u64) < n.need {
            lake_short += 1;
        }
    }
    let layers = checks::CheckLayers {
        raw,
        ground,
        surface: layers::Layer::open(s.world, &work.path("water-surface.f32"))?,
        body: layers::Layer::open(s.world, &work.path("water-body.u32"))?,
        level: final_rasters.level.clone(),
        node: final_rasters.node.clone(),
        flow: final_rasters.flow.clone(),
        discharge: final_rasters.discharge.clone(),
        flow_written: rasters.flow.clone(),
    };
    let tiles = store::for_tiles(&s.tiles(), s.threads, "checks", |_, core| checks::check_tile(&s, &layers, core, &lakes))?;
    let relax = match &o.relax {
        Some(p) => Some(serde_json::from_slice(&io_context("read relax report", fs::read(p))?)?),
        None => None,
    };
    let stages: Value = serde_json::from_slice(&io_context("read stages.json", fs::read(work.path("stages.json")))?)?;
    let n = |kind: &str| list.iter().filter(|e| e["kind"] == kind).count();
    let mut figures = Map::new();
    figures.insert("stages".into(), stages);
    figures.insert("lakes".into(), json!(n("lake")));
    figures.insert("rivers".into(), json!(n("river")));
    figures.insert("river_km".into(), json!(list.iter().filter(|e| e["kind"] == "river").filter_map(|e| e["length_km"].as_f64()).sum::<f64>()));
    let extra = checks::CheckExtra { cycles: m3.cycles, bodies: n("lake") + n("river"), lake_short, relax, lakes };
    let (report, passed) = checks::report(tiles, extra, figures);

    let mut stage = crate::world::hydrology::grid::Stage::new(&o.out)?;
    stage.copy("elevation.f32", &o.ground)?;
    stage.copy("water-surface.f32", &work.path("water-surface.f32"))?;
    stage.copy("water-body.u32", &work.path("water-body.u32"))?;
    stage.copy("discharge.f32", &work.path(&format!("{m3_tag}-discharge.f32")))?;
    stage.copy("flow.u8", &work.path(&format!("{m3_tag}-flow.u8")))?;
    stage.json("bodies.json", &bodies)?;
    stage.json("report.json", &report)?;
    let mut manifest: Map<String, Value> = serde_json::from_slice(&read_small(&o.grid.join("grid.json"))?)?;
    manifest.insert("water_model".into(), json!("hydrology v3"));
    manifest.insert("discharge".into(), json!({
        "file": "discharge.f32",
        "unit": "km² at the mean land precipitation (drainage area weighted by rain / mean)",
        "mean_land_mm_h": s.rain.mean_land_mm_h,
    }));
    manifest.insert("flow".into(), json!({
        "file": "flow.u8",
        "bits": "low 4: D8 receiver (0 none; 1 N, 2 S, 3 W, 4 E, 5 NW, 6 NE, 7 SW, 8 SE); 16 river thalweg; 32 dry basin; 64 endorheic",
    }));
    manifest.insert("provenance".into(), json!({
        "tool": "topography world water", "version": env!("CARGO_PKG_VERSION"),
        "stamp": stamp, "outputs": stage.digests.clone(),
    }));
    stage.json("grid.json", &Value::Object(manifest))?;
    let out = stage.publish()?;
    let summary = format!(
        "{}\nwater {}: {} lakes, {} rivers{}",
        out.display(),
        if passed { "passed" } else { "FAILED" },
        n("lake"),
        n("river"),
        if passed { String::new() } else { format!("\nfailed checks: {}", report["failures"]) },
    );
    Ok(Outcome { out, passed, summary })
}
```

`Layer` needs `Clone` (derived in Task 2). `Stage::new` refuses an existing `out`.

- [ ] **Step 6: Run the tests to see them pass**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water`
Expected: all pass, including `the_synthetic_world_passes_its_checks` and `a_raised_rim_below_a_lake_fails_the_dam_check`. If the synthetic world fails a check, the check output names cells: fix the model, not the threshold.
Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet hydrology` (v2 still green).
Run clippy. Expected: clean.

- [ ] **Step 7: Commit**

Run the task-finish block first.

```bash
git add crates/topography-core/src/world/water crates/topography-core/src/world/hydrology/grid.rs
git commit -m "feat(water): hard checks, figures and water-finish publishing the run

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 8: Expert review**

Expert review protocol. Real-terrain output: rerun Task 12's canyon probe through `finish` (ground = `carved.f32`, no relaxation yet) and give the reviewer `report.json`. Ask them to judge each check's definition against the spec's text and whether any threshold acts as an exemption.

---
## Task 14: CLI commands `world water` and `world water-finish`

**Files:**
- Modify: `crates/topography-cli/src/main.rs` (usage lines after `world hydrology-seam`, two match arms after the `hydrology` arm, two handler functions)
- Modify: `crates/topography-core/src/world/water/mod.rs` (remove `#![allow(dead_code)]`, delete what is still dead)
- Modify: `scripts/check.sh` (real runs on a fixture, usage-error checks)
- Create: `tests/fixtures/water/grid/` (`grid.json`, `elevation.f32`, `moisture.f32`, `water.toml`: the 64² synthetic world, 16 KB of elevation), written once by an `#[ignore]` test in `water/mod.rs`

**Interfaces:**
- Consumes: `water::{Options, FinishOptions, Outcome, run, finish, DEFAULT_TILE}`, `testing::grid_dir`.
- Produces (command lines):
  - `topography world water --grid <grid-dir> --work <work-dir> [--tile 4096] [--threads 4] [--water water.toml] [--river-km2 0.05]` → exit 0 with a summary line; resumable.
  - `topography world water-finish --grid <grid-dir> --work <work-dir> --ground <final-ground.f32> --out <new-output-dir> [--tile 4096] [--threads 4] [--relax relax.json] [--water water.toml] [--river-km2 0.05]` → exit 0 when every check passes, 1 when one fails (outputs still written), 2 on usage errors.

- [ ] **Step 1: Write the fixture and the failing check**

Add to the `mod.rs` tests and run it once to write the fixture (commit its output):

```rust
    /// Writes tests/fixtures/water/grid for scripts/check.sh: cargo test … write_check_fixture -- --ignored
    #[test]
    #[ignore]
    fn write_check_fixture() {
        let dir = std::path::Path::new("../../tests/fixtures/water");
        let _ = std::fs::remove_dir_all(dir.join("grid"));
        testing::grid_dir(dir, geom::World { w: 64, h: 64 });
    }
```

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core world::water::tests::write_check_fixture -- --ignored`

In `scripts/check.sh`, after the `world hydrology` checks (or after `check_success '9 classes…'` if there are none), add. These run the real commands on the fixture and check what they write (N9):

```bash
# Hydrology v3 on a 64² fixture: the commands run and write their outputs.
water_tmp="$(mktemp -d)"
water_grid=tests/fixtures/water/grid
water_args=(--tile 16 --threads 2 --water "$water_grid/water.toml" --river-km2 0.00001)
check_success 'next: relax' world water --grid "$water_grid" --work "$water_tmp/work" "${water_args[@]}"
check_success 'water passed' world water-finish --grid "$water_grid" --work "$water_tmp/work" \
  --ground "$water_tmp/work/carved.f32" --out "$water_tmp/out" "${water_args[@]}"
for f in elevation.f32 water-surface.f32 water-body.u32 discharge.f32 flow.u8 bodies.json report.json grid.json; do
  test -s "$water_tmp/out/$f" || { printf 'world water-finish wrote no %s\n' "$f" >&2; exit 1; }
done
grep -q '"passed": true' "$water_tmp/out/report.json"
rm -rf "$water_tmp"
check_failure 2 'Unsupported arguments' world water --grid
check_failure 2 'Unsupported arguments' world water-finish --grid g --work w
```

Run: `bash scripts/check.sh`
Expected: FAIL at the first new line (`world water` is not a command yet).

- [ ] **Step 2: Implement the commands**

Usage lines (after `world hydrology-seam`):

```text
       topography world water --grid <grid-dir> --work <work-dir> [--tile 4096] [--threads 4] [--water water.toml] [--river-km2 0.05]
       topography world water-finish --grid <grid-dir> --work <work-dir> --ground <ground.f32> --out <new-output-dir> [--tile 4096] [--threads 4] [--relax relax.json] [--water water.toml] [--river-km2 0.05]
```

Match arms (after the `hydrology` arm):

```rust
        [command, action, rest @ ..] if command == "world" && action == "water" => {
            return water(rest);
        }
        [command, action, rest @ ..] if command == "world" && action == "water-finish" => {
            return water_finish(rest);
        }
```

Handlers:

```rust
/// Flags shared by `world water` and `world water-finish`; false on an unknown flag or value.
fn water_flag(
    flag: &std::ffi::OsStr,
    value: &std::ffi::OsStr,
    grid: &mut std::path::PathBuf,
    work: &mut std::path::PathBuf,
    tile: &mut u64,
    threads: &mut usize,
    water: &mut Option<std::path::PathBuf>,
    river_km2: &mut Option<f64>,
) -> bool {
    match flag.to_str() {
        Some("--grid") => *grid = value.into(),
        Some("--work") => *work = value.into(),
        Some("--water") => *water = Some(value.into()),
        Some("--tile") => match number(value) {
            Some(v) => *tile = v,
            None => return false,
        },
        Some("--threads") => match number(value) {
            Some(v) => *threads = v,
            None => return false,
        },
        Some("--river-km2") => match number(value) {
            Some(v) => *river_km2 = Some(v),
            None => return false,
        },
        _ => return false,
    }
    true
}

/// `world water --grid DIR --work DIR [--tile N] [--threads N] [--water FILE] [--river-km2 A]`
fn water(args: &[std::ffi::OsString]) -> ExitCode {
    use topography_core::world::water;
    let mut o = water::Options {
        grid: Default::default(),
        work: Default::default(),
        tile: water::DEFAULT_TILE,
        threads: 4,
        water: None,
        river_km2: None,
    };
    for pair in args.chunks(2) {
        let [flag, value] = pair else { return usage_error() };
        if !water_flag(flag, value, &mut o.grid, &mut o.work, &mut o.tile, &mut o.threads, &mut o.water, &mut o.river_km2) {
            return usage_error();
        }
    }
    if o.grid.as_os_str().is_empty() || o.work.as_os_str().is_empty() {
        return usage_error();
    }
    workflow_result(water::run(&o))
}

/// `world water-finish --grid DIR --work DIR --ground FILE --out DIR [--relax FILE] …`
fn water_finish(args: &[std::ffi::OsString]) -> ExitCode {
    use topography_core::world::water;
    let mut o = water::FinishOptions {
        grid: Default::default(),
        work: Default::default(),
        ground: Default::default(),
        out: Default::default(),
        tile: water::DEFAULT_TILE,
        threads: 4,
        water: None,
        river_km2: None,
        relax: None,
    };
    for pair in args.chunks(2) {
        let [flag, value] = pair else { return usage_error() };
        match flag.to_str() {
            Some("--ground") => o.ground = value.into(),
            Some("--out") => o.out = value.into(),
            Some("--relax") => o.relax = Some(value.into()),
            _ => {
                if !water_flag(flag, value, &mut o.grid, &mut o.work, &mut o.tile, &mut o.threads, &mut o.water, &mut o.river_km2) {
                    return usage_error();
                }
            }
        }
    }
    if [&o.grid, &o.work, &o.ground, &o.out].iter().any(|p| p.as_os_str().is_empty()) {
        return usage_error();
    }
    match water::finish(&o) {
        Ok(outcome) => {
            println!("{}", outcome.summary);
            if outcome.passed {
                ExitCode::SUCCESS
            } else {
                eprintln!("Water checks failed; see {}", outcome.out.join("report.json").display());
                ExitCode::FAILURE
            }
        }
        Err(error) => workflow_result(Err(error)),
    }
}
```

`water_flag` has eight parameters: add `#[allow(clippy::too_many_arguments)]` above it.

Remove `#![allow(dead_code)]` from `water/mod.rs`; delete any item clippy now reports as unused (for example `Hierarchy::is_leaf` if unused — it is used by `balance`; keep what is used).

- [ ] **Step 3: Run the checks**

Run: `"${CAP6[@]}" cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: clean.
Run: `bash scripts/check.sh`
Expected: passes, including the two new lines.
Run on the Task 12 probe grid (a real 2,048² canyon window):

```bash
"${CAP6[@]}" target/debug/topography world water --grid scratch/hydrology-v3/probes/t12-canyon-grid --work scratch/hydrology-v3/t14-work --tile 1024 --threads 4
"${CAP6[@]}" target/debug/topography world water-finish --grid scratch/hydrology-v3/probes/t12-canyon-grid --work scratch/hydrology-v3/t14-work \
    --ground scratch/hydrology-v3/t14-work/carved.f32 --out scratch/hydrology-v3/t14-out --tile 1024 --threads 4; echo "exit $?"
```

Expected: the first prints a summary; the second prints `water passed` or `FAILED` with the failed checks (exit 0 or 1); `scratch/hydrology-v3/t14-out/report.json` exists. A rerun of the first command finishes immediately (all cached).

- [ ] **Step 4: Commit**

Run the task-finish block first.

```bash
git add crates/topography-cli/src/main.rs crates/topography-core/src/world/water scripts/check.sh tests/fixtures/water
git commit -m "feat(cli): world water and world water-finish commands

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 5: Expert review**

Expert review protocol. Real-terrain output: `scratch/hydrology-v3/t14-out/report.json` and `stages.json` for the canyon window. Ask specifically about `thalweg_beside` and `straight_share` on the canyon (the doubled river and E49249 defects).

---

## Task 15: GPU bank relaxation

**Files:**
- Create: `training/src/topography_training/water/__init__.py`
- Create: `training/src/topography_training/water/relax.py`
- Create: `training/tests/test_water_relax.py`

**Interfaces:**
- Consumes: nothing from Rust; arrays as the work directory holds them (`carved.f32`, `elevation-raw` window, `water-surface.f32`, `corridor.u8`, `m2-flow.u8`).
- Produces: `relax.relax(ground, raw, surface, corridor, flow, spacing_m=1.0, device=None) -> np.ndarray` (float32, same shape; only `corridor == 1` cells change, only down), constants `ITERATIONS = 48`, `HALO = 64`, `EDGE_EFFECT = 56` (window-edge effects reach at most this deep: `ITERATIONS` + `TAPER_CELLS`), `STRIP = HALO - EDGE_EFFECT` (8: the overlap strip outside a tile's core that is still exact), `LIMIT_DEG = 38.0`, `MAX_LOWER_M = 2.5`, `RECEIVER_M = 0.001` (a lowered cell stays strictly above its receiver, I7).

- [ ] **Step 1: Write the failing tests**

`training/tests/test_water_relax.py`:

```python
"""Bank relaxation: lowering only, slope limits, determinism and tile overlaps."""

import math
import unittest

import numpy as np

from topography_training.water import relax as R

TAN = math.tan(math.radians(R.LIMIT_DEG))


def channel_case(w=60, h=40):
    """Flat banks at 10 m, a 4-cell channel cut to 7.9 m under water at 8 m, corridor around."""
    ground = np.full((h, w), 10.0, np.float32)
    raw = ground.copy()
    surface = np.full((h, w), np.nan, np.float32)
    corridor = np.zeros((h, w), np.uint8)
    ground[:, 28:32] = 7.9
    surface[:, 28:32] = 8.0
    corridor[:, 20:40] = 1
    corridor[:, 28:32] = 2
    flow = np.zeros((h, w), np.uint8)
    flow[:, 20:28] = 4  # banks drain east, toward the channel
    flow[:, 32:40] = 3  # and west
    return ground, raw, surface, corridor, flow


class RelaxTest(unittest.TestCase):
    def test_banks_are_cut_back_to_the_limit_and_never_raised(self):
        g, raw, s, c, f = channel_case()
        out = R.relax(g, raw, s, c, f)
        self.assertTrue((out <= g + 1e-6).all(), "raised")
        self.assertTrue((out[c == 0] == g[c == 0]).all(), "moved outside the corridor")
        self.assertTrue((out[c == 2] == g[c == 2]).all(), "moved a channel bed")
        self.assertTrue((out >= raw - R.MAX_LOWER_M - 1e-5).all())
        self.assertTrue((out[:, 27] >= 8.0 - 1e-5).all(), "a bank below the water beside it")
        # The wall above the channel is no steeper than the limit.
        self.assertTrue(((out[:, 27] - out[:, 28]) <= TAN + 1e-3).all(), out[0, 20:30])
        # Every lowered bank cell stays at least 1 mm above its receiver (I7).
        moved = (out < g) & (c == 1)
        east = np.roll(out, -1, axis=1)
        west = np.roll(out, 1, axis=1)
        recv = np.where(f == 4, east, np.where(f == 3, west, -np.inf))
        self.assertTrue((out[moved] >= recv[moved] + R.RECEIVER_M - 1e-6).all())

    def test_a_raw_cliff_is_never_steepened(self):
        g, raw, s, c, f = channel_case()
        cliff = np.where(np.arange(60) < 24, 14.0, 10.0).astype(np.float32)[None, :].repeat(40, 0)
        g, raw = np.minimum(g, cliff) + (cliff - 10.0), raw + (cliff - 10.0)
        out = R.relax(g, raw, s, c, f)
        raw_step = raw[:, 23] - raw[:, 24]
        self.assertTrue(((out[:, 23] - out[:, 24]) <= raw_step + 1e-4).all())

    def test_runs_are_deterministic_and_tiles_agree(self):
        rng = np.random.default_rng(3)
        g, raw, s, c, f = channel_case(w=300, h=50)
        g = (g + rng.random(g.shape, np.float32) * 0.3).astype(np.float32)
        raw = g.copy()
        g[:, 28:32] -= 2.0
        # A corridor from column 10 to 200: the noisy banks move everywhere in it, right up to
        # the window cut at 30 + HALO, so a halo too small would change the core (N9).
        c[:, 10:200] = np.where(c[:, 10:200] == 2, 2, 1)
        whole = R.relax(g, raw, s, c, f)
        self.assertTrue(np.array_equal(whole, R.relax(g, raw, s, c, f)))
        self.assertTrue((whole[:, 90:100] < g[:, 90:100]).any(), "cells move near the cut")
        core = slice(0, 30)
        win = slice(0, 30 + R.HALO)
        part = R.relax(g[:, win], raw[:, win], s[:, win], c[:, win], f[:, win])
        self.assertTrue(np.array_equal(part[:, core], whole[:, core]), "core differs")
        strip = slice(30, 30 + R.STRIP)
        self.assertLessEqual(float(np.abs(part[:, strip] - whole[:, strip]).max()), 1e-3)


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run the tests to see them fail**

Run (repository root, `P=data/models/diffusion-env-v1/.venv/bin/python`): `PYTHONPATH=training/src $P -m unittest discover -s training/tests -p 'test_water_relax.py' -v`
Expected: `ModuleNotFoundError: topography_training.water`.

- [ ] **Step 3: Implement**

`training/src/topography_training/water/__init__.py`:

```python
"""Hydrology v3 at 1 m: GPU bank relaxation (`relax`), the resumable world driver (`run`) and
review figures (`review`). The exact graph passes are the Rust `topography world water`."""
```

`training/src/topography_training/water/relax.py`:

```python
"""Bank relaxation inside the 1 m channel corridor (hydrology v3), on the GPU.

Only dry corridor cells (corridor.u8 == 1) move, and only down. Each of ITERATIONS Jacobi
steps smooths them toward the mean of their eight neighbours and cuts back any slope to a
neighbour steeper than max(LIMIT_DEG, the cell's raw slope), never below the water beside
them, their receiver + RECEIVER_M (strictly above it, so drainage still holds and no flat
forms; review I7), or raw - MAX_LOWER_M tapered to 0 over TAPER_CELLS at the corridor's edge.
The stencil is shifted slices of an edge-padded tensor combined in a fixed order, never
conv2d (whose MIOpen algorithm depends on the shape), with deterministic algorithms on; no
atomics, no reductions across cells. A cell's result depends only on cells within
ITERATIONS of it, so tiles with a HALO >= ITERATIONS x 1 agree on their overlaps.
"""

from __future__ import annotations

import math

import numpy as np
import torch
from scipy import ndimage

ITERATIONS = 48
RADIUS = 1
TAPER_CELLS = 8
HALO = 64  # >= ITERATIONS * RADIUS
EDGE_EFFECT = ITERATIONS * RADIUS + TAPER_CELLS  # how deep a window edge can change results
STRIP = HALO - EDGE_EFFECT  # overlap cells outside a tile's core that are still exact
LIMIT_DEG = 38.0
ALPHA = 0.25
MAX_LOWER_M = 2.5  # the 2 m channel cap + 0.5 m (the change budget's max)
RECEIVER_M = 0.001  # a lowered cell stays at least this far above its receiver (strict)
# (row, column) offsets in flow.u8 order: 1 N, 2 S, 3 W, 4 E, 5 NW, 6 NE, 7 SW, 8 SE.
OFFSETS = ((-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (-1, 1), (1, -1), (1, 1))

assert HALO >= ITERATIONS * RADIUS and STRIP > 0


def _shift(a: torch.Tensor, dy: int, dx: int) -> torch.Tensor:
    """a[y + dy, x + dx] with the edges repeated."""
    p = torch.nn.functional.pad(a[None, None], (1, 1, 1, 1), mode="replicate")[0, 0]
    h, w = a.shape
    return p[1 + dy : 1 + dy + h, 1 + dx : 1 + dx + w]


def relax(ground: np.ndarray, raw: np.ndarray, surface: np.ndarray, corridor: np.ndarray, flow: np.ndarray,
          spacing_m: float = 1.0, device: str | None = None) -> np.ndarray:
    """The relaxed ground (float32) of one window."""
    move_np = corridor == 1
    if not move_np.any():
        return np.asarray(ground, np.float32).copy()
    dev = torch.device(device or ("cuda" if torch.cuda.is_available() else "cpu"))
    torch.use_deterministic_algorithms(True)
    t = lambda a, dtype=torch.float32: torch.from_numpy(np.ascontiguousarray(a)).to(dev, dtype)  # noqa: E731
    g, r = t(np.asarray(ground, np.float32)), t(np.asarray(raw, np.float32))
    ws = t(np.where(np.isfinite(surface), surface, -np.inf).astype(np.float32))
    move = t(move_np, torch.bool)
    direction = t(np.asarray(flow) & 15, torch.int64)
    dist = [spacing_m * (math.sqrt(2.0) if dy and dx else 1.0) for dy, dx in OFFSETS]
    # Static limits: the slope allowed (never past raw), the water beside, the taper.
    raw_slope = torch.stack([(r - _shift(r, dy, dx)) / d for (dy, dx), d in zip(OFFSETS, dist)]).amax(0)
    limit = torch.clamp(raw_slope, min=math.tan(math.radians(LIMIT_DEG)))
    bank = torch.stack([_shift(ws, dy, dx) for dy, dx in OFFSETS]).amax(0)
    edge = ndimage.distance_transform_cdt(corridor > 0, metric="chessboard").astype(np.float32)
    taper = t(np.clip(edge / TAPER_CELLS, 0.0, 1.0))
    floor = torch.maximum(bank, r - taper * MAX_LOWER_M)
    weights = [1.0 / d for d in dist]
    total = sum(weights)
    for _ in range(ITERATIONS):
        nbs = [_shift(g, dy, dx) for dy, dx in OFFSETS]
        # Fixed-order sum of shifted slices (no conv2d): the same bits in any window.
        mean = sum(w * n for w, n in zip(weights, nbs)) / total
        smooth = g + ALPHA * (mean - g)
        cone = torch.stack([n + limit * d for n, d in zip(nbs, dist)]).amin(0)
        target = torch.minimum(g, torch.minimum(smooth, cone))
        recv = torch.full_like(g, -math.inf)
        for k, n in enumerate(nbs):
            recv = torch.where(direction == k + 1, n, recv)
        low = torch.minimum(torch.maximum(floor, recv + RECEIVER_M), g)  # strictly above the receiver; never raise
        g = torch.where(move, torch.maximum(target, low), g)
    return g.cpu().numpy()
```

- [ ] **Step 4: Run the tests to see them pass**

Run: same command as Step 2.
Expected: 3 tests OK. If `torch.use_deterministic_algorithms(True)` raises for an op on ROCm, replace that op with an elementwise equivalent (never turn determinism off).

- [ ] **Step 5: Commit**

Run the task-finish block first.

```bash
git add training/src/topography_training/water training/tests/test_water_relax.py
git commit -m "feat(water): deterministic GPU bank relaxation inside the channel corridor

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Expert review**

Expert review protocol. Real-terrain output: relax the Task 14 canyon run (`scratch/hydrology-v3/t14-work`) in one window with a short `"${CAP6[@]}" $P -c` call (6 GB cap) that memory-maps `carved.f32`, the raw window from the grid's `elevation.f32`, `water-surface.f32`, `corridor.u8` and `m2-flow.u8`, writes `relaxed.f32`, then runs `water-finish` with `--ground …/relaxed.f32` into `scratch/hydrology-v3/t15-out`. Give the reviewer both reports and a hillshade pair (carved vs relaxed) from `review.py` once Task 17 exists, or `cascade_eval.shade` now.

---

## Task 16: The world driver

**Files:**
- Create: `training/src/topography_training/water/run.py`
- Create: `training/tests/test_water_run.py`

**Interfaces:**
- Consumes: `relax.relax`, `relax.HALO`, `relax.STRIP`; `diffusion.hydrology.{pin_binary, sha256, check_disk, WATER_FILL}`; `diffusion.cascade.{save_json, block_mean}`; `diffusion.cascade_chain._png`; the Rust commands of Task 14.
- Produces:
  - `run.build_world_grid(src: Path, out: Path) -> Path`: `<out>/hydrology/grid` with `elevation.f32` → `../../elevation-raw.f32` (symlink), `classes.u8` → the source world's 4 m classes (symlink, `classes_cell_m` 4), `moisture.f32` → the source's, and `grid.json` (size, spacing 1, origin, the `moisture` entry, `discharge.mean_land_mm_h` from the source world's `hydrology/4m/grid.json`).
  - `run.build_window_grid(src: Path, level: int, centre: tuple[float, float], size: int, dest: Path) -> Path` (levels 1, 4, 32; see Step 3).
  - `run.relax_tiles(grid: Path, work: Path, tile: int) -> dict` (writes `work/relaxed.f32` and `work/relax.json` with `iterations`, `halo`, `strip`, `tiles_relaxed`, `max_overlap_diff_m`; resumable by per-tile markers).
  - `run.run_world(src: Path, out: Path, tile: int, threads: int) -> int` and `run.run_window(src, level, centre, size, out, tile, threads) -> int` (exit codes: 0 passed, 1 checks failed, 2 refused).
  - CLI: `python -m topography_training.water.run world --source <l4 world> --out <new world> [--tile 4096] [--threads 4]` and `… window --source <world> --level {1,4,32} --centre E,N --size N --out <dir> [--tile N] [--threads N]`.
  - World layout written: `elevation-raw.f32` (reflink), `elevation.f32`, `water-surface.f32`, `water-body.u32`, `bodies.json` (reflinked copies of `hydrology/1m`'s, never hard links, N17), `water.u8` (255 where the surface is finite), `snow-ice.u8`, `vegetation.u8` (reflinks), `moisture.f32`, `world-input/world.json`, `description.json`/`.md` (copied), `overview.png`, `detail.png`, `progress.json`, and `manifest.json` (or `manifest-failed.json` when a check failed) with `kind: world_preview`, `source_world` (the source's own source world, so export finds the 4 m hydrology), `hydrology_input`, `water`, `moisture`, `layers`.

- [ ] **Step 1: Write the failing test**

`training/tests/test_water_run.py`:

```python
"""World driver end to end on a tiny fake L4 world (needs the release topography binary)."""

import json
import tempfile
import unittest
from pathlib import Path

import numpy as np

# The driver reflinks (`cp --reflink=always`, N12), which /tmp (tmpfs) cannot: test worlds
# live in the repository's scratch directory, on the btrfs /home.
SCRATCH = Path(__file__).resolve().parents[2] / "scratch" / "test-water"
SCRATCH.mkdir(parents=True, exist_ok=True)

from topography_training.water import run as W


def fake_world(root: Path, n: int = 256) -> Path:
    """An L4-shaped world: elevation-raw.f32, a 4 m source world with classes and a 4 m
    hydrology grid.json (for mean rain), moisture, snow/vegetation, world-input."""
    chain = root / "chain"
    (chain / "hydrology" / "4m").mkdir(parents=True)
    np.full((n // 4, n // 4), 18, np.uint8).tofile(chain / "classes.u8")
    (chain / "hydrology" / "4m" / "grid.json").write_text(json.dumps({"discharge": {"mean_land_mm_h": 2.0}}))
    world = root / "l4"
    (world / "world-input").mkdir(parents=True)
    y, x = np.mgrid[0:n, 0:n].astype(np.float32)
    z = 40.0 - 0.15 * x + 3.0 * np.sin(x / 9.0) * np.cos(y / 11.0) - 6.0 * np.exp(-((x - 80) ** 2 + (y - 80) ** 2) / 300.0)
    z.astype("<f4").tofile(world / "elevation-raw.f32")
    np.ones((2, 2), "<f4").tofile(world / "moisture.f32")
    for name in ("snow-ice.u8", "vegetation.u8"):
        np.zeros((n, n), np.uint8).tofile(world / name)
    (world / "world-input" / "world.json").write_text(json.dumps({"name": "fake", "origin": [0.0, float(n)]}))
    manifest = {
        "schema_version": 1, "kind": "world_preview", "spacing_m": 1.0, "size": [n, n],
        "world_input": str(world / "world-input"), "source_world": str(chain), "class_id": "per cell",
        "moisture": {"file": "moisture.f32", "size": [2, 2], "spacing_m": n / 2, "nw_m": [0.0, float(n)],
                     "scale_mm_h": 2.0, "dtype": "float32"},
        "hydrology_input": {"file": "elevation-raw.f32"},
        "layers": {},
    }
    (world / "manifest.json").write_text(json.dumps(manifest))
    return world


class RunTest(unittest.TestCase):
    def test_a_world_runs_resumes_and_is_assembled(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as tmp:
            root = Path(tmp)
            src = fake_world(root)
            out = root / "v3"
            code = W.run_world(src, out, tile=64, threads=2)
            self.assertIn(code, (0, 1))
            manifest = out / ("manifest.json" if code == 0 else "manifest-failed.json")
            m = json.loads(manifest.read_text())
            self.assertEqual(m["source_world"], str(root / "chain"))
            for name in ("elevation.f32", "water-surface.f32", "water-body.u32", "bodies.json", "water.u8",
                         "snow-ice.u8", "vegetation.u8", "moisture.f32", "overview.png"):
                self.assertTrue((out / name).exists(), name)
            relax = json.loads((out / "hydrology" / "work" / "relax.json").read_text())
            self.assertLessEqual(relax["max_overlap_diff_m"], 1e-3)
            raw = np.fromfile(out / "elevation-raw.f32", "<f4")
            self.assertTrue(np.array_equal(raw, np.fromfile(src / "elevation-raw.f32", "<f4")), "raw kept")
            # A rerun resumes and changes nothing.
            before = (out / "elevation.f32").read_bytes()
            self.assertEqual(W.run_world(src, out, tile=64, threads=2), code)
            self.assertEqual((out / "elevation.f32").read_bytes(), before)

    def test_a_window_grid_is_cut_from_the_raw_surface(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as tmp:
            src = fake_world(Path(tmp))
            grid = W.build_window_grid(src, 1, (128.0, 128.0), 64, Path(tmp) / "g")
            g = json.loads((grid / "grid.json").read_text())
            self.assertEqual(g["size"], [64, 64])
            self.assertEqual(g["offset_m"], [96.0, 96.0])
            z = np.fromfile(grid / "elevation.f32", "<f4").reshape(64, 64)
            raw = np.fromfile(src / "elevation-raw.f32", "<f4").reshape(256, 256)
            self.assertTrue(np.array_equal(z, raw[96:160, 96:160]))


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `PYTHONPATH=training/src $P -m unittest discover -s training/tests -p 'test_water_run.py' -v`
Expected: `ImportError` (no `run` module).

- [ ] **Step 3: Implement `run.py`**

```python
"""Hydrology v3 at 1 m: the water model on a world's kept raw surface (or a test window).

python -m topography_training.water.run world --source <l4 world> --out <new world> [--tile 4096] [--threads 4]
python -m topography_training.water.run window --source <world> --level {1,4,32} --centre E,N --size N --out <dir>

The Rust `topography world water` runs the exact graph passes into <out>/hydrology/work
(resumable); the bank relaxation runs here on the GPU tile by tile (resumable); `world
water-finish` checks the result and publishes <out>/hydrology/1m, and the world's layers link
to it. Rerunning the same command resumes. Never writes to the source world.
See docs/workflows/terrain-cascade.md ("Hydrology v3").
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path

import numpy as np

from ..diffusion import cascade, hydrology
from . import relax as R

RAW = "elevation-raw.f32"
# Disk per 1 m cell while running (N14): 12 work rasters of 4 B (breached, carved, relaxed,
# m2 level/node/discharge, body, water-surface, water-body, m3 level/node/discharge), 3 of
# 1 B (m2 flow, m3 flow, corridor) = 51 B, plus the world's water.u8 (1 B); published copies
# are reflinks. Required free: this, times 1.1, plus 10 GB.
DISK_BYTES = 52
DISK_MARGIN = 1.1
DISK_SPARE = 10e9


def _json(path: Path) -> dict:
    return json.loads(path.read_text())


def _moisture(src: Path, manifest: dict) -> tuple[dict, float]:
    """The source's moisture entry and the mean land rain discharge is weighted by, from the 4 m
    hydrology of the chain world (the source itself, or the source's source world)."""
    entry = dict(manifest["moisture"])
    for world in (src, Path(manifest.get("source_world") or src)):
        grid4 = world / "hydrology" / "4m" / "grid.json"
        if grid4.exists():
            return entry, float(_json(grid4)["discharge"]["mean_land_mm_h"])
    raise FileNotFoundError(f"no hydrology/4m/grid.json in {src} or its source world")


def _symlink(target: Path, link: Path) -> None:
    link.unlink(missing_ok=True)
    link.symlink_to(target.resolve())


def build_world_grid(src: Path, out: Path) -> Path:
    """The 1 m grid over the whole world: links to the reflinked raw surface, the 4 m classes and
    the moisture field."""
    m = _json(src / "manifest.json")
    n = m["size"][0]
    grid = out / "hydrology" / "grid"
    grid.mkdir(parents=True, exist_ok=True)
    _symlink(out / RAW, grid / "elevation.f32")
    chain = Path(m["source_world"])
    _symlink(chain / "classes.u8", grid / "classes.u8")
    _symlink(src / "moisture.f32", grid / "moisture.f32")
    entry, mean = _moisture(src, m)
    origin = _json(Path(m["world_input"]) / "world.json").get("origin", [0.0, float(n)])
    (grid / "grid.json").write_text(json.dumps({
        "size": [n, n], "spacing_m": 1.0, "origin": origin, "classes_cell_m": 4.0,
        "moisture": entry | {"file": "moisture.f32"}, "discharge": {"mean_land_mm_h": mean},
    }, indent=1))
    return grid


def build_window_grid(src: Path, level: int, centre: tuple[float, float], size: int, dest: Path) -> Path:
    """A test window at level 1 (the L4 raw surface), 4 (the chain world's ground) or 32 (the
    chain's 32 m `l2_raw`), centred on (east, north) metres."""
    m = _json(src / "manifest.json")
    dest.mkdir(parents=True, exist_ok=True)
    if level == 1:
        n, spacing = m["size"][0], 1.0
        field = np.memmap(src / RAW, "<f4", "r", shape=(n, n))
        chain = Path(m["source_world"])
        classes, k = np.memmap(chain / "classes.u8", np.uint8, "r", shape=(n // 4, n // 4)), 4
    elif level == 4:
        n, spacing = m["size"][0], 4.0
        field = np.memmap(src / "elevation.f32", "<f4", "r", shape=(n, n))
        classes, k = np.memmap(src / "classes.u8", np.uint8, "r", shape=(n, n)), 1
    elif level == 32:
        field = np.load(src / "arrays.npz")["l2_raw"]
        n, spacing = field.shape[0], 32.0
        c4 = np.memmap(src / "classes.u8", np.uint8, "r", shape=(n * 8, n * 8))
        # Most common 4 m class in each 32 m cell.
        blocks = np.asarray(c4).reshape(n, 8, n, 8).transpose(0, 2, 1, 3).reshape(n, n, 64)
        classes, k = np.apply_along_axis(lambda b: np.bincount(b, minlength=256).argmax(), 2, blocks).astype(np.uint8), 1
    else:
        raise ValueError(f"level {level}: use 1, 4 or 32")
    origin = _json(Path(m["world_input"]) / "world.json").get("origin", [0.0, n * spacing])
    col = int(round((centre[0] - origin[0]) / spacing - size / 2)) // k * k
    row = int(round((origin[1] - centre[1]) / spacing - size / 2)) // k * k
    col, row = min(max(col, 0), n - size), min(max(row, 0), n - size)
    np.ascontiguousarray(field[row : row + size, col : col + size], "<f4").tofile(dest / "elevation.f32")
    np.ascontiguousarray(classes[row // k : (row + size) // k, col // k : (col + size) // k], np.uint8).tofile(dest / "classes.u8")
    entry, mean = _moisture(src, m)
    _symlink(src / "moisture.f32", dest / "moisture.f32")
    (dest / "grid.json").write_text(json.dumps({
        "size": [size, size], "spacing_m": spacing, "origin": origin,
        "offset_m": [col * spacing, row * spacing], "classes_cell_m": spacing * k,
        "moisture": entry | {"file": "moisture.f32"}, "discharge": {"mean_land_mm_h": mean},
    }, indent=1))
    return dest


def _bin(out: Path) -> Path:
    path, digest = hydrology.pin_binary(out / "hydrology" / "bin")
    print(f"water binary: {path} (sha256 {digest[:12]})", flush=True)
    return path


def _rust(cmd: list[str]) -> int:
    print("$", " ".join(cmd), flush=True)
    return subprocess.run(cmd).returncode


def relax_tiles(grid: Path, work: Path, tile: int) -> dict:
    """Relax every tile whose window holds corridor cells into work/relaxed.f32 (a copy of
    carved.f32 elsewhere); then compare each tile's exact overlap strip with its neighbours'
    results. Resumes after the tiles already marked done."""
    g = _json(grid / "grid.json")
    w, h = g["size"]
    spacing = float(g["spacing_m"])
    _reflink(work / "carved.f32", work / "relaxed.f32")
    mm = lambda name, dtype, mode="r": np.memmap(work / name if name != "raw" else grid / "elevation.f32", dtype, mode, shape=(h, w))  # noqa: E731
    carved, raw = mm("carved.f32", "<f4"), mm("raw", "<f4")
    surface, corridor, flow = mm("water-surface.f32", "<f4"), mm("corridor.u8", np.uint8), mm("m2-flow.u8", np.uint8)
    relaxed = mm("relaxed.f32", "<f4", "r+")
    marks = work / "relax"
    marks.mkdir(exist_ok=True)
    tiles = [(y, x) for y in range(0, h, tile) for x in range(0, w, tile)]
    done, started = 0, time.time()
    for i, (y0, x0) in enumerate(tiles):
        if (marks / f"{i:05d}.done").exists():
            continue
        y1, x1 = min(y0 + tile, h), min(x0 + tile, w)
        wy0, wx0 = max(y0 - R.HALO, 0), max(x0 - R.HALO, 0)
        wy1, wx1 = min(y1 + R.HALO, h), min(x1 + R.HALO, w)
        win = (slice(wy0, wy1), slice(wx0, wx1))
        c = np.asarray(corridor[win])
        if (c == 1).any():
            res = R.relax(np.asarray(carved[win]), np.asarray(raw[win]), np.asarray(surface[win]), c,
                          np.asarray(flow[win]), spacing)
            relaxed[y0:y1, x0:x1] = res[y0 - wy0 : y1 - wy0, x0 - wx0 : x1 - wx0]
            # The exact strip just outside the core, kept for the overlap check.
            sy0, sx0 = max(y0 - R.STRIP, 0), max(x0 - R.STRIP, 0)
            sy1, sx1 = min(y1 + R.STRIP, h), min(x1 + R.STRIP, w)
            np.save(marks / f"{i:05d}.npy", res[sy0 - wy0 : sy1 - wy0, sx0 - wx0 : sx1 - wx0])
            (marks / f"{i:05d}.json").write_text(json.dumps([sy0, sy1, sx0, sx1, y0, y1, x0, x1]))
            done += 1
        relaxed.flush()
        (marks / f"{i:05d}.done").write_text("")
        if (i + 1) % 16 == 0:
            print(f"relax: {i + 1}/{len(tiles)} tiles ({time.time() - started:.0f} s)", flush=True)
    relaxed.flush()
    worst = 0.0
    for meta in sorted(marks.glob("*.json")):
        sy0, sy1, sx0, sx1, y0, y1, x0, x1 = json.loads(meta.read_text())
        strip = np.load(meta.with_suffix(".npy"))
        final = np.asarray(relaxed[sy0:sy1, sx0:sx1])
        ring = np.ones(strip.shape, bool)
        ring[y0 - sy0 : y1 - sy0, x0 - sx0 : x1 - sx0] = False
        if ring.any():
            worst = max(worst, float(np.abs(strip[ring] - final[ring]).max()))
    report = {"iterations": R.ITERATIONS, "halo": R.HALO, "strip": R.STRIP, "limit_deg": R.LIMIT_DEG,
              "tiles": len(tiles), "tiles_relaxed": len(list(marks.glob("*.json"))),
              "max_overlap_diff_m": worst}
    cascade.save_json(work / "relax.json", report)
    print(f"relax: done, overlap max diff {worst:.6f} m", flush=True)
    return report


def _finish(binary: Path, grid: Path, work: Path, out1m: Path, tile: int, threads: int,
            relaxed: bool = True) -> int:
    """`world water-finish` into a fresh `out1m`: on the relaxed ground with its overlap report,
    or (coarse windows, no relaxation) on the carved ground. One command builder (N16)."""
    if out1m.exists():
        shutil.rmtree(out1m)
    ground = work / ("relaxed.f32" if relaxed else "carved.f32")
    cmd = [str(binary), "world", "water-finish", "--grid", str(grid), "--work", str(work),
           "--ground", str(ground), "--out", str(out1m), "--tile", str(tile), "--threads", str(threads)]
    if relaxed:
        cmd += ["--relax", str(work / "relax.json")]
    return _rust(cmd)


def _water_u8(surface: Path, dest: Path, n: int) -> None:
    s = np.memmap(surface, "<f4", "r", shape=(n, n))
    tmp = dest.with_suffix(".tmp")
    o = np.memmap(tmp, np.uint8, "w+", shape=(n, n))
    for y in range(0, n, 1024):
        o[y : y + 1024] = np.where(np.isfinite(s[y : y + 1024]), 255, 0)
    o.flush()
    del o
    tmp.replace(dest)


def _reflink(src: Path, dst: Path, replace: bool = False) -> None:
    """A btrfs reflink of `src` at `dst` (R18, N12): no space, and nothing written to one can
    reach the other. Fails loudly where the file system cannot reflink: a 17 GB copy is never
    made silently. Kept unless `replace`."""
    if dst.exists() and not replace:
        return
    tmp = dst.with_suffix(dst.suffix + ".tmp")
    done = subprocess.run(["cp", "--reflink=always", str(src), str(tmp)], capture_output=True, text=True)
    if done.returncode != 0:
        raise SystemExit(f"error: cannot reflink {src} to {tmp} ({done.stderr.strip()}); "
                         "the world must live on the same btrfs file system as its source")
    tmp.replace(dst)


def _publish(src_dir: Path, dst_dir: Path, names) -> dict[str, str]:
    """Reflinked copies (never hard links, N17) of finished files into a world; their SHA-256."""
    for name in names:
        _reflink(src_dir / name, dst_dir / name, replace=True)
    return {name: hydrology.sha256(dst_dir / name) for name in names}


def run_world(src: Path, out: Path, tile: int, threads: int) -> int:
    m = _json(src / "manifest.json")
    n = m["size"][0]
    if m.get("spacing_m") != 1.0 or not (src / RAW).exists():
        print(f"error: {src} is not a 1 m world with a kept {RAW}", file=sys.stderr)
        return 2
    if out.resolve() == src.resolve():
        print("error: --out must be a new world, not the source", file=sys.stderr)
        return 2
    out.mkdir(parents=True, exist_ok=True)
    progress_path = out / "progress.json"
    progress = _json(progress_path) if progress_path.exists() else {"source": str(src.resolve()), "steps": []}
    if progress["source"] != str(src.resolve()):
        print(f"error: {out} was started from {progress['source']}", file=sys.stderr)
        return 2
    save = lambda: cascade.save_json(progress_path, progress)  # noqa: E731
    hydrology.check_disk(out, n * n * DISK_BYTES * DISK_MARGIN + DISK_SPARE, "the 1 m water run")
    if "raw" not in progress["steps"]:
        _reflink(src / RAW, out / RAW)
        digest = hydrology.sha256(out / RAW)
        want = (m.get("hydrology_input") or {}).get("sha256")
        if want and digest != want:
            print(f"error: {RAW} does not match the source's recorded sha256", file=sys.stderr)
            return 2
        progress["raw_sha256"] = digest
        progress["steps"].append("raw")
        save()
    grid = build_world_grid(src, out)
    binary = _bin(out)
    work = out / "hydrology" / "work"
    if _rust([str(binary), "world", "water", "--grid", str(grid), "--work", str(work),
              "--tile", str(tile), "--threads", str(threads)]) != 0:
        return 2
    relax = relax_tiles(grid, work, tile)
    out1m = out / "hydrology" / "1m"
    code = _finish(binary, grid, work, out1m, tile, threads)
    if code not in (0, 1):
        return 2
    report = _json(out1m / "report.json")
    # The world: water layers from the 1 m output, the rest from the source.
    digests = _publish(out1m, out, ("elevation.f32", "water-surface.f32", "water-body.u32", "bodies.json"))
    _water_u8(out / "water-surface.f32", out / "water.u8", n)
    for name in ("snow-ice.u8", "vegetation.u8"):
        _reflink(src / name, out / name)
    digests |= {name: hydrology.sha256(out / name) for name in ("water.u8", "snow-ice.u8", "vegetation.u8")}
    digests |= _publish(src, out, ("moisture.f32",))
    world_input = out / "world-input"
    world_input.mkdir(exist_ok=True)
    shutil.copy(Path(m["world_input"]) / "world.json", world_input / "world.json")
    extra = {}
    if "description" in m:
        for name in ("description.json", "description.md"):
            shutil.copy(src / name, out / name)
        digests["description.json"] = hydrology.sha256(out / "description.json")
        extra["description"] = "description.json"
    from ..diffusion.cascade_chain import _png

    over = np.concatenate([cascade.block_mean(np.asarray(np.memmap(out / "elevation.f32", "<f4", "r", shape=(n, n))[y : y + 1024]), 8)
                           for y in range(0, n, 1024)])
    wover = np.concatenate([cascade.block_mean(np.asarray(np.memmap(out / "water.u8", np.uint8, "r", shape=(n, n))[y : y + 1024], np.float32), 8)
                            for y in range(0, n, 1024)]) > 127
    _png(out / "overview.png", over, 8.0, wover, 2048)
    mid, r = n // 2, min(512, n // 2)
    e = np.memmap(out / "elevation.f32", "<f4", "r", shape=(n, n))
    wm = np.memmap(out / "water.u8", np.uint8, "r", shape=(n, n))
    _png(out / "detail.png", np.asarray(e[mid - r : mid + r, mid - r : mid + r]), 1.0,
         np.asarray(wm[mid - r : mid + r, mid - r : mid + r]) == 255, 1024)
    manifest = {
        "schema_version": 1, "kind": "world_preview", "world_input": str(world_input.resolve()),
        "spacing_m": 1.0, "size": [n, n], "class_id": m.get("class_id"),
        "method": "hydrology v3 1 m water over the kept L4 raw surface of the source world",
        "water_fill": hydrology.WATER_FILL, "moisture": m["moisture"], **extra,
        **{k: m[k] for k in ("layout",) if k in m},
        "source_world": m["source_world"], "hydrology_source": str(src.resolve()),
        "hydrology_input": {"file": RAW, "sha256": progress["raw_sha256"],
                            "rerun": "python -m topography_training.water.run world --source <source> --out <this world>"},
        "water": {"model": "hydrology v3", "report": "hydrology/1m/report.json", "passed": report["passed"],
                  "failures": report["failures"], "relax": relax, "binary": str(binary.relative_to(out)),
                  "tile": tile},
        "layers": digests,
    }
    name = "manifest.json" if report["passed"] else "manifest-failed.json"
    cascade.save_json(out / name, manifest)
    (out / ("manifest-failed.json" if report["passed"] else "manifest.json")).unlink(missing_ok=True)
    progress["steps"] = sorted(set(progress["steps"]) | {"world"})
    save()
    print(f"done: {out} ({'passed' if report['passed'] else 'FAILED: ' + ', '.join(report['failures'])})", flush=True)
    return 0 if report["passed"] else 1


def run_window(src: Path, level: int, centre: tuple[float, float], size: int, out: Path, tile: int, threads: int) -> int:
    grid = build_window_grid(src, level, centre, size, out / "grid")
    binary = _bin(out)
    work = out / "work"
    if _rust([str(binary), "world", "water", "--grid", str(grid), "--work", str(work),
              "--tile", str(tile), "--threads", str(threads)]) != 0:
        return 2
    if level == 1:
        relax_tiles(grid, work, tile)
    return _finish(binary, grid, work, out / "out", tile, threads, relaxed=level == 1)


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = p.add_subparsers(dest="mode", required=True)
    w = sub.add_parser("world", help="the whole 1 m world into a new world directory")
    w.add_argument("--source", type=Path, required=True, help="L4 world with a kept elevation-raw.f32")
    w.add_argument("--out", type=Path, required=True, help="new world directory (resumed if it exists)")
    w.add_argument("--tile", type=int, default=4096)
    w.add_argument("--threads", type=int, default=4)
    v = sub.add_parser("window", help="a test window at 1 m, 4 m or 32 m")
    v.add_argument("--source", type=Path, required=True, help="1 m: the L4 world; 4 and 32 m: the chain world")
    v.add_argument("--level", type=int, choices=(1, 4, 32), required=True)
    v.add_argument("--centre", required=True, help="E,N in metres")
    v.add_argument("--size", type=int, required=True, help="cells along a side")
    v.add_argument("--out", type=Path, required=True)
    v.add_argument("--tile", type=int)
    v.add_argument("--threads", type=int, default=4)
    a = p.parse_args(argv)
    if a.mode == "world":
        return run_world(a.source, a.out, a.tile, a.threads)
    e, n = (float(t) for t in a.centre.split(","))
    return run_window(a.source, a.level, (e, n), a.size, a.out, a.tile or a.size, a.threads)


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `PYTHONPATH=training/src $P -m unittest discover -s training/tests -p 'test_water_*.py' -v`
Expected: 5 tests OK (the first run builds the release binary; that is a Rust build, no warning needed).

- [ ] **Step 5: Commit**

Run the task-finish block first.

```bash
git add training/src/topography_training/water/run.py training/tests/test_water_run.py
git commit -m "feat(water): resumable 1 m world driver with relaxation and world assembly

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Expert review**

Expert review protocol. Real-terrain output: `PYTHONPATH=training/src "${CAP6[@]}" $P -m topography_training.water.run window --source data/worlds/fujix-55-l4v2 --level 1 --centre 49249,16381 --size 2048 --tile 512 --threads 4 --out scratch/hydrology-v3/t16-e49249` (6 GB cap): its `out/report.json`, `work/relax.json` and a hillshade of `work/relaxed.f32` with water from `work/water-surface.f32` (use `cascade_eval.shade`). This is a small run (4 M cells, CPU minutes, GPU seconds) and needs no warning.

---
## Task 17: Review figures: hillshade crops, straight-run crops and the coarse guide

The coarse guide is a figure only in Plan A (R17). Plan B makes "level over 5 m, gone or doubled" a hard check once the 4 m water is v3's own.

**Files:**
- Create: `training/src/topography_training/water/review.py`
- Create: `training/tests/test_water_review.py`
- Modify: `training/src/topography_training/water/run.py` (call both after `water-finish`; record in the manifest)

**Interfaces:**
- Consumes: a finished world (Task 16) or a window run directory; the 4 m hydrology output (`water-body.u32`, `discharge.f32`, `bodies.json`, `grid.json`); `diffusion.cascade_eval.shade`.
- Produces:
  - `review.SPOTS` (the spec's eleven E/N pairs), `review.CROP_M = 512`.
  - `review.crops(world: Path, dest: Path, n: int, origin: list[float], spots=SPOTS) -> list[Path]`: one PNG per spot (reads `elevation-raw.f32`, `elevation.f32`, `water-body.u32`, `bodies.json` of `world`), raw hillshade beside the result (lakes and sea blue, rivers cyan), named `E<e>-N<n>.png`.
  - `review.window_pair(run_dir: Path, dest: Path) -> Path`: the same pair for a window run (`grid/elevation.f32` vs `work/relaxed.f32`, else `work/carved.f32`).
  - `review.coarse_guide(world: Path, coarse: Path, fine_dir: Path | None = None) -> dict` (`fine_dir` holds the 1 m `water-body.u32`, `bodies.json` and `grid.json`; the world itself when None): `lakes_compared`, `level_diff_m` (by coarse lake id), `lakes_over_5m`, `lakes_gone`, `lakes_doubled`, `coarse_river_share_with_fine_river_within_100m`, `hard_failures` (over 5 m, gone or doubled). Written to `<world>/hydrology/1m/coarse-guide.json` by the driver and summarised in the manifest's `water.coarse_guide` (ruling R17: recorded, explained at gate 2, not used to withhold `manifest.json`).

- [ ] **Step 1: Write the failing test**

`training/tests/test_water_review.py`:

```python
"""Coarse guide on a tiny pair of worlds; crops on a small world."""

import json
import tempfile
import unittest
from pathlib import Path

import numpy as np

from topography_training.water import review


def write(dirpath: Path, n: int, body: np.ndarray, bodies: list, discharge: np.ndarray | None = None, spacing=1.0):
    dirpath.mkdir(parents=True, exist_ok=True)
    body.astype("<u4").tofile(dirpath / "water-body.u32")
    (dirpath / "bodies.json").write_text(json.dumps({"bodies": bodies}))
    (dirpath / "grid.json").write_text(json.dumps({"size": [n, n], "spacing_m": spacing}))
    if discharge is not None:
        discharge.astype("<f4").tofile(dirpath / "discharge.f32")


class CoarseGuideTest(unittest.TestCase):
    def case(self, fine_level: float, split: bool = False):
        tmp = Path(tempfile.mkdtemp())
        cb = np.zeros((16, 16), np.uint32)
        cb[4:8, 4:8] = 2  # a 4 m lake
        cb[10, 2:14] = 3  # a 4 m river
        cq = np.where(cb == 3, 1.0, 0.0)
        write(tmp / "coarse", 16, cb, [{"id": 2, "kind": "lake", "level_m": 100.0}, {"id": 3, "kind": "river"}], cq, 4.0)
        fb = np.zeros((64, 64), np.uint32)
        fb[16:32, 16:32] = 5
        if split:
            fb[16:32, 24:32] = 6
        fb[41, 8:56] = 7
        bodies = [{"id": 5, "kind": "lake", "level_m": fine_level}, {"id": 6, "kind": "lake", "level_m": fine_level},
                  {"id": 7, "kind": "river"}]
        write(tmp / "fine", 64, fb, bodies)
        return review.coarse_guide(tmp / "fine", tmp / "coarse", fine_dir=tmp / "fine")

    def test_matching_lakes_and_rivers_pass(self):
        g = self.case(101.0)
        self.assertEqual(g["hard_failures"], 0)
        self.assertEqual(g["level_diff_m"]["2"], 1.0)
        self.assertEqual(g["coarse_river_share_with_fine_river_within_100m"], 1.0)

    def test_gross_level_or_doubled_lakes_fail(self):
        self.assertEqual(self.case(106.0)["lakes_over_5m"], [2])
        self.assertEqual(self.case(100.0, split=True)["lakes_doubled"], [2])


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run the test to see it fail**

Run: `PYTHONPATH=training/src $P -m unittest discover -s training/tests -p 'test_water_review.py' -v`
Expected: `ImportError`.

- [ ] **Step 3: Implement `review.py`**

```python
"""Review figures for hydrology v3 at 1 m: hillshade crops at the owner's spots (the raw surface
beside the result, water tinted) and the coarse guide: the 1 m water against the 4 m world's
(figures; a lake more than 5 m off its 4 m level, gone or doubled is a hard failure, spec
"Checks")."""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np
from scipy import ndimage

from ..diffusion.cascade_eval import shade

# The owner's spots (spec "Review"), east and north in metres.
SPOTS = ((25645, 24400), (25307, 24585), (20071, 30501), (27169, 44546), (43966, 21633), (49249, 16381),
         (47962, 36851), (48148, 36810), (48810, 19452), (51610, 20478), (42581, 56193))
CROP_M = 512
LEVEL_GROSS_M = 5.0
DOUBLED_SHARE = 0.25
RIVER_NEAR_M = 100.0
RIVER_KM2_4M = 0.5


def _kinds(dirpath: Path) -> tuple[dict[int, str], dict[int, float]]:
    bodies = json.loads((dirpath / "bodies.json").read_text())["bodies"]
    return ({b["id"]: b["kind"] for b in bodies},
            {b["id"]: float(b["level_m"]) for b in bodies if b.get("level_m") is not None})


def _lookup(kinds: dict[int, str], kind: str) -> np.ndarray:
    top = max(kinds, default=0) + 1
    table = np.zeros(top, bool)
    for i, k in kinds.items():
        table[i] = k == kind
    return table


def _pair(raw: np.ndarray, result: np.ndarray, lake: np.ndarray, river: np.ndarray) -> np.ndarray:
    left = shade(raw, 1.0, np.zeros(raw.shape, bool))
    right = shade(result, 1.0, lake)
    right[river] = (right[river] * np.array([0.4, 0.85, 0.95])).astype(np.uint8)
    gap = np.full((raw.shape[0], 8, 3), 255, np.uint8)
    return np.concatenate([left, gap, right], axis=1)


def crops(world: Path, dest: Path, n: int, origin: list[float], spots=SPOTS) -> list[Path]:
    from PIL import Image

    mm = lambda name, dt: np.memmap(world / name, dt, "r", shape=(n, n))  # noqa: E731
    raw, elev, body = mm("elevation-raw.f32", "<f4"), mm("elevation.f32", "<f4"), mm("water-body.u32", "<u4")
    kinds, _ = _kinds(world)
    is_river = _lookup(kinds, "river")
    dest.mkdir(parents=True, exist_ok=True)
    out = []
    half = CROP_M // 2
    for e, nn in spots:
        col, row = int(e - origin[0]), int(origin[1] - nn)
        r0, c0 = min(max(row - half, 0), n - CROP_M), min(max(col - half, 0), n - CROP_M)
        win = (slice(r0, r0 + CROP_M), slice(c0, c0 + CROP_M))
        b = np.asarray(body[win])
        river = is_river[np.minimum(b, len(is_river) - 1)] & (b < len(is_river))
        lake = (b > 0) & ~river
        img = _pair(np.asarray(raw[win]), np.asarray(elev[win]), lake, river)
        path = dest / f"E{e}-N{nn}.png"
        Image.fromarray(img).save(path)
        out.append(path)
    return out


def window_pair(run_dir: Path, dest: Path) -> Path:
    from PIL import Image

    g = json.loads((run_dir / "grid" / "grid.json").read_text())
    w, h = g["size"]
    work = run_dir / "work"
    ground = work / "relaxed.f32" if (work / "relaxed.f32").exists() else work / "carved.f32"
    raw = np.fromfile(run_dir / "grid" / "elevation.f32", "<f4").reshape(h, w)
    result = np.fromfile(ground, "<f4").reshape(h, w)
    body = np.fromfile(work / "water-body.u32", "<u4").reshape(h, w)
    flow = np.fromfile(work / "m2-flow.u8", np.uint8).reshape(h, w)
    surface = np.fromfile(work / "water-surface.f32", "<f4").reshape(h, w)
    river = (flow & 16 != 0) | (np.isfinite(surface) & (body > 1) & ~(np.fromfile(work / "m2-node.u32", "<u4").reshape(h, w) >= 2))
    lake = np.isfinite(surface) & ~river
    dest.parent.mkdir(parents=True, exist_ok=True)
    Image.fromarray(_pair(raw, result, lake, river)).save(dest)
    return dest


def coarse_guide(world: Path, coarse: Path, fine_dir: Path | None = None) -> dict:
    """`fine_dir` holds the 1 m water-body.u32 and bodies.json (the world, or its hydrology/1m)."""
    fine_dir = fine_dir or world
    cg = json.loads((coarse / "grid.json").read_text())
    cn = cg["size"][0]
    fg = json.loads((fine_dir / "grid.json").read_text()) if (fine_dir / "grid.json").exists() else \
        json.loads((world / "manifest.json").read_text())
    fn = fg["size"][0]
    k = fn // cn
    cb = np.memmap(coarse / "water-body.u32", "<u4", "r", shape=(cn, cn))
    cq = np.memmap(coarse / "discharge.f32", "<f4", "r", shape=(cn, cn))
    fb = np.memmap(fine_dir / "water-body.u32", "<u4", "r", shape=(fn, fn))
    ckinds, clevels = _kinds(coarse)
    fkinds, flevels = _kinds(fine_dir)
    f_lake, f_river = _lookup(fkinds, "lake"), _lookup(fkinds, "river")
    # Coarse lakes: bounding boxes by streaming rows.
    boxes: dict[int, list[int]] = {}
    for y0 in range(0, cn, 1024):
        rows = np.asarray(cb[y0 : y0 + 1024])
        for i in np.unique(rows):
            if ckinds.get(int(i)) != "lake":
                continue
            ys, xs = np.nonzero(rows == i)
            b = boxes.setdefault(int(i), [cn, cn, -1, -1])
            b[:] = [min(b[0], y0 + ys.min()), min(b[1], xs.min()), max(b[2], y0 + ys.max()), max(b[3], xs.max())]
    diff, over, gone, doubled = {}, [], [], []
    for i, (r0, c0, r1, c1) in sorted(boxes.items()):
        mask = np.asarray(cb[r0 : r1 + 1, c0 : c1 + 1]) == i
        fine = np.asarray(fb[r0 * k : (r1 + 1) * k, c0 * k : (c1 + 1) * k])
        inside = np.kron(mask, np.ones((k, k), bool))
        ids, counts = np.unique(fine[inside], return_counts=True)
        lakes = [(int(c), int(j)) for j, c in zip(ids, counts) if j < len(f_lake) and f_lake[j]]
        if not lakes:
            gone.append(i)
            continue
        area = int(inside.sum())
        if sum(1 for c, _ in lakes if c > DOUBLED_SHARE * area) >= 2:
            doubled.append(i)
        _, best = max(lakes)
        d = abs(flevels[best] - clevels[i])
        diff[str(i)] = round(d, 3)
        if d > LEVEL_GROSS_M:
            over.append(i)
    # Coarse rivers: a fine river cell within 100 m (a k x k block max, then a square dilation).
    near = int(round(RIVER_NEAR_M / float(cg["spacing_m"])))
    has = np.zeros((cn, cn), bool)
    for y0 in range(0, cn, 256):
        rows = np.asarray(fb[y0 * k : (y0 + 256) * k])
        riv = f_river[np.minimum(rows, len(f_river) - 1)] & (rows < len(f_river))
        has[y0 : y0 + rows.shape[0] // k] = riv.reshape(rows.shape[0] // k, k, cn, k).any(axis=(1, 3))
    has = ndimage.maximum_filter(has, size=2 * near + 1)
    c_river = _lookup(ckinds, "river")
    hits = total = 0
    for y0 in range(0, cn, 1024):
        rows, q = np.asarray(cb[y0 : y0 + 1024]), np.asarray(cq[y0 : y0 + 1024])
        sel = c_river[np.minimum(rows, len(c_river) - 1)] & (rows < len(c_river)) & (q >= RIVER_KM2_4M)
        total += int(sel.sum())
        hits += int((sel & has[y0 : y0 + 1024]).sum())
    return {
        "lakes_compared": len(boxes), "level_diff_m": diff, "lakes_over_5m": over, "lakes_gone": gone,
        "lakes_doubled": doubled,
        "coarse_river_share_with_fine_river_within_100m": hits / total if total else 1.0,
        "hard_failures": len(over) + len(gone) + len(doubled),
    }
```

In `run.py`, in `run_world`, after the pictures (`detail.png`) and before `manifest = {…}`:

```python
    from . import review

    coarse = Path(m["source_world"]) / "hydrology" / "4m"
    guide = review.coarse_guide(out, coarse, fine_dir=out1m) if (coarse / "water-body.u32").exists() else None
    if guide is not None:
        cascade.save_json(out1m / "coarse-guide.json", guide)
    origin = _json(Path(m["world_input"]) / "world.json").get("origin", [0.0, float(n)])
    shots = review.crops(out, out1m / "review", n, origin) if n >= review.CROP_M else []
    # The 20 longest straight runs get crops too (review I6), centred on each run's middle.
    runs = report["figures"].get("longest_straight_runs", [])
    run_spots = [(int(origin[0] + (r["from"][0] + r["to"][0]) / 2), int(origin[1] - (r["from"][1] + r["to"][1]) / 2))
                 for r in runs]
    if n >= review.CROP_M and run_spots:
        shots += review.crops(out, out1m / "review" / "straight-runs", n, origin, spots=run_spots)
```

and add to the manifest's `"water"` entry:

```python
                  "coarse_guide": {k: guide[k] for k in ("hard_failures", "lakes_over_5m", "lakes_gone",
                                   "lakes_doubled", "coarse_river_share_with_fine_river_within_100m")} if guide else None,
                  "review": [str(p.relative_to(out)) for p in shots],
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `PYTHONPATH=training/src $P -m unittest discover -s training/tests -p 'test_water_*.py' -v`
Expected: all OK (7 tests).

- [ ] **Step 5: Commit**

Run the task-finish block first.

```bash
git add training/src/topography_training/water training/tests/test_water_review.py
git commit -m "feat(water): review crops at the owner's spots and the coarse-guide figures

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Expert review**

Expert review protocol. Real-terrain output, run as `PYTHONPATH=training/src "${CAP6[@]}" $P -c …` (6 GB cap): `review.window_pair` for the Task 16 window (`scratch/hydrology-v3/t16-e49249`) and the coarse guide of that window against `data/worlds/fujix-55-chain/hydrology/4m` (pass `fine_dir` = the window's `out`; add the window's offset when comparing, or skip the guide for windows and say so).

---

## Task 18: Gate 1: unit-scale windows, the 16,384² exactness run, docs

**Files:**
- Modify: `docs/workflows/terrain-cascade.md` (new section "Hydrology v3 (1 m)" after "Hydrology v2"; the contents link in the intro if there is one)
- Output only (not committed): `scratch/hydrology-v3/gate1/`

**Interfaces:**
- Consumes: everything above.
- Produces: gate-1 evidence (window reports and crops at 32 m, 4 m and 1 m; the exactness comparison; measured time and memory) and the workflow documentation.

- [ ] **Step 1: Windows at 1 m (small, no warning needed)**

```bash
P=data/models/diffusion-env-v1/.venv/bin/python; G=scratch/hydrology-v3/gate1
CAP6=(systemd-run --user --scope -q -p MemoryMax=6G)
for s in 25645,24400 25307,24585 20071,30501 27169,44546 43966,21633 49249,16381 47962,36851 48810,19452 51610,20478 42581,56193; do
  PYTHONPATH=training/src "${CAP6[@]}" $P -m topography_training.water.run window --source data/worlds/fujix-55-l4v2 \
      --level 1 --centre $s --size 2048 --tile 512 --threads 4 --out $G/1m/E${s%,*}-N${s#*,} | tail -n 3
done
```

(E48148 N36810 lies inside the E47962 N36851 window.) Expected: each prints `water passed` or `FAILED` with the failed checks; every window has `out/report.json`.

- [ ] **Step 2: Windows at 4 m and 32 m**

```bash
for s in 25645,24400 25307,24585 20071,30501 27169,44546 43966,21633; do
  PYTHONPATH=training/src "${CAP6[@]}" $P -m topography_training.water.run window --source data/worlds/fujix-55-chain \
      --level 4 --centre $s --size 1024 --threads 4 --out $G/4m/E${s%,*}-N${s#*,} | tail -n 3
  PYTHONPATH=training/src "${CAP6[@]}" $P -m topography_training.water.run window --source data/worlds/fujix-55-chain \
      --level 32 --centre $s --size 256 --threads 4 --out $G/32m/E${s%,*}-N${s#*,} | tail -n 3
done
```

Note in the gate record: the 4 m input is v2's carved 4 m ground (the raw L3 surface was not kept), so these windows show the model's behaviour, not v3's 4 m result (Plan B). They are checked with their own level's thresholds from `water-v2.toml` (`[levels.4]`, `[levels.32]`; `change_max` = breach cap + 0.5 m, N19). The spec's coarse change budget (real interiors outside breaches, p99 ≤ 1 m, max ≤ 3 m) needs the provenance layer and comes with Plan B. Gate 1 passes on the 1 m windows' hard checks; the 4 m and 32 m reports are read by the reviewer, check by check, as evidence of the model's behaviour.

- [ ] **Step 3: Pictures**

```bash
PYTHONPATH=training/src "${CAP6[@]}" $P -c "
from pathlib import Path
from topography_training.water import review
for d in sorted(Path('$G').glob('*/E*')):
    print(review.window_pair(d, Path('$G/png') / f'{d.parent.name}-{d.name}.png'))"
```

- [ ] **Step 3b: Lake count and pit sizes before gate 2 (review C2.4)**

The canyon, dam and dune windows ran in Step 1. Add an alpine window: the 2 km block with the most class 6 (alpine ridges and cirques) in the chain world's 4 m classes.

```bash
ALP=$(PYTHONPATH=training/src "${CAP6[@]}" $P -c "
import numpy as np
c = np.memmap('data/worlds/fujix-55-chain/classes.u8', np.uint8, 'r', shape=(16384, 16384))
b = (np.asarray(c[::4, ::4]) == 6).reshape(32, 128, 32, 128).sum(axis=(1, 3))
r, k = np.unravel_index(b.argmax(), b.shape)
print(f'{k * 2048 + 1024},{65536 - (r * 2048 + 1024)}')")
PYTHONPATH=training/src "${CAP6[@]}" $P -m topography_training.water.run window --source data/worlds/fujix-55-l4v2 \
    --level 1 --centre $ALP --size 2048 --tile 512 --threads 4 --out $G/1m/alpine | tail -n 3
PYTHONPATH=training/src "${CAP6[@]}" $P -c "
import json, pathlib
for name in ['E27169-N44546', 'E25645-N24400', 'E47962-N36851', 'alpine']:
    s = json.loads(pathlib.Path('$G/1m', name, 'work/stages.json').read_text())
    per_km2 = (s['lakes'] + s['rivers']) / 4.194
    print(name, 'nodes', s['nodes'], 'lakes', s['lakes'], 'rivers', s['rivers'], 'pools', s['river_pools'],
          'bodies/km2', round(per_km2, 1), 'world (4,295 km2) upper bound', round(per_km2 * 4295))
    print('  area log2 m2', s['node_area_log2_m2_hist'])
    print('  depth dm', s['node_depth_dm_hist'])"
```

Record the histograms and the extrapolation in `TASKS.md`. The world bound is high, since the world is largely sea, but if even the densest window extrapolates over about 40,000 lakes + rivers, stop before gate 2. Then show the owner the histograms and agree a larger `lake_min_m2` (a game setting in `water-v2.toml`, R7). Never loosen `export_bodies`. Rivers count as bodies too; their number follows the 0.05 km² river start, as v2 had about 15,000.

- [ ] **Step 4: HEAVY — warn the user, then the 16,384² exactness run**

Tell the user: two CPU runs of the Rust water model on a 16,384² (268 M cells) window at the canyon: one as a single tile (peak RAM expected 6–9 GB, uncertain: the global graph size depends on the pit density and the contraction share measured in Tasks 5 and 8b), one as 16 tiles with 4 threads (expected 3–5 GB); each tens of minutes (uncertain); about 20 GB of scratch disk each. Ask them to close memory-heavy applications and wait for the go.

On I/O (review M4): every pass recomputes receivers from the ground, so a model run reads the ground about seven times (passes A, B, L, C, D, E, F). With three model runs, breach, channels and checks, the world reads its 17 GB ground about 25 times: roughly 425 GB, about an hour at 2 GB/s. This plan accepts that and states it. If the 16k timings show I/O dominating, caching the receivers per tile (`u8`, 4.3 GB for the world) is the optimisation to add, behind the same tests.

Then:

```bash
X=scratch/hydrology-v3/gate1/x16k; mkdir -p $X
PYTHONPATH=training/src "${CAP6[@]}" $P -c "
from pathlib import Path
from topography_training.water import run
run.build_window_grid(Path('data/worlds/fujix-55-l4v2'), 1, (27169.0, 44546.0), 16384, Path('$X/grid'))"
( systemd-run --user --scope -q -p MemoryMax=10G target/release/topography world water --grid $X/grid --work $X/whole \
    --tile 16384 --threads 1 > $X/whole.log 2>&1; echo $? > $X/whole.exit ) &
echo $! > $X/whole.pid
```

Watch it per the heavy-run protocol; when `whole.exit` exists and is 0, run the tiled one the same way, in the same 10 GB cap, with `--work $X/tiled --tile 4096 --threads 4` (logs `tiled.log`, `tiled.exit`, `tiled.pid`). Then compare:

```bash
for f in breached.f32 m2-level.f32 m2-node.u32 m2-discharge.f32 m2-flow.u8 body.u32 carved.f32 water-surface.f32 water-body.u32 corridor.u8 bodies.json; do
  cmp -s $X/whole/$f $X/tiled/$f && echo "same $f" || echo "DIFFERENT $f"
done
grep -h 'peak\|done in' $X/whole.log $X/tiled.log
```

Expected: `same` for every file. A difference is a bug in the tiling (Task 9's synthetic test missed a case): find the first differing cell, reproduce it in a synthetic unit test, fix, rerun both.

Record in `TASKS.md`: both runs' wall times and peaks, `stages.json` `leaves`, `depressions`, `settled_trees_solved_in_tiles` and `nodes`, and the extrapolation to the world. The world has 16 times the cells, and the global graph scales with the leaves left after contraction. If the extrapolated global graph would not fit the 10 GB cap, stop and report before gate 2 (review C3): the contraction share measured here is the deciding number.

- [ ] **Step 5: Write the docs section**

Add to `docs/workflows/terrain-cascade.md` after "## Hydrology v2" a section "## Hydrology v3 (1 m)" covering, in the file's style:
- what it is (one water field; exact tiled passes; Rust graph passes, GPU relaxation) and that it replaces the band-by-band `--hydrology-only` rerun for 1 m worlds;
- the commands: `water.run world` and `water.run window`, and the two Rust commands for running by hand;
- the grid directory (`grid.json` with `moisture` and `discharge.mean_land_mm_h`) and the work directory files;
- the outputs (world layers unchanged; `hydrology/1m` with `discharge.f32`, `flow.u8`, `report.json`, `coarse-guide.json`, `review/`);
- the checks table (copy from Task 13) and the rulings that define them (link this plan);
- resuming, the memory cap and the measured time and memory from Step 4;
- `water-v2.toml` (`r`).

Verify the commands in the section by running `--help` of each and checking every path exists.

- [ ] **Step 6: Commit**

Run the task-finish block first.

```bash
git add docs/workflows/terrain-cascade.md
git commit -m "docs(water): hydrology v3 1 m workflow, checks and measured cost

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 7: Expert review (gate 1)**

Expert review protocol with: all window reports (`$G/*/E*/out/report.json`), the pictures in `$G/png`, the exactness comparison output and the 16k runs' `stages.json`. Gate 1 passes when the exactness comparison is all `same`, the 1 m windows pass their hard checks (failures at the dam spots caused by steps the raw surface carries are expected to pass via the raw-step rule; any other failure is a bug to fix first), and the reviewer reports no blocking problem. Record the verdict in `TASKS.md`; show the user the pictures of the canyon, E49249 and the facet spot (E47962 N36851) before Task 19.

---

## Task 19: Gate 2: the whole 1 m world, game layers and export

**Files:**
- Output only: `data/worlds/fujix-55-l4v3-water/` (new world), `data/exports/fujix-55-l4v3-water/` (new export), `scratch/hydrology-v3/gate2/`
- Modify: `TASKS.md` (gate record; not committed)

**Interfaces:**
- Consumes: the driver (Task 16/17), `game_layers`, `topography export heightmap`.
- Produces: the gate-2 world and export, its report and crops, and a gate note.

Expectation, stated before the run (spec staging 2): **the dams baked into `elevation-raw.f32` stay.** The three dam spots (E25645 N24400, E25307 N24585, E20071 N30501) and the E43966 staircase come from the 32 m and 4 m v2 stages and are in the raw surface; this run cannot remove them and is not judged on them. It proves tiling, run time and memory, and the fixes for the canyon (E27169 N44546), E49249 N16381 and the facets (E47962 N36851, E48148 N36810).

- [ ] **Step 1: HEAVY — warn the user**

Tell the user, with the gate-1 measurements: the whole 1 m world (4.3 G cells) through the Rust passes (two model runs, breach, bodies and channels, then a third model run and the checks in `water-finish`) on 4 threads in a 10 GB memory cap, then the GPU relaxation of every tile with a channel corridor (16 GB VRAM ceiling, expected under 2 GB per tile), then world assembly and pictures. Duration: the gate-1 tiled 16k time × 16 plus the global steps (state the figure and its uncertainty). Disk: about 220 GB new on `/home` (52 B per cell; the driver requires that × 1.1 + 10 GB free and stops otherwise; 589 GB were free at planning time). The source world `fujix-55-l4v2` is only read. Wait for the go.

- [ ] **Step 2: Launch**

```bash
RUN=scratch/hydrology-v3/gate2; mkdir -p $RUN
( systemd-run --user --scope -q -p MemoryMax=10G env PYTHONPATH=training/src \
    data/models/diffusion-env-v1/.venv/bin/python -m topography_training.water.run world \
    --source data/worlds/fujix-55-l4v2 --out data/worlds/fujix-55-l4v3-water --tile 4096 --threads 4 \
    > $RUN/log 2>&1; echo $? > $RUN/exit_code ) &
echo $! > $RUN/pid
```

Watch per the heavy-run protocol (Monitor on `water: |relax: |done|Traceback|Error|error|OOM|Killed|FAILED`, hourly heartbeat with progress, rate and remaining time). On a crash or kill, rerun the same command: it resumes.

- [ ] **Step 3: Read the result**

```bash
W=data/worlds/fujix-55-l4v3-water
cat $RUN/exit_code
jq '{passed, failures, checks}' $W/hydrology/1m/report.json
jq '.figures | {stages, lakes, rivers, river_km, straight_share, change_p99_m, change_max_m, pond_cells, relax}' $W/hydrology/1m/report.json
jq . $W/hydrology/1m/coarse-guide.json | head -40
ls $W/hydrology/1m/review/
```

Check that the raw-independent dam figure sees the dams (R31, review I5): `jq '.figures.dam_drops_any' $W/hydrology/1m/report.json` must list cells within 100 m of each of E25645 N24400, E25307 N24585, E20071 N30501 and E43966 N21633 (row = 65,536 − N). If one is missing, the detector is wrong: fix it before going on. This figure is Plan B's baseline; the hard `dam_steps` check passes here only because those steps are in the raw surface.

Exit 0: all hard checks pass. Exit 1: list each failed check with its examples; a failure that is not explained by the raw surface is a bug: reproduce it in a window run (Task 16's `window` mode at the example cell), fix it with a test, rerun (the work directory resumes only if the binary's inputs stamp is unchanged; a code fix changes the version stamp only if `CARGO_PKG_VERSION` changes — remove `hydrology/work` passes after the fix's stage, or the whole work dir, deliberately).

Write the gate note in `TASKS.md`. Cover:
- per spot, what the crop shows: canyon, one river on the floor; E49249, a meandering river with no straight run; E47962/E48148, no raised box or cone facets; the three dam spots and E43966, the raw steps remain, as expected, and appear in `dam_drops_any`;
- the 20 longest straight runs with their crops (`review/straight-runs/`);
- the coarse-guide mismatches, each with its cause (R17: figures only here, such as v2's slot-lowered 4 m lakes and lip rims);
- the label agreement, lakes, ponds, pools and bodies against the 50,000 budget;
- time, peak memory and disk.

Closing note (review M3): **passing gate 2 does not mean the dams are solved.** The dams are in the raw L4 surface and need the regeneration. That is Plan B: v3 water at 32 m and 4 m with its hand-off checks, `dam_drops_any` hard on the 4 m hand-off, the coarse guide hard, and the L4 test strip at the dam spots and the canyon with the owner's Yes before the 35–41 h L4 run.

- [ ] **Step 3b: Show the user** the eleven crops (`$W/hydrology/1m/review/*.png`) and the gate note. The owner's Yes is not required for gate 2 by the spec, but the owner reviews spots; record their comments.

- [ ] **Step 4: Game layers**

```bash
( systemd-run --user --scope -q -p MemoryMax=10G env PYTHONPATH=training/src \
    data/models/diffusion-env-v1/.venv/bin/python -m topography_training.diffusion.game_layers \
    --world data/worlds/fujix-55-l4v3-water --layout data/worlds/fujix-collage-55 > $RUN/game.log 2>&1; echo $? > $RUN/game.exit ) &
echo $! > $RUN/game.pid
```

This is a CPU job over the 1 m world (it took minutes to tens of minutes for l4v2; check its log there); warn the user briefly if they have not already agreed to the gate-2 session as a whole. Expected: exit 0 and `game/*.u8` with digests in `manifest.json`. If the world has `manifest-failed.json` only (checks failed), stop: `game_layers` and export need `manifest.json`.

- [ ] **Step 5: Export**

```bash
( systemd-run --user --scope -q -p MemoryMax=10G target/release/topography export heightmap \
    data/worlds/fujix-55-l4v3-water data/exports/fujix-55-l4v3-water > $RUN/export.log 2>&1; echo $? > $RUN/export.exit ) &
echo $! > $RUN/export.pid
```

Expected: exit 0; the export's checks pass (water bodies under 65,535, guaranteed by `export_bodies` at 50,000; levels finite). Compare the export's `manifest.json` water figures with the l4v2 export's (`data/exports/` lists it, if one exists) and note differences.

- [ ] **Step 6: Expert review (gate 2)**

Expert review protocol with the report, coarse guide, crops, gate note and the export log. Gate 2 passes when every hard check passes, the export succeeds, the reviewer reports no blocking problem, and the crops show the canyon, E49249 and facet fixes. Record the verdict. Do not start Plan B work.

---

## Self-review

**Spec coverage** (spec section → task):

| Spec item | Task |
| --- | --- |
| One library, one set of rules per level; ground never raised; routing surface only for routing | 5–9 (model never writes ground), 11–12 (only lowering), checks in 13 |
| Priority-flood spill levels and nested hierarchy; Barnes 2016 tiling; hierarchy built per tile and merged | 5 (label graph), 6 (hierarchy, seam joins, sorted `Vec`s), 8b (settled trees per tile, R25), 9 (pass L) |
| Hybrid breach-or-fill, graded gorge ≥ 20 × depth, ≥ 3 cells, level caps, breach figures | 11 (R9, R10) |
| r per class, A_lake = r · A_catch with upstream overflow, FSM over the hierarchy across tiles, r column replaces shares, lagoons | 1 (R1, R2), 7 (R3, R20), 9 (sea/lagoon), R8 |
| Fall-line receivers (Horn, dithered) | 3 |
| Lakes and flats as one node; dry flats routed through the tile-edge graph | 8 (priority flood from the outlet, Barnes 2014 mask as tie-break, I2), 9 (ponds gathered across tiles, R7, R21) |
| Barnes 2017 accumulation; rain = 128 m moisture bilinear × cell area; Q in km² at mean rain; no injection | 2 (rain), 4 (port solve), 9 |
| Labels: lake ≥ 0.3 m (and ≥ 500 m², C2), river Q threshold (0.05 km² at 1 m), sea; pools on a river labelled as the river | 7 (`is_lake`), 9, 10, 11 (`pools`) (R6, R7, R16, R26) |
| Outputs in today's formats (`bodies.json`, `water-body.u32`, `water-surface.f32`, flow layer) | 10, 13, 16 (R15) |
| Exact graph passes in Rust on the CPU; iterative passes on the GPU, deterministic, no atomics | 2–13 (Rust), 15 (torch) |
| 1 m tiling with halos, no frozen band, no seam rules | 2 (tiles), 9, 11, 12, 13 (halos) |
| 1 m inputs: `elevation-raw.f32` and 128 m moisture; nothing from 4 m | 16 (grid), 9 |
| Channels: w = 3·Q^0.5, d = 0.3·Q^0.4, cap 2 m, stated factor | 11 (`Geometry`, R5), 12 |
| Only river cells lowered; bumps breached to the cap, else pool | 11 (1 m breach), 12 |
| Water surface never rises downstream; bed may rise under water | 12 (R12), 13 (`drainage_rises`) |
| Relaxation in the corridor, ≤ 35–40°, never steeper than raw, smooths, halo ≥ iterations × radius, overlaps identical | 15 (R11, R19, R24), 16 (overlap check), 13 (`relax_overlap`) |
| No provenance scaling at 1 m | 12 (Q alone sets the section) |
| Hard checks: drainage, lakes (lowest saddle, leaks, exit surface), dams, one thalweg (plus parallel rivers), straightness (share and per run), facets, walls, change budget, tiles, label agreement, body budget | 13 (all; R14, R27, R29, R30, R31), 9/18 (tile exactness, contraction), 15/16 (overlaps) |
| Coarse guide (level > 5 m, gone/doubled; rivers within 100 m figure) | 17 (R17: a figure in Plan A, hard in Plan B) |
| Review: hillshade crops at the owner's spots | 17, 18, 19 |
| Gate 1: windows at 32 m, 4 m, 1 m; the tiled-exactness test | 9 (synthetic), 18 (windows, 16,384² real run) |
| Gate 2: whole 1 m world on the existing raw surface, dams expected to stay | 19 |
| Every stage keeps its raw output and is resumable; log, exit code, heartbeat, capped memory | 2 (work store), 9–13 (cached passes), 16 (driver resume, reflinked raw), shared heavy-run protocol, 18–19 |
| Expert review of each task's diff and real-terrain output | every task's last step |

Not in Plan A, by the brief: coarse-stage erosion, provenance K, the 4 m hand-off checks, the 4 m lake-exit contact sheet, the raw L3 surface, the coarse guide and `dam_drops_any` as hard checks, removal of v2 code (Plan B); stages 3–5.

**Plan review coverage** (`scratch/hydrology-v3-plan-a-review.md`):

| Item | Where |
| --- | --- |
| C1 lake check on final surfaces | R14; Task 13 `lake_off_sill`, `lake_leaks`, `lake_exit_surface`; Task 12 (no section water below an adjacent lake) |
| C2 lake confetti | R2 (r = 0 for badlands and dunes), R7 and Task 1 (`lake_min_m2` = 500 m²), R26 and Task 11 (`pools`), Task 9 probe and Task 18 Step 3b (histograms, body extrapolation), R16 and Task 13 (`export_bodies` ≤ 50,000) |
| C3 global graph size | R25; Task 6 (sorted `Vec`s), Task 8b (contraction), Task 9 (pass L, contraction on/off bit-identical test), Task 18 Step 4 (measured share) |
| I1 breach cap | R9; Task 11 `candidates` |
| I2 pond routing | R21; Task 8 |
| I3 exit surface, dominance, third run | R14, R6, R27; Tasks 12, 13 |
| I4 parallel rivers | R29; Task 13 `parallel_rivers` and its test |
| I5 dam baseline | R31; Task 13 figures; Task 19 Step 3 validation |
| I6 straight runs | R30; Task 13 `straight_runs`, top 20 runs; Task 17 crops |
| I7 relaxation strictness, fixed-order stencil | R19; Task 15 |
| I8 m³/s factor | R5; Task 1 (`m3s_per_km2` = 0.56), Task 11 test |
| M1 gorge section | Task 11 `gorge` (Euclidean disc, parabolic bed) |
| M2 big rivers into closed nodes | Task 13 figure |
| M3 gate 2 is not "dams solved" | Task 19 closing note |
| M4 I/O | Task 18 Step 4 statement |
| M5 `dam_rims` a guard | R31; Task 13 table |
| M6 r for every class | R2; Task 1 table (classes 0–19) |

**Placeholder scan:** every code step carries code; the two places that tell the implementer to adapt code (Task 13's grid writer moved into `testing.rs` in Task 12, Task 17's driver hook) give the exact code or the exact change. Probe tests in Tasks 10, 11, 12 are described by reference to Task 9's probe with their exact differences (window, file names, extra calls); they are review aids, not product code.

**Pre-flight scan coverage** (`.superpowers/sdd/2026-11-05-hydrology-v3-a-water-model/preflight-scan.md`):

| Finding | Where |
| --- | --- |
| B1 pours across tiles | `Frag::ring_pit`/`ring_leaf`, `Local::leaf_of_end` (Tasks 8b, 9); no `expect` on the path |
| B2 pass L exactness | R25; Task 9 passes L (in-tile only when counts and pours match) and L2 (gathered from every tile); `settled_trees_solved_in_their_tile_or_across_seams_match_the_whole` |
| N1, N2 | Task 8b test imports `build`; Task 13 tests import `geom::World` |
| N3, N4 | unused imports removed (Task 4 test, Task 12 `mod.rs`); `iter().enumerate()` in Tasks 3 and 4 |
| N5 | the task-finish block (Shared protocols) before every commit; `check.sh` from Task 14 |
| N6 | `Model::local` decodes `(Local, Nodes, Vec<u64>)` |
| N7 | `with_spills` gives solved tops the global spill and outlet |
| N8 | `dammed_at` walls the west end; the cap test asserts the outlet at the dam and passes a 3 m cap |
| N9 | breach (radius 24 on 128 cells), channel (256² with 32-cell tiles) and relaxation (corridor up to the cut) tests cut through work; Task 4 mass assert; Task 14 `check.sh` runs both commands on a fixture |
| N10 | Interfaces and Files blocks of Tasks 2, 8b, 11, 13, 17 |
| N11 | `breach.rs` doc comment |
| N12 | `_reflink` uses `--reflink=always` and stops with an error; Python test worlds live under `scratch/` (btrfs) |
| N13 | every real-data run capped; PID files for the 16k, game-layers and export runs |
| N14 | `DISK_BYTES = 52`, × 1.1 + 10 GB; Task 19 says about 220 GB |
| N15 | R32; `solve` starts zero-depth depressions full; Task 7 test |
| N16 | `crop`, `water_in`, `direction`, `_finish(relaxed=…)` shared |
| N17 | `Stage::copy` and `_publish` copy by reflink, never hard links |
| N18 | Task 12 backwater (16 cells, inside the halo); `a_tributary_meets_a_larger_river_at_its_surface`; `drainage_rises` |
| N19 | `[levels.*]` in `water-v2.toml`, `params(table, …)`, `change_max` = the level's cap + 0.5 m |

**Type consistency:** each type below is defined once and used with the same fields in later tasks: `Table` (with `Level` and `level()`), `Params` (with `breach_radius`, `change_max_m()`; built by `params(table, spacing, river)`), `Next`, `Edge`, `Ring`, `TileGraph`, `Hierarchy`, `Resolver` (sorted `Vec`s), `Balance` (with `pours`), `Counts`/`TileCounts`, `Frag` (with `ring_pit`), `Tops`, `Node`/`Nodes`, `Setup` (with `contract`), `Water` (with `minpit`, `nodes`, `solved`, `remote`, `remote_of`), `Local` (`solved: Tops`, `leaf_of_end`), `Model` (with `locals`), `Rasters`, `Wet` (node ordinals), `TileState` (`held: Vec<bool>`), `Candidate` (node `key`, `radius`)/`Gorge` (node `key`), `Geometry` (built from the table's factor), `Stage::copy`, `Layers` (channel), `CheckLayers` (with `flow_written`), `LakeInfo`, `CheckExtra` (with `lakes`), `Options`, `FinishOptions`, `Outcome`.

Other names are fixed in one task and used verbatim afterwards:
- `solve` takes `sea_at_zero` everywhere (Task 7 tests, Task 8b, Task 9).
- `bodies::label` takes `pools` everywhere (Tasks 10, 11, 12).
- The raster encodings (`node` = 2 + node ordinal in `Nodes`; the `flow` bits) are defined in Task 9 and read with those meanings in Tasks 10, 12, 13 and 17.
- Lake entries in `bodies.json` carry `node` (the key), which `finish` uses (Task 13).
- Work file names are defined in Task 12, and the m3 rasters in Task 13 (`m3-<digest>-*`); later tasks use them verbatim.

**Known risks to watch during execution:**
- The global graph's size at 1 m decides whether the world fits a 10 GB cap: the leaves left after contraction, times the edges. Contraction settles only trees whose merges lie below their frontier. On slopes draining out of a tile, many micro-pits spill straight into an outflowing watershed and stay global leaves, so the gain is uncertain. Task 5's and 8b's probes and Task 18's 16k run measure it, and gate 2 stops if the extrapolation does not fit. A further cut (Barnes 2016-style perimeter labels for the hierarchy) is not in this plan.
- The lake count at 1 m depends on `lake_min_m2` and the diffusion surface's pit population; Task 18 Step 3b measures it before gate 2.
- Fall-line receivers make watersheds slightly differ from steepest-descent ones; the drainage check proves the result drains.
- The relaxation's lowering-only smoothing could leave cone-like facets; the facet check compares with raw and fails the run if so (fix the relaxation, never the threshold).
- A dry cell just outside a channel section can lie below the river surface beside it (the thalweg is the flow path, not always the lowest cell); `drainage_rises` catches it. The fix belongs in `channel.rs` (such cells join the section), not in the check.
- Breach decisions at 1 m use model 1's node inflow; model 2 may route slightly differently through the cut. The exactness and drainage checks cover the result; breach figures are reported from model 1.
