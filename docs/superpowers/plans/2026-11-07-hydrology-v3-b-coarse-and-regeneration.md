# Hydrology v3, Plan B: coarse stages, v2 removal and the regeneration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run the v3 water model at 32 m and 4 m with erosion, a provenance layer and per-provenance K, make the hand-off checks, the coarse guide and the lake-exit dam drops hard, delete the v2 object hydrology, and regenerate fujix 32 m → L3 → 4 m → L4 → 1 m water → game layers → export through staging gates 3, 4 and 5.

**Architecture:** Plan A's level-independent model (`crates/topography-core/src/world/water/`) gains, at coarse spacings only, an erosion stage between the breach and the second model run: a global, single-threaded implicit stream-power pass (Braun–Willett, n = 1) on lake-aware routing read from a model run, a lowering-only hillslope diffusion step and Plan A's parabolic gorge taper applied to every cut, repeated over a few routings. A new water table (`water-v3.toml`) holds per-level erosion, wall angle and the hot-dry flag; a provenance layer replayed from the collage sets K and carries two flags (inland below-sea basin allowed, steep real source). A Python level runner in `topography_training.water.run` drives one coarse level (rain, water, finish, review, hard coarse guide) the way Plan A's window mode drives a window; `cascade_chain` uses it at 32 m and 4 m, `cascade_l4` only diffuses, and `water.run world` makes the 1 m water world as in Plan A. `world::hydrology` (v2), its CLI commands, its Python wrappers and `water-v1.toml` are deleted.

**Tech Stack:** Rust 1.85 (edition 2024; workspace crates only: serde, serde_json, toml, sha2, tempfile, png, rustfft), Python 3.11 with numpy, scipy, Pillow and PyTorch 2.14 on ROCm 7.2 (`data/models/diffusion-env-v1/.venv`), stdlib `unittest` for Python tests.

**Spec:** `docs/superpowers/specs/2026-11-05-hydrology-v3-design.md` (binding, including its "Plan A departures"). Plan A: `docs/superpowers/plans/2026-11-05-hydrology-v3-a-water-model.md` (its rulings R1–R32 stay in force unless a B ruling below replaces one). Gate history: `.superpowers/sdd/2026-11-05-hydrology-v3-a-water-model/progress.md`, `scratch/hydrology-v3/rulings.md`, `TASKS.md` ("Hydrology v3 Plan A — gate 2"). Workflow as built: `docs/workflows/terrain-cascade.md` ("Hydrology v3 (1 m)").

## Global Constraints

Copied verbatim from the spec; every task's requirements include them.

- Water is one field: every cell has a ground height, a water surface (or none) and a discharge. Lakes and rivers are labels read off that field, never objects that shape it. No rule raises banks, cuts slots or imposes paths, and no rule exempts a check.
- **Coarse guides, fine decides:** the coarser level guides through the terrain it hands on, since each diffusion level is conditioned on the level above. The finer level's own water model always sets courses and levels. Coarse levels and courses are compared as figures and never imposed.
- **Rerun:** regenerate from the 32 m hydrology through the 4 m and 1 m diffusion (about 35–41 h of GPU).
- **Ground** is never raised. It is lowered only by erosion (32 m and 4 m), by breaching (all levels) and by channel carving (1 m).
- **Breach shape:** a graded gorge, never a slot. The bed falls linearly over at least 20 × the cut depth, is at least 3 cells wide, and its sides are left to the diffusion or relaxation.
- **Model:** stream power (`E = K·A^m·S^n`) plus hillslope diffusion, with a fixed erosion budget per level (a dimensionless K·t, or a fixed iteration count with a stated K·dt). With no uplift there is no steady state to iterate to.
- **Lowering cap per cell:** for example 30 m at 32 m and 10 m at 4 m. Breach gorges are excepted.
- **Routing:** lake-aware (Cordonnier et al. 2019, or Braun–Willett with depressions routed to their outlets), so sills incise and outlets grade.
- **Solver:** chosen by benchmark in the plan, an implicit CPU or an explicit GPU solver.
- Fujix's whole 32 m surface is a collage of real GLO-30 pieces. The collage build writes a provenance layer: piece interior, blend zone, painted feature or exemplar fill.
- **Real interiors:** a low K plus the breach-or-fill policy, so the canyon is barely eroded and its GLO-30 pits do not become chains of ponds.
- **Blend zones, painted features and fills:** full K, so lakes do not form behind seams between pieces.
- Ground only, as today. The 4 m ground must carry no carved channel narrower than 3 cells and no cut steeper than 35°, because L4 copies narrow cuts; that is how v2's slots reached 1 m.
- Outlets stay smooth valleys several cells wide, so the next level reproduces a clean sill.
- The raw L3 (4 m) diffusion surface is kept from now on, so 4 m hydrology can be redone without L3.
- **Below sea level:** ground there that is not connected to open sea is a lake at its balance level, which may be below 0 m (a lagoon).
- **No injection:** discharge is never fed in from a coarser level. The 1 m accumulation covers the whole world exactly.
- **Outputs:** `bodies.json`, `water-body.u32`, `water-surface.f32` and the flow layer, with the same names and formats as today, so `game_layers` and export are unchanged.
- There is no provenance scaling at 1 m. All 1 m detail is diffusion output, and Q alone sets the channel.
- Hard checks have no exemptions, only these thresholds.
- **Change budget:** At 32 m and 4 m, for real-interior cells outside breaches: p99 |Δ| ≤ 1 m, max ≤ 3 m. Breaches get their own figures.
- **Hand-off (4 m):** no carved channel narrower than 3 cells and no cut steeper than 35°.
- **Coarse guide:** these are figures, with a hard failure only on gross mismatch: |level − coarse level| per lake: hard failure over 5 m, or when a lake is gone or doubled; each coarse river with Q over its threshold has a fine river within 100 m.
- An auto-generated contact sheet of every 4 m lake exit.
- Every stage keeps its raw output and is resumable. Long jobs follow AGENTS.md: a log, an exit code file, an hourly heartbeat and capped memory.
- The object-based hydrology in `crates/topography-core/src/world/hydrology/` is deleted once v3 replaces it. Reusable parts (grid I/O, reports, moisture) move to the new module. The band-by-band `--hydrology-only` driver is replaced by the tiled 1 m stage, which keeps the raw-surface and resume guarantees.
- A hydrology-expert subagent reviews this spec, the implementation plan, and each implementation task's diff together with its real-terrain output.

Owner rulings (binding, from the gate history):

- Straight and parallel rivers made by the pre-hydrology terrain are fine: `straight_runs`, `straight_share` and `parallel_rivers` stay figures.
- Inland lakes below sea level are fine only far from the coast in hot dry areas; elsewhere they are a defect (lake 22 at −17.8 m in `fujix-55-l4v3-water` is the known case; 13 such lakes at gate 2).
- The owner wants one water model, not hacks.
- Keep expensive intermediates (raw diffusion outputs) so downstream fixes never need a GPU rerun.
- Shipped terrain stays under about 10 GB; storage and long runs are pain points.
- Ask the owner only for terrain Yes/No and genuine direction changes.

Project constraints (AGENTS.md and this brief):

- 15 GB RAM: every real-data job runs under `systemd-run --user --scope -q -p MemoryMax=<cap>`. 16 GB VRAM ceiling (AMD RX 9060 XT, ROCm).
- Warn the owner before each heavy run (32 m regeneration is light; L3, the 4 m world water, the L4 squares, the full L4 and the 1 m world water are heavy) with time, memory and disk estimates and their uncertainty; wait for the go on heavy runs. Rust builds and downloads need no warning.
- Never write into `data/worlds/fujix-55-l4v2` or `data/worlds/fujix-55-l4v3-water` (read only). Plan B makes new world directories. `data/worlds/fujix-55-chain` is read only too.
- Executors: Opus 5.5 at medium effort, never Sonnet. The hydrology expert reviews this plan and each task's diff with its real-terrain output.
- Commits: small Conventional Commits on `feat/hydrology-v3-b`, each ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Never commit plan or task files (`TASKS.md`, `PLAN*.md`).

---

## Rulings on what the spec leaves open

The spec and Plan A leave these open; Plan B implements the ruling given. The owner may overrule any before execution. "Cost" is what happens if the ruling is wrong.

| # | Question | Ruling | Cost if wrong |
| --- | --- | --- | --- |
| B1 | Erosion solver: implicit CPU or explicit GPU? | Implicit CPU in Rust (`water/erosion.rs`): Braun–Willett (2013) with n = 1, one global single-threaded pass per step in ascending ground order; a cell whose receiver is not yet updated (a tie) has its receiver chain processed first (M3), so no receiver is ever stale. Task 9 benchmarks it at 32 m (2048²) and on a 16,384² 4 m grid. **Decision rule:** (a) the 16,384² stage's own passes take ≤ 45 min; (b) peak RSS ≤ 7 GB under `MemoryMax=10G`; (c) no cell rises, no new pit (`erosion_new_pits` 0), `walls_left` 0. Only a failure of (a) sends work to Task 9b (explicit GPU). A failure of (b) is fixed by slimming the stage wrapper (drop `classes`/`flags` once the bit sets are built, read `raw` only for the taper, sort `u32` indices by key; review M2), which 9b would not change. Reason: F = K·dt·A^m/L reaches about 2 on the 4 m trunk, so an explicit scheme needs ≥ F substeps per step, each still needing the exact Rust accumulation. | Task 9b: about a day of work; the stage interface does not change. |
| B2 | Erosion budget per level | Fixed iteration count with a stated K·dt (spec option 2). A is discharge in m² (km² at mean rain × 10⁶, rain-weighted, as v2), m = 0.5, n = 1, F = k_dt·A^m / L per step. **32 m:** 5 routings × 1 step, k_dt = 7 × 10⁻⁴ (v2's value, calibrated on this collage so seams grade and real floors move a few metres). **4 m:** 2 routings × 1 step, k_dt = 8.75 × 10⁻⁵ (= 7 × 10⁻⁴ × 4/32: the same F for the same A). A routing is one model run on the current ground. Task 9 calibrates 32 m and Task 15 calibrates 4 m with stated decision rules. | Too little: lakes behind seams remain (`seam_lakes` figure); too much: the real-interior change budget fails (hard). |
| B3 | K per provenance | Interior (and glacier class 10, which v2 also froze): 0.05 × full K and no diffusion. Blend, painted and fill: full K and diffusion. Sea: never eroded (roots). A base below 0 m on a river into the sea is taken as 0 m (v2: never below 0 m). | Canyon over-eroded (caught by `interior_p99`/`interior_max`) or pits kept in real interiors (the breach-or-fill policy handles them, per spec). |
| B4 | Lowering caps and the coarse change budget | Erosion caps per cell, relative to the breached ground (so breach gorges are excepted): 32 m 30 m, 4 m 10 m on full-K cells; 3 m on interior cells at both levels (the spec's change-budget max). `change_max` at a coarse level is breach cap + erosion cap + 0.5 m (50.5 m at 32 m, 15.5 m at 4 m), since erosion may deepen a gorge (review C3). At levels with erosion, Plan A's all-cell `change_p99` is a figure: the spec's coarse budget is `interior_p99`/`interior_max` (v2's same stream power lowered half the 32 m cells, river cells by a median 2 m). | A cap stops a sill from grading → a seam lake stays; visible in `seam_lakes`. |
| B5 | Breach caps at 32 m and 4 m (R9 left 20 m / 5 m provisional) | Confirmed: 20 m at 32 m, 5 m at 4 m. A gorge is refused when a changed cell exceeds the level's wall angle (B8). Fix found while planning: `breach.rs` computes the taper radius as `2·d / tan` **cells**, right only at 1 m; Task 2 divides by the spacing (at 32 m a 20 m cut gave a 57-cell disc instead of 1.8). | Deeper coarse dams stay filled lakes; erosion still grades their sills over the routings. |
| B6 | Hillslope diffusion | Explicit Jacobi, 4-neighbour, α = D·dt/dx² = 0.05 per routing, **lowering only** (convex cells move down, concave cells are not filled; the spec never raises ground and leaves deposition out of scope), and **only within R_taper + 1 cells of a cell the same routing's stream power lowered by ≥ 1 cm** (review I3): ridges, painted calderas and blend-zone relief away from river cuts keep their shape. Figures `hillslope_lowering_p99_m`/`_max_m` over full-K cells outside that footprint (expected about 0), read in Task 9. | Valley sides stay a little V-shaped; L3/L4 repaint the detail. |
| B7 | Shape of an erosion cut | After each routing's stream power and diffusion, every cell lowered by d ≥ 1 cm is re-expressed as Plan A's gorge taper: a disc lowered by d·(1 − (r/R)²), R = max(1.5 cells, 2·d / (tan(wall_deg)·spacing)); the result is the deepest lowering over all discs, never below the floor, never under water or sea. Then up to 16 wall rounds halve the lowering of any changed cell still steeper than max(wall_deg, raw) + 0.01°, alternating with a bed-grade pass (B33). This gives cuts ≥ 3 cells wide with sides under the wall angle. | More cells beside rivers are lowered (wider valleys, by design); a few walls left after 16 rounds fail `walls` loudly. |
| B8 | Wall angle per level | `wall_deg` per level in the table: 1 m 40° (Plan A), 4 m and 32 m 35° (the hand-off's "no cut steeper than 35°"). `walls`, the breach refusal and the erosion wall rounds use max(wall_deg, raw slope) + 0.01°. | More coarse gorges refused (filled lakes instead). |
| B9 | Hand-off checks | Hard at both coarse levels (the spec names 4 m; L3 copies 32 m ground the same way): `handoff_narrow` counts cells where the cut Δ = raw − final exceeds its 3 × 3 grey opening by more than **max(0.5 m, 0.5 × Δ)** (review C2): a one-cell slot (opening ≈ 0) fails at any depth over 0.5 m, every B7 disc (R ≥ 1.5 > √2 cells) and every flat-bedded gorge passes. The absolute excess is a figure (`handoff_excess_max_m`). Steepness is `walls` at 35° (B8). | A narrow cut that L3 would have smoothed fails; fix the taper, not the check. |
| B10 | Provenance for the existing collage: rebuild or derive? | Derive by **replay**: `collage.surface()` recomputes the surface steps in memory from the layout and release. Accepted when the layout sha256 matches `collage.json`, `region32` reproduces exactly, `class32` agrees on ≥ 99.9 % of cells (the dither), and the replayed `l2` matches the stored one with p99 \|Δ\| ≤ 0.5 m and max ≤ 5 m outside painted cells (review I6: provenance needs weights, coverage and regions, not centimetre heights; the build was `git_dirty` at c0a9233 and seven commits touched the code since). The steep flag and the painted test use the **stored** `l2` (painted: \|stored l2 − replayed blend\| > 0.5 m). Written as the sidecar `provenance32.u8` + `provenance32.json` beside the collage (additive). Kinds (bits 0–2): 0 sea outside every region, 1 interior (dominant region weight share ≥ 0.99, one piece reaching the cell with weight ≥ 0.99, ≥ 1 km from any other kind), 2 blend, 3 painted, 4 fill. Flags: bit 6 (64) below-sea basin allowed, bit 7 (128) steep source (stored-l2 Horn slope ≥ 30°; a figure tag since B14 changed). | Refused only on a layout or region mismatch: then a rebuilt collage is a new world, an owner decision (owner question 2). |
| B11 | Provenance at 4 m and 1 m | The 32 m layer, nearest. At 4 m it sets K like at 32 m (L3 is conditioned on it); at 1 m only its flags are read (spec: no provenance scaling at 1 m). 1 m grids get it cropped and upsampled to 4 m cells, like the classes. | — |
| B12 | Does the 32 m regeneration change the moisture field? | No. `topography world rain` computes it from the raw 32 m ground with v2's code and sea rule (ground < 0 m 8-connected to the edge), before any erosion. Task 1 proves it byte-identical to `fujix-55-chain/hydrology/32m/moisture.f32` and `mean_land_mm_h` equal; 4 m and 1 m reuse it. | If not identical, rain moves slightly; recorded and accepted (no GPU impact). |
| B13 | Inland lakes below sea level (owner ruling) | One model rule per depression, the coastal water table (review I4): if any sub-zero cell of a depression lacks bit 64, its level is floored at min(spill, the lowest ground at or above 0 m among its cells): the k-th lowest cell with k ≥ its sub-zero cell count + 1, or it fills and spills. No supply is added, so no water flows on downstream. Bit 64 = a hot-dry class (`hot_dry = true`: canyons 2, 13, 19, badlands 7, dunes 8, 16) and ≥ 5 km from open sea at 32 m. Two integer counts per drainage end (sub-zero cells, sub-zero cells without bit 64) are summed like the supply, so the rule is exact at any tiling. Hard check `lake_below_sea` uses the same predicate: a labelled lake with level < −0.01 m with any cell lacking bit 64. Ground is never raised. | Near-coast lakes a bit larger than their r-balance; a real arid near-coast salt pan would fill. |
| B14 | `dam_drops_any` as a hard check (R31) | New hard check `dam_drops` at every level, a **knickpoint** rule (review C1): on a labelled lake's 50 m exit walk, a drop D of the shown surface between adjacent cells fails when **D > 1.5 m and D > 3 × the mean fall per cell over the next 100 m of the walk** (step-then-gentle is a dam; a steady steep cascade passes, at any spacing). Raw is ignored; there is no steep-source exemption (bit 128 hid two of the three known dams: the 32 m collage is ≥ 30° beside them). Pond exits stay in the figure `dam_drops_any`. Task 7 proves it on Plan A's raw windows: it must fire at E25645, E25307, E20071 and E43966; its other firings are listed with crops and calibrate the factor 3. | A sheer real waterfall right at an outlet fails: owner question 3 with crops, before gate 4; never a code exemption. |
| B15 | Coarse guide as a hard check (R17) | In the level runner and the 1 m driver, fine against the next coarser v3 level. Hard (review I5) only for coarse lakes of **≥ 0.05 km²** (32 m: 49 cells; 4 m: 3,125 cells) **deeper than the finer level's breach cap + 1 m** (6 m from 32 m to 4 m, 3 m from 4 m to 1 m) and **≥ 1 km from a window or square edge** (inner edges only): level difference over 5 m, gone, or doubled (two fine lakes each over 25 % of its area). Every other coarse lake and the river share are figures (`lakes_small`, `lakes_shallow`, `lakes_near_edge`). A hard failure withholds `manifest.json` and exits 1. | A small or shallow coarse lake that the finer level breaches passes unseen (the finer level decides, per spec). |
| B16 | Lakes without an outlet trickle (gate-2 park) | A full labelled lake always starts a river at its outflow cell, labelled down its m2 path to the next water, the sea or the map edge, whatever its Q; channel size from its Q. At most one river per full lake (638 lakes at gate 2; 50,000 budget). | A few hair-thin rivers on tiny catchments. |
| B17 | What is reused from Plan A and what is regenerated? | Reused unchanged: `fujix-collage-55` (plus the provenance sidecar), L3 `cascade-l3-run-5` (+ experts), L4 `cascade-l4-run-8` (+ experts), seeds, guidance 2 on class 19, the water model code. Regenerated: 32 m water, L3 raw, 4 m water, L4 raw, 1 m water, game layers, export. `fujix-55-l4v3-water` and its export stay the shipped fallback and are only read (Task 13's reproduction window, Task 7's controls). `fujix-55-chain` is only read. Task 14 deletes v2 before gates 3–5; that is safe because the Plan A world and export remain the fallback and git keeps v2 (review M6). | — |
| B18 | New directories | `data/worlds/fujix-55-chain-v3` (32 m, L3, 4 m); `data/worlds/fujix-55-l4v3b-{dams,canyon,e43966}` and `…-water` (L4 squares); `data/worlds/fujix-55-l4v3b-raw` (full L4); `data/worlds/fujix-55-l4v3b` (1 m water world); `data/exports/fujix-55-l4v3b`. Probe and calibration runs under `scratch/hydrology-v3b/`. | — |
| B19 | L4 test strip extent | Four squares via `cascade_l4 --km --centre` (x east, y south from the north-west corner; y = 65,536 − N; `--km` must be a power of two): **dams** 8 km at (22858, 38086), covering E20071–E25645 and N24400–N30501 with at least 0.95 km margin; **canyon** 4 km at (27169, 20990); **E43966** (v2's staircase scarp) 4 km at (43966, 43903); **alpine** 4 km at the gate-3 4 m world's densest cluster of labelled lakes in classes 3, 6 and 10 (review I8; chosen in Task 17 Step 1). The full L4 run does not reuse them. Q in a square is window-limited (no inflow, spec "no injection"), so the canyon trunk is probably not a main river there and the doubled-river test is exercised only at gate 5 (review M5). | About 2–3 GPU hours spent twice. |
| B20 | L4 and band hydrology | `cascade_l4` only diffuses: it writes `elevation-raw.f32`, `snow-ice.u8`, `vegetation.u8` and a `world_raw` manifest; `water.run world` makes the water world. `--hydrology-only`, `--seam-audit`, `--drop-raw` and the per-band Rust calls go. | — |
| B21 | Disk budget per stage | Bytes per cell from Plan A. chain-v3: about 12 GB plus the 4 m work directory about 15 GB (54 B/cell × 268 M) plus erosion routings about 6 GB; L4 squares: about 6 GB with their water; full L4 raw: about 26 GB (6 B/cell); 1 m water: work about 232 GB plus published about 55 GB; game layers and export: about 5 GB. **Total about 380 GB.** `df` showed 651 GB free on `/home` while planning; the brief says about 300 GB. Each heavy step checks free space before it starts; if the 1 m water run lacks room, ask the owner (owner question 1). | A run stops on its disk check; nothing is deleted without the owner. |
| B22 | `water-v1.toml` users outside fujix | Deleted with v2. The L1 128 m `drain` used only by authored and region layouts (not collages) keeps one share for every class: lakes 0.01, dry basins 0.01, dry classes = r ≤ 0.05 in `water-v3.toml`. `cascade_chain --lake-share/--tarn-share` go. | A future Fujion regeneration gets different L1 lakes. |
| B23 | Stage order at coarse levels | m1 (raw) → b (breach) → e (routings × [model on current ground, stream power, diffusion, taper, wall rounds]; routing 0 is the model on the breached ground) → m2 (model on the eroded ground) → l → c (no cut at coarse spacings) → m3 → s. The ground before channels (`pre_channel`) is `eroded.f32` with erosion, else `breached.f32`. 1 m has no `e` stage. | — |
| B24 | Water table file | New `configs/hydrology/water-v3.toml` (`schema_version = 3`): v2's r, `lake_min_m2`, `m3s_per_km2`, plus `hot_dry` per class, `below_sea_coast_km`, per level `wall_deg` and an optional `[levels.N.erosion]` table. `water-v2.toml` and `water-v1.toml` are deleted (git keeps them; the Plan A world records the v2 table's sha256). | — |
| B25 | Calibration figure | `seam_lakes`: labelled lakes whose outlet saddle cell is blend, painted or fill; listed with cells. | — |
| B26 | Comb-cut channel pits (gate-2 park, E41058 N56173) | Reproduced in a 1 m window on Plan A's raw surface; new hard check `carve_pits` (a cell the breach or the channels lowered that ends dry and strictly below all 8 neighbours' D); fixed in `channel.rs` (or `breach.rs`) before gate 4. | — |
| B27 | Bank steps (104 k over 0.5 m at gate 2, raw) | A figure. The GPU relaxation (git `d0c0055`) is restored only if the owner flags banks at gate 4 or the squares' `bank_steps_over_0_5m` density exceeds Plan A's 24 per km². | Box-ish banks stay where the raw had them. |
| B28 | v2 coarse-guide mismatches (lip rims, coastal ponds, 66 gone, 26 over 5 m) | v2 artefacts. Plan B compares v3 against v3, so they need no task. | — |
| B29 | Exactness of the erosion pass | One global, single-threaded pass in a fixed order: deterministic and independent of the model runs' tiling (which stay bit-exact, Plan A). Plan A's 1 mm overlap rule does not apply (no tiled stencil). | — |
| B30 | Tiles and threads | `--tile 2048 --threads 4` at every level (gate-1 measurement). | — |
| B31 | Heavy-run consent | Warn and wait: L3 whole map, 4 m world water, L4 squares, full L4, 1 m world water. Announce only (minutes, under 6 GB): 32 m runs, 4 km L3 probe squares, windows. | — |
| B32 | Coarse water reaching the diffusion levels (review I1) | Not named by the spec: L3 is conditioned on the 32 m lake/river masks and clamps its 4 m raw to the 32 m ground under them; L4 zeroes its residual under all 4 m water. Ruling: keep the clamps for lakes and sea (the models learned land only). The **river** clamp is A/B tested: L3 on the dams square with and without rivers in the clamp (Task 15, flag `--no-l3-river-clamp`), compared on 4 m `thalweg_beside`, `dam_drops`, straight/parallel figures and spot crops; the better one is kept. The L4 river residual gets a flag (`--keep-river-residual`) and an optional A/B at gate 4 on the dams square (about 1 GPU h), else the reason to keep it is recorded. B16 trickles at 4 m become L4 water strips through this mask (M4). The outcome is a Plan B departure ("ground plus the existing water conditioning"). | Coarse courses stay imprinted on the finer raw (the likely cause of E43966's staircase) if the A/B is not decisive. |
| B33 | New pits made by the erosion code (review I2) | After the wall rounds, Plan A's bed-grade pass to a fixed point, alternating with the wall check: z[c] = min(before[c], max(z[c], z[down(c)])) along the routing's receivers. Hard check `erosion_new_pits` at coarse levels: cells that are strict local minima of the eroded ground but not of the breached ground. | — |
| B34 | Minor review items | Applied: M1 (margin stated as ≥ 0.95 km; 8.5 km is not a power of two), M2 (B1 (b)), M3 (receiver-first on ties), M4 (in B32), M5 (in B19 and Task 17), M6 (in B17), M7 (owner question 1), M8 (Task 9 readout), M9 (Task 13 scope cap). None declined. | — |

## File structure

Rust, `crates/topography-core/src/world/water/`:

| File | Change |
| --- | --- |
| `grid.rs` | Moved from `world/hydrology/grid.rs` (grid.json shape, `Stage`, windowed reads) |
| `moisture.rs` | Moved from `world/hydrology/moisture.rs`; `open_sea` and a land closure replace v2's flags |
| `erosion.rs` | New: stream power, diffusion, taper, wall rounds, `Bits`, the stage wrapper `erode` |
| `table.rs` | `water-v3.toml`: `hot_dry`, `below_sea_coast_km`, `wall_deg`, `Erosion` |
| `mod.rs` | `rain` (`world rain`), provenance in `setup`, `Params.wall_deg/erosion`, the `e` stage, `pre_channel` |
| `pipeline.rs` | `Setup.provenance`, the coastal water table in `supply` |
| `breach.rs` | Taper radius in cells (B5), wall angle per level |
| `channel.rs` | Wall angle per level; the comb-cut fix (Task 13) |
| `bodies.rs` | Outlet trickle (B16) |
| `checks.rs` | `lake_below_sea`, `dam_drops`, `interior_p99`, `interior_max`, `handoff_narrow`, `carve_pits`, `seam_lakes`, wall angle per level, `change_max` per level |
| `testing.rs` | Coarse synthetic grid with a provenance layer and a seam dam |

Other files:

| File | Change |
| --- | --- |
| `configs/hydrology/water-v3.toml` | New (B24); `water-v1.toml`, `water-v2.toml` deleted |
| `crates/topography-core/src/world/hydrology/` | Deleted (Task 14) |
| `crates/topography-core/src/world/mod.rs` | `pub mod hydrology;` removed |
| `crates/topography-core/src/export/description.rs` | `world::water::grid` import |
| `crates/topography-cli/src/main.rs` | `world rain`; `world hydrology` and `world hydrology-seam` removed |
| `scripts/check.sh` | v2 lines out; coarse fixture and `world rain` lines in |
| `scripts/hydrology_grid.py`, `scripts/README.md` | Deleted / its section removed |
| `tests/fixtures/water/coarse/` | New coarse fixture (written by a test) |
| `training/src/topography_training/water/table.py` | New: the table for Python |
| `training/src/topography_training/water/run.py` | `write_grid`, `run_level`, `level` mode, provenance in grids, squares |
| `training/src/topography_training/water/review.py` | Coarse guide: minimum lake size, coarse river start |
| `training/src/topography_training/diffusion/collage.py` | `surface`, `provenance`, `provenance` subcommand |
| `training/src/topography_training/diffusion/cascade_chain.py` | v3 at 32 m and 4 m; L1 shares (B22) |
| `training/src/topography_training/diffusion/cascade_l4.py` | Diffusion only (B20) |
| `training/src/topography_training/diffusion/hydrology.py` | v2 wrappers and the v1 table removed; numpy L1 helpers and shared file helpers stay |
| `training/tests/test_water_table.py`, `test_collage_provenance.py`, `test_water_level.py`, `test_cascade_l4_raw.py` | New `unittest` tests |
| `docs/workflows/terrain-cascade.md`, the spec | v2 section out, v3 at every level, Plan B departures |

## Shared protocols

### Expert review (last step of every task)

Dispatch one subagent (Opus 5.5, medium effort) with this prompt, filling in the task number, commit range and the real-terrain output the task names:

```text
You are a hydrology expert reviewing one implementation task of hydrology v3 Plan B for a game
terrain tool (plausible-looking terrain, not science). Read the spec
docs/superpowers/specs/2026-11-05-hydrology-v3-design.md and the plan
docs/superpowers/plans/2026-11-07-hydrology-v3-b-coarse-and-regeneration.md (Task <N> and the
B rulings). Review `git diff <base>..<head>` and the real-terrain output at <paths>. Judge:
(1) does the code do what the spec and rulings say, with no rule that raises ground, cuts
slots, imposes paths or exempts a check; (2) is it deterministic, and exact across tilings
where the plan says so; (3) does the real-terrain output look like water behaving like water
and like terrain a dirigible pilot would believe (name cells and E/N coordinates for every
defect). Report blocking problems first, then non-blocking ones, each with file:line or
world coordinates. Do not edit files.
```

Fix every blocking finding (each fix its own commit), rerun the task's tests and record the verdict in `.superpowers/sdd/2026-11-07-hydrology-v3-b/progress.md` (never committed).

### Heavy runs

Before a step marked **HEAVY**, stop and tell the owner what runs, the expected peak RAM / VRAM / disk and duration with the uncertainty stated, and that other applications should be closed; wait for the go. Then launch as a background Bash job:

```bash
RUN=<run-dir>; mkdir -p "$RUN"; rm -f "$RUN/exit_code"
( systemd-run --user --scope -q -p MemoryMax=<cap> <command> >> "$RUN/log" 2>&1; echo $? > "$RUN/exit_code" ) &
echo $! > "$RUN/pid"
```

Watch it with a Monitor on `grep -E --line-buffered 'water: |erosion: |band |L3 band|done|Traceback|Error|error|OOM|Killed|nan|FAILED' "$RUN/log"` (re-armed on expiry, at most 30 min each) and, for runs over an hour, a background `timeout 3600 tail --pid=$(cat $RUN/pid) -f /dev/null` heartbeat; on each heartbeat report progress, rate and remaining time from the log. A missing PID without `exit_code` is a crash. Rerunning the same command resumes.

### Commands used throughout

```bash
CAP6=(systemd-run --user --scope -q -p MemoryMax=6G)
CAP10=(systemd-run --user --scope -q -p MemoryMax=10G)
CAP12=(systemd-run --user --scope -q -p MemoryMax=12G)
P=data/models/diffusion-env-v1/.venv/bin/python
PY=(env PYTHONPATH=training/src $P)
"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water
"${CAP6[@]}" cargo build --release --locked --offline -p topography-cli
```

### Task-finish block (before every commit)

```bash
cargo fmt --all
"${CAP6[@]}" cargo clippy --workspace --all-targets --locked --offline -- -D warnings
"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water
"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet hydrology   # until Task 14 deletes v2
PYTHONPATH=training/src $P -m unittest discover -s training/tests -p 'test_*.py'
bash scripts/check.sh
```

All must pass; fix the code, never the check. Tasks that touch only Python may skip the Rust lines if `git diff --stat` shows no Rust change.

---
## Task 1: Grid I/O and rain move into `world::water`; `topography world rain`

Spec "Removed": reusable parts move to the new module. Ruling B12: the 32 m regeneration keeps the moisture field.

**Files:**
- Move: `crates/topography-core/src/world/hydrology/grid.rs` → `crates/topography-core/src/world/water/grid.rs` (`git mv`, content unchanged)
- Move: `crates/topography-core/src/world/hydrology/moisture.rs` → `crates/topography-core/src/world/water/moisture.rs` (`git mv`, then the edits below)
- Modify: `crates/topography-core/src/world/hydrology/mod.rs` (lines 11 and 13: use the moved modules; line 422: the land closure)
- Modify: `crates/topography-core/src/world/hydrology/{coarse,band}.rs` (`use super::grid` → `use crate::world::water::grid`)
- Modify: `crates/topography-core/src/world/water/mod.rs` (module declarations, `grid::` paths at lines 119 and 734, new `RainOptions` and `rain`)
- Modify: `crates/topography-core/src/export/description.rs:7`
- Modify: `crates/topography-cli/src/main.rs` (`world rain` and its usage line)
- Modify: `scripts/check.sh` (a `world rain` run on a copy of the fixture)

**Interfaces:**
- Consumes: v2 `moisture::{precipitation, Stats, PARAMS, NEUTRAL_MOISTURE}`, `grid::{shape, Stage, read_window, read_bodies}`.
- Produces:
  - `crate::world::water::grid` (`pub(crate)`), same items as v2's.
  - `crate::world::water::moisture` (`pub(crate)`): `open_sea(ground: &[f32], w: usize, h: usize) -> Vec<bool>`; `Stats::new(rain: &Rain, fine_w: usize, land: impl Fn(usize) -> bool, windless: bool) -> Stats`; `precipitation`, `PARAMS`, `NEUTRAL_MOISTURE` as before, now `pub(crate)`.
  - `pub struct RainOptions { pub grid: PathBuf, pub wind_deg: Option<f64> }` and `pub fn rain(o: &RainOptions) -> Result<String>` in `world::water`: writes `<grid>/moisture.f32` and adds `moisture`, `discharge.mean_land_mm_h` and `precipitation` to `<grid>/grid.json`; refuses a grid.json that already has `moisture`.
  - CLI: `topography world rain --grid <dir> [--wind-deg D]`.

- [ ] **Step 1: Move the files and fix the paths**

```bash
git mv crates/topography-core/src/world/hydrology/grid.rs crates/topography-core/src/world/water/grid.rs
git mv crates/topography-core/src/world/hydrology/moisture.rs crates/topography-core/src/world/water/moisture.rs
```

In `world/water/mod.rs`, add beside the other module lines:

```rust
pub(crate) mod grid;
pub(crate) mod moisture;
```

and replace `crate::world::hydrology::grid::shape` (line 119) and `crate::world::hydrology::grid::Stage` (line 734) with `grid::shape` and `grid::Stage`.

In `world/hydrology/mod.rs`, replace `pub(crate) mod grid;` with `pub(crate) use crate::world::water::grid;` and `mod moisture;` with `use crate::world::water::moisture;`. In `coarse.rs` and `band.rs`, `use super::grid::{self, Grid};` becomes `use crate::world::water::grid::{self, Grid};`. In `export/description.rs:7`, `use crate::world::hydrology::grid;` becomes `use crate::world::water::grid;`.

In `world/water/moisture.rs`, change every `pub(super)` to `pub(crate)`, delete `use super::flood::SEA;`, and give `Stats::new` the fine grid's height and a land closure instead of v2's flags:

```rust
    pub fn new(rain: &Rain, fine_w: usize, fine_h: usize, land: impl Fn(usize) -> bool, windless: bool) -> Self {
```

Keep the body: its loops become `for i in 0..fine_w * fine_h { if land(i) { … } }` where they read `flags.iter().enumerate()` and `f & SEA == 0`; `fine_h` is the parameter instead of `flags.len() / fine_w`. The v2 call site in `world/hydrology/mod.rs:422` becomes:

```rust
        let stats = moisture::Stats::new(&rain, grid.w, grid.h, |i| flags[i] & SEA == 0, wind_from_deg.is_none());
```

Add the sea rule v2 used for rain (`flood::open_sea` seeded at the grid edge, ground < 0 m, 8-connected):

```rust
/// v2's sea for the rain (ruling B12): ground below 0 m connected 8-way to the grid edge.
pub(crate) fn open_sea(ground: &[f32], w: usize, h: usize) -> Vec<bool> {
    let mut sea = vec![false; ground.len()];
    let edge = |i: usize| {
        let (x, y) = (i % w, i / w);
        x == 0 || y == 0 || x + 1 == w || y + 1 == h
    };
    let mut stack: Vec<usize> = (0..ground.len()).filter(|&i| edge(i) && ground[i] < 0.0).collect();
    for &i in &stack {
        sea[i] = true;
    }
    while let Some(i) = stack.pop() {
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        for (dy, dx) in super::geom::OFFSETS {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                continue;
            }
            let j = ny as usize * w + nx as usize;
            if !sea[j] && ground[j] < 0.0 {
                sea[j] = true;
                stack.push(j);
            }
        }
    }
    sea
}
```

- [ ] **Step 2: Write the failing tests**

In `world/water/moisture.rs` `mod tests`, add:

```rust
    #[test]
    fn the_rain_sea_is_below_zero_and_reaches_the_edge() {
        // 5 x 4: a sea bay on the west edge, an inland pit below 0 m, a 0 m cell on the edge.
        #[rustfmt::skip]
        let g = [
            -1.0, -1.0, 5.0, 5.0, 0.0,
            -1.0,  2.0, 5.0, -3.0, 5.0,
             3.0, -1.0, 5.0, 5.0, 5.0,
             3.0,  3.0, 3.0, 3.0, 3.0f32,
        ];
        let sea = open_sea(&g, 5, 4);
        let want = [0usize, 1, 5, 11];
        for (i, &s) in sea.iter().enumerate() {
            assert_eq!(s, want.contains(&i), "cell {i}");
        }
    }
```

In `world/water/mod.rs` `mod tests`, add:

```rust
    #[test]
    fn rain_writes_the_moisture_field_once() {
        let dir = tempfile::tempdir().expect("temp");
        let world = geom::World { w: 64, h: 64 };
        let grid = testing::grid_dir(dir.path(), world);
        // A trunk grid: no moisture yet, a wind from the west, 32 m cells.
        let mut m: Map<String, Value> =
            serde_json::from_slice(&fs::read(grid.join("grid.json")).unwrap()).unwrap();
        m.remove("moisture");
        m.remove("discharge");
        m.insert("spacing_m".into(), json!(32.0));
        m.insert("origin".into(), json!([0.0, 64.0 * 32.0]));
        m.insert("wind_from_deg".into(), json!(270.0));
        fs::write(grid.join("grid.json"), serde_json::to_vec(&m).unwrap()).unwrap();
        fs::remove_file(grid.join("moisture.f32")).unwrap();
        let o = RainOptions { grid: grid.clone(), wind_deg: None };
        rain(&o).expect("rain");
        let m: Map<String, Value> =
            serde_json::from_slice(&fs::read(grid.join("grid.json")).unwrap()).unwrap();
        let entry = &m["moisture"];
        assert_eq!(entry["size"], json!([16, 16]), "128 m cells over a 2,048 m grid");
        assert!(m["discharge"]["mean_land_mm_h"].as_f64().unwrap() > 0.0);
        assert_eq!(fs::read(grid.join("moisture.f32")).unwrap().len(), 16 * 16 * 4);
        // The model reads it.
        let (s, _, _) = setup(&grid, 64, 1, Some(&grid.join("water.toml")), None).expect("setup");
        assert!(s.rain.mean_land_mm_h > 0.0);
        assert!(rain(&o).is_err(), "a grid with moisture is refused");
    }
```

- [ ] **Step 3: Run them to see them fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water -- moisture rain_writes`
Expected: compile errors (`open_sea`, `RainOptions`, `rain` not defined).

- [ ] **Step 4: Implement `rain`**

In `world/water/mod.rs`:

```rust
pub struct RainOptions {
    pub grid: PathBuf,
    /// Degrees the wind comes from (clockwise from north); else grid.json `wind_from_deg`;
    /// without either the rain is uniform (moisture 0.5).
    pub wind_deg: Option<f64>,
}

/// The trunk grid's 128 m moisture field (Smith and Barstad 2004, v2's code and sea rule) from
/// its own ground, before any erosion (ruling B12): writes `moisture.f32` and the grid.json
/// entries `moisture`, `discharge.mean_land_mm_h` and `precipitation` that `world water`
/// reads. Finer levels copy them from the trunk.
pub fn rain(o: &RainOptions) -> Result<String> {
    let path = o.grid.join("grid.json");
    let mut manifest: Map<String, Value> = serde_json::from_slice(&read_small(&path)?)?;
    ensure(
        !manifest.contains_key("moisture"),
        format!("{} already has a moisture entry", path.display()),
    )?;
    let g = grid::shape(&manifest)?;
    let world = geom::World { w: g.w as u64, h: g.h as u64 };
    let ground = layers::Layer::<f32>::open(world, &o.grid.join("elevation.f32"))?.to_vec()?;
    let sea = moisture::open_sea(&ground, g.w, g.h);
    let wind = o
        .wind_deg
        .or_else(|| manifest.get("wind_from_deg").and_then(Value::as_f64));
    let p = &moisture::PARAMS;
    let rain = moisture::precipitation(&ground, |i| !sea[i], g.w, g.h, g.spacing_m, wind, p);
    let stats = moisture::Stats::new(&rain, g.w, g.h, |i| !sea[i], wind.is_none());
    let bytes: Vec<u8> = rain
        .mm_h
        .iter()
        .flat_map(|&v| stats.moisture(v).to_le_bytes())
        .collect();
    io_context("write moisture.f32", fs::write(o.grid.join("moisture.f32"), bytes))?;
    let mut precipitation = stats.json();
    if let Value::Object(m) = &mut precipitation {
        m.insert("wind_from_deg".into(), json!(wind));
        m.insert("params".into(), p.json());
    }
    manifest.insert(
        "moisture".into(),
        json!({
            "file": "moisture.f32", "size": [rain.w, rain.h],
            "spacing_m": g.spacing_m * rain.k as f64, "nw_m": g.nw_m, "dtype": "float32",
            "value": if wind.is_some() {
                json!("precipitation / scale_mm_h, clipped to 0..1")
            } else {
                json!(format!("{} everywhere (no wind)", moisture::NEUTRAL_MOISTURE))
            },
            "scale_mm_h": stats.p95_land_mm_h,
            "scale": "95th percentile of precipitation on land cells",
        }),
    );
    manifest.insert(
        "discharge".into(),
        json!({"unit": "km² at the mean land precipitation", "mean_land_mm_h": stats.mean_land_mm_h}),
    );
    manifest.insert("precipitation".into(), precipitation);
    manifest.insert("wind_from_deg".into(), json!(wind));
    io_context(
        "write grid.json",
        fs::write(&path, serde_json::to_vec_pretty(&manifest)?),
    )?;
    Ok(format!(
        "{}: moisture {}x{} at {} m, mean land rain {:.3} mm/h",
        o.grid.display(),
        rain.w,
        rain.h,
        g.spacing_m * rain.k as f64,
        stats.mean_land_mm_h
    ))
}
```

In `crates/topography-cli/src/main.rs`, beside the `world water` arm:

```rust
        [command, action, rest @ ..] if command == "world" && action == "rain" => {
            return rain(rest);
        }
```

and the handler plus a usage line `topography world rain --grid <grid-dir> [--wind-deg 225]` next to the `world water` usage lines:

```rust
/// `world rain --grid DIR [--wind-deg D]`
fn rain(args: &[std::ffi::OsString]) -> ExitCode {
    use topography_core::world::water;
    let mut o = water::RainOptions { grid: Default::default(), wind_deg: None };
    for pair in args.chunks(2) {
        let [flag, value] = pair else {
            return usage_error();
        };
        match flag.to_str() {
            Some("--grid") => o.grid = value.into(),
            Some("--wind-deg") => match number::<f64>(value) {
                Some(d) if d.is_finite() => o.wind_deg = Some(d),
                _ => return usage_error(),
            },
            _ => return usage_error(),
        }
    }
    if o.grid.as_os_str().is_empty() {
        return usage_error();
    }
    workflow_result(water::rain(&o))
}
```

(`number::<T>(&OsStr)` is the CLI's existing numeric parser, used for `context serve`.)

- [ ] **Step 5: Run the tests**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water` and `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet hydrology`
Expected: PASS (v2 still builds and passes on the moved code).

- [ ] **Step 6: Prove the moisture field is unchanged (B12)**

```bash
"${CAP6[@]}" cargo build --release --locked --offline -p topography-cli
G=scratch/hydrology-v3b/t1-rain; rm -rf $G; mkdir -p scratch/hydrology-v3b
"${CAP6[@]}" "${PY[@]}" -c "
import json, numpy as np
from pathlib import Path
from topography_training.diffusion import hydrology, layout
c = Path('data/worlds/fujix-collage-55')
l2 = np.load(c / 'arrays.npz')['l2'].astype(np.float32)
wind = layout.load(c).spec.get('wind_from_deg')
g = json.loads(Path('data/worlds/fujix-55-chain/hydrology/32m/grid.json').read_text())
hydrology.write_grid(Path('$G'), l2, 32.0, tuple(g['origin']), wind_from_deg=wind)"
target/release/topography world rain --grid $G
cmp $G/moisture.f32 data/worlds/fujix-55-chain/hydrology/32m/moisture.f32 && echo "moisture identical"
jq '.discharge.mean_land_mm_h' $G/grid.json data/worlds/fujix-55-chain/hydrology/32m/grid.json
```

Expected: `moisture identical` and two equal `mean_land_mm_h` values. If they differ, find why (the v2 trunk computed rain from the same input ground; a difference means the land closure or the sea seed differs) and fix the port; only if the cause is a v2 quirk that cannot be ported cleanly, record the largest |Δ| and the mean-rain change in the progress file and go on (B12 cost line).

- [ ] **Step 7: `check.sh`**

After the `water-finish` lines, add:

```bash
rain_grid="$water_tmp/rain"
cp -r "$water_grid" "$rain_grid"
jq 'del(.moisture, .discharge) | .spacing_m = 32 | .wind_from_deg = 270' "$water_grid/grid.json" > "$rain_grid/grid.json"
rm "$rain_grid/moisture.f32"
"${cli[@]}" world rain --grid "$rain_grid" >/dev/null ||
  { printf 'world rain failed\n' >&2; exit 1; }
test -s "$rain_grid/moisture.f32" || { printf 'world rain wrote no moisture\n' >&2; exit 1; }
check_failure 2 'Unsupported arguments' world rain --grid
```

- [ ] **Step 8: Commit**

Run the task-finish block first.

```bash
git add -A crates/topography-core/src/world crates/topography-core/src/export/description.rs \
  crates/topography-cli/src/main.rs scripts/check.sh
git commit -m "refactor(water): move grid io and rain into world::water; add world rain

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 9: Expert review** — real-terrain output: `$G/moisture.f32` and the `cmp` result.

---

## Task 2: Water table v3, wall angle per level and the coarse breach taper

Rulings B4, B5, B8, B13, B24.

**Files:**
- Create: `configs/hydrology/water-v3.toml`
- Delete: `configs/hydrology/water-v2.toml`
- Modify: `crates/topography-core/src/world/water/table.rs` (schema 3, `Class.hot_dry`, `Level.wall_deg`, `Erosion`, `below_sea_coast_km`)
- Modify: `crates/topography-core/src/world/water/mod.rs` (`Params.wall_deg`, `Params.erosion`; the built-in table name in `setup`; `Tags::b` digests `wall_deg`)
- Modify: `crates/topography-core/src/world/water/checks.rs:62,276-278,845` (`is_wall` takes the angle)
- Modify: `crates/topography-core/src/world/water/breach.rs:215,273-276` (radius in cells, angle)
- Modify: `crates/topography-core/src/world/water/channel.rs:444,1437,1458` (angle)
- Modify: `crates/topography-core/src/world/water/testing.rs` (the fixture table replaces the v3 line)
- Regenerate: `tests/fixtures/water/grid/water.toml`
- Create: `training/src/topography_training/water/table.py`, `training/tests/test_water_table.py`

**Interfaces:**
- Produces (Rust):
  - `table::Erosion { routings: u32, k_dt: f64, m: f64, diffusion: f32, cap_m: f32, interior_k: f64, interior_cap_m: f32 }` (`Clone, Copy, Debug, Deserialize, PartialEq`).
  - `table::Level { river_km2: f64, breach_cap_m: f32, wall_deg: f64, erosion: Option<Erosion> }`.
  - `Table::hot_dry(class: u8) -> bool`, `Table.below_sea_coast_km: f64`.
  - `Params { river_km2, breach_cap_m, breach_radius, channels, wall_deg: f64, erosion: Option<Erosion> }`; `Params::change_max_m()` = breach cap + erosion cap + 0.5 (B4).
  - `checks::is_wall(slope: f64, raw_slope: f64, wall_deg: f64) -> bool`.
- Produces (Python): `topography_training.water.table`: `TABLE: Path`, `load(path=TABLE) -> dict`, `r(cls: int) -> float`, `hot_dry_classes() -> list[int]`, `lake_min_m2() -> float`, `below_sea_coast_km() -> float`, `level(spacing_m: float) -> dict`.

- [ ] **Step 1: Write the table**

`configs/hydrology/water-v3.toml` — v2's file with these changes (copy the r values unchanged):

```toml
# Hydrology v3 water table (legend topography terrain-v3), read by `topography world water`
# and Python (`topography_training.water.table`). Per class: the lake-to-catchment ratio
# r ≈ P/E (a depression stands where its lake area equals Σ r over its catchment's cells,
# with upstream overflow; r >= 1 fills to the spill, r = 0 stays dry) and `hot_dry` (an
# inland basin below 0 m far from the sea may keep a lake below sea level, owner ruling
# 2026-11-06). Lake supply ignores the moisture field, which drives only Q. Classes missing
# here take class 0. Judgment values 2026-11-05 (r) and 2026-11-07 (hot_dry, levels).
schema_version = 3
kind = "water_table"
legend = "topography terrain-v3"
# Game settings: standing water is a labelled lake from this area (smaller: a pond), as
# well as 0.3 m deep; Q (km² at mean rain) to m³/s for channel sizes.
lake_min_m2 = 2000.0
m3s_per_km2 = 0.56
# A basin below 0 m keeps a lake below sea level only in a hot_dry class at least this far
# from open sea (ruling B13); elsewhere the coastal water table holds it at 0 m or more.
below_sea_coast_km = 5.0

# Per level (grid spacing in m): river start (km² at mean rain), deepest breach cut, the
# steepest changed cell (walls, breach refusal, erosion wall rounds: max(wall_deg, raw)).
[levels.1]
river_km2 = 0.05
breach_cap_m = 2.0
wall_deg = 40.0

[levels.4]
river_km2 = 0.5
breach_cap_m = 5.0
wall_deg = 35.0
# Erosion (rulings B2-B7): `routings` model runs, each followed by one implicit stream-power
# step (F = k_dt·A^m / L, A in m², n = 1) and one lowering-only diffusion step (alpha per
# routing); cuts tapered to wall_deg. Caps are below the breached ground. Interior cells
# (real GLO-30 piece interiors, glaciers) take interior_k × k_dt, no diffusion, and their cap.
[levels.4.erosion]
routings = 2
k_dt = 8.75e-5
m = 0.5
diffusion = 0.05
cap_m = 10.0
interior_k = 0.05
interior_cap_m = 3.0

[levels.32]
river_km2 = 10.0
breach_cap_m = 20.0
wall_deg = 35.0
[levels.32.erosion]
routings = 5
k_dt = 7e-4
m = 0.5
diffusion = 0.05
cap_m = 30.0
interior_k = 0.05
interior_cap_m = 3.0

[classes.0]  # unlabelled
r = 1.0
[classes.1]  # doline, pinnacle and plateau karst (drains underground: dolines stay dry)
r = 0.02
[classes.2]  # gorges and slot-like canyons
r = 0.1
hot_dry = true
[classes.3]  # fjords and glacial troughs
r = 3.0
[classes.4]  # volcanic calderas and craters
r = 1.0
[classes.5]  # archipelagos, rias and skerries
r = 2.0
[classes.6]  # alpine ridges and cirques
r = 3.0
[classes.7]  # badlands, mesas and eroded gullies (arid: dry hollows)
r = 0.0
hot_dry = true
[classes.8]  # star and mountain dunes (interdune hollows stay dry)
r = 0.0
hot_dry = true
[classes.9]  # braided river basins and floodplains
r = 2.0
[classes.10]  # glaciers and ice fields
r = 1.0
[classes.11]  # stratovolcanoes (porous)
r = 0.5
[classes.12]  # coastal cliffs and sea stacks
r = 1.5
[classes.13]  # layered plateau canyons (2)
r = 0.05
hot_dry = true
[classes.14]  # cone and tower karst (1, drains underground)
r = 0.02
[classes.15]  # drowned tower karst (5, drains underground)
r = 0.02
[classes.16]  # crest-train dunes (8)
r = 0.0
hot_dry = true
[classes.17]  # cinder cones and lava fields (11, porous)
r = 0.03
[classes.18]  # plain and rolling ground (0)
r = 1.0
[classes.19]  # Grand Canyon (2)
r = 0.05
hot_dry = true
```

```bash
git rm configs/hydrology/water-v2.toml
```

- [ ] **Step 2: Write the failing Rust test**

Replace `the_built_in_table_parses_with_r_in_thousandths` in `table.rs` with:

```rust
    #[test]
    fn the_built_in_table_parses_with_r_levels_and_erosion() {
        let t = Table::parse(DEFAULT).expect("built-in table");
        assert_eq!(t.r_milli(0), 1000);
        assert_eq!(t.r_milli(8), 0, "dunes are playas");
        assert_eq!(t.r_milli(19), 50);
        assert_eq!(t.r_milli(200), 1000, "missing classes take class 0");
        assert!(t.hot_dry(19) && t.hot_dry(8) && t.hot_dry(7) && t.hot_dry(2));
        assert!(!t.hot_dry(1) && !t.hot_dry(17) && !t.hot_dry(0), "karst and lava are not hot-dry");
        assert_eq!(t.below_sea_coast_km, 5.0);
        let l1 = t.level(1.0);
        assert_eq!((l1.river_km2, l1.breach_cap_m, l1.wall_deg), (0.05, 2.0, 40.0));
        assert!(l1.erosion.is_none(), "no erosion at 1 m");
        let e4 = t.level(4.0).erosion.expect("4 m erodes");
        assert_eq!((e4.routings, e4.cap_m, e4.interior_cap_m), (2, 10.0, 3.0));
        assert_eq!(t.level(4.0).wall_deg, 35.0);
        let e32 = t.level(32.0).erosion.expect("32 m erodes");
        assert_eq!((e32.routings, e32.k_dt, e32.cap_m), (5, 7e-4, 30.0));
    }

    #[test]
    fn schema_2_tables_and_bad_erosion_are_refused() {
        let v2 = DEFAULT.replace("schema_version = 3", "schema_version = 2");
        assert!(Table::parse(&v2).is_err());
        let bad = DEFAULT.replace("routings = 2", "routings = 0");
        assert!(Table::parse(&bad).is_err(), "at least one routing");
        let bad = DEFAULT.replace("diffusion = 0.05\ncap_m = 10.0", "diffusion = 0.3\ncap_m = 10.0");
        assert!(Table::parse(&bad).is_err(), "explicit diffusion needs alpha <= 0.25");
    }
```

and in `mod.rs` tests, beside the stage-tag test:

```rust
    #[test]
    fn params_carry_the_level_angle_erosion_and_change_bound() {
        let t = table::Table::parse(table::DEFAULT).unwrap();
        let p1 = params(&t, 1.0, None);
        assert_eq!((p1.wall_deg, p1.erosion, p1.change_max_m()), (40.0, None, 2.5));
        let p32 = params(&t, 32.0, None);
        assert_eq!(p32.wall_deg, 35.0);
        assert_eq!(p32.change_max_m(), 50.5, "breach 20 + erosion 30 + 0.5 (B4: erosion may deepen a gorge)");
        assert_eq!(params(&t, 4.0, None).change_max_m(), 15.5);
    }
```

- [ ] **Step 3: Run to see them fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water -- table params_carry`
Expected: compile errors (`hot_dry`, `wall_deg`, `erosion` missing).

- [ ] **Step 4: Implement**

`table.rs`:

```rust
pub(crate) const DEFAULT: &str = include_str!("../../../../../configs/hydrology/water-v3.toml");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Class {
    r: f64,
    #[serde(default)]
    hot_dry: bool,
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
    below_sea_coast_km: f64,
    levels: BTreeMap<String, Level>,
    classes: BTreeMap<String, Class>,
}

/// Coarse-level erosion (rulings B2–B7).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Erosion {
    pub routings: u32,
    pub k_dt: f64,
    pub m: f64,
    /// Lowering-only diffusion: alpha = D·dt/dx² per routing (explicit, at most 0.25).
    pub diffusion: f32,
    pub cap_m: f32,
    pub interior_k: f64,
    pub interior_cap_m: f32,
}

/// Per-level thresholds (review N19, rulings B4, B8).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Level {
    pub river_km2: f64,
    pub breach_cap_m: f32,
    pub wall_deg: f64,
    #[serde(default)]
    pub erosion: Option<Erosion>,
}
```

`Table` gains `hot: Vec<bool>` (by class id 0–255, missing ids take class 0's) and `pub below_sea_coast_km: f64`; `parse` checks `schema_version == 3`, `below_sea_coast_km >= 0`, per level `wall_deg` in (0, 90), and for erosion `routings >= 1`, `k_dt >= 0`, `m` in (0, 1], `0 <= diffusion <= 0.25`, caps `>= 0`, `interior_k` in [0, 1]:

```rust
            if let Some(e) = &l.erosion {
                ensure(
                    e.routings >= 1
                        && e.k_dt.is_finite() && e.k_dt >= 0.0
                        && e.m > 0.0 && e.m <= 1.0
                        && (0.0..=0.25).contains(&e.diffusion)
                        && e.cap_m >= 0.0 && e.interior_cap_m >= 0.0
                        && (0.0..=1.0).contains(&e.interior_k),
                    format!("water table level {key}: erosion routings >= 1, k_dt >= 0, 0 < m <= 1, \
                             0 <= diffusion <= 0.25, caps >= 0, 0 <= interior_k <= 1"),
                )?;
            }
```

```rust
    /// The class may keep a lake below 0 m far from the sea (ruling B13).
    pub fn hot_dry(&self, class: u8) -> bool {
        self.hot[class as usize]
    }
```

`mod.rs`:

```rust
pub(crate) struct Params {
    pub river_km2: f64,
    pub breach_cap_m: f32,
    pub breach_radius: u64,
    pub channels: bool,
    /// Steepest changed cell: max(wall_deg, raw) (B8).
    pub wall_deg: f64,
    /// Coarse levels only (B2).
    pub erosion: Option<table::Erosion>,
}

impl Params {
    /// The `change_max` check's bound: the deepest cut this level may make + 0.5 m (B4):
    /// erosion's floor is the breached ground less its cap, so the two caps add.
    pub fn change_max_m(&self) -> f32 {
        self.breach_cap_m + self.erosion.map_or(0.0, |e| e.cap_m) + 0.5
    }
}

pub(crate) fn params(table: &table::Table, spacing_m: f64, river_km2: Option<f64>) -> Params {
    let level = table.level(spacing_m);
    Params {
        river_km2: river_km2.unwrap_or(level.river_km2),
        breach_cap_m: level.breach_cap_m,
        breach_radius: BREACH_RADIUS,
        channels: spacing_m < 2.0,
        wall_deg: level.wall_deg,
        erosion: level.erosion,
    }
}
```

`setup`'s built-in source string becomes `"built-in water-v3.toml"`. `Tags::b` adds `"wall_deg": p.wall_deg` to its JSON. Update every `Params { … }` literal in the tests (`mod.rs` around lines 1009–1027) with `wall_deg: 40.0, erosion: None`.

`checks.rs`: delete `const WALL_DEG`, and:

```rust
/// A changed cell's slope (Horn, degrees) fails `walls` against its raw slope and the level's
/// angle (B8; shared with breach, channels and erosion).
pub(crate) fn is_wall(slope: f64, raw_slope: f64, wall_deg: f64) -> bool {
    slope > wall_deg.max(raw_slope) + 0.01
}
```

Every caller passes `s.params.wall_deg` (`checks.rs:845`, `breach.rs:273`, `channel.rs:444,1437`; the breach test at `breach.rs:471` passes `40.0`; `channel.rs:1458` in tests passes `40.0`).

`breach.rs` taper radius (B5), at line 215:

```rust
        // Radius in cells: the parabola's edge slope 2·d / R metres per metre stays under the
        // taper angle (Plan A wrote 2·d / tan in cells, right only at 1 m).
        let radius = (c.half as f32 + 1.0).max(2.0 * d / (tan * spacing_m as f32));
```

Add to `breach.rs` tests:

```rust
    #[test]
    fn the_taper_radius_is_in_cells_at_coarse_spacings() {
        // A 20 m cut at 32 m: 2·20 / (tan 35° · 32) = 1.79 cells, not 57.
        let tan = TAPER_DEG.to_radians().tan();
        let r = |d: f32, spacing: f32| 2.0f32.max(2.0 * d / (tan * spacing));
        assert!((r(20.0, 32.0) - 2.0).abs() < 1e-6, "half + 1 = 2 wins at 32 m");
        assert!((r(20.0, 1.0) - 57.13).abs() < 0.05, "the 1 m disc is unchanged");
    }
```

`testing.rs`: the fixture table now replaces `"lake_min_m2 = 2000.0"` in the v3 `DEFAULT` (same line, no code change beyond the comment's file name `water-v3.toml`). Regenerate the check fixture:

```bash
"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet write_check_fixture -- --ignored
```

- [ ] **Step 5: Python loader, failing test first**

`training/tests/test_water_table.py`:

```python
import unittest

from topography_training.water import table


class WaterTableTest(unittest.TestCase):
    def test_reads_r_hot_dry_and_levels(self):
        self.assertEqual(table.r(19), 0.05)
        self.assertEqual(table.r(200), table.r(0))
        self.assertEqual(sorted(table.hot_dry_classes()), [2, 7, 8, 13, 16, 19])
        self.assertEqual(table.lake_min_m2(), 2000.0)
        self.assertEqual(table.below_sea_coast_km(), 5.0)
        self.assertEqual(table.level(4.0)["wall_deg"], 35.0)
        self.assertEqual(table.level(32.0)["erosion"]["routings"], 5)
        self.assertNotIn("erosion", table.level(1.0))
        self.assertEqual(table.level(2.0)["river_km2"], 0.05, "the largest level not coarser")


if __name__ == "__main__":
    unittest.main()
```

Run: `PYTHONPATH=training/src $P -m unittest training/tests/test_water_table.py` — Expected: `ModuleNotFoundError`.

`training/src/topography_training/water/table.py`:

```python
"""The hydrology v3 water table (configs/hydrology/water-v3.toml) for Python: r per class, the
hot-dry classes, the game settings and per-level thresholds, as the Rust model reads them."""

from __future__ import annotations

import tomllib
from functools import lru_cache
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
TABLE = ROOT / "configs" / "hydrology" / "water-v3.toml"


@lru_cache(maxsize=None)
def load(path: Path = TABLE) -> dict:
    t = tomllib.loads(path.read_text())
    if t.get("schema_version") != 3 or t.get("kind") != "water_table":
        raise ValueError(f"{path} is not a schema 3 water table")
    return t


def _class(cls: int) -> dict:
    classes = load()["classes"]
    return classes.get(str(cls), classes["0"])


def r(cls: int) -> float:
    return float(_class(cls)["r"])


def hot_dry_classes() -> list[int]:
    return [int(k) for k, v in load()["classes"].items() if v.get("hot_dry", False)]


def lake_min_m2() -> float:
    return float(load()["lake_min_m2"])


def below_sea_coast_km() -> float:
    return float(load()["below_sea_coast_km"])


def level(spacing_m: float) -> dict:
    """The largest level not coarser than `spacing_m` (the finest level below every level)."""
    levels = sorted((float(k), v) for k, v in load()["levels"].items())
    fit = [v for s, v in levels if s <= spacing_m + 1e-9]
    return dict(fit[-1] if fit else levels[0][1])
```

Run the test again — Expected: PASS.

- [ ] **Step 6: Run everything**

Run the task-finish block. The 1 m behaviour is unchanged (angle 40°, no erosion), so Plan A's tests pass as they were.

- [ ] **Step 7: Commit**

```bash
git add -A configs/hydrology crates/topography-core/src/world/water tests/fixtures/water \
  training/src/topography_training/water/table.py training/tests/test_water_table.py
git commit -m "feat(water): water table v3 with hot-dry classes, wall angle and erosion per level

Fixes the breach taper radius at coarse spacings (cells, not metres).

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 8: Expert review** — output: the table and a 32 m window run on the v2 chain's `l2_raw` showing breaches now cut 1–2 cell discs:

```bash
"${CAP6[@]}" cargo build --release --locked --offline -p topography-cli
"${CAP6[@]}" "${PY[@]}" -m topography_training.water.run window --source data/worlds/fujix-55-chain \
    --level 32 --centre 25645,24400 --size 256 --out scratch/hydrology-v3b/t2-32m | tail -n 3
jq '.figures.stages.breach' scratch/hydrology-v3b/t2-32m/out/report.json
```

---
## Task 3: The provenance layer (collage replay)

Spec "Provenance and K"; rulings B10, B11, B13, B14.

**Files:**
- Modify: `training/src/topography_training/diffusion/collage.py` (`region_surface` returns piece weights; `Surface`, `surface`, `provenance`, `slope_deg`, `replay`, `write_provenance`; `_build` uses `surface` and writes the layer; `main` gains the `provenance` subcommand)
- Create: `training/tests/test_collage_provenance.py`
- Output (not committed): `data/worlds/fujix-collage-55/provenance32.u8`, `provenance32.json`, `provenance32.png` (additive sidecar, B10)

**Interfaces:**
- Consumes: `topography_training.water.table` (`hot_dry_classes`, `below_sea_coast_km`) from Task 2; `cascade.open_sea`.
- Produces:
  - Constants `SEA_P = 0, INTERIOR = 1, BLEND = 2, PAINTED = 3, FILL = 4, KIND_MASK = 7, BELOW_SEA_OK = 64, STEEP = 128`.
  - `@dataclass Surface`: `h32, blended, cls32, region32, forest32, ice, dips, pre, cal, sds, land_w, covered, wmax, npieces` (arrays at 32 m; lists per region).
  - `surface(spec, regions, srcs) -> Surface` (no files written).
  - `provenance(s: Surface, hot_dry: list[int], coast_km: float, l2: np.ndarray | None = None) -> np.ndarray` (uint8, bits as above; `l2` is the stored surface, used for the painted test, the steep flag and the sea; default `s.h32`).
  - `slope_deg(h: np.ndarray, cell_m: float) -> np.ndarray` (Horn).
  - `replay(collage: Path) -> tuple[Surface, dict]` (the surface and its agreement: `region_equal`, `class_agreement`, `l2_p99_m`, `l2_max_outside_painted_m`).
  - `write_provenance(collage: Path) -> Path` (refuses on a layout or region mismatch, class agreement < 0.999, l2 p99 > 0.5 m or max > 5 m outside painted cells; B10).
  - CLI: `python -m topography_training.diffusion.collage provenance <collage-dir>`.
  - Sidecar `provenance32.json`: `{"kind": "collage_provenance", "file": "provenance32.u8", "size": [n, n], "cell_m": 32.0, "kinds": {...}, "bits": {...}, "l2_sha256", "replay", "thresholds": {...}, "shares": {...}}`.

- [ ] **Step 1: Write the failing test**

`training/tests/test_collage_provenance.py`:

```python
import unittest

import numpy as np

from topography_training.diffusion import collage as c


def _surface(n=96):
    """Two regions split at column 48 with a 20-cell blend; one piece covers region 0's west
    60 columns (feathered over its last 8), nothing covers region 1 (fill); a painted dip at
    (70, 20); sea outside rows >= 88; a 45° wall at columns 10-11."""
    x = np.arange(n, dtype=np.float32)[None, :].repeat(n, 0)
    w0 = np.clip((58 - x) / 20, 0, 1)
    w1 = 1 - w0
    piece = np.clip((60 - x) / 8, 0, 1)
    h = np.full((n, n), 100.0, np.float32)
    h[:, :10] = 100.0 + 32.0 * 2
    h[:, 10] = 100.0 + 32.0
    blended = h.copy()
    h[15:25, 65:75] -= 30.0  # painted dip
    region32 = np.where(x < 48, 1, 2).astype(np.uint8)
    region32[88:] = 0
    h[88:] = -50.0
    blended[88:] = -50.0
    cls = np.full((n, n), 18, np.uint8)
    cls[:, 80:] = 8  # dunes in the east
    z = np.zeros((n, n), np.float32)
    return c.Surface(h32=h, blended=blended, cls32=cls, region32=region32, forest32=z, ice=z > 0,
                     dips={}, pre=h, cal=None, sds=[], land_w=[w0, w1],
                     covered=[piece > 1e-3, np.zeros((n, n), bool)],
                     wmax=[piece, np.zeros((n, n), np.float32)],
                     npieces=[(piece >= c.PIECE_MIN_W).astype(np.uint8), np.zeros((n, n), np.uint8)])


class ProvenanceTest(unittest.TestCase):
    def test_kinds_and_flags(self):
        p = c.provenance(_surface(), hot_dry=[8], coast_km=1.0)
        kind = p & c.KIND_MASK
        self.assertEqual(kind[50, 2], c.INTERIOR, "past the feather and 1 km from any blend")
        self.assertEqual(kind[50, 20], c.BLEND, "within 1 km (32 cells) of the region blend at x >= 39")
        self.assertEqual(kind[20, 70], c.PAINTED)
        self.assertEqual(kind[50, 85], c.FILL, "no piece covers region 1")
        self.assertEqual(kind[90, 50], c.SEA_P)
        self.assertTrue(p[50, 10] & c.STEEP, "the 45° wall is steep source")
        self.assertFalse(p[50, 40] & c.STEEP)
        self.assertTrue(p[30, 90] & c.BELOW_SEA_OK, "dunes over 1 km from the sea")
        self.assertFalse(p[86, 90] & c.BELOW_SEA_OK, "dunes beside the sea")
        self.assertFalse(p[30, 60] & c.BELOW_SEA_OK, "plain is not hot-dry")

    def test_a_wide_single_piece_has_an_interior(self):
        s = _surface(n=192)
        n = 192
        x = np.arange(n, dtype=np.float32)[None, :].repeat(n, 0)
        one = np.ones((n, n), np.float32)
        s.land_w = [one, np.zeros((n, n), np.float32)]
        s.wmax = [one, np.zeros((n, n), np.float32)]
        s.npieces = [np.ones((n, n), np.uint8), np.zeros((n, n), np.uint8)]
        s.covered = [np.ones((n, n), bool), np.zeros((n, n), bool)]
        s.region32 = np.ones((n, n), np.uint8)
        s.h32 = np.full((n, n), 100.0, np.float32)
        s.blended = s.h32.copy()
        s.cls32 = np.full((n, n), 18, np.uint8)
        kind = c.provenance(s, hot_dry=[], coast_km=5.0) & c.KIND_MASK
        self.assertEqual(kind[96, 96], c.INTERIOR)
        self.assertTrue((kind == c.INTERIOR).all(), "no other kind anywhere: all interior")
        del x

    def test_slope_deg_is_horn(self):
        h = (np.arange(8, dtype=np.float32)[None, :] * 32.0).repeat(8, 0)
        self.assertAlmostEqual(float(c.slope_deg(h, 32.0)[4, 4]), 45.0, places=4)


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run it to see it fail**

Run: `PYTHONPATH=training/src $P -m unittest training/tests/test_collage_provenance.py`
Expected: `AttributeError: module ... has no attribute 'Surface'`.

- [ ] **Step 3: Implement**

In `collage.py`, `region_surface` also returns the largest single piece weight and the count of pieces reaching each cell:

```python
PIECE_MIN_W = 0.01  # a piece reaches a cell from this weight


def region_surface(r: Region, srcs: dict[str, Source], x: np.ndarray, y: np.ndarray, noise: np.ndarray):
    """Heights (filled smoothly past the pieces), classes dithered by piece weight, covered
    cells, the largest single piece weight and how many pieces reach each cell."""
    num = np.zeros(x.shape, np.float32)
    den = np.zeros(x.shape, np.float32)
    wmax = np.zeros(x.shape, np.float32)
    npieces = np.zeros(x.shape, np.uint8)
    classes, weights = [np.full(x.shape, r.class_id, np.uint8)], [np.full(x.shape, 1e-4, np.float32)]
    for p in r.pieces:
        src = srcs[p.source]
        h, cl, w = place(p, src, x, y)
        num += w * h
        den += w
        wmax = np.maximum(wmax, w)
        npieces += (w >= PIECE_MIN_W).astype(np.uint8)
        classes.append(np.where(cl == src.class_id, p.class_id or r.class_id, cl).astype(np.uint8))
        weights.append(w)
    covered = den > 1e-3
    return (fill_smooth(num / np.maximum(den, 1e-6), covered), dither(classes, weights, noise), covered,
            wmax, npieces)
```

Add after `region_map`:

```python
SEA_P, INTERIOR, BLEND, PAINTED, FILL = 0, 1, 2, 3, 4  # provenance kinds (bits 0-2)
KIND_MASK = 7
BELOW_SEA_OK = 64  # an inland basin may keep a lake below 0 m (hot-dry, far from the sea)
STEEP = 128  # the stored collage is at least STEEP_DEG steep (a figure tag on dam_drops, B14)
INTERIOR_SHARE = 0.99  # dominant region's weight share and its piece's weight for an interior
INTERIOR_MARGIN_M = 1000.0  # the pyramid's coarsest band mixes this far past any blend
PAINTED_M = 0.5  # authored changes after the blend (dips, calderas, edge limit, sinking)
STEEP_DEG = 30.0
REPLAY_P99_M = 0.5  # provenance needs weights and regions, not centimetre heights (B10)
REPLAY_MAX_M = 5.0
REPLAY_CLASS_AGREEMENT = 0.999


@dataclass
class Surface:
    """The collage's 32 m surface and what made it (one entry per region in the lists)."""

    h32: np.ndarray
    blended: np.ndarray  # the pyramid blend before sinking, edge limit, dips and calderas
    cls32: np.ndarray
    region32: np.ndarray
    forest32: np.ndarray
    ice: np.ndarray
    dips: dict
    pre: np.ndarray
    cal: object
    sds: list
    land_w: list
    covered: list
    wmax: list
    npieces: list


def surface(spec: dict, regions: list[Region], srcs: dict[str, Source]) -> Surface:
    """Every surface step of a build, in memory (no files)."""
    n = int(spec["extent_m"] / CELL_M)
    x, y = grid(n)
    sds, surfaces, classes, covered, wmax, npieces = [], [], [], [], [], []
    piece_noise = blue_noise(n, seed=PIECE_NOISE_SEED)
    dx, dy = warp_field(n)
    for r in regions:
        sds.append(warp(description.polygon_sd(r.outline, n, CELL_M), dx, dy))
        h, cl, cov, wm, npc = region_surface(r, srcs, x, y, piece_noise)
        surfaces.append(h)
        classes.append(cl)
        covered.append(cov)
        wmax.append(wm)
        npieces.append(npc)
    isd = island_sd(sds)
    land_w = [weight(sd, r.border_m) for sd, r in zip(sds, regions)]
    sea = ocean(isd, spec)
    blended = blend(surfaces + [sea], land_w + [weight(-isd, OCEAN_BORDER_M)])
    h32 = sink_outside(blended, isd, sea)
    h32 = np.minimum(h32, edge_limit(n, spec))
    h32, dips = dip_passes(h32, spec)
    pre = h32.astype(np.float64)
    h32, ice, cal = calderas(h32, spec, x, y)
    total = np.maximum(np.sum(land_w, axis=0), 1e-6)
    forest32 = np.where(h32 >= 0, sum(w * r.forest for w, r in zip(land_w, regions)) / total, 0).astype(np.float32)
    forest32[ice] = 0
    cls32 = dither(classes, land_w, blue_noise(n, seed=REGION_NOISE_SEED))
    cls32[ice] = GLACIER_CLASS
    region32 = region_map(h32, sds, land_w)
    return Surface(h32, blended, cls32, region32, forest32, ice, dips, pre, cal, sds, land_w, covered,
                   wmax, npieces)


def slope_deg(h: np.ndarray, cell_m: float) -> np.ndarray:
    """Horn's slope in degrees, edges padded."""
    p = np.pad(h.astype(np.float64), 1, mode="edge")
    a, b, cc = p[:-2, :-2], p[:-2, 1:-1], p[:-2, 2:]
    d, f = p[1:-1, :-2], p[1:-1, 2:]
    g, hh, i = p[2:, :-2], p[2:, 1:-1], p[2:, 2:]
    gx = ((cc + 2 * f + i) - (a + 2 * d + g)) / (8 * cell_m)
    gy = ((g + 2 * hh + i) - (a + 2 * b + cc)) / (8 * cell_m)
    return np.degrees(np.arctan(np.hypot(gx, gy)))


def provenance(s: Surface, hot_dry: list[int], coast_km: float, l2: np.ndarray | None = None) -> np.ndarray:
    """Kind per 32 m cell (bits 0-2) with the below-sea and steep-source flags (ruling B10).
    Heights (painted test, steep flag, sea) come from the stored surface `l2` when given."""
    from scipy import ndimage

    h = s.h32 if l2 is None else l2

    lw = np.stack(s.land_w)
    k = lw.argmax(axis=0)[None]
    share = np.take_along_axis(lw, k, 0)[0] / np.maximum(lw.sum(axis=0), 1e-6)

    def pick(a):
        return np.take_along_axis(np.stack(a), k, 0)[0]

    out = np.full(s.h32.shape, BLEND, np.uint8)
    out[(share >= INTERIOR_SHARE) & (pick(s.wmax) >= INTERIOR_SHARE) & (pick(s.npieces) == 1)] = INTERIOR
    out[~pick(s.covered)] = FILL
    margin = int(round(INTERIOR_MARGIN_M / CELL_M))
    near = ndimage.binary_dilation(out != INTERIOR, np.ones((3, 3), bool), iterations=margin)
    out[(out == INTERIOR) & near] = BLEND
    out[np.abs(h - s.blended) > PAINTED_M] = PAINTED
    out[s.region32 == 0] = SEA_P
    sea = cascade.open_sea(h)
    far = ndimage.distance_transform_edt(~sea) * CELL_M >= coast_km * 1000.0
    out[np.isin(s.cls32, hot_dry) & far & ~sea] |= BELOW_SEA_OK
    out[slope_deg(h, CELL_M) >= STEEP_DEG] |= STEEP
    return out
```

(Check `region_map`'s outline-less sea: `region32 == 0` is "sea outside every region outline", per `collage.json`.)

Replace the body of `_build` up to `write_l0` with a call to `surface`, and write the layer:

```python
def _build(spec: dict, text: str, regions: list[Region], srcs: dict[str, Source], out: Path) -> np.ndarray:
    s = surface(spec, regions, srcs)
    h32, cls32, region32 = s.h32, s.cls32, s.region32
    write_l0(out, text, spec, h32, cls32, s.forest32)
    cls32.tofile(out / "class32.u8")
    region32.tofile(out / "region32.u8")
    np.savez_compressed(out / "arrays.npz", l1=cascade.block_mean(h32, int(L0_CELL_M / CELL_M)), l2=h32)
    _write_provenance_files(out, s, h32, replay={"built": True})
    l0 = layout.load(out)
    layout.overview(l0, out / "overview.png")
    text_r, warnings = report(spec, regions, h32, s.sds, s.land_w, s.covered, region32, s.ice, s.dips, s.pre, s.cal)
    (out / "report.txt").write_text(text_r)
    print(text_r, end="")
    text_b, bands, _ = layout.reach(h32, CELL_M, spec)
    (out / "reach-l2.txt").write_text(text_b)
    layout.reach_png(h32, CELL_M, bands, spec, out / "reach-l2.png")
    description.write(out, description.measure(spec, h32, CELL_M, "collage", region32))
    sheet(out, regions, srcs, h32)
    return h32
```

and the replay and sidecar:

```python
def replay(collage: Path) -> tuple[Surface, dict]:
    """The collage's surface recomputed from its layout and release, and how well it agrees with
    the stored build (B10)."""
    meta = json.loads((collage / "collage.json").read_text())
    layout_path = Path(meta["layout"])
    text = layout_path.read_text()
    if hashlib.sha256(text.encode()).hexdigest() != meta["layout_sha256"]:
        raise SystemExit(f"error: {layout_path} changed since {collage} was built; cannot replay")
    spec = tomllib.loads(text)
    regions = parse(spec)
    srcs = sources(Path(meta["release"]), {p.source for r in regions for p in r.pieces})
    s = surface(spec, regions, srcs)
    l2 = np.load(collage / "arrays.npz")["l2"]
    region = np.fromfile(collage / "region32.u8", np.uint8).reshape(l2.shape)
    cls = np.fromfile(collage / "class32.u8", np.uint8).reshape(l2.shape)
    d = np.abs(s.h32.astype(np.float64) - l2)
    painted = np.abs(l2 - s.blended) > PAINTED_M
    return s, {"region_equal": bool((region == s.region32).all()),
               "class_agreement": float((cls == s.cls32).mean()),
               "l2_p99_m": float(np.percentile(d, 99)),
               "l2_max_outside_painted_m": float(d[~painted].max(initial=0.0))}


def _write_provenance_files(out: Path, s: Surface, l2: np.ndarray, replay: dict) -> Path:
    from PIL import Image

    from ..water import table

    prov = provenance(s, table.hot_dry_classes(), table.below_sea_coast_km(), l2)
    path = out / "provenance32.u8"
    prov.tofile(path)
    kind = prov & KIND_MASK
    land = l2 >= 0
    shares = {name: round(float((kind[land] == v).mean()), 4)
              for name, v in (("interior", INTERIOR), ("blend", BLEND), ("painted", PAINTED), ("fill", FILL))}
    (out / "provenance32.json").write_text(json.dumps({
        "kind": "collage_provenance", "file": path.name, "size": list(prov.shape), "cell_m": CELL_M,
        "kinds": {"0": "sea outside every region", "1": "piece interior", "2": "blend zone",
                  "3": "painted feature", "4": "exemplar fill"},
        "bits": {"64": "below-sea basin may keep its lake (hot-dry, far from open sea)",
                 "128": f"steep source (Horn slope >= {STEEP_DEG} deg)"},
        "l2_sha256": hashlib.sha256(np.ascontiguousarray(l2, "<f4").tobytes()).hexdigest(),
        "replay": replay,
        "thresholds": {"interior_share": INTERIOR_SHARE, "interior_margin_m": INTERIOR_MARGIN_M,
                       "painted_m": PAINTED_M, "steep_deg": STEEP_DEG,
                       "coast_km": table.below_sea_coast_km(), "hot_dry": table.hot_dry_classes()},
        "shares_of_land": shares}, indent=1))
    colours = np.array([[40, 70, 140], [70, 160, 70], [230, 200, 60], [220, 80, 60], [160, 100, 200],
                        [0, 0, 0], [0, 0, 0], [0, 0, 0]], np.uint8)
    rgb = colours[kind]
    rgb[(prov & STEEP) > 0] //= 2
    Image.fromarray(rgb).save(out / "provenance32.png")
    return path


def write_provenance(collage: Path) -> Path:
    """The provenance sidecar of an existing collage (ruling B10): replayed, checked, additive."""
    if (collage / "provenance32.u8").exists():
        raise SystemExit(f"error: {collage / 'provenance32.u8'} exists")
    s, agree = replay(collage)
    ok = (agree["region_equal"] and agree["class_agreement"] >= REPLAY_CLASS_AGREEMENT
          and agree["l2_p99_m"] <= REPLAY_P99_M and agree["l2_max_outside_painted_m"] <= REPLAY_MAX_M)
    if not ok:
        raise SystemExit(f"error: the replay does not reproduce {collage} ({agree}); "
                         "a rebuilt collage is a new world (owner question 2)")
    l2 = np.load(collage / "arrays.npz")["l2"]
    return _write_provenance_files(collage, s, l2, agree)
```

`main` gains a subcommand without breaking the existing call form:

```python
def main(argv=None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    if argv[:1] == ["provenance"]:
        p = argparse.ArgumentParser(description="write a collage's provenance sidecar (replayed)")
        p.add_argument("collage", type=Path)
        a = p.parse_args(argv[1:])
        print(write_provenance(a.collage))
        return 0
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("layout", type=Path, help="layout v2 TOML")
    p.add_argument("--release", type=Path, required=True, help="context release.json")
    p.add_argument("--out", type=Path, required=True, help="new output directory")
    args = p.parse_args(argv)
    for d in (args.out, partial_dir(args.out)):
        if d.exists():
            print(f"error: {d} exists" + (" (left by a killed build; remove it)" if d != args.out else ""),
                  file=sys.stderr)
            return 2
    return build(args.layout, args.release, args.out)
```

- [ ] **Step 4: Run the tests**

Run: `PYTHONPATH=training/src $P -m unittest training/tests/test_collage_provenance.py`
Expected: PASS.

- [ ] **Step 5: Write the fujix sidecar** (CPU about 90 s, under 6 GB; announce only)

```bash
"${CAP6[@]}" "${PY[@]}" -m topography_training.diffusion.collage provenance data/worlds/fujix-collage-55
jq '{replay, shares_of_land}' data/worlds/fujix-collage-55/provenance32.json
```

Expected: the replay's agreement printed within B10's tolerances and the shares printed. If it refuses, stop and ask owner question 2 (a rebuilt collage is a new world). Look at `provenance32.png` next to `overview.png`: interiors (green) inside each region away from borders, the canyon plateau largely interior, blends (yellow) along region borders, painted (red) at the dips and the caldera, fill (purple) where no piece reaches.

- [ ] **Step 6: Commit**

```bash
git add training/src/topography_training/diffusion/collage.py training/tests/test_collage_provenance.py
git commit -m "feat(collage): provenance layer (interior, blend, painted, fill; below-sea and steep flags)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 7: Expert review** — output: `provenance32.png`, `provenance32.json`, and the class shares of the canyon plateau region (`region32 == 3`).

---

## Task 4: Provenance in the model; the coastal water table and `lake_below_sea`

Ruling B13; owner ruling on below-sea lakes.

**Files:**
- Modify: `crates/topography-core/src/world/water/pipeline.rs:31-44,73-82,706` (`Setup.provenance`, `supply` with ground)
- Modify: `crates/topography-core/src/world/water/mod.rs` (`setup` reads `provenance.u8`; stamp)
- Modify: `crates/topography-core/src/world/water/checks.rs` (`lake_below_sea`)
- Modify: `crates/topography-core/src/world/water/testing.rs` (`setup` gets `provenance: None`; a sub-zero bowl helper)

**Interfaces:**
- Produces:
  - `pipeline::Setup.provenance: Option<Classes>` (u8 per provenance cell, `provenance_cell_m` in grid.json; absent → every cell 0: full K, no flags).
  - `pub(crate) const BELOW_SEA_OK: u8 = 64; pub(crate) const STEEP: u8 = 128; pub(crate) const KIND_MASK: u8 = 7; pub(crate) const INTERIOR: u8 = 1;` in `mod.rs`.
  - `Setup::below_sea(&self, core: Rect, ground: &[f32]) -> Result<Vec<(u64, u64)>>`: per core cell (1 if ground < 0 m, 1 if also without bit 64), summed per drainage end beside the supply (`TileGraph.below0: BTreeMap<Next, (u64, u64)>`) and up the hierarchy as Fill-Spill-Merge carries the supply (integer sums, exact at any tiling, R4).
  - `balance::coastal_floor(k_balance: u64, sub_zero: u64, bad: u64) -> u64`: the closed lake's cell count k after the coastal water table (B13).
  - `Setup::flags(&self, r: Rect) -> Result<Vec<u8>>` (provenance over a window; zeros without a layer).
  - Hard check `lake_below_sea`.

- [ ] **Step 1: Write the failing test**

In `pipeline.rs` tests (or `balance.rs` tests where Plan A's arid-basin test lives), add:

```rust
    /// The coastal water table (B13): a closed dune basin below 0 m fills to at least 0 m
    /// unless its provenance allows a below-sea lake.
    #[test]
    fn a_basin_below_sea_level_stands_at_sea_level_unless_hot_dry_and_far() {
        let world = World { w: 64, h: 64 };
        // A dune bowl (r = 0) at (16, 46) whose floor is 6 m below sea level, rim 4 m above.
        let mut z = testing::terrain(world);
        for (i, v) in z.iter_mut().enumerate() {
            let (x, y) = ((i as u64 % 64) as f32, (i as u64 / 64) as f32);
            let d = ((x - 16.0).powi(2) + (y - 46.0).powi(2)).sqrt();
            if d < 6.0 {
                *v = -6.0 + d;
            } else if d < 8.0 {
                *v = v.max(4.0);
            }
        }
        let bowl_with = |flags: Vec<u8>| -> Node {
            let mut s = testing::setup(world, 64, 1e-6);
            s.provenance = Some(Classes::new(Layer::memory(world, flags), 1, world).unwrap());
            let m = model(&s, &Layer::memory(world, z.clone()), None, "m", None).expect("model");
            // The only node reaching 5 m below sea level is the bowl (the lagoon's floor is -2 m).
            *m.water.nodes.0.iter().find(|n| n.low < -5.0).expect("the bowl is a node")
        };
        let bowl = |flags: u8| bowl_with(vec![flags; 64 * 64]);
        let humid = bowl(0);
        assert!(humid.level >= 0.0, "near the coast or not hot-dry: at least sea level, got {}", humid.level);
        let arid = bowl(crate::world::water::BELOW_SEA_OK);
        assert!(arid.level < 0.0, "a hot-dry basin far from the sea keeps its r = 0 balance, got {}", arid.level);
        // A mixed basin (review I4): one sub-zero cell without the flag floors the whole basin.
        let mut mixed = vec![crate::world::water::BELOW_SEA_OK; 64 * 64];
        mixed[(46 * 64 + 18) as usize] = 0;
        let m = bowl_with(mixed);
        assert!(m.level >= 0.0, "one coastal cell floors the basin, got {}", m.level);
    }
```

(`Node` from `nodes.rs`. With r = 0 and the flag set the bowl holds no water, so its node's level is its low.)

And in `checks.rs` tests, beside the other per-detector tests (Plan A's `Fixture` in `checks.rs` ~line 1600 builds `CheckLayers` from vectors):

```rust
    #[test]
    fn lake_below_sea_fails_without_the_flag() {
        let mut f = Fixture::lake_at(-3.0); // a labelled lake at -3 m
        let t = f.check();
        assert!(t.fails.get("lake_below_sea").copied().unwrap_or(0) > 0);
        f.flags = vec![crate::world::water::BELOW_SEA_OK; f.cells()];
        assert_eq!(f.check().fails.get("lake_below_sea").copied().unwrap_or(0), 0);
    }
```

If Plan A's check fixture has no `lake_at` constructor, add one in the test module: a 16² window with a bowl whose cells under −3 m carry a lake body (id 2) at level −3 m in `bodies`, the rest dry; and a `flags: Vec<u8>` field that `check()` passes to `check_tile` through `Setup.provenance`.

- [ ] **Step 2: Run them to see them fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water -- below_sea`
Expected: compile errors (`provenance`, `BELOW_SEA_OK`).

- [ ] **Step 3: Implement**

`mod.rs`:

```rust
/// Provenance bits (collage provenance32.u8; ruling B10): kind in bits 0–2.
pub(crate) const KIND_MASK: u8 = 7;
pub(crate) const INTERIOR: u8 = 1;
/// An inland basin here may keep a lake below 0 m (hot-dry, far from the sea; B13).
pub(crate) const BELOW_SEA_OK: u8 = 64;
/// The collage is at least 30° steep here: a real cliff or cascade (B14).
pub(crate) const STEEP: u8 = 128;
```

In `setup`, after the classes (same pattern, its own cell size):

```rust
    let provenance = if grid.join("provenance.u8").exists() {
        let cell_m = manifest
            .get("provenance_cell_m")
            .and_then(Value::as_f64)
            .unwrap_or(g.spacing_m);
        let k = cell_m / g.spacing_m;
        ensure(
            k >= 1.0 && k.fract() == 0.0,
            "provenance_cell_m must be a whole multiple of spacing_m",
        )?;
        let k = k as u64;
        let pw = geom::World { w: world.w.div_ceil(k), h: world.h.div_ceil(k) };
        let layer = layers::Layer::open(pw, &grid.join("provenance.u8"))?;
        Some(layers::Classes::new(layer, k, world)?)
    } else {
        None
    };
```

Add `"provenance.u8": identity("provenance.u8")?` to the stamp and `provenance` to the `Setup` literal.

`pipeline.rs`:

```rust
pub(crate) struct Setup {
    // … as before …
    /// Collage provenance (kind and flags), nearest; None: every cell 0.
    pub provenance: Option<Classes>,
}

    /// Provenance over a window (zeros without a layer).
    pub fn flags(&self, r: Rect) -> Result<Vec<u8>> {
        match &self.provenance {
            Some(p) => p.read(r),
            None => Ok(vec![0; r.cells()]),
        }
    }

    /// Per core cell: (ground below 0 m, and without bit 64) as 0/1 counts, summed per
    /// drainage end like the supply (B13).
    pub fn below_sea(&self, core: Rect, ground: &[f32]) -> Result<Vec<(u64, u64)>> {
        let flags = self.flags(core)?;
        Ok((0..core.cells())
            .map(|k| {
                let low = u64::from(ground[k] < 0.0);
                (low, low * u64::from(flags[k] & BELOW_SEA_OK == 0))
            })
            .collect())
    }
```

At line 706, beside the supply:

```rust
                let (win, zw, dir) = read(core)?;
                let zc = crop(win, &zw, core);
                let supply = s.supply(core)?;
                let below0 = s.below_sea(core, &zc)?;
```

`watershed::tile_graph` takes `below0: impl Fn(usize) -> (u64, u64)` and sums it into `TileGraph.below0` exactly as it sums `supply(k)` into `sup` (line 188), with the `Blob` impl extended the same way. Wherever the hierarchy and `balance.rs` sum a node's supply from its leaves, children and `foreign_supply`, sum `below0` alongside (a `(u64, u64)` added component-wise). Then, where `balance.rs` turns a closed node's supply into its cell count (R20, k = ⌈supply / 1000⌉):

```rust
/// The coastal water table (B13, review I4): a depression holding a sub-zero cell that may
/// not keep a lake below sea level stands at least at its lowest ground at or above 0 m: one
/// cell more than its sub-zero cells (all of which are its lowest). Never adds water: a
/// node whose k reaches its capacity fills and spills as before.
pub(crate) fn coastal_floor(k_balance: u64, sub_zero: u64, bad: u64) -> u64 {
    if bad > 0 { k_balance.max(sub_zero + 1) } else { k_balance }
}
```

and use `k = coastal_floor(k, below0.0, below0.1)`. Because the floor raises only a node's own level and adds no supply, nothing overflows downstream. Unit test in `balance.rs`:

```rust
    #[test]
    fn the_coastal_floor_needs_one_bad_cell_and_never_lowers() {
        assert_eq!(coastal_floor(3, 10, 0), 3, "all sub-zero cells hot-dry and far: balance");
        assert_eq!(coastal_floor(3, 10, 1), 11, "one cell near the coast floors the basin");
        assert_eq!(coastal_floor(40, 10, 5), 40, "a fuller balance stays");
    }
```

Every other `Setup { … }` literal (tests, `testing::setup`) adds `provenance: None`.

`checks.rs`, in `check_tile` beside `lake_not_flat` (read `let flags = s.flags(core)?;` once with the classes):

```rust
        // B13: a labelled lake below sea level only where every one of its cells allows it
        // (the floor's predicate: its cells are all sub-zero).
        if is_labelled_lake(body[j]) && surf[j] < -SILL_M && flags[k] & BELOW_SEA_OK == 0 {
            t.fail("lake_below_sea", x, y);
        }
```

(`is_labelled_lake`, `j` (window index) and `k` (core index) are the names the surrounding loop uses for lake cells; use the existing lake test there.) Add `"lake_below_sea"` to the `fails` list in `report`.

- [ ] **Step 4: Run the tests**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water`
Expected: PASS, including Plan A's lagoon test (the synthetic lagoon at (54, 32) is plain class r = 1 and fills to its spill anyway).

- [ ] **Step 5: Real-terrain probe on Plan A's below-sea lakes** (seconds; announce only)

The worst Plan A cases are 1 m closed lakes at −51 to −17 m in the south (`fujix-55-l4v3-water/bodies.json` ids 629, 630, 627, 638, 639, 626, 635, 637). Cut a 1 m window at lake 635 from the l4v2 raw surface, add the provenance by hand (Task 10 automates it), and run the two Rust commands:

```bash
"${CAP6[@]}" cargo build --release --locked --offline -p topography-cli
C=$(jq -r '.bodies[] | select(.id == 635) | .node' data/worlds/fujix-55-l4v3-water/bodies.json)
X=$(( C % 65536 )); N=$(( 65536 - C / 65536 ))
R=scratch/hydrology-v3b/t4-below-sea; rm -rf $R; mkdir -p $R
"${CAP6[@]}" "${PY[@]}" -c "
import json, numpy as np
from pathlib import Path
from topography_training.water import run
g = run.build_window_grid(Path('data/worlds/fujix-55-l4v2'), 1, ($X.0, $N.0), 2048, Path('$R/grid'))
m = json.loads((g / 'grid.json').read_text())
p = np.fromfile('data/worlds/fujix-collage-55/provenance32.u8', np.uint8).reshape(2048, 2048)
c0, r0 = (int(v) for v in m['offset_m']); n = m['size'][0]
p[np.ix_((np.arange(r0, r0 + n) // 32), (np.arange(c0, c0 + n) // 32))].tofile(g / 'provenance.u8')
m['provenance_cell_m'] = 1.0
(g / 'grid.json').write_text(json.dumps(m, indent=1))"
B=target/release/topography
"${CAP6[@]}" $B world water --grid $R/grid --work $R/work --tile 512 --threads 4
"${CAP6[@]}" $B world water-finish --grid $R/grid --work $R/work --ground $R/work/carved.f32 --out $R/out --tile 512 --threads 4
echo "lake 635 at E$X N$N"
jq '{lake_below_sea: .checks.lake_below_sea, lakes: .figures.lakes}' $R/out/report.json
jq '[.bodies[] | select(.kind == "lake") | .level_m] | min' $R/out/bodies.json
```

Expected: `lake_below_sea` 0 and no labelled lake below 0 m in the window (the bowl's lake now stands at 0 m or above, or is an unlabelled pond if under 2,000 m²).

- [ ] **Step 6: Commit**

```bash
git add crates/topography-core/src/world/water
git commit -m "feat(water): provenance layer in the model, coastal water table, lake_below_sea

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 7: Expert review** — output: the probe's report and its spot crop.

---
## Task 5: The erosion passes (`erosion.rs`)

Spec "Erosion"; rulings B1–B4, B6, B7, B29. Pure passes on whole-grid slices; Task 6 wires them into the run.

**Files:**
- Create: `crates/topography-core/src/world/water/erosion.rs`
- Modify: `crates/topography-core/src/world/water/mod.rs` (`mod erosion;`)

**Interfaces:**
- Consumes: `checks::{horn_deg, is_wall, CHANGED_M}`, `geom::{World, OFFSETS}`, `nodes::Nodes`.
- Produces:
  - `pub(crate) struct Bits` with `new(n)`, `get(i) -> bool`, `set(i)`.
  - `pub(crate) struct Sill { cell: u32, down: u32, node: u32, other_z: f32, ka: f32 }`.
  - `pub(crate) struct Routing<'a> { world, spacing_m, floor: &[f32], ka: &[f32], flow: &[u8], held: &Bits, sea: &Bits, interior: &Bits, shore: &[(u32, u32)], sills: &[Sill], nodes: &Nodes }`.
  - `pub(crate) struct Figures { lowered_cells, max_lowering_m, capped_cells, sills_lowered, order_ties, diffused_cells, tapered_cells, wall_cells_restored, walls_left, new_pits, hillslope_p99_m, hillslope_max_m }` with `to_blob() -> (Vec<u64>, f32)`, `from_blob((Vec<u64>, f32)) -> Figures`, `add(&mut self, other)`, `json() -> Value`.
  - `pub(crate) fn receiver(w: usize, h: usize, c: usize, d: u8) -> Option<usize>`.
  - `pub(crate) fn stream_power(r: &Routing, z: &mut [f32]) -> Figures` (receiver-first on ties, M3).
  - `pub(crate) fn footprint(r: &Routing, before: &[f32], z: &[f32], wall_deg: f64) -> Bits` (B6).
  - `pub(crate) fn diffuse(r: &Routing, z: &mut [f32], alpha: f32, near: &Bits) -> u64`.
  - `pub(crate) fn taper(r: &Routing, z: &mut [f32], before: &[f32], raw: &[f32], wall_deg: f64, lower: &mut [f32]) -> (u64, u64, u64)` (tapered cells, cells restored by wall rounds, walls left).
  - Constants `SHAPE_MIN_M = 0.01`, `WALL_ROUNDS = 16`, `TAPER_MIN_CELLS = 1.5`.

- [ ] **Step 1: Write the failing tests**

At the bottom of the new `erosion.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::nodes::Node;

    /// A w × 1 strip flowing east (direction 4) to an outlet at the east end; discharge 10 km²
    /// everywhere (A^0.5 = 3162 m), k_dt 7e-4 → ka = 2.21; 32 m cells.
    struct Strip {
        z: Vec<f32>,
        floor: Vec<f32>,
        ka: Vec<f32>,
        flow: Vec<u8>,
        held: Bits,
        sea: Bits,
        interior: Bits,
        shore: Vec<(u32, u32)>,
        sills: Vec<Sill>,
        nodes: Nodes,
    }

    impl Strip {
        fn new(z: &[f32]) -> Self {
            let n = z.len();
            let mut flow = vec![4u8; n];
            flow[n - 1] = 0;
            Self {
                z: z.to_vec(),
                floor: vec![f32::NEG_INFINITY; n],
                ka: vec![(7e-4 * (10.0f64 * 1e6).sqrt()) as f32; n],
                flow,
                held: Bits::new(n),
                sea: Bits::new(n),
                interior: Bits::new(n),
                shore: Vec::new(),
                sills: Vec::new(),
                nodes: Nodes::default(),
            }
        }
        fn routing(&self) -> Routing<'_> {
            Routing {
                world: World { w: self.z.len() as u64, h: 1 },
                spacing_m: 32.0,
                floor: &self.floor,
                ka: &self.ka,
                flow: &self.flow,
                held: &self.held,
                sea: &self.sea,
                interior: &self.interior,
                shore: &self.shore,
                sills: &self.sills,
                nodes: &self.nodes,
            }
        }
        fn erode(&mut self) -> Figures {
            let mut z = self.z.clone();
            let f = stream_power(&self.routing(), &mut z);
            self.z = z;
            f
        }
    }

    #[test]
    fn a_seam_step_on_a_river_grades_and_nothing_rises() {
        let before = [40.0, 40.0, 40.0, 40.0, 10.0, 10.0, 10.0, 0.0f32];
        let mut s = Strip::new(&before);
        s.erode();
        assert!(s.z[3] < 40.0 - 1.0, "the step's lip is cut, got {}", s.z[3]);
        for i in 0..before.len() {
            assert!(s.z[i] <= before[i], "cell {i} rose");
        }
        for i in 0..before.len() - 1 {
            assert!(s.z[i] >= s.z[i + 1], "cell {i} below its receiver");
        }
    }

    #[test]
    fn interior_k_barely_moves_real_ground() {
        let before = [40.0, 39.0, 38.0, 37.0, 36.0, 35.0, 34.0, 0.0f32];
        let mut s = Strip::new(&before);
        for k in s.ka.iter_mut() {
            *k *= 0.05;
        }
        s.erode();
        let most = (0..before.len()).map(|i| before[i] - s.z[i]).fold(0.0f32, f32::max);
        assert!(most < 1.0, "an interior reach moves under 1 m per step, got {most}");
    }

    #[test]
    fn the_floor_caps_lowering() {
        let before = [40.0, 40.0, 40.0, 40.0, 10.0, 10.0, 10.0, 0.0f32];
        let mut s = Strip::new(&before);
        s.floor = before.iter().map(|v| v - 3.0).collect();
        let f = s.erode();
        assert!(s.z[3] >= 37.0 - 1e-4 && f.capped_cells > 0);
    }

    #[test]
    fn a_full_lake_drains_through_its_sill_and_its_level_follows() {
        // Cells 2–4 are a lake (node 0) at level 30; its spill edge is [4, 5] with cell 5 the
        // saddle (30 m); cell 1 drains into the lake at cell 2.
        let before = [45.0, 35.0, 25.0, 24.0, 26.0, 30.0, 12.0, 0.0f32];
        let mut s = Strip::new(&before);
        for c in [2, 3, 4] {
            s.held.set(c);
        }
        s.nodes = Nodes(vec![Node {
            key: 3,
            level: 30.0,
            low: 24.0,
            full: true,
            lake: true,
            outlet: [4, 5],
            cells: 3,
            ..Default::default()
        }]);
        s.shore = vec![(2, 0)];
        s.sills = vec![Sill { cell: 5, down: 6, node: 0, other_z: 26.0, ka: s.ka[5] * 2.0 }];
        let f = s.erode();
        assert!(s.z[5] < 30.0 - 1.0, "the sill incises, got {}", s.z[5]);
        assert_eq!(f.sills_lowered, 1);
        assert!(s.z[1] < 35.0, "the cell above the lake erodes toward the lowered level");
        assert!(s.z[1] >= s.z[5].max(26.0) - 1e-4, "never below the new level");
        assert_eq!(&s.z[2..5], &before[2..5], "ground under the lake does not erode");
    }

    #[test]
    fn rivers_into_the_sea_stop_at_sea_level() {
        let before = [20.0, 10.0, 2.0, -30.0, -40.0f32];
        let mut s = Strip::new(&before);
        s.sea.set(3);
        s.sea.set(4);
        s.erode();
        assert!(s.z[2] >= 0.0, "base 0 m at a mouth, got {}", s.z[2]);
        assert_eq!(&s.z[3..], &before[3..]);
    }

    #[test]
    fn diffusion_only_lowers_convex_cells() {
        // 5 × 5 with a 2 m bump at the centre and a 1 m pit beside it.
        let w = 5usize;
        let mut z = vec![10.0f32; 25];
        z[12] = 12.0;
        z[13] = 9.0;
        let mut s = Strip::new(&z);
        s.flow = vec![0; 25];
        let mut r = s.routing();
        r.world = World { w: w as u64, h: 5 };
        let mut out = z.clone();
        let mut all = Bits::new(25);
        (0..25).for_each(|i| all.set(i));
        let n = diffuse(&r, &mut out, 0.05, &all);
        assert!(out[12] < 12.0 && n >= 1, "the bump drops");
        assert_eq!(out[13], 9.0, "the pit is not filled");
        assert!(out.iter().zip(&z).all(|(a, b)| a <= b), "nothing rises");
        let mut far = z.clone();
        assert_eq!(diffuse(&r, &mut far, 0.05, &Bits::new(25)), 0, "no cut nearby: the bump stays (B6)");
    }

    #[test]
    fn a_tie_processes_the_receiver_first() {
        // Cell 3 equals its receiver 4 (a tie); 4 is cut, so 3 must see 4's new height.
        let before = [40.0, 30.0, 20.0, 10.0, 10.0, 5.0, 0.0f32];
        let mut s = Strip::new(&before);
        let f = s.erode();
        assert!(f.order_ties >= 1);
        assert!(s.z[3] < 10.0, "the tie cell erodes toward its lowered receiver, got {}", s.z[3]);
        for i in 0..before.len() - 1 {
            assert!(s.z[i] >= s.z[i + 1] - 1e-5, "cell {i} below its receiver");
        }
    }

    #[test]
    fn wall_rounds_never_leave_a_bed_below_its_receiver() {
        // A 9 × 9 trunk falling east on 4 m cells; its middle cut 6 m on a 36° raw side slope, so
        // the wall rounds halve some cuts; the bed grade must leave no thalweg cell below its
        // receiver (Plan A's 16b bed-rise bug).
        let (w, h) = (9usize, 9usize);
        let raw: Vec<f32> = (0..w * h)
            .map(|i| 100.0 - (i % w) as f32 * 0.5 + ((i / w) as f32 - 4.0).abs() * 2.9)
            .collect();
        let mut z = raw.clone();
        for x in 1..w - 1 {
            z[4 * w + x] -= 6.0;
        }
        let mut s = Strip::new(&raw);
        s.flow = (0..w * h).map(|i| if i % w + 1 < w { 4 } else { 0 }).collect();
        s.ka = vec![0.0; w * h];
        let mut r = s.routing();
        r.world = World { w: w as u64, h: h as u64 };
        r.spacing_m = 4.0;
        let mut lower = vec![0.0f32; w * h];
        taper(&r, &mut z, &raw, &raw, 35.0, &mut lower);
        for x in 0..w - 1 {
            let c = 4 * w + x;
            assert!(z[c] >= z[c + 1] - 1e-5, "thalweg cell {x} below its receiver");
        }
        assert!(z.iter().zip(&raw).all(|(a, b)| a <= b), "nothing rises above raw");
    }

    #[test]
    fn a_deep_cut_is_tapered_three_cells_wide_and_under_the_wall_angle() {
        // 21 × 21 flat at 100 m, 4 m cells; the centre cell was cut 5 m.
        let (w, h) = (21usize, 21usize);
        let before = vec![100.0f32; w * h];
        let mut z = before.clone();
        z[10 * w + 10] = 95.0;
        let mut s = Strip::new(&before);
        s.flow = vec![0; w * h];
        s.ka = vec![0.0; w * h];
        let mut r = s.routing();
        r.world = World { w: w as u64, h: h as u64 };
        r.spacing_m = 4.0;
        let mut lower = vec![0.0f32; w * h];
        let (tapered, _, left) = taper(&r, &mut z, &before, &before, 35.0, &mut lower);
        assert!(tapered >= 8);
        assert!(z[10 * w + 11] < 99.0 && z[10 * w + 9] < 99.0, "both sides lowered: >= 3 cells wide");
        assert_eq!(left, 0, "no wall left");
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                assert!(horn_deg(&z, w, x, y, 4.0) <= 35.0 + 0.01, "({x}, {y}) too steep");
            }
        }
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Add `mod erosion;` to `mod.rs`, then run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::erosion`
Expected: compile errors (nothing defined).

- [ ] **Step 3: Implement**

```rust
//! Coarse-level erosion (spec "Coarse stages"; rulings B1–B7, B23, B29). Each step reads one
//! model run on the current ground and lowers it:
//! 1. implicit stream power (Braun and Willett 2013, n = 1, no uplift) in ascending ground
//!    order: a dry receiver is strictly lower (R12), so it is updated first. Routing is
//!    lake-aware: cells under a node's water do not erode, cells draining into a node take
//!    its level as their base, and a full node's sill cell erodes with the node's whole
//!    discharge while the level follows it (Cordonnier et al. 2019 with one outlet per node);
//! 2. lowering-only hillslope diffusion (convex cells; no deposition, out of the spec's scope);
//! 3. each cut re-expressed as Plan A's parabolic gorge taper, then wall rounds.
//! Never below the floor (the breached ground less the cell's cap) and never raised. One
//! global single-threaded pass in a fixed order: deterministic.

use super::checks::{CHANGED_M, horn_deg, is_wall};
use super::geom::{OFFSETS, World};
use super::nodes::Nodes;
use serde_json::{Value, json};

/// Cuts shallower than this are not tapered.
pub(crate) const SHAPE_MIN_M: f32 = 0.01;
/// Wall rounds after the taper: each halves this routing's lowering in the 3 × 3 of a wall.
pub(crate) const WALL_ROUNDS: usize = 16;
/// The narrowest taper (cells): a disc of radius 1.5 lowers a 3-cell-wide cut.
pub(crate) const TAPER_MIN_CELLS: f64 = 1.5;

/// A bit per cell.
pub(crate) struct Bits(Vec<u64>);

impl Bits {
    pub fn new(n: usize) -> Self {
        Self(vec![0; n.div_ceil(64)])
    }
    pub fn get(&self, i: usize) -> bool {
        (self.0[i >> 6] >> (i & 63)) & 1 == 1
    }
    pub fn set(&mut self, i: usize) {
        self.0[i >> 6] |= 1 << (i & 63);
    }
}

/// A full node's sill: the higher cell of its spill edge, eroded with the node's discharge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sill {
    pub cell: u32,
    /// The cell it drains to: the spill edge's outside cell, or that cell's receiver.
    pub down: u32,
    /// The node's ordinal in `Nodes`.
    pub node: u32,
    /// The other spill-edge cell's ground: the level never falls below it.
    pub other_z: f32,
    /// k·A^m with A the outside cell's discharge (the node's whole flow).
    pub ka: f32,
}

/// One routing: the model run on the current ground, as the passes read it.
pub(crate) struct Routing<'a> {
    pub world: World,
    pub spacing_m: f64,
    /// The breached ground less the cell's cap.
    pub floor: &'a [f32],
    /// k·A^m per cell (k by provenance, A in m²); 0 where nothing erodes.
    pub ka: &'a [f32],
    /// The model's flow layer (low 4 bits: D8 receiver, 0 none).
    pub flow: &'a [u8],
    /// Cells under a node's water (node raster >= 2).
    pub held: &'a Bits,
    pub sea: &'a Bits,
    /// Interior and glacier cells: no diffusion.
    pub interior: &'a Bits,
    /// Held cells some dry cell drains into, with their node ordinal, sorted by cell.
    pub shore: &'a [(u32, u32)],
    /// Sorted by cell.
    pub sills: &'a [Sill],
    pub nodes: &'a Nodes,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Figures {
    pub lowered_cells: u64,
    pub max_lowering_m: f32,
    pub capped_cells: u64,
    pub sills_lowered: u64,
    pub order_ties: u64,
    pub diffused_cells: u64,
    pub tapered_cells: u64,
    pub wall_cells_restored: u64,
    pub walls_left: u64,
    /// Strict local minima of the eroded ground that were none in the breached ground (B33).
    pub new_pits: u64,
    /// Lowering of full-K cells outside the cut footprint (B6), cm-binned p99 and max.
    pub hillslope_p99_m: f32,
    pub hillslope_max_m: f32,
}

impl Figures {
    pub fn to_blob(self) -> (Vec<u64>, f32) {
        (
            vec![
                self.lowered_cells,
                self.capped_cells,
                self.sills_lowered,
                self.order_ties,
                self.diffused_cells,
                self.tapered_cells,
                self.wall_cells_restored,
                self.walls_left,
                self.new_pits,
                u64::from(self.hillslope_p99_m.to_bits()),
                u64::from(self.hillslope_max_m.to_bits()),
            ],
            self.max_lowering_m,
        )
    }
    pub fn from_blob((v, max): (Vec<u64>, f32)) -> Self {
        let g = |i: usize| v.get(i).copied().unwrap_or(0);
        Self {
            lowered_cells: g(0),
            capped_cells: g(1),
            sills_lowered: g(2),
            order_ties: g(3),
            diffused_cells: g(4),
            tapered_cells: g(5),
            wall_cells_restored: g(6),
            walls_left: g(7),
            new_pits: g(8),
            hillslope_p99_m: f32::from_bits(g(9) as u32),
            hillslope_max_m: f32::from_bits(g(10) as u32),
            max_lowering_m: max,
        }
    }
    pub fn add(&mut self, o: Figures) {
        self.lowered_cells += o.lowered_cells;
        self.max_lowering_m = self.max_lowering_m.max(o.max_lowering_m);
        self.capped_cells += o.capped_cells;
        self.sills_lowered += o.sills_lowered;
        self.order_ties += o.order_ties;
        self.diffused_cells += o.diffused_cells;
        self.tapered_cells += o.tapered_cells;
        self.wall_cells_restored += o.wall_cells_restored;
        self.walls_left += o.walls_left;
        self.new_pits = o.new_pits; // the last routing's ground is what m2 reads
        self.hillslope_p99_m = self.hillslope_p99_m.max(o.hillslope_p99_m);
        self.hillslope_max_m = self.hillslope_max_m.max(o.hillslope_max_m);
    }
    pub fn json(&self) -> Value {
        json!({
            "lowered_cells": self.lowered_cells, "max_lowering_m": self.max_lowering_m,
            "capped_cells": self.capped_cells, "sills_lowered": self.sills_lowered,
            "order_ties": self.order_ties, "diffused_cells": self.diffused_cells,
            "tapered_cells": self.tapered_cells, "wall_cells_restored": self.wall_cells_restored,
            "walls_left": self.walls_left, "erosion_new_pits": self.new_pits,
            "hillslope_lowering_p99_m": self.hillslope_p99_m, "hillslope_lowering_max_m": self.hillslope_max_m,
        })
    }
}

/// The D8 receiver of cell `c` for direction byte `d` (1 N … 8 SE; 0 none).
pub(crate) fn receiver(w: usize, h: usize, c: usize, d: u8) -> Option<usize> {
    if d == 0 || d > 8 {
        return None;
    }
    let (dy, dx) = OFFSETS[d as usize - 1];
    let (x, y) = ((c % w) as i64 + dx, (c / w) as i64 + dy);
    (x >= 0 && y >= 0 && x < w as i64 && y < h as i64).then(|| y as usize * w + x as usize)
}

/// f32 → u32 with the same order (negative values included).
fn orderable(v: f32) -> u32 {
    let b = v.to_bits();
    if b >> 31 == 1 { !b } else { b | 0x8000_0000 }
}

fn dist_cells(w: usize, a: usize, b: usize) -> f64 {
    let (dx, dy) = ((a % w) as f64 - (b % w) as f64, (a / w) as f64 - (b / w) as f64);
    dx.hypot(dy).max(1.0)
}

/// Implicit stream power, one step: z ← (z + F·base) / (1 + F), F = k·A^m / L, in ascending
/// ground order; a cell whose receiver (or, draining into a lake, the lake's sill) is not yet
/// updated has that chain processed first (review M3), so no base is stale. Never below the
/// floor or the base, never raised.
pub(crate) fn stream_power(r: &Routing, z: &mut [f32]) -> Figures {
    let (w, h) = (r.world.w as usize, r.world.h as usize);
    let sill_at = |c: usize| r.sills.binary_search_by_key(&(c as u32), |s| s.cell).ok();
    let erodible = |c: usize| !r.sea.get(c) && (!r.held.get(c) || sill_at(c).is_some());
    let shore_node = |t: usize| {
        r.shore
            .binary_search_by_key(&(t as u32), |p| p.0)
            .ok()
            .map(|i| r.shore[i].1 as usize)
    };
    // The cell whose new height this cell's base needs: its receiver, or the sill of the lake
    // it drains into.
    let needs = |c: usize| -> Option<usize> {
        if let Some(i) = sill_at(c) {
            return Some(r.sills[i].down as usize);
        }
        let t = receiver(w, h, c, r.flow[c] & 15)?;
        if r.held.get(t) && sill_at(t).is_none() {
            let o = shore_node(t)?;
            return r.sills.iter().find(|s| s.node as usize == o).map(|s| s.cell as usize);
        }
        Some(t)
    };
    let mut order: Vec<u64> = (0..z.len())
        .filter(|&c| erodible(c))
        .map(|c| (u64::from(orderable(z[c])) << 32) | c as u64)
        .collect();
    order.sort_unstable();
    let mut level: Vec<f32> = r.nodes.0.iter().map(|n| n.level).collect();
    let mut done = Bits::new(z.len());
    let mut fig = Figures::default();
    let mut stack: Vec<usize> = Vec::new();
    for key in order {
        let c0 = (key & 0xffff_ffff) as usize;
        if done.get(c0) {
            continue;
        }
        stack.push(c0);
        while let Some(&c) = stack.last() {
            if let Some(t) = needs(c).filter(|&t| erodible(t) && !done.get(t)) {
                // A tie (receiver not lower) or a lake whose sill is not done: that first.
                if stack.len() <= z.len() && !stack.contains(&t) {
                    fig.order_ties += 1;
                    stack.push(t);
                    continue;
                }
            }
            stack.pop();
            done.set(c);
            let zc = z[c];
            let (to, ka, base) = if let Some(i) = sill_at(c) {
                let s = r.sills[i];
                (s.down as usize, s.ka, z[s.down as usize])
            } else {
                let Some(to) = receiver(w, h, c, r.flow[c] & 15) else {
                    continue;
                };
                let base = if r.held.get(to) && sill_at(to).is_none() {
                    shore_node(to).map_or(z[to], |o| level[o])
                } else {
                    z[to]
                };
                (to, r.ka[c], base)
            };
            // v2: rivers into the sea grade to 0 m, not to the seabed.
            let base = if r.sea.get(to) { base.max(0.0) } else { base };
            if ka <= 0.0 || zc <= base {
                continue;
            }
            let f = f64::from(ka) / (dist_cells(w, c, to) * r.spacing_m);
            let mut v = ((f64::from(zc) + f * f64::from(base)) / (1.0 + f)) as f32;
            if v < r.floor[c] {
                v = r.floor[c];
                fig.capped_cells += 1;
            }
            let v = v.min(zc);
            if zc - v > CHANGED_M {
                fig.lowered_cells += 1;
                fig.max_lowering_m = fig.max_lowering_m.max(zc - v);
            }
            z[c] = v;
            if let Some(i) = sill_at(c) {
                let s = r.sills[i];
                let n = &r.nodes.0[s.node as usize];
                let lv = v.max(s.other_z).max(n.low).min(n.level);
                if lv < level[s.node as usize] - CHANGED_M {
                    fig.sills_lowered += 1;
                }
                level[s.node as usize] = lv;
            }
        }
    }
    fig
}

/// Cells within R_taper + 1 of a cell this routing's stream power lowered by at least
/// SHAPE_MIN_M: where diffusion may act (B6, review I3).
pub(crate) fn footprint(r: &Routing, before: &[f32], z: &[f32], wall_deg: f64) -> Bits {
    let (w, h) = (r.world.w as usize, r.world.h as usize);
    let tan = wall_deg.to_radians().tan();
    let mut near = Bits::new(z.len());
    for c in 0..z.len() {
        let d = before[c] - z[c];
        if d < SHAPE_MIN_M {
            continue;
        }
        let radius = (2.0 * f64::from(d) / (tan * r.spacing_m)).max(TAPER_MIN_CELLS) + 1.0;
        let k = radius.ceil() as i64;
        let (cx, cy) = ((c % w) as i64, (c / w) as i64);
        for dy in -k..=k {
            for dx in -k..=k {
                let (x, y) = (cx + dx, cy + dy);
                if ((dx * dx + dy * dy) as f64).sqrt() <= radius && x >= 0 && y >= 0 && x < w as i64 && y < h as i64 {
                    near.set(y as usize * w + x as usize);
                }
            }
        }
    }
    near
}

/// Lowering-only hillslope diffusion, one explicit Jacobi step (alpha = D·dt/dx² <= 0.25),
/// only on `near` cells (the river cuts' footprint, B6): a convex cell moves toward its 4
/// neighbours' mean, never below the floor; concave cells, interior, held, sea and edge cells
/// stay. Returns the cells lowered.
pub(crate) fn diffuse(r: &Routing, z: &mut [f32], alpha: f32, near: &Bits) -> u64 {
    let (w, h) = (r.world.w as usize, r.world.h as usize);
    if alpha <= 0.0 || w < 3 || h < 3 {
        return 0;
    }
    let mut above = z[..w].to_vec();
    let mut row = vec![0f32; w];
    let mut n = 0;
    for y in 1..h - 1 {
        row.copy_from_slice(&z[y * w..(y + 1) * w]);
        for x in 1..w - 1 {
            let c = y * w + x;
            if !near.get(c) || r.held.get(c) || r.sea.get(c) || r.interior.get(c) {
                continue;
            }
            // above: row y - 1 before this step; z[c + w]: row y + 1, not yet updated.
            let lap = above[x] + z[c + w] + row[x - 1] + row[x + 1] - 4.0 * row[x];
            if lap >= 0.0 {
                continue;
            }
            let v = (row[x] + alpha * lap).max(r.floor[c]);
            if v < z[c] {
                z[c] = v;
                n += 1;
            }
        }
        std::mem::swap(&mut above, &mut row);
    }
    n
}

/// Each cut (before − z ≥ SHAPE_MIN_M) as a parabolic disc d·(1 − (ρ/R)²), R = max(1.5 cells,
/// 2·d / (tan(wall)·spacing)); the deepest lowering over all discs, not under water or sea,
/// never below the floor. Then up to WALL_ROUNDS rounds, each halving this step's lowering in
/// the 3 × 3 of every changed cell steeper than max(wall, raw) and then grading beds (a
/// changed cell below its receiver is lifted to it, never above `before`; B33, Plan A 16b),
/// until neither changes anything. Returns (cells tapered, cells restored, walls left).
pub(crate) fn taper(
    r: &Routing,
    z: &mut [f32],
    before: &[f32],
    raw: &[f32],
    wall_deg: f64,
    lower: &mut [f32],
) -> (u64, u64, u64) {
    let (w, h) = (r.world.w as usize, r.world.h as usize);
    let tan = wall_deg.to_radians().tan();
    lower.fill(0.0);
    for c in 0..z.len() {
        let d = before[c] - z[c];
        if d < SHAPE_MIN_M || r.held.get(c) || r.sea.get(c) {
            continue;
        }
        let radius = (2.0 * f64::from(d) / (tan * r.spacing_m)).max(TAPER_MIN_CELLS);
        let k = radius.ceil() as i64;
        let (cx, cy) = ((c % w) as i64, (c / w) as i64);
        for dy in -k..=k {
            for dx in -k..=k {
                let rho = ((dx * dx + dy * dy) as f64).sqrt();
                let (x, y) = (cx + dx, cy + dy);
                if rho >= radius || x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                    continue;
                }
                let j = y as usize * w + x as usize;
                if r.held.get(j) || r.sea.get(j) {
                    continue;
                }
                let lw = (f64::from(d) * (1.0 - (rho / radius).powi(2))) as f32;
                if lw > lower[j] {
                    lower[j] = lw;
                }
            }
        }
    }
    let mut changed: Vec<usize> = Vec::new();
    let mut tapered = 0;
    for j in 0..z.len() {
        if lower[j] <= 0.0 {
            continue;
        }
        let v = (before[j] - lower[j]).max(r.floor[j]);
        if v < z[j] {
            z[j] = v;
            tapered += 1;
        }
        if before[j] - z[j] > CHANGED_M {
            changed.push(j);
        }
    }
    // Wall rounds (as Plan A's channel restore): cells near this step's cuts only.
    let inner = |j: usize| {
        let (x, y) = (j % w, j / w);
        x > 0 && y > 0 && x + 1 < w && y + 1 < h
    };
    let mut restored = 0;
    let mut walls = Vec::new();
    for round in 0..=WALL_ROUNDS {
        walls.clear();
        for &c in &changed {
            let (cx, cy) = ((c % w) as i64, (c / w) as i64);
            for (dy, dx) in OFFSETS.iter().copied().chain([(0, 0)]) {
                let (x, y) = (cx + dx, cy + dy);
                if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                    continue;
                }
                let j = y as usize * w + x as usize;
                if inner(j)
                    && raw[j] - z[j] > CHANGED_M
                    && is_wall(
                        horn_deg(z, w, j % w, j / w, r.spacing_m),
                        horn_deg(raw, w, j % w, j / w, r.spacing_m),
                        wall_deg,
                    )
                {
                    walls.push(j);
                }
            }
        }
        walls.sort_unstable();
        walls.dedup();
        // Bed grade (to its fixed point inside the round): no changed cell below its receiver.
        let mut graded = false;
        loop {
            let mut moved = false;
            for &c in &changed {
                let Some(t) = receiver(w, h, c, r.flow[c] & 15) else { continue };
                if r.held.get(t) || r.sea.get(t) {
                    continue;
                }
                let v = before[c].min(z[c].max(z[t]));
                if v > z[c] {
                    z[c] = v;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
            graded = true;
        }
        if (walls.is_empty() && !graded) || round == WALL_ROUNDS {
            break;
        }
        let mut fix: Vec<usize> = Vec::new();
        for &j in &walls {
            let (cx, cy) = ((j % w) as i64, (j / w) as i64);
            for (dy, dx) in OFFSETS.iter().copied().chain([(0, 0)]) {
                let (x, y) = (cx + dx, cy + dy);
                if x >= 0 && y >= 0 && x < w as i64 && y < h as i64 {
                    fix.push(y as usize * w + x as usize);
                }
            }
        }
        fix.sort_unstable();
        fix.dedup();
        for j in fix {
            if before[j] - z[j] > CHANGED_M {
                z[j] = before[j] - (before[j] - z[j]) / 2.0;
                restored += 1;
            }
        }
    }
    (tapered, restored, walls.len() as u64)
}
```

- [ ] **Step 4: Run the tests**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::erosion`
Expected: PASS. If the taper test's last loop fails at the disc's rim on a flat, the radius rule is wrong (the parabola's edge gradient is 2d/(R·spacing) = tan by construction); fix the code, not the test.

- [ ] **Step 5: Commit**

Run the task-finish block (clippy flags the unused passes until Task 6; add `#[allow(dead_code)]` on the module line in `mod.rs` with the comment `// Wired into the run in the next commit (Task 6).` and remove it in Task 6).

```bash
git add crates/topography-core/src/world/water/erosion.rs crates/topography-core/src/world/water/mod.rs
git commit -m "feat(water): implicit lake-aware stream power, lowering-only diffusion and cut taper

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Expert review** — output: the test file (synthetic only; the real-terrain output comes with Task 6).

---

## Task 6: The erosion stage in the run

Rulings B2–B4, B23. Coarse levels only; 1 m runs are unchanged.

**Files:**
- Modify: `crates/topography-core/src/world/water/erosion.rs` (the stage wrapper `erode`)
- Modify: `crates/topography-core/src/world/water/mod.rs` (`Tags::e`, `Tags::m2` after `e`, `pre_channel`, `run` and `finish` use it; `stages.json` `erosion`)
- Modify: `crates/topography-core/src/world/water/checks.rs` (`CheckLayers` gets `pre_channel`; `breached` keeps the breach-only ground)
- Modify: `crates/topography-core/src/world/water/testing.rs` (`coarse_grid_dir`)

**Interfaces:**
- Consumes: Task 5's passes; `pipeline::{model, Rasters}`; `Setup::flags`, `Setup.classes`.
- Produces:
  - `erosion::erode(s: &Setup, breached: &Layer<f32>, raw: &Layer<f32>, work: &Work, tag: &str, e: &Erosion) -> Result<(PathBuf, Figures, Vec<Figures>)>`: routings into `work/e<k>.f32`; returns the last one's path, the summed figures and each routing's.
  - `fn pre_channel(work: &store::Work, p: &Params) -> PathBuf` in `mod.rs`: `e<routings−1>.f32` with erosion, else `breached.f32`.
  - `stages.json` key `erosion` (figures summed over routings, `per_routing`, `routings`).
  - `testing::coarse_grid_dir(dir: &Path, world: World, spacing_m: f64) -> PathBuf`: a coarse grid with classes, moisture, a provenance layer (interior west of x = 30, blend east of it) and a 6 m seam dam across a valley on the blend side.

- [ ] **Step 1: Write the failing test**

In `testing.rs`:

```rust
/// A coarse grid (`spacing_m`, 64² or larger): `dammed(world)` scaled ×10 in height, with a
/// 6 m seam dam across the valley at x = 44 (blend), interior provenance west of x = 30 and
/// blend east of it, classes and a 2 × 2 moisture field.
pub(crate) fn coarse_grid_dir(dir: &std::path::Path, world: World, spacing_m: f64) -> std::path::PathBuf {
    use std::fs;
    let grid = grid_dir(dir, world);
    let mut z: Vec<f32> = dammed(world).iter().map(|v| v * 10.0).collect();
    for y in 0..=20u64 {
        let i = (y * world.w + 44) as usize;
        z[i] = z[i].max(z[i - 1] + 6.0);
    }
    fs::write(grid.join("elevation.f32"), z.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>())
        .expect("elevation");
    let prov: Vec<u8> = (0..world.w * world.h)
        .map(|i| if i % world.w < 30 { super::INTERIOR } else { 2 })
        .collect();
    fs::write(grid.join("provenance.u8"), prov).expect("provenance");
    let path = grid.join("grid.json");
    let mut m: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&fs::read(&path).expect("grid.json")).expect("json");
    let h = world.h as f64 * spacing_m;
    m.insert("spacing_m".into(), serde_json::json!(spacing_m));
    m.insert("origin".into(), serde_json::json!([0.0, h]));
    m["moisture"]["spacing_m"] = serde_json::json!(world.w.max(world.h) as f64 * spacing_m / 2.0);
    m["moisture"]["nw_m"] = serde_json::json!([0.0, h]);
    fs::write(&path, serde_json::to_vec(&m).expect("json")).expect("grid.json");
    grid
}
```

In `mod.rs` tests:

```rust
    #[test]
    fn coarse_runs_erode_grade_the_seam_dam_and_spare_interiors() {
        let dir = tempfile::tempdir().expect("temp");
        let world = geom::World { w: 64, h: 64 };
        let grid = testing::coarse_grid_dir(dir.path(), world, 32.0);
        let water = Some(grid.join("water.toml"));
        let o = Options {
            grid: grid.clone(),
            work: dir.path().join("work"),
            tile: 32,
            threads: 2,
            water: water.clone(),
            river_km2: Some(0.05),
        };
        run(&o).expect("run");
        let stages: Value = serde_json::from_slice(&fs::read(o.work.join("stages.json")).unwrap()).unwrap();
        let e = &stages["erosion"];
        assert_eq!(e["routings"], json!(5));
        assert!(e["lowered_cells"].as_u64().unwrap() > 0);
        assert!(e["sills_lowered"].as_u64().unwrap() >= 1, "the seam dam's lake drains through its sill");
        assert_eq!(e["erosion_new_pits"], json!(0), "no pit made by the erosion code (B33)");
        let raw = layers::Layer::<f32>::open(world, &grid.join("elevation.f32")).unwrap().to_vec().unwrap();
        let eroded = layers::Layer::<f32>::open(world, &o.work.join("e4.f32")).unwrap().to_vec().unwrap();
        let breached = layers::Layer::<f32>::open(world, &o.work.join("breached.f32")).unwrap().to_vec().unwrap();
        for i in 0..raw.len() {
            assert!(eroded[i] <= breached[i] + 1e-6 && breached[i] <= raw[i] + 1e-6, "cell {i} rose");
            if (i as u64) % 64 < 30 && breached[i] == raw[i] {
                assert!(raw[i] - eroded[i] <= 3.0 + 1e-4, "interior cap 3 m at {i}");
            }
        }
        assert!(raw[20 * 64 + 44] - eroded[20 * 64 + 44] > 1.0 || stages["breach"]["breaches"].as_u64().unwrap() > 0,
                "the dam is cut by erosion or breached");
        // A second run resumes from the cache with the same answer.
        assert!(run(&o).is_ok());
        // 1 m runs have no erosion stage.
        let one = testing::grid_dir(&dir.path().join("one"), world);
        let o1 = Options { grid: one.clone(), work: dir.path().join("w1"), water: Some(one.join("water.toml")), ..o };
        run(&o1).expect("1 m run");
        let s1: Value = serde_json::from_slice(&fs::read(o1.work.join("stages.json")).unwrap()).unwrap();
        assert!(s1["erosion"].is_null());
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::tests::coarse_runs_erode`
Expected: FAIL (`stages["erosion"]` is null, no `e4.f32`).

- [ ] **Step 3: Implement the stage wrapper**

In `erosion.rs`:

```rust
use super::layers::Layer;
use super::pipeline::{self, Rasters, Setup};
use super::store::Work;
use super::table::Erosion;
use super::{INTERIOR, KIND_MASK};
use crate::acquisition::Result;
use std::path::PathBuf;

/// Glacier class: frozen like v2's ice catchments (B3: interior K).
const GLACIER: u8 = 10;

/// Strict local minima (8 neighbours) of `z` that are none in `breached`, off water and sea
/// (B33; Task 7 makes the same test a hard check on the published ground).
pub(crate) fn new_pits(world: World, z: &[f32], breached: &[f32], held: &Bits, sea: &Bits) -> u64 {
    let (w, h) = (world.w as usize, world.h as usize);
    let strict_min = |v: &[f32], c: usize| {
        let (x, y) = ((c % w) as i64, (c / w) as i64);
        OFFSETS.iter().all(|&(dy, dx)| {
            let (nx, ny) = (x + dx, y + dy);
            nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 || v[ny as usize * w + nx as usize] > v[c]
        })
    };
    (0..z.len())
        .filter(|&c| {
            let (x, y) = (c % w, c / w);
            x > 0 && y > 0 && x + 1 < w && y + 1 < h && !held.get(c) && !sea.get(c)
        })
        .filter(|&c| strict_min(z, c) && !strict_min(breached, c))
        .count() as u64
}

/// The `e` stage (B23): `e.routings` × [model on the current ground, stream power, diffusion,
/// taper and wall rounds] from the breached ground into `work/e<k>.f32`. Each routing is
/// cached; returns the last ground's path and the summed figures.
pub(crate) fn erode(
    s: &Setup,
    breached: &Layer<f32>,
    raw: &Layer<f32>,
    work: &Work,
    tag: &str,
    e: &Erosion,
) -> Result<(PathBuf, Figures, Vec<Figures>)> {
    let world = s.world;
    let n = (world.w * world.h) as usize;
    let whole = super::geom::Rect { x0: 0, y0: 0, w: world.w as usize, h: world.h as usize };
    let mut per = Vec::new();
    let mut total = Figures::default();
    for k in 0..e.routings as usize {
        let ground = if k == 0 { breached.clone() } else { Layer::open(world, &work.path(&format!("e{}.f32", k - 1)))? };
        let out = Layer::open_or_create(world, &work.path(&format!("e{k}.f32")), 0.0)?;
        let blob = work.cached(&format!("{tag}/r{k}"), || {
            let rasters = Rasters::files(world, &work.path(""), &format!("e{k}"))?;
            let m = pipeline::model(s, &ground, Some(work), &format!("{tag}/m{k}"), Some(&rasters))?;
            let started = std::time::Instant::now();
            let flags = s.flags(whole)?;
            let classes = match &s.classes {
                Some(c) => c.read(whole)?,
                None => vec![0; n],
            };
            let interior_at = |c: usize| flags[c] & KIND_MASK == INTERIOR || classes[c] == GLACIER;
            // Floor: the breached ground less the cap (B4); breach cuts are not counted.
            let bz = breached.to_vec()?;
            let floor: Vec<f32> = (0..n)
                .map(|c| bz[c] - if interior_at(c) { e.interior_cap_m } else { e.cap_m })
                .collect();
            drop(bz);
            let node = rasters.node.to_vec()?;
            let flow = rasters.flow.to_vec()?;
            let (mut held, mut sea, mut interior) = (Bits::new(n), Bits::new(n), Bits::new(n));
            for c in 0..n {
                if node[c] >= 2 { held.set(c); }
                if node[c] == 1 { sea.set(c); }
                if interior_at(c) { interior.set(c); }
            }
            let (w, h) = (world.w as usize, world.h as usize);
            let mut shore: Vec<(u32, u32)> = (0..n)
                .filter(|&c| node[c] < 2)
                .filter_map(|c| receiver(w, h, c, flow[c] & 15))
                .filter(|&r| node[r] >= 2)
                .map(|r| (r as u32, node[r] - 2))
                .collect();
            shore.sort_unstable();
            shore.dedup();
            drop(node);
            let mut z = ground.to_vec()?;
            let before = z.clone();
            // k·A^m per cell, in place of the discharge (A in m²).
            let mut ka = rasters.discharge.to_vec()?;
            let kfor = |c: usize| if interior.get(c) { e.k_dt * e.interior_k } else { e.k_dt };
            let q_out: Vec<(usize, f32)> = m.water.nodes.0.iter().filter(|nd| nd.full)
                .map(|nd| (nd.outlet[1] as usize, ka[nd.outlet[1] as usize])).collect();
            for c in 0..n {
                ka[c] = if held.get(c) || sea.get(c) {
                    0.0
                } else {
                    (kfor(c) * (f64::from(ka[c]) * 1e6).powf(e.m)) as f32
                };
            }
            let q_at: std::collections::HashMap<usize, f32> = q_out.into_iter().collect();
            let mut sills: Vec<Sill> = m.water.nodes.0.iter().enumerate()
                .filter(|(_, nd)| nd.full)
                .filter_map(|(o, nd)| {
                    let [a, b] = [nd.outlet[0] as usize, nd.outlet[1] as usize];
                    let (cell, other) = if z[a] >= z[b] { (a, b) } else { (b, a) };
                    let down = if cell == a { Some(b) } else { receiver(w, h, b, flow[b] & 15) }?;
                    let q = f64::from(q_at[&b]);
                    Some(Sill {
                        cell: cell as u32, down: down as u32, node: o as u32, other_z: z[other],
                        ka: (kfor(cell) * (q * 1e6).powf(e.m)) as f32,
                    })
                })
                .collect();
            sills.sort_unstable_by_key(|s| s.cell);
            sills.dedup_by_key(|s| s.cell);
            let r = Routing {
                world, spacing_m: s.spacing_m, floor: &floor, ka: &ka, flow: &flow, held: &held,
                sea: &sea, interior: &interior, shore: &shore, sills: &sills, nodes: &m.water.nodes,
            };
            let mut fig = stream_power(&r, &mut z);
            let near = footprint(&r, &before, &z, s.params.wall_deg);
            fig.diffused_cells = diffuse(&r, &mut z, e.diffusion, &near);
            drop(r);
            let rawv = raw.to_vec()?;
            let mut lower = ka; // reused as the taper's buffer
            let r = Routing {
                world, spacing_m: s.spacing_m, floor: &floor, ka: &[], flow: &flow, held: &held,
                sea: &sea, interior: &interior, shore: &shore, sills: &sills, nodes: &m.water.nodes,
            };
            let (t, restored, left) = taper(&r, &mut z, &before, &rawv, s.params.wall_deg, &mut lower);
            fig.tapered_cells = t;
            fig.wall_cells_restored = restored;
            fig.walls_left = left;
            // B6 figure: full-K land outside the river cuts' footprint should not move.
            let mut hist = vec![0u64; super::checks::BINS];
            let mut hmax = 0f32;
            for c in 0..n {
                if !near.get(c) && !interior.get(c) && !held.get(c) && !sea.get(c) {
                    let d = (before[c] - z[c]).max(0.0);
                    hist[((d * 100.0) as usize).min(super::checks::BINS - 1)] += 1;
                    hmax = hmax.max(d);
                }
            }
            fig.hillslope_p99_m = super::checks::p99_m(&hist) as f32;
            fig.hillslope_max_m = hmax;
            drop((rawv, lower));
            let bz = breached.to_vec()?;
            fig.new_pits = new_pits(world, &z, &bz, &held, &sea);
            out.write(whole, &z)?;
            eprintln!("erosion: routing {}/{} lowered {} cells (max {:.2} m), {} sills, {} walls left; \
                       passes {:.0} s{}",
                      k + 1, e.routings, fig.lowered_cells, fig.max_lowering_m, fig.sills_lowered, left,
                      started.elapsed().as_secs_f64(), super::peak());
            Ok(fig.to_blob())
        })?;
        let fig = Figures::from_blob(blob);
        total.add(fig);
        per.push(fig);
    }
    Ok((work.path(&format!("e{}.f32", e.routings - 1)), total, per))
}
```

Notes for the implementer: `Layer` is `Clone` (it is cloned in `finish`); `Work::cached` writes the blob only after the closure returns, so a kill mid-routing reruns that routing from `e<k−1>.f32`, which is complete. `taper` does not read `ka` (it is passed empty the second time). Keep the stage's memory to: z, before, floor, ka/lower (4 B each), flow (1 B), three bit sets and the shore list: about 17 B per cell (4.6 GB at 16,384²), plus `rawv` (4 B) during the taper. Task 9 measures it.

In `mod.rs`:

```rust
    /// e reads the breached ground, the provenance and the classes (b's tag, B23).
    fn e(b: &str, e: &table::Erosion, wall_deg: f64) -> String {
        stage_tag(
            "e",
            &json!({"after": b, "routings": e.routings, "k_dt": e.k_dt, "m": e.m,
                    "diffusion": e.diffusion, "cap_m": e.cap_m, "interior_k": e.interior_k,
                    "interior_cap_m": e.interior_cap_m, "wall_deg": wall_deg,
                    "taper_min_cells": erosion::TAPER_MIN_CELLS, "wall_rounds": erosion::WALL_ROUNDS,
                    "shape_min_m": erosion::SHAPE_MIN_M, "glacier_frozen": true, "sea_base_0": true}),
        )
    }
```

`Tags::m2(b)` becomes `Tags::m2(pre)` where `pre` is the `e` tag with erosion, else the `b` tag. The provenance file's identity is already in the work stamp (Task 4), so a changed layer refuses the work directory.

```rust
/// The ground the channels cut into (B23): the last erosion routing, else the breached ground.
fn pre_channel(work: &store::Work, p: &Params) -> PathBuf {
    match p.erosion {
        Some(e) => work.path(&format!("e{}.f32", e.routings - 1)),
        None => work.path("breached.f32"),
    }
}
```

In `run`, after the breach:

```rust
    let (pre_tag, pre, erosion_json) = match p.erosion {
        Some(e) => {
            let e_tag = Tags::e(&b_tag, &e, p.wall_deg);
            let (path, total, per) = erosion::erode(&s, &breached, &raw, &work, &e_tag, &e)?;
            let mut j = total.json();
            j["routings"] = json!(e.routings);
            j["per_routing"] = json!(per.iter().map(|f| f.json()).collect::<Vec<_>>());
            (e_tag, layers::Layer::open(s.world, &path)?, j)
        }
        None => (b_tag.clone(), breached.clone(), Value::Null),
    };
    let m2_tag = Tags::m2(&pre_tag);
    let m2 = pipeline::model(&s, &pre, wk, &m2_tag, Some(&rasters))?;
```

Then pass `&pre` where `run` passed `&breached` to `channel::channels` (the ground argument) and to `pipeline::model_on` (the prior), use `pre_channel(&work, p)` in `final_tag`, and add `"erosion": erosion_json` to `stages`. `Tags::all` (used in tests) follows the same order. In `finish`, open `pre_channel(&work, &s.params)` where it opens `breached.f32` for m2, the m3 tag and the m3 prior, and build `CheckLayers` with both:

```rust
        breached: breached_only.clone(),   // work/breached.f32: breach cuts (interior budget)
        pre_channel: pre.clone(),          // the ground channels cut into (bed_rises)
```

In `checks.rs`, add `pub pre_channel: Layer<f32>` to `CheckLayers`, and read `l.pre_channel` where `check_tile` reads `l.breached` today (line 325, `bz`, used by `bed_rises`); keep `l.breached` for Task 7's interior budget. Update the test fixture (`checks.rs` ~1600) to fill both with `self.bz`.

- [ ] **Step 4: Run the tests**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water`
Expected: PASS, including Plan A's tag test with the `e` stage absent at 1 m. Remove the `#[allow(dead_code)]` added in Task 5.

- [ ] **Step 5: Real-terrain probe: a 32 m window at the dam spot** (seconds; announce only)

```bash
"${CAP6[@]}" cargo build --release --locked --offline -p topography-cli
G=scratch/hydrology-v3b/t6-32m
"${CAP6[@]}" "${PY[@]}" -c "
import json, numpy as np
from pathlib import Path
src = Path('scratch/hydrology-v3b/t2-32m/grid')   # Task 2's window grid (l2_raw)
g = Path('$G/grid'); g.mkdir(parents=True, exist_ok=True)
m = json.loads((src / 'grid.json').read_text())
for f in ('elevation.f32', 'classes.u8'):
    (g / f).write_bytes((src / f).read_bytes())
(g / 'moisture.f32').write_bytes((src / 'moisture.f32').resolve().read_bytes())
p = np.fromfile('data/worlds/fujix-collage-55/provenance32.u8', np.uint8).reshape(2048, 2048)
c0, r0 = (int(v // 32) for v in m['offset_m']); n = m['size'][0]
p[r0:r0 + n, c0:c0 + n].tofile(g / 'provenance.u8'); m['provenance_cell_m'] = 32.0
(g / 'grid.json').write_text(json.dumps(m, indent=1))"
B=target/release/topography
"${CAP6[@]}" $B world water --grid $G/grid --work $G/work --tile 256 --threads 4
"${CAP6[@]}" $B world water-finish --grid $G/grid --work $G/work --ground $G/work/carved.f32 --out $G/out --tile 256 --threads 4
jq '.erosion | {routings, lowered_cells, max_lowering_m, sills_lowered, capped_cells, walls_left}' $G/work/stages.json
jq '{passed, failures}' $G/out/report.json
"${CAP6[@]}" "${PY[@]}" -c "
from pathlib import Path
from topography_training.water import review
print(review.window_pair(Path('$G'), Path('$G/pair.png')))"
```

Expected: erosion lowers cells and drains at least one sill; `walls_left` 0. Checks may fail here on Task 7's not-yet-written checks only; any Plan A check failure is a bug to fix now.

- [ ] **Step 6: Commit**

```bash
git add crates/topography-core/src/world/water
git commit -m "feat(water): erosion stage at coarse levels between breach and the second model run

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 7: Expert review** — output: `$G/work/stages.json` `erosion`, `$G/out/report.json`, `$G/pair.png` (raw vs eroded at the dam spot E25645 N24400).

---
## Task 7: Coarse, hand-off and dam checks

Spec "Checks" (change budget, hand-off); rulings B9, B14, B25; R31 made hard.

**Files:**
- Modify: `crates/topography-core/src/world/water/checks.rs` (`check_tile`, `Tile`, `report`, tests)
- Modify: `docs/workflows/terrain-cascade.md` is updated in Task 14, not here.

**Interfaces:**
- Consumes: `Setup::flags` (Task 4), `CheckLayers.breached` (breach-only) and `pre_channel` (Task 6), `Params.erosion`.
- Produces (hard checks, all must be 0):
  - `interior_p99`: 1 when, at a level with erosion, the p99 of |raw − final| over interior cells outside breaches exceeds 1 m (global).
  - `interior_max`: interior cells outside breaches with |raw − final| > 3 m.
  - `handoff_narrow`: at a level with erosion, cells where Δ = raw − final exceeds its 3 × 3 grey opening by more than max(0.5 m, 0.5 × Δ) (B9, review C2).
  - `dam_drops`: on a labelled lake's 50 m exit walk, a drop D > 1.5 m between adjacent cells with D > 3 × the mean fall per cell over the next 100 m of the walk (B14, review C1).
  - `erosion_new_pits`: at a level with erosion, a cell lowered from raw by more than `CHANGED_M` that is a strict local minimum of the ground the channels cut into (`pre_channel`) but not of the breached ground (B33).
- Changes: at levels with erosion, `change_p99` (all cells) moves from the checks to the figures (B4, review C3).
- Produces (figures): `interior_change_p99_m`, `interior_change_max_m`, `interior_cells`, `handoff_excess_max_m`, `dam_drops_steep_source` (how many `dam_drops` cells carry bit 128; a tag only), `seam_lakes` (`count`, `lakes`: up to 50 of `{lake, cell, kind}`).

- [ ] **Step 1: Write the failing tests**

In `checks.rs` tests, using Plan A's check fixture (`Fixture` around line 1600; give it a `flags: Vec<u8>` field fed to `Setup.provenance`, and a `params` field so a test can set `erosion`):

```rust
    #[test]
    fn interior_budget_counts_interior_cells_outside_breaches_only() {
        let mut f = Fixture::flat(32, 100.0);
        f.coarse(); // spacing 32 m, params from levels.32 (erosion on)
        f.flags = vec![crate::world::water::INTERIOR; f.cells()];
        f.ground[f.at(10, 10)] = 96.0; // an interior cell lowered 4 m by erosion
        let t = f.check();
        assert_eq!(t.fails.get("interior_max").copied().unwrap_or(0), 1);
        // The same cut inside a breach (breached ground lower than raw there) is the breach's.
        f.breached[f.at(10, 10)] = 96.0;
        assert_eq!(f.check().fails.get("interior_max").copied().unwrap_or(0), 0);
    }

    #[test]
    fn a_one_cell_trench_fails_handoff_narrow_and_a_tapered_cut_passes() {
        let mut f = Fixture::flat(32, 100.0);
        f.coarse();
        for y in 4..28 {
            let i = f.at(16, y);
            f.ground[i] = 98.0; // a 2 m slot, one cell wide
        }
        assert!(f.check().fails.get("handoff_narrow").copied().unwrap_or(0) > 0, "a one-cell slot");
        // A V of profile 0, 1, 2, 1, 0: the opening is 1, the excess 1 m = 0.5 × the 2 m cut: passes.
        for y in 4..28 {
            for (dx, d) in [(-1i64, 1.0f32), (1, 1.0)] {
                let i = f.at((16 + dx) as usize, y);
                f.ground[i] = 100.0 - d;
            }
        }
        assert_eq!(f.check().fails.get("handoff_narrow").copied().unwrap_or(0), 0, "a V three cells wide");
        // A trench of B7 discs (R = 1.5 cells, 6 m deep): d·(1 − (ρ/R)²) across, passes.
        let mut g = Fixture::flat(32, 100.0);
        g.coarse();
        for y in 4..28 {
            for dx in -1i64..=1 {
                let rho = dx.abs() as f32;
                let i = g.at((16 + dx) as usize, y);
                g.ground[i] = 100.0 - 6.0 * (1.0 - (rho / 1.5).powi(2));
            }
        }
        assert_eq!(g.check().fails.get("handoff_narrow").copied().unwrap_or(0), 0, "the erosion taper passes");
    }

    #[test]
    fn dam_drops_fail_on_a_step_then_gentle_and_pass_a_steady_cascade() {
        // Shown surface along a labelled lake's exit walk (1 m cells), level 10 m.
        let dam: Vec<f32> = std::iter::once(7.0).chain((1..150).map(|i| 7.0 - 0.01 * i as f32)).collect();
        let f = Fixture::lake_with_exit_walk(10.0, &dam); // 3 m step, then 1 cm per cell
        assert!(f.check().fails.get("dam_drops").copied().unwrap_or(0) > 0, "a dam");
        let cascade: Vec<f32> = (0..150).map(|i| 8.0 - 2.0 * i as f32).collect();
        let g = Fixture::lake_with_exit_walk(10.0, &cascade); // 2 m every cell
        assert_eq!(g.check().fails.get("dam_drops").copied().unwrap_or(0), 0, "a steady cascade");
        let mut p = Fixture::lake_with_exit_walk(10.0, &dam);
        p.unlabel_lake(); // the same step below a pond: a figure only
        assert_eq!(p.check().fails.get("dam_drops").copied().unwrap_or(0), 0);
        assert!(!p.check().drops_any.is_empty());
    }

    #[test]
    fn erosion_new_pits_counts_pits_the_coarse_cutting_made() {
        let mut f = Fixture::flat(32, 100.0);
        f.coarse();
        let i = f.at(10, 10);
        f.pre_channel[i] = 99.0;
        f.ground[i] = 99.0;
        assert_eq!(f.check().fails.get("erosion_new_pits").copied().unwrap_or(0), 1);
        f.breached[i] = 99.0; // already a pit after the breach: not erosion's
        f.raw[i] = 99.0;
        assert_eq!(f.check().fails.get("erosion_new_pits").copied().unwrap_or(0), 0);
    }
```

Fixture helpers to add (test-only): `flat(n, z)` (raw, ground, breached, pre_channel all `z`; dry; no bodies), `coarse()` (spacing 32 m and `params(&table, 32.0, None)`), `at(x, y)`, `cells()`, `lake_with_exit_walk(level, shown: &[f32])` (Plan A's dam-steps fixture behind the `t.drops_any` asserts at lines 2061–2131, generalised: a labelled lake at `level` whose exit walk runs east along one row with the given shown surface, written as ground with raw equal to it so `dam_steps` stays quiet) and `unlabel_lake()` (the node stays, its body id becomes 0).

- [ ] **Step 2: Run them to see them fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::checks`
Expected: FAIL (unknown checks count 0).

- [ ] **Step 3: Implement**

In `check_tile`, read once per tile:

```rust
    let flags = s.flags(win)?;
    let bo = l.breached.read(win)?; // breach-only ground (work/breached.f32)
    let coarse = s.params.erosion.is_some();
```

Interior budget, in the per-core-cell loop (where `delta` = |raw − ground| is computed for `change_max`):

```rust
            let interior = flags[j] & KIND_MASK == INTERIOR && raw[j] - bo[j] <= CHANGED_M;
            if coarse && interior {
                t.interior_cells += 1;
                t.hist_interior[((delta * 100.0) as usize).min(BINS - 1)] += 1;
                t.max_interior = t.max_interior.max(delta);
                if delta > INTERIOR_MAX_M {
                    t.fail("interior_max", x, y);
                }
            }
```

Hand-off narrowness (coarse levels), same loop, with `cut(j) = (raw[j] − z[j]).max(0.0)` on the window:

```rust
            if coarse {
                // Grey opening of the cut with a 3 × 3 square: max over the 3 × 3 of the min
                // over each one's 3 × 3. A cut it removes by over NARROW_M is under 3 cells wide.
                let open = (-1i64..=1)
                    .flat_map(|dy| (-1i64..=1).map(move |dx| (dx, dy)))
                    .map(|(dx, dy)| {
                        (-1i64..=1)
                            .flat_map(|ey| (-1i64..=1).map(move |ex| (ex, ey)))
                            .map(|(ex, ey)| cut_at(lx as i64 + dx + ex, ly as i64 + dy + ey))
                            .fold(f32::INFINITY, f32::min)
                    })
                    .fold(f32::NEG_INFINITY, f32::max);
                let cut = cut_at(lx as i64, ly as i64);
                t.handoff_excess = t.handoff_excess.max(cut - open);
                if cut - open > NARROW_M.max(NARROW_SHARE * cut) {
                    t.fail("handoff_narrow", x, y);
                }
            }
```

where `cut_at(x, y)` reads the window (0 outside it) and `lx, ly` are window coordinates; `inner` cells only (two cells from the window edge, which the 576-cell halo guarantees for core cells).

Dam drops (B14, knickpoint). Before Plan A's exit walk loop (line ~1046), walk the same receivers for FALL_M[2] + KNICK_AHEAD_M and keep the shown surface, then judge every drop of the first 50 m against the mean fall ahead of it:

```rust
        if e.lake.is_some() {
            // (x, y, D, metres from the exit) along the walk, 50 m checked + 100 m ahead.
            let mut walk: Vec<(u64, u64, f32, f64)> = Vec::new();
            let (mut cx, mut cy, mut m) = (bx, by, 0.0f64);
            while m <= FALL_M[2] + KNICK_AHEAD_M {
                let Some(c) = at(cx, cy) else { break };
                walk.push((cx, cy, dsurf(c), m));
                let d = flow[c] & 15;
                let Some((nx, ny)) = (d > 0).then(|| world.neighbour(cx, cy, d as usize - 1)).flatten() else { break };
                m += step_len(d) * s.spacing_m;
                (cx, cy) = (nx, ny);
            }
            let mut prev = lv;
            for (i, &(cx, cy, dv, m)) in walk.iter().enumerate() {
                if m > FALL_M[2] {
                    break;
                }
                let drop = prev - dv;
                if drop > DAM_STEP_M {
                    let ahead: Vec<_> = walk[i..].iter().take_while(|q| q.3 <= m + KNICK_AHEAD_M).collect();
                    let cells = (ahead.len().max(2) - 1) as f32;
                    let mean = (dv - ahead.last().map_or(dv, |q| q.2)).max(0.0) / cells;
                    if drop > KNICK_FACTOR * mean {
                        t.fail("dam_drops", cx, cy);
                        let c = at(cx, cy).expect("on the walk");
                        t.dam_steep += u64::from(flags[c] & STEEP != 0);
                    }
                }
                prev = dv;
            }
        }
```

(Use the receiver the existing walk follows if it is not `flow & 15`; the window's halo of 576 cells covers 150 m at 1 m and 4,600 m at 4 m, and at 32 m the walk ends at the window edge, which only shortens `ahead`.)

New pits at coarse levels, per core cell (with `pc` = `l.pre_channel` read on the window):

```rust
            if coarse && raw[j] - z[j] > CHANGED_M && inner && strict_min(&pc, lx, ly) && !strict_min(&bo, lx, ly) {
                t.fail("erosion_new_pits", x, y);
            }
```

with `strict_min(v, lx, ly)` true when all 8 neighbours in the window are higher.

Seam lakes, beside the exit's `lake_exit_surface` test:

```rust
        if let Some(lake) = e.lake {
            let (sx, sy) = if z[at(ix, iy).unwrap_or(b)] >= z[b] { (ix, iy) } else { (bx, by) };
            if let Some(sc) = at(sx, sy) {
                let kind = flags[sc] & KIND_MASK;
                if (2..=4).contains(&kind) {
                    t.seam_lakes.push((lake, [sx, sy], kind));
                }
            }
        }
```

Constants:

```rust
/// Change budget for real interiors at coarse levels (spec "Change budget").
const INTERIOR_P99_M: f64 = 1.0;
const INTERIOR_MAX_M: f32 = 3.0;
/// A cut its 3 × 3 opening removes by more than this is narrower than 3 cells (B9).
const NARROW_M: f32 = 0.5;
const NARROW_SHARE: f32 = 0.5;
/// Dam drops (B14): judged against the mean fall per cell over this far ahead, by this factor.
const KNICK_AHEAD_M: f64 = 100.0;
const KNICK_FACTOR: f32 = 3.0;
```

`Tile` gains `interior_cells: u64`, `hist_interior: Vec<u64>` (BINS), `max_interior: f32`, `handoff_excess: f32`, `dam_steep: u64`, `seam_lakes: Vec<(u32, [u64; 2], u8)>`. In `report`, add `"interior_max", "handoff_narrow", "dam_drops", "erosion_new_pits"` to the initial `fails` list, sum the new fields, report `handoff_excess_max_m` and `dam_drops_steep_source` as figures, and make `change_p99` a figure at levels with erosion (`report` gets `coarse: bool` from `finish`):

```rust
    if coarse {
        // B4 (review C3): the spec's coarse budget is the interior one; the all-cell p99 is a figure.
        fails.remove("change_p99");
        figures.insert("change_p99_all_cells_m".into(), json!(p99));
    }
```

and:

```rust
    let ip99 = p99_m(&hist_interior);
    fails.insert("interior_p99", u64::from(interior_cells > 0 && ip99 > INTERIOR_P99_M));
    figures.insert("interior_change_p99_m".into(), json!(ip99));
    figures.insert("interior_change_max_m".into(), json!(max_interior));
    figures.insert("interior_cells".into(), json!(interior_cells));
    seam_lakes.sort_unstable();
    seam_lakes.dedup_by_key(|s| s.0);
    figures.insert("seam_lakes".into(), json!({
        "count": seam_lakes.len(),
        "lakes": seam_lakes.iter().take(50).map(|(l, c, k)| json!({"lake": l, "cell": c, "kind": k})).collect::<Vec<_>>(),
    }));
```

(`figures` must be the report's figure map; `report` takes it as `mut figures`.) The `walls` check already reads `s.params.wall_deg` (Task 2) and `change_max` already reads `s.params.change_max_m()` (Plan A), so 35° and 50.5/15.5 m apply at coarse levels.

- [ ] **Step 4: Run the tests**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water`
Expected: PASS. A Plan A 1 m fixture whose lake exit has a step-then-gentle drop over 1.5 m now fails `dam_drops`, which is the ruling; update such a test's expectation only if its scene is a dam (read it first).

- [ ] **Step 4b: Positive and negative controls on Plan A's raw surface (review C1)** (windows: seconds each; the 16k window about 12 min under `MemoryMax=10G`: announce)

The check must see the known dams. Run it on the five dam windows and Plan A's gate-1 16k canyon window, all on the l4v2 raw surface (Plan A's 1 m raw, which still carries the v2 dams):

```bash
"${CAP6[@]}" cargo build --release --locked --offline -p topography-cli
C=scratch/hydrology-v3b/t7-controls; mkdir -p $C
for s in 25645,24400 25307,24585 20071,30501 43966,21633 51610,20478; do
  "${CAP6[@]}" "${PY[@]}" -m topography_training.water.run window --source data/worlds/fujix-55-l4v2       --level 1 --centre $s --size 2048 --tile 512 --threads 4 --out $C/E${s%,*}-N${s#*,} | tail -n 1
  jq -c --arg s "$s" '{spot: $s, dam_drops: .checks.dam_drops, examples: .examples.dam_drops}' $C/E${s%,*}-N${s#*,}/out/report.json
done
"${CAP10[@]}" "${PY[@]}" -m topography_training.water.run window --source data/worlds/fujix-55-l4v2     --level 1 --centre 27169,44546 --size 16384 --tile 2048 --threads 4 --out $C/x16k | tail -n 1
jq -c '{dam_drops: .checks.dam_drops, steep: .figures.dam_drops_steep_source}' $C/x16k/out/report.json
```

Positive control: `dam_drops` fires within 100 m of each of E25645 N24400, E25307 N24585, E20071 N30501 and E43966 N21633 (cell rows are 65,536 − N; read the `examples`). If one is missed, the detector is wrong: fix it before going on. Negative control: list every other firing (the windows' and the 16k window's) with its crop from `review/failures/`; most should be v2 lips (v2 raised a lip at every 4 m exit). Natural cascades among them calibrate `KNICK_FACTOR` (raise it only as far as the four dams still fire). Record both counts and any factor change in the progress file; those crops are what owner question 3 shows if natural cascades remain.

- [ ] **Step 5: Real-terrain run** (seconds): rebuild and rerun Task 6's 32 m window from scratch (the work stamp ties a work directory to its binary):

```bash
G=scratch/hydrology-v3b/t6-32m; rm -rf $G/work $G/out
"${CAP6[@]}" cargo build --release --locked --offline -p topography-cli
B=target/release/topography
"${CAP6[@]}" $B world water --grid $G/grid --work $G/work --tile 256 --threads 4
"${CAP6[@]}" $B world water-finish --grid $G/grid --work $G/work --ground $G/work/carved.f32 --out $G/out --tile 256 --threads 4
jq '{passed, failures, c: (.checks | {interior_p99, interior_max, handoff_narrow, dam_drops, erosion_new_pits, walls, lake_below_sea}),
     f: (.figures | {interior_change_p99_m, interior_change_max_m, seam_lakes: .seam_lakes.count})}' $G/out/report.json
```

- [ ] **Step 6: Commit**

```bash
git add crates/topography-core/src/world/water/checks.rs
git commit -m "feat(water): interior change budget, hand-off narrowness and lake-exit dam drops as hard checks

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 7: Expert review** — output: the 32 m window report and `review/failures/` crops of any failing check.

---

## Task 8: An outflow from every full lake (outlet trickle)

Ruling B16 (gate-2 park "lakes without outlet trickle").

**Files:**
- Modify: `crates/topography-core/src/world/water/bodies.rs` (`trickles`, used by `label`)
- Modify: `crates/topography-core/src/world/water/mod.rs` (`Tags::l` adds `"trickle": true`; `finish` marks the same cells on m3's flow before the checks)
- Test: `bodies.rs` tests

**Interfaces:**
- Consumes: `Nodes` (full, lake, outlet), the m2 (or m3) `Rasters` (`flow`, `node`).
- Produces: `pub(crate) fn trickles(world: World, nodes: &Nodes, r: &Rasters) -> Result<Vec<u64>>` — sorted world cells on each full labelled lake's outflow path, from its outside spill cell along the flow receivers to (not including) the first cell that is a node, the sea or already a thalweg (`flow & 16`), or to the map edge; at most `TRICKLE_MAX` (4,096) cells per lake. `label` sets bit 16 on them in `r.flow` before labelling, so they become river cells of a body headed at the lake's outflow.

- [ ] **Step 1: Write the failing test**

In `bodies.rs` tests (Plan A's synthetic world: the humid bowl at (16, 16) is a full lake; raise the river start so its outflow is not a river):

```rust
    #[test]
    fn a_full_lake_always_starts_a_river_at_its_outflow() {
        let world = World { w: 64, h: 64 };
        let s = testing::setup(world, 64, 1e9); // no river reaches the start
        let z = testing::terrain(world);
        let r = Rasters::memory(world);
        let m = model(&s, &Layer::memory(world, z), None, "m", Some(&r)).expect("model");
        let lake = m.water.nodes.0.iter().find(|n| n.full && n.lake).expect("a full lake");
        let cells = trickles(world, &m.water.nodes, &r).expect("trickles");
        assert!(cells.contains(&lake.outlet[1]), "the trickle starts at the outflow cell");
        let body = Layer::filled(world, 0u32);
        let v = label(&s, &m.water, &r, &body, &BTreeMap::new(), None, "l").expect("label");
        let rivers = v["bodies"].as_array().unwrap().iter().filter(|b| b["kind"] == "river").count();
        assert!(rivers >= 1, "the outflow is a labelled river");
        assert!(body.read(Rect::whole(world)).unwrap()[lake.outlet[1] as usize] > 1);
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water::bodies::tests::a_full_lake`
Expected: compile error (`trickles`).

- [ ] **Step 3: Implement**

```rust
/// Cells per lake a trickle may run (it ends at the first river, water or edge).
const TRICKLE_MAX: usize = 4096;

/// The outflow paths of full labelled lakes that are not rivers yet (B16): from the outside
/// spill cell along the receivers to the first node, sea or thalweg cell, or the map edge.
pub(crate) fn trickles(world: World, nodes: &Nodes, r: &Rasters) -> Result<Vec<u64>> {
    let one = |l: &Layer<u8>, c: u64| -> Result<u8> {
        let (x, y) = world.xy(c);
        Ok(l.read(Rect { x0: x, y0: y, w: 1, h: 1 })?[0])
    };
    let node_at = |c: u64| -> Result<u32> {
        let (x, y) = world.xy(c);
        Ok(r.node.read(Rect { x0: x, y0: y, w: 1, h: 1 })?[0])
    };
    let mut out = Vec::new();
    for n in nodes.0.iter().filter(|n| n.full && n.lake) {
        let mut c = n.outlet[1];
        for _ in 0..TRICKLE_MAX {
            let f = one(&r.flow, c)?;
            if f & 16 != 0 || node_at(c)? >= 1 {
                break;
            }
            out.push(c);
            let (x, y) = world.xy(c);
            let d = f & 15;
            if d == 0 {
                break;
            }
            let Some((nx, ny)) = world.neighbour(x, y, d as usize - 1) else { break };
            c = world.index(nx, ny);
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}
```

At the start of `label` (before it reads thalwegs), and in `finish` on m3's flow before the checks run:

```rust
    for c in trickles(s.world, &w.nodes, r)? {
        let (x, y) = s.world.xy(c);
        let at = Rect { x0: x, y0: y, w: 1, h: 1 };
        let f = r.flow.read(at)?[0];
        r.flow.write(at, &[f | 16])?;
    }
```

(`label` already runs once per tag and caches; the flow write is idempotent, so a resumed run is unchanged. `finish` applies it to the m3 rasters it reads so `label_agreement` compares like with like.) `Tags::l` adds `"trickle": true`.

- [ ] **Step 4: Run the tests**

Run: `"${CAP6[@]}" cargo test --locked --offline -p topography-core --quiet world::water`
Expected: PASS; `export_bodies` stays far under 50,000 on the fixtures.

- [ ] **Step 5: Commit**

```bash
git add crates/topography-core/src/world/water
git commit -m "feat(water): every full labelled lake starts a river at its outflow

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Expert review** — output: a 2048² 1 m window on the l4v2 raw at E25645 N24400 (`water.run window --level 1 …`): `lakes` with and without an outflow river before/after (figure `rivers`), and the spot crop.

---
## Task 9: Erosion benchmark (solver choice) and the 32 m calibration

Spec "Solver: chosen by benchmark in the plan"; rulings B1, B2, B25. No new code unless the decision rule sends work to Task 9b or the calibration changes the table.

**Files:**
- Modify (only if the calibration moves a value): `configs/hydrology/water-v3.toml` (`[levels.32.erosion]`)
- Output (not committed): `scratch/hydrology-v3b/t9/`
- Record: `.superpowers/sdd/2026-11-07-hydrology-v3-b/progress.md`

**Interfaces:**
- Consumes: Tasks 1–8 (`world rain`, `world water` with the `e` stage, the checks).
- Produces: the B1 decision (implicit CPU kept, or Task 9b), the 32 m `k_dt`, and measured erosion time and memory at 2048² and 16,384².

- [ ] **Step 1: Build the 32 m whole-collage grid** (seconds)

```bash
"${CAP6[@]}" cargo build --release --locked --offline -p topography-cli
T=scratch/hydrology-v3b/t9; mkdir -p $T
"${CAP6[@]}" "${PY[@]}" -c "
import json, numpy as np, shutil
from pathlib import Path
from topography_training.diffusion import hydrology, layout
c = Path('data/worlds/fujix-collage-55')
l2 = np.load(c / 'arrays.npz')['l2'].astype(np.float32)
cls = np.fromfile(c / 'class32.u8', np.uint8).reshape(l2.shape)
o = json.loads(Path('data/worlds/fujix-55-chain/hydrology/32m/grid.json').read_text())['origin']
g = hydrology.write_grid(Path('$T/base/grid'), l2, 32.0, tuple(o), cls,
                         wind_from_deg=layout.load(c).spec.get('wind_from_deg'))
shutil.copy(c / 'provenance32.u8', g / 'provenance.u8')
m = json.loads((g / 'grid.json').read_text()); m['provenance_cell_m'] = 32.0
(g / 'grid.json').write_text(json.dumps(m, indent=1))"
target/release/topography world rain --grid $T/base/grid
cmp $T/base/grid/moisture.f32 data/worlds/fujix-55-chain/hydrology/32m/moisture.f32 && echo "moisture identical (B12)"
```

- [ ] **Step 2: Calibration tables**

```bash
for k in 0 3.5e-4 7e-4 1.4e-3; do
  sed "/^\[levels.32.erosion\]/,/^interior_cap_m/ s/^k_dt = .*/k_dt = $k/" configs/hydrology/water-v3.toml > $T/water-k$k.toml
done
# "off": no erosion table at 32 m.
awk 'BEGIN{skip=0} /^\[levels.32.erosion\]/{skip=1; next} /^\[/{skip=0} !skip' configs/hydrology/water-v3.toml > $T/water-off.toml
grep -A3 'levels.32.erosion' $T/water-k7e-4.toml | head -4
```

- [ ] **Step 3: Run the five 32 m variants** (each about 1–3 min CPU, under 3 GB; announce only)

```bash
B=target/release/topography
for v in off k0 k3.5e-4 k7e-4 k1.4e-3; do
  R=$T/32m-$v; rm -rf $R; mkdir -p $R; ln -s ../base/grid $R/grid
  /usr/bin/time -v "${CAP6[@]}" $B world water --grid $R/grid --work $R/work --tile 2048 --threads 4 \
      --water $T/water-$v.toml > $R/log 2>&1
  "${CAP6[@]}" $B world water-finish --grid $R/grid --work $R/work --ground $R/work/carved.f32 --out $R/out \
      --tile 2048 --threads 4 --water $T/water-$v.toml >> $R/log 2>&1
  echo "$v: $(jq -c '{passed, failures, seam: .figures.seam_lakes.count, ip99: .figures.interior_change_p99_m,
        imax: .figures.interior_change_max_m, dams: .checks.dam_drops, below_sea: .checks.lake_below_sea,
        lakes: .figures.lakes, rivers: .figures.rivers}' $R/out/report.json)"
  jq -c '.erosion | {hillslope_lowering_p99_m, hillslope_lowering_max_m, erosion_new_pits}' $R/work/stages.json
  grep -E 'erosion: routing|Maximum resident' $R/log | tail -n 6
done
```

- [ ] **Step 4: Apply the calibration rule (B2)**

Let S_off be `seam_lakes` of `off`. Among `k3.5e-4`, `k7e-4`, `k1.4e-3`, keep those whose hard checks all pass (`passed`; straight and parallel runs are figures). Choose the **smallest** k_dt with `seam_lakes ≤ max(5, 0.2 × S_off)`; if none reaches that, the passing one with the fewest seam lakes; if none passes its hard checks, stop: diagnose the failing check on the real cells (systematic debugging), fix the code (never a threshold) and rerun Step 3. If the chosen k_dt is not 7e-4, set it in `configs/hydrology/water-v3.toml` and commit:

```bash
git add configs/hydrology/water-v3.toml
git commit -m "chore(water): calibrate 32 m erosion k_dt on the fujix collage

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

Pictures for the record and the reviewer:

```bash
for v in off k7e-4; do
  "${CAP6[@]}" "${PY[@]}" -c "
from pathlib import Path
from topography_training.water import review
print(review.window_pair(Path('$T/32m-$v'), Path('$T/32m-$v/pair.png')))
print(review.crops(Path('$T/32m-$v/out'), Path('$T/32m-$v/review'), raw=Path('$T/base/grid/elevation.f32'))[-1])"
done
ls $T/32m-*/review/spots/E25645-N24400.png $T/32m-*/review/spots/E25307-N24585.png $T/32m-*/review/spots/E20071-N30501.png
```

Put the calibration table (variant × seam lakes, interior p99/max, `dam_drops`, `lake_below_sea`, hillslope lowering, new pits) and the three dam-spot crops per variant in the progress file (review M8): the chosen k_dt must be seen to remove the seam lakes behind the dam spots, not only to lower the count. Hillslope lowering outside the cut footprint should be about 0 (B6); if not, the footprint is wrong.

- [ ] **Step 5: HEAVY — warn the owner, then the 16,384² timing run**

Tell the owner: one CPU run of the water model with erosion on a 16,384² 4 m grid (268 M cells; the v2 chain's 4 m ground as a timing stand-in, read only), 4 threads under `MemoryMax=10G`. Expected: 2 erosion routings with a model run each, plus the usual model runs: 40–90 min (uncertain: the erosion passes are unmeasured at this size; a model run is about 6–8 min here from Plan A's 1 m rate); peak RAM 5–7 GB (erosion arrays about 17 B per cell); about 25 GB of scratch disk. Close memory-heavy applications. Wait for the go.

```bash
X=$T/x16k; mkdir -p $X
"${CAP6[@]}" "${PY[@]}" -c "
import json, os, shutil
from pathlib import Path
chain = Path('data/worlds/fujix-55-chain'); g = Path('$X/grid'); g.mkdir(parents=True, exist_ok=True)
g4 = json.loads((chain / 'hydrology/4m/grid.json').read_text())
g32 = json.loads((chain / 'hydrology/32m/grid.json').read_text())
for name, src in (('elevation.f32', chain / 'elevation.f32'), ('classes.u8', chain / 'classes.u8'),
                  ('moisture.f32', chain / 'hydrology/32m/moisture.f32')):
    if not (g / name).exists(): os.symlink(src.resolve(), g / name)
shutil.copy('data/worlds/fujix-collage-55/provenance32.u8', g / 'provenance.u8')
m = {'size': g4['size'], 'spacing_m': 4.0, 'origin': g4['origin'], 'classes_cell_m': 4.0,
     'provenance_cell_m': 32.0, 'moisture': g32['moisture'] | {'file': 'moisture.f32'},
     'discharge': {'mean_land_mm_h': g32['discharge']['mean_land_mm_h']}}
(g / 'grid.json').write_text(json.dumps(m, indent=1))"
( /usr/bin/time -v systemd-run --user --scope -q -p MemoryMax=10G target/release/topography world water \
    --grid $X/grid --work $X/work --tile 2048 --threads 4 >> $X/log 2>&1; echo $? > $X/exit_code ) &
echo $! > $X/pid
```

Watch per the heavy-run protocol. When `exit_code` is 0:

```bash
grep -E 'erosion: routing|done in|Maximum resident|Elapsed' $X/log
```

- [ ] **Step 6: Apply the solver rule (B1)**

Apply B1: (a) the two routings' `passes` seconds sum to ≤ 45 min; (b) `Maximum resident set size` ≤ 7 GB (7,340,032 kB); (c) `stages.json` `erosion.walls_left` and `erosion_new_pits` are 0 and no cell rose (`target/release/topography world water-finish` on this work dir, under the same cap). `order_ties` is a figure (ties are processed receiver-first, M3). If (a) fails, do Task 9b next. If only (b) fails, slim the wrapper as B1 says (drop `classes`/`flags` once the bit sets exist, read `raw` only for the taper, sort `u32` indices), rerun and remeasure; 9b would not help memory. If (c) fails, it is a bug: fix it with a test. Record the numbers and the decision in the progress file and `TASKS.md`.

- [ ] **Step 7: Expert review** — output: the five 32 m reports, the two `pair.png`, the `seam_lakes` lists, the 16,384² timing lines and the decision.

---

## Task 9b (contingency, only if Task 9's B1 rule fails): explicit GPU stream power

**Files:**
- Create: `training/src/topography_training/water/erosion_gpu.py`, `training/tests/test_water_erosion_gpu.py`
- Modify: `crates/topography-core/src/world/water/erosion.rs` (`erode` writes the routing's inputs and stops when `TOPOGRAPHY_EROSION=gpu`; reads the GPU result back)
- Modify: `training/src/topography_training/water/run.py` (`run_level` runs the GPU pass between the two Rust calls)

**Interfaces:**
- Rust writes, per routing k, into the work directory: `e<k>-in-z.f32` (current ground), `e<k>-in-floor.f32`, `e<k>-in-ka.f32`, `e<k>-in-down.u32` (the receiver index used by `stream_power`: the sill's `down`, a held receiver mapped to the node's sill cell, `u32::MAX` for none) and `e<k>-in-base.f32` (a fixed base where the receiver is a closed node's level or the sea: the value, else NaN); exits with code 3 and the line `erosion: gpu inputs for routing k`.
- Python: `erosion_gpu.step(z, floor, ka, down, base, spacing_m, substeps) -> np.ndarray` and `erosion_gpu.main(work, k)` writing `e<k>-gpu.f32`; Rust, rerun, finds it and continues with diffusion and taper exactly as before.

- [ ] **Step 1: Failing test** — `training/tests/test_water_erosion_gpu.py`:

```python
import unittest

import numpy as np

from topography_training.water import erosion_gpu


class ExplicitStreamPowerTest(unittest.TestCase):
    def test_matches_the_implicit_step_on_a_strip(self):
        z = np.array([40, 40, 40, 40, 10, 10, 10, 0], np.float32)
        ka = np.full(8, 7e-4 * np.sqrt(10e6), np.float32)
        down = np.array([1, 2, 3, 4, 5, 6, 7, 2**32 - 1], np.uint32)
        out = erosion_gpu.step(z, np.full(8, -1e9, np.float32), ka, down, np.full(8, np.nan, np.float32),
                               32.0, substeps=64)
        # The implicit step's lip (Rust test) ends between 30 and 39 m; explicit with many
        # substeps lands near it and never raises a cell or drops one below its receiver.
        self.assertTrue(30.0 < out[3] < 39.0)
        self.assertTrue((out <= z).all())
        self.assertTrue((out[:-1] >= out[1:] - 1e-4).all())


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Implement** — `erosion_gpu.py`:

```python
"""Explicit stream power on the GPU (Task 9b contingency): the same step as the Rust implicit
pass, z <- z - dt·F·(z - z_receiver) in `substeps` deterministic gathers (no atomics), never
below the floor or the receiver, never raised."""

from __future__ import annotations

import sys
from pathlib import Path

import numpy as np

NONE = 2**32 - 1


def step(z, floor, ka, down, base, spacing_m: float, substeps: int) -> np.ndarray:
    import torch

    torch.use_deterministic_algorithms(True)
    dev = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    zt = torch.from_numpy(z.astype(np.float32)).to(dev)
    fl = torch.from_numpy(floor).to(dev)
    has = torch.from_numpy(down != NONE).to(dev)
    idx = torch.from_numpy(np.where(down == NONE, 0, down).astype(np.int64)).to(dev)
    fixed = torch.from_numpy(base).to(dev)
    step_len = torch.ones_like(zt) * spacing_m  # diagonal lengths are folded into ka by Rust
    f = torch.from_numpy(ka).to(dev) / step_len
    dt = 1.0 / substeps
    z0 = zt.clone()
    for _ in range(substeps):
        zr = torch.where(torch.isnan(fixed), zt[idx], fixed)
        drop = torch.clamp(zt - zr, min=0.0)
        zt = torch.where(has, zt - dt * f * drop / (1.0 + dt * f), zt)
        zt = torch.maximum(torch.maximum(zt, fl), torch.where(has, zr, zt))
    return torch.minimum(zt, z0).cpu().numpy()


def main(work: Path, k: int) -> int:
    def rd(name, dt):
        return np.fromfile(work / f"e{k}-in-{name}", dt)

    z, floor, ka = rd("z.f32", "<f4"), rd("floor.f32", "<f4"), rd("ka.f32", "<f4")
    down, base = rd("down.u32", "<u4"), rd("base.f32", "<f4")
    substeps = int(max(1, np.ceil(float(ka.max()) / 32.0 * 4)))
    out = step(z, floor, ka, down, base, float((work / "spacing").read_text()), substeps)
    tmp = work / f"e{k}-gpu.f32.tmp"
    out.astype("<f4").tofile(tmp)
    tmp.replace(work / f"e{k}-gpu.f32")
    print(f"erosion: gpu routing {k} done ({substeps} substeps)", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main(Path(sys.argv[1]), int(sys.argv[2])))
```

On the Rust side, in `erode`, when `std::env::var("TOPOGRAPHY_EROSION").as_deref() == Ok("gpu")`: after building `Routing`, if `e<k>-gpu.f32` is missing, write the five inputs (the `down` of each cell as `stream_power` resolves it, ka divided by the step length factor `dist_cells` so the GPU uses `spacing_m` only, and `spacing`), and return `Err(AcquisitionError::Invalid("erosion: gpu inputs for routing {k}"))` with process exit code 3 mapped in the CLI; else read `e<k>-gpu.f32` into `z` instead of calling `stream_power`, then continue with `diffuse` and `taper`. `run_level` (Task 10) loops: run `world water`; on exit 3, run `python -m topography_training.water.erosion_gpu <work> <k>` and rerun `world water`.

- [ ] **Step 3:** Run the test (`PYTHONPATH=training/src $P -m unittest training/tests/test_water_erosion_gpu.py`), repeat Task 9 Steps 5–6 with `TOPOGRAPHY_EROSION=gpu`, and keep the GPU path only if it meets the B1 rule's (a)–(d); commit (`feat(water): explicit GPU stream power for large coarse grids`), expert review.

---
## Task 10: The level runner, provenance in every grid and the hard coarse guide

Rulings B11, B15, B30; spec "Coarse guide", "An auto-generated contact sheet of every 4 m lake exit".

**Files:**
- Create: `training/src/topography_training/water/grids.py` (grid writing shared by the chain and the driver)
- Modify: `training/src/topography_training/water/run.py` (`run_level`, `level` mode, `_water` takes `river_km2`, `_review` with a hard guide, `_assemble` withholds on a guide failure, `build_world_grid`/`build_window_grid` add provenance and handle L4 squares and v3 chains)
- Modify: `training/src/topography_training/water/review.py` (`coarse_guide` gets `min_km2`, `min_depth_m`, `edge_m` and `coarse_river_km2`)
- Modify: `training/src/topography_training/diffusion/hydrology.py` (`write_grid` re-exported from `water.grids`)
- Modify: `crates/topography-core/src/world/water/mod.rs` (`finish` publishes `moisture.f32`)
- Modify: `training/tests/test_water_run.py`, `training/tests/test_water_review.py`; Create: `training/tests/test_water_level.py`

**Interfaces:**
- Consumes: `world rain` (Task 1), the provenance sidecar (Task 3), the checks (Task 7).
- Produces:
  - `grids.write_grid(path, h, spacing_m, origin, classes=None, offset_m=None, core_rows=None, classes_cell_m=None, wind_from_deg=None, provenance=None, provenance_cell_m=None, elevation_link: Path | None = None) -> Path` (as v2's, plus provenance; `elevation_link` symlinks a large ground instead of copying `h`, which may then be None).
  - `grids.add_moisture(grid: Path, coarse_out: Path) -> None`: copies `moisture.f32` and the `moisture` and `discharge.mean_land_mm_h` entries from a coarser level's published output.
  - `grids.provenance_window(prov32: np.ndarray, x0_m: float, y0_m: float, n: int, cell_m: float) -> np.ndarray`: nearest 32 m provenance on an n² grid of `cell_m` cells whose NW corner is (x0_m, y0_m) metres from the map's NW corner (y south).
  - `run.run_level(run: Path, tile: int = 2048, threads: int = 4, coarse: Path | None = None, river_km2: float | None = None) -> int` — `run/grid` must exist; runs `world rain` when grid.json lacks `moisture`, then `world water` and `world water-finish` into `run/work` and `run/out`, then the review (crops, lake-exit sheets) into `run/review` and the coarse guide against `coarse` (a coarser level's `out`); writes `run/level.json` `{"passed", "failures", "report", "coarse_guide", "review"}`; returns 0 passed, 1 failed (report or guide), raises `Refused` (exit 2) otherwise.
  - CLI: `python -m topography_training.water.run level --run <dir> [--coarse <out>] [--tile 2048] [--threads 4] [--river-km2 X]`.
  - `review.coarse_guide(world, coarse, fine_dir=None, min_km2=0.05, min_depth_m=None, edge_m=1000.0, coarse_river_km2=None) -> dict` (B15, review I5): a coarse lake can fail only when its area is ≥ `min_km2`, its depth (level − lowest coarse ground under it) exceeds `min_depth_m` (default: the fine level's `breach_cap_m` + 1 m from the table) and it lies ≥ `edge_m` from every inner edge of the fine footprint; the others are counted in `lakes_small`, `lakes_shallow`, `lakes_near_edge` and still get their level difference listed; `coarse_river_km2` defaults to the coarse level's `river_km2`. `GUIDE_SUMMARY` gains the three counts. The coarse output's `elevation.f32` is read for the depth.
  - `water-finish` publishes `moisture.f32` beside `grid.json` when the grid has one.

- [ ] **Step 1: Write the failing tests**

`training/tests/test_water_level.py`:

```python
"""The level runner on a tiny coarse grid (needs the release topography binary)."""

import json
import tempfile
import unittest
from pathlib import Path

import numpy as np

SCRATCH = Path(__file__).resolve().parents[2] / "scratch" / "test-water"
SCRATCH.mkdir(parents=True, exist_ok=True)

from topography_training.water import grids, review, run as W  # noqa: E402


def coarse_grid(run: Path, n: int = 64, spacing: float = 32.0) -> Path:
    y, x = np.mgrid[0:n, 0:n].astype(np.float32)
    z = 300.0 - 5.0 * x + 30.0 * np.sin(x / 7.0) * np.cos(y / 9.0) - 60.0 * np.exp(-((x - 20) ** 2 + (y - 20) ** 2) / 40.0)
    prov = np.where(x < 30, 1, 2).astype(np.uint8)
    return grids.write_grid(run / "grid", z, spacing, (0.0, n * spacing), np.full((n, n), 18, np.uint8),
                            wind_from_deg=270.0, provenance=prov, provenance_cell_m=spacing)


class LevelTest(unittest.TestCase):
    def test_a_coarse_level_rains_erodes_checks_and_reviews(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as tmp:
            run = Path(tmp) / "run-32m"
            coarse_grid(run)
            code = W.run_level(run, tile=32, threads=2, river_km2=0.05)
            self.assertIn(code, (0, 1))
            level = json.loads((run / "level.json").read_text())
            self.assertEqual(level["passed"], code == 0)
            for name in ("elevation.f32", "water-surface.f32", "water-body.u32", "bodies.json", "report.json",
                         "moisture.f32", "grid.json", "flow.u8", "discharge.f32"):
                self.assertTrue((run / "out" / name).exists(), name)
            stages = json.loads((run / "work" / "stages.json").read_text())
            self.assertEqual(stages["erosion"]["routings"], 5)
            self.assertTrue(any(p.parts[-2] == "lake-exits" for p in map(Path, level["review"])) or
                            not any(b["kind"] == "lake" for b in json.loads((run / "out" / "bodies.json").read_text())["bodies"]))
            # Resuming does nothing new.
            self.assertEqual(W.run_level(run, tile=32, threads=2, river_km2=0.05), code)

    def test_a_finer_level_takes_the_coarse_rain_and_is_guided(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as tmp:
            c = Path(tmp) / "run-32m"
            coarse_grid(c)
            W.run_level(c, tile=32, threads=2, river_km2=0.05)
            f = Path(tmp) / "run-4m"
            z32 = np.fromfile(c / "out" / "elevation.f32", "<f4").reshape(64, 64)
            z4 = np.kron(z32[16:32, 16:32], np.ones((8, 8), np.float32))  # 128² at 4 m inside it
            prov = np.fromfile(c / "grid" / "provenance.u8", np.uint8).reshape(64, 64)[16:32, 16:32]
            g = grids.write_grid(f / "grid", z4, 4.0, (0.0, 64 * 32.0), np.full((16, 16), 18, np.uint8),
                                 offset_m=(16 * 32.0, 16 * 32.0), classes_cell_m=32.0, provenance=prov,
                                 provenance_cell_m=32.0)
            grids.add_moisture(g, c / "out")
            code = W.run_level(f, tile=64, threads=2, coarse=c / "out", river_km2=0.005)
            level = json.loads((f / "level.json").read_text())
            self.assertIn("hard_failures", level["coarse_guide"])
            self.assertEqual(code == 0, level["passed"])
            if level["coarse_guide"]["hard_failures"]:
                self.assertIn("coarse_guide", level["failures"])

    def test_provenance_window_is_nearest(self):
        p = np.arange(16, dtype=np.uint8).reshape(4, 4)
        w = grids.provenance_window(p, 32.0, 64.0, 16, 4.0)  # 64 m square at (32, 64) m, 4 m cells
        self.assertEqual(w.shape, (16, 16))
        self.assertTrue((w[:8, :8] == p[2, 1]).all() and (w[8:, 8:] == p[3, 2]).all())


if __name__ == "__main__":
    unittest.main()
```

In `test_water_review.py`, the existing coarse-guide tests use 16-cell coarse lakes at 4 m without a coarse `elevation.f32`: write one (100 m everywhere, 90 m under their lakes) in `write()` when `discharge` is given, and pass `min_km2=0, min_depth_m=0, edge_m=0` in `case()` and in `test_a_window_is_compared_at_its_offset`. Then add (replace the earlier draft's minimum-size test with these):

```python
    def test_only_large_deep_lakes_away_from_edges_can_fail(self):
        tmp = Path(tempfile.mkdtemp())
        n = 128  # 4 m coarse cells: 512 m; footprint = the whole grid (no inner edge)
        cb = np.zeros((n, n), np.uint32)
        cb[10:12, 10:12] = 2  # 64 m²: small
        cb[40:100, 40:100] = 3  # 0.0576 km², 10 m deep: hard
        cb[20:80, 105:125] = 4  # 0.0192 km²: small
        ground = np.full((n, n), 100.0, np.float32)
        ground[40:100, 40:100] = 90.0
        bodies = [{"id": i, "kind": "lake", "level_m": 100.0} for i in (2, 3, 4)]
        write(tmp / "coarse", n, cb, bodies, np.zeros((n, n)), 4.0)
        ground.astype("<f4").tofile(tmp / "coarse" / "elevation.f32")
        write(tmp / "fine", n * 4, np.zeros((n * 4, n * 4), np.uint32), [])
        g = review.coarse_guide(tmp / "fine", tmp / "coarse", fine_dir=tmp / "fine", min_km2=0.05, min_depth_m=3.0)
        self.assertEqual(g["lakes_gone"], [3])
        self.assertEqual(g["lakes_small"], 2)
        self.assertEqual(g["hard_failures"], 1)
        shallow = review.coarse_guide(tmp / "fine", tmp / "coarse", fine_dir=tmp / "fine", min_km2=0.05,
                                      min_depth_m=12.0)
        self.assertEqual((shallow["hard_failures"], shallow["lakes_shallow"]), (0, 1))

    def test_lakes_near_a_window_edge_are_figures(self):
        tmp = Path(tempfile.mkdtemp())
        n = 1024  # 4 m coarse cells: 4,096 m
        ground = np.full((n, n), 100.0, np.float32)

        def coarse_with(r0, c0):
            cb = np.zeros((n, n), np.uint32)
            cb[r0 : r0 + 80, c0 : c0 + 80] = 5  # 0.1 km², 20 m deep
            g = ground.copy()
            g[r0 : r0 + 80, c0 : c0 + 80] = 80.0
            write(tmp / "coarse", n, cb, [{"id": 5, "kind": "lake", "level_m": 100.0}], np.zeros((n, n)), 4.0,
                  origin=[0.0, 4096.0])
            g.astype("<f4").tofile(tmp / "coarse" / "elevation.f32")
            return review.coarse_guide(tmp / "fine", tmp / "coarse", fine_dir=tmp / "fine", min_km2=0.05,
                                       min_depth_m=3.0)

        # A 1 m window over x, y in [0, 3,072) m: its east and south edges are inner, west and north are the world's.
        write(tmp / "fine", 3072, np.zeros((3072, 3072), np.uint32), [], origin=[0.0, 4096.0], offset_m=[0.0, 0.0])
        self.assertEqual(coarse_with(300, 10)["hard_failures"], 1, "beside the world's west edge: judged")
        g = coarse_with(300, 680)  # ends 32 m from the window's inner east edge
        self.assertEqual((g["hard_failures"], g["lakes_near_edge"]), (0, 1))
```

In `test_water_run.py`, change `test_a_failing_review_never_withholds_the_manifest`: the crops failing still never withhold; a coarse guide that **fails to run** now withholds (B15):

```python
            report = json.loads((out / "hydrology" / "1m" / "report.json").read_text())
            self.assertEqual(code, 1, "a coarse guide that cannot run fails the world (B15)")
            m = json.loads((out / "manifest-failed.json").read_text())
            self.assertIn("KeyError", m["water"]["coarse_guide"]["error"])
            self.assertIn("coarse_guide", m["water"]["failures"])
            self.assertEqual(m["water"]["review"], {"error": "RuntimeError: boom"})
            del report
```

- [ ] **Step 2: Run them to see them fail**

Run: `PYTHONPATH=training/src $P -m unittest training/tests/test_water_level.py training/tests/test_water_run.py training/tests/test_water_review.py`
Expected: `ModuleNotFoundError: topography_training.water.grids` and the changed assertions failing.

- [ ] **Step 3: `water-finish` publishes the moisture field**

In `finish` (`mod.rs`, after `stage.copy("flow.u8", …)`):

```rust
    if let Some(file) = s.rain.entry.get("file").and_then(Value::as_str) {
        stage.copy("moisture.f32", &o.grid.join(file))?;
    }
```

and the published `grid.json`'s `moisture.file` stays `moisture.f32`. Add `moisture.f32` to the `for f in …` file list in `scripts/check.sh`.

- [ ] **Step 4: `grids.py`**

```python
"""Grid directories for `topography world rain|water|water-finish` (docs/workflows/terrain-cascade.md):
grid.json, little-endian rasters (rows north to south), optional classes and provenance."""

from __future__ import annotations

import json
import os
import shutil
from pathlib import Path

import numpy as np


def write_grid(path: Path, h: np.ndarray | None, spacing_m: float, origin: tuple[float, float],
               classes: np.ndarray | None = None, offset_m: tuple[float, float] | None = None,
               core_rows: tuple[int, int] | None = None, classes_cell_m: float | None = None,
               wind_from_deg: float | None = None, provenance: np.ndarray | None = None,
               provenance_cell_m: float | None = None, elevation_link: Path | None = None,
               size: tuple[int, int] | None = None) -> Path:
    """A grid directory (replaced if present). `origin` is the world's north-west corner in the
    layout plane; `offset_m` [east, south] places the grid inside it. With `elevation_link` the
    ground is a symlink (give `size` as (w, h)) instead of a copy of `h`."""
    if path.exists():
        shutil.rmtree(path)
    path.mkdir(parents=True)
    w, hh = size if h is None else (h.shape[1], h.shape[0])
    grid = {"size": [int(w), int(hh)], "spacing_m": float(spacing_m),
            "origin": [float(origin[0]), float(origin[1])]}
    if offset_m is not None:
        grid["offset_m"] = [float(offset_m[0]), float(offset_m[1])]
    if core_rows is not None:
        grid["core_rows"] = [int(core_rows[0]), int(core_rows[1])]
    if classes_cell_m is not None:
        grid["classes_cell_m"] = float(classes_cell_m)
    if wind_from_deg is not None:
        grid["wind_from_deg"] = float(wind_from_deg)
    if elevation_link is not None:
        os.symlink(Path(elevation_link).resolve(), path / "elevation.f32")
    else:
        np.ascontiguousarray(h, dtype="<f4").tofile(path / "elevation.f32")
    if classes is not None:
        np.ascontiguousarray(classes, dtype=np.uint8).tofile(path / "classes.u8")
    if provenance is not None:
        np.ascontiguousarray(provenance, dtype=np.uint8).tofile(path / "provenance.u8")
        grid["provenance_cell_m"] = float(provenance_cell_m or spacing_m)
    (path / "grid.json").write_text(json.dumps(grid, indent=1))
    return path


def add_moisture(grid: Path, coarse_out: Path) -> None:
    """The 128 m moisture field and mean land rain of a coarser level's published output."""
    cg = json.loads((coarse_out / "grid.json").read_text())
    m = json.loads((grid / "grid.json").read_text())
    shutil.copy(coarse_out / cg["moisture"]["file"], grid / "moisture.f32")
    m["moisture"] = cg["moisture"] | {"file": "moisture.f32"}
    m["discharge"] = {"mean_land_mm_h": cg["discharge"]["mean_land_mm_h"]}
    (grid / "grid.json").write_text(json.dumps(m, indent=1))


def provenance_window(prov32: np.ndarray, x0_m: float, y0_m: float, n: int, cell_m: float) -> np.ndarray:
    """Nearest 32 m provenance on an n² grid of `cell_m` cells whose north-west corner lies
    (x0_m east, y0_m south) of the map's."""
    c = (np.arange(n) + 0.5) * cell_m
    rows = np.clip(((y0_m + c) // 32.0).astype(int), 0, prov32.shape[0] - 1)
    cols = np.clip(((x0_m + c) // 32.0).astype(int), 0, prov32.shape[1] - 1)
    return prov32[np.ix_(rows, cols)]
```

In `diffusion/hydrology.py`, replace `write_grid`'s body with `from ..water.grids import write_grid  # noqa: F401` at module level (and delete the old function).

- [ ] **Step 5: `review.coarse_guide`**

```python
def coarse_guide(world: Path, coarse: Path, fine_dir: Path | None = None, min_km2: float = 0.05,
                 min_depth_m: float | None = None, edge_m: float = 1000.0,
                 coarse_river_km2: float | None = None) -> dict:
```

At the start:

```python
    from . import table as water_table

    cs_m = float(_json(coarse / "grid.json").get("spacing_m", 1.0))
    fs_m = float(_json((fine_dir or world) / "grid.json").get("spacing_m", 1.0))
    if min_depth_m is None:
        min_depth_m = float(water_table.level(fs_m)["breach_cap_m"]) + 1.0
    if coarse_river_km2 is None:
        coarse_river_km2 = float(water_table.level(cs_m)["river_km2"])
    min_cells = math.ceil(min_km2 * 1e6 / (cs_m * cs_m))
    edge_cells = int(math.ceil(edge_m / cs_m))
```

In the lake loop, after the existing `cut` test and before comparing, decide whether the lake can fail:

```python
        cells = int(mask.sum())
        lowest = float(_rows(coarse / "elevation.f32", "<f4", cw, (y0 + r0, x0 + c0, r1 - r0 + 1, c1 - c0 + 1))[mask].min())
        near = ((inner[0] and r0 < edge_cells) or (inner[1] and c0 < edge_cells)
                or (inner[2] and r1 >= ph - edge_cells) or (inner[3] and c1 >= pw - edge_cells))
        hard = True
        if cells < min_cells:
            small += 1
            hard = False
        elif clevels[i] - lowest <= min_depth_m:
            shallow += 1
            hard = False
        elif near:
            near_edge += 1
            hard = False
```

and add a found mismatch to `gone`, `over` and `doubled` (and so to `hard_failures`) only when `hard`; the level difference is listed for every compared lake. Initialise `small = shallow = near_edge = 0`, return them as `lakes_small`, `lakes_shallow`, `lakes_near_edge` (plus `min_cells`, `min_depth_m`, `edge_m`), add the three counts to `GUIDE_SUMMARY`, and replace the `RIVER_KM2_4M` uses with `coarse_river_km2`.

- [ ] **Step 6: `run.py`**

`_water` gains `river_km2: float | None = None` appended to `flags` as `["--river-km2", str(river_km2)]` when given. `_review` gains `hard: bool`:

```python
def _review(src: Path, fine: Path, coarse: Path | None, dest: Path, hard: bool = True) -> dict:
```

and in its guide `except` branch, when `hard`, also set `entry["guide_failed"] = True`. After the guide is computed, `entry["guide_failed"] = bool(guide["hard_failures"])`. `_assemble` decides with both:

```python
    guide_failed = bool(reviewed.get("guide_failed"))
    passed = report["passed"] and not guide_failed
    failures = report["failures"] + (["coarse_guide"] if guide_failed else [])
    # In the manifest dict, the "water" entry's first keys become (the other keys stay):
    #     "passed": passed, "failures": failures, "relaxation": "none",
    name = "manifest.json" if passed else "manifest-failed.json"
```

(`_review` must run before the manifest name is chosen; move the call above it.) The return value is `0 if passed else 1`.

`run_level`:

```python
def run_level(run: Path, tile: int = 2048, threads: int = 4, coarse: Path | None = None,
              river_km2: float | None = None) -> int:
    """One coarse level in `run` (grid/ built by the caller): rain if the grid has none, the water
    model with erosion, finish, review and the hard coarse guide against `coarse`."""
    grid = run / "grid"
    if not (grid / "grid.json").exists():
        raise Refused(f"{grid} has no grid.json")
    progress = Progress(run / "progress.json", grid)
    m = _json(grid / "grid.json")
    work = run / "work"
    _check_disk(run, m["size"][0] * m["size"][1], work)
    binary, digest = _pin(run / "bin")
    if "moisture" not in m:
        if _rust([str(binary), "world", "rain", "--grid", str(grid)]) != 0:
            raise Refused("world rain failed")
    progress.key(digest, tile, grid, work)
    code = _water(progress, binary, grid, work, run / "out", tile, threads, river_km2)
    if not (step := progress.done("review")):
        step = _review(run, run / "out", coarse, run / "review", hard=coarse is not None)
        progress.mark("review", **step)
    report = _json(run / "out" / "report.json")
    guide_failed = bool(step.get("guide_failed"))
    passed = code == 0 and report["passed"] and not guide_failed
    level = {"passed": passed, "failures": report["failures"] + (["coarse_guide"] if guide_failed else []),
             "report": "out/report.json", "coarse_guide": step.get("coarse_guide"), "review": step.get("review")}
    cascade.save_json(run / "level.json", level)
    print(f"level {run}: {'passed' if passed else 'FAILED: ' + ', '.join(level['failures'])}", flush=True)
    return 0 if passed else 1
```

`Progress.__init__` compares its stored `source` with the path given; for a level run the source is the grid directory. `main` gains:

```python
    lv = sub.add_parser("level", help="one coarse level (32 m or 4 m) from a prepared grid")
    lv.add_argument("--run", type=Path, required=True, help="run directory holding grid/")
    lv.add_argument("--coarse", type=Path, help="the next coarser level's out/ (coarse guide)")
    lv.add_argument("--tile", type=int, default=2048)
    lv.add_argument("--threads", type=int, default=4)
    lv.add_argument("--river-km2", type=float)
```

dispatching `code = run_level(a.run, a.tile, a.threads, a.coarse, a.river_km2)` inside the existing `try`, with `Refused` mapped to exit 2 like `run_world`.

`build_world_grid` handles whole worlds and L4 squares and adds the provenance:

```python
def build_world_grid(src: Path, out: Path) -> Path:
    """The 1 m grid of an L4 world (whole or a square): the reflinked raw surface, the chain's
    4 m classes and provenance (cropped for a square), the real moisture field."""
    m = _json(src / "manifest.json")
    n = m["size"][0]
    field, entry, mean = _moisture(src, m)
    chain = Path(m["source_world"])
    n4 = _json(chain / "manifest.json")["size"][0]
    c4 = n // 4
    col4, row4 = (m.get("params") or {}).get("window_4m") or [(n4 - c4) // 2] * 2
    grid = out / "hydrology" / "grid"
    grid.mkdir(parents=True, exist_ok=True)
    _symlink(Path("..", "..", RAW), grid / "elevation.f32")
    classes = chain / "classes.u8"
    prov = chain / "provenance.u8"
    if not prov.exists():
        raise Refused(f"{chain} has no provenance.u8 (a v3 chain world writes it)")
    extra = {}
    if c4 == n4:
        _symlink(classes.resolve(), grid / "classes.u8")
        _symlink(prov.resolve(), grid / "provenance.u8")
        extra["provenance_cell_m"] = 32.0
    else:
        cls = np.fromfile(classes, np.uint8).reshape(n4, n4)[row4 : row4 + c4, col4 : col4 + c4]
        _write_if_bytes_differ(grid / "classes.u8", np.ascontiguousarray(cls))
        p32 = np.fromfile(prov, np.uint8).reshape(n4 // 8, n4 // 8)
        _write_if_bytes_differ(grid / "provenance.u8",
                               grids.provenance_window(p32, col4 * 4.0, row4 * 4.0, c4, 4.0))
        extra["provenance_cell_m"] = 4.0
    _symlink(field.resolve(), grid / "moisture.f32")
    _write_if_changed(grid / "grid.json", {
        "size": [n, n], "spacing_m": 1.0, "origin": _origin(m, n, 1.0), "classes_cell_m": 4.0, **extra,
        "moisture": entry | {"file": "moisture.f32"}, "discharge": {"mean_land_mm_h": mean},
        "inputs": _inputs(classes, field),
    })
    return grid


def _write_if_bytes_differ(path: Path, a: np.ndarray) -> None:
    b = a.tobytes()
    if not path.exists() or path.read_bytes() != b:
        tmp = path.with_suffix(path.suffix + ".tmp")
        tmp.write_bytes(b)
        tmp.replace(path)
```

(`_origin(m, n, 1.0)` reads the square's own `world-input/world.json`, whose origin `cascade_l4` already offsets by the square.) `build_window_grid` adds the provenance the same way (`grids.provenance_window` over the window at its spacing, `provenance_cell_m` = the window's spacing; read `provenance.u8` from the chain world, or from the source world when it is the chain) and, for `--level 4` on a v3 chain (`l3-raw.f32` present), reads the raw L3 surface instead of `elevation.f32`:

```python
    elif level == 4:
        ground = src / "l3-raw.f32" if (src / "l3-raw.f32").exists() else src / "elevation.f32"
        field = _read_window(ground, "<f4", n, row, col, size, size)
```

- [ ] **Step 7: Run the tests**

```bash
"${CAP6[@]}" cargo build --release --locked --offline -p topography-cli
PYTHONPATH=training/src "${CAP6[@]}" $P -m unittest discover -s training/tests -p 'test_water_*.py'
```

Expected: PASS.

- [ ] **Step 8: Commit**

Run the task-finish block.

```bash
git add training/src/topography_training/water training/src/topography_training/diffusion/hydrology.py \
  training/tests crates/topography-core/src/world/water/mod.rs scripts/check.sh
git commit -m "feat(water): level runner for coarse levels, provenance in grids, hard coarse guide

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 9: Expert review** — output: the 32 m whole-collage run of Task 9 rerun through `run_level` (`python -m topography_training.water.run level --run scratch/hydrology-v3b/t10-32m` after copying `$T/base/grid` to `scratch/hydrology-v3b/t10-32m/grid`): `level.json`, `review/lake-exits/` and `review/overview.png`.

---
## Task 11: `cascade_chain` on v3 at 32 m and 4 m

Spec "Coarse stages", "Hand-off", staging 3; rulings B11, B12, B17, B22.

**Files:**
- Modify: `training/src/topography_training/diffusion/cascade_chain.py` (`Source` drops `water`, gains `provenance32`; `_collage_source` reads the sidecar; `_l1_l2` runs the 32 m level; `main` runs the 4 m level, writes `provenance.u8`, withholds the manifest on a failed level; `--lake-share`/`--tarn-share` removed; disk constant)
- Modify: `docs/workflows/terrain-cascade.md` ("Generating fujix from the collage" commands; full rewrite in Task 14)

**Interfaces:**
- Consumes: `water.run.run_level`, `water.grids.{write_grid, add_moisture}` (Task 10), `water.table.r` (Task 2), the provenance sidecar (Task 3).
- Produces (a v3 chain world, e.g. `data/worlds/fujix-55-chain-v3`):
  - `hydrology/run-32m/{grid,work,out,review,bin,level.json}` and `hydrology/32m` → `run-32m/out` (symlink);
  - `l3-raw.f32` (kept, d5dc446);
  - `hydrology/run-4m/…` and `hydrology/4m` → `run-4m/out`;
  - world layers as today (`elevation.f32` = the 4 m level's final ground, `water-surface.f32`, `water-body.u32`, `bodies.json`, `water.u8`, `moisture.f32`, `classes.u8`, `biome.u8`, `snow-ice.u8`, `vegetation.u8`, `description.*`) plus `provenance.u8` (32 m, the L3 square's cells);
  - `manifest.json` with `"water": {"model": "hydrology v3", "32m": {...level.json}, "4m": {...}}` and `"provenance": {"file": "provenance.u8", "cell_m": 32.0}`; when either level fails, `manifest-failed.json` and exit 1 (a failed 32 m level stops before L3).
- Removed: `Source.water`, `hydrology.Water` uses, `--lake-share`, `--tarn-share`.
- New flag (B32): `--no-l3-river-clamp` drops the 32 m rivers from the L3 clamp (lakes and sea stay); default keeps today's clamp until Task 15's A/B decides; recorded in `progress.json` params and the manifest.

- [ ] **Step 1: Source and L1 shares (B22)**

```python
# L1 128 m lake picker for authored and region layouts (not collages; B22): one share for every
# class, dry basins in classes whose lake-to-catchment ratio is at most L1_DRY_R.
L1_LAKE_SHARE = 0.01
L1_DRY_SHARE = 0.01
L1_DRY_R = 0.05
```

`Source`: replace `water: hydrology.Water` with nothing and add `provenance32: np.ndarray | None = None` after `region32`. `dry_at`:

```python
    def dry_at(self, k: int):
        """Cells of classes with dry basins (authored layouts; a real region has one class)."""
        if isinstance(self.classes, int):
            return None
        from ..water import table as water_table

        return np.array([water_table.r(c) <= L1_DRY_R for c in range(NUM_CLASSES)])[self.classes_at(k)]
```

`_region_source` and `_layout_source` drop the `Water` argument (and `_layout_source`'s profile mean). `_collage_source` reads the sidecar:

```python
    prov_path = d / "provenance32.u8"
    if not prov_path.exists():
        raise SystemExit(f"error: {d} has no provenance32.u8; run "
                         f"`python -m topography_training.diffusion.collage provenance {d}`")
    prov = np.fromfile(prov_path, np.uint8).reshape(l2.shape)
    return dataclasses.replace(src, reference=("collage.png", l2, 32.0, cascade.open_sea(l2)),
                               marks_from=l1, l2=l2, classes32=grid("class32"), region32=grid("region32"),
                               provenance32=prov)
```

In `_l1_l2`, the L1 `drain` call uses `L1_LAKE_SHARE` and `dry_share=L1_DRY_SHARE`; its signature loses `water, table`.

- [ ] **Step 2: The 32 m level**

Replace the block from `# Hydrology at 32 m (Rust)` to `times["hydrology_32m_s"] = …` with:

```python
    # Hydrology v3 at 32 m: rain, breach, erosion (provenance K), water, checks, review. Its
    # final ground is what L3 refines.
    from ..water import grids, run as water_run

    t = time.time()
    run32 = hyd / "run-32m"
    if not (run32 / "level.json").exists():
        grids.write_grid(run32 / "grid", h32, 32.0, src.origin, src.class_grid(4, np.s_[:, :], h32.shape),
                         wind_from_deg=src.wind, provenance=src.provenance32, provenance_cell_m=32.0)
    code = water_run.run_level(run32, river_km2=args.stream_km2)
    _link_dir(hyd / "32m", "run-32m/out")
    times["hydrology_32m_s"] = time.time() - t
    if code != 0:
        raise SystemExit(f"error: the 32 m water failed ({run32 / 'level.json'}); fix it before L3")
    return h128, sea128, lake128, raw32
```

with the helper:

```python
def _link_dir(link: Path, target: str) -> None:
    """A relative directory symlink (replaced if it points elsewhere)."""
    if link.is_symlink() and str(link.readlink()) == target:
        return
    if link.is_symlink():
        link.unlink()
    link.symlink_to(target, target_is_directory=True)
```

(`run_level` resumes, so a rerun of the chain after a crash during the 32 m level continues it; once `level.json` exists the grid is not rewritten.)

- [ ] **Step 3: The 4 m level and the world**

Replace the `# Hydrology at 4 m` block:

```python
    # Hydrology v3 at 4 m on the kept raw L3 surface, guided by the 32 m level (coarse guide).
    if not progress["hydrology_4m_done"]:
        t = time.time()
        run4 = hyd / "run-4m"
        if not (run4 / "grid" / "grid.json").exists():
            g = grids.write_grid(run4 / "grid", None, 4.0, src.origin, src.class_grid(4, sl, (n32, n32)),
                                 offset_m=(x0 * 32.0, y0 * 32.0), classes_cell_m=32.0,
                                 provenance=None if src.provenance32 is None else src.provenance32[sl],
                                 provenance_cell_m=32.0, elevation_link=args.out / "l3-raw.f32", size=(n4, n4))
            grids.add_moisture(g, hyd / "32m")
        code4 = water_run.run_level(run4, coarse=hyd / "32m")
        _link_dir(hyd / "4m", "run-4m/out")
        times["hydrology_4m_s"] = time.time() - t
        progress["hydrology_4m_done"] = True
        progress["hydrology_4m_passed"] = code4 == 0
        save()
```

Keep the rest (pictures, links, description, cover, classes, biome) as it is: they read `hyd / "4m"` and `hyd / "32m"` through the symlinks. Add before the manifest:

```python
    if src.provenance32 is not None:
        digests["provenance.u8"] = _write_layer(args.out / "provenance.u8", [src.provenance32[sl]])
    levels = {k: json.loads((hyd / f"run-{k}" / "level.json").read_text()) for k in ("32m", "4m")}
```

and in the manifest dict: `"water": {"model": "hydrology v3", **levels}`, `"provenance": {"file": "provenance.u8", "cell_m": 32.0}` (when written), and drop `"water": vars(water)` / `"water_fill"` stays. The manifest's name:

```python
    passed = all(v["passed"] for v in levels.values())
    name = "manifest.json" if passed else "manifest-failed.json"
    # The existing `(args.out / "manifest.json").write_text(json.dumps({…}, indent=1))` writes to
    # `args.out / name` instead; its dict is unchanged apart from the entries named above.
    print(f"wrote {args.out}" + ("" if passed else " (FAILED: see hydrology/run-*/level.json)"), flush=True)
    return 0 if passed else 1
```

(`_write_layer` takes an iterable of row blocks; one block is fine at 2048².) Keep the resume files while the world is failed, so a fixed 4 m level can be rerun without L3:

```python
    if passed:  # only now the run cannot be resumed any more
        (args.out / "l2.npz").unlink(missing_ok=True)
        cascade.drop_overlaps(args.out, 1 << 30)
```

replacing the unconditional `unlink` at the end of `main`. The disk check becomes `hydrology.check_disk(args.out, 130.0 * n4 * n4, "the chain")` with the comment `# Disk: world layers and raw L3 (44 B a 4 m cell), the 4 m water work (54 B) and erosion routings (28 B), with margin.` The "output is already complete" guard also refuses an existing `manifest-failed.json` only when `progress.json` says the 4 m level ran (rerun the level with `water.run level --run <world>/hydrology/run-4m` instead).

Remove `--lake-share`, `--tarn-share` and the `overrides`/`table` plumbing from `main`.

- [ ] **Step 3b: The L3 river clamp flag (B32)**

```python
    p.add_argument("--no-l3-river-clamp", dest="l3_river_clamp", action="store_false",
                   help="L3: ease the ground to the 32 m surface under lakes and sea only, not rivers (B32 A/B)")
```

and in `_l3`, the clamp line becomes:

```python
        # L3 learned land only: under lakes (and, unless --no-l3-river-clamp, rivers) the ground
        # eases down to the 32 m ground (B32: rivers in the clamp are under A/B test).
        clamp = (lake4 | river4) if args.l3_river_clamp else lake4
        under = smooth(clamp.astype(np.float32), 2.0)
```

The conditioning masks are unchanged. Add `"l3_river_clamp": args.l3_river_clamp` to the manifest.

- [ ] **Step 4: E2E check without a GPU-heavy run** (minutes, CPU; announce only)

A 2 km L3 square at the dam spot with no band diffused (`--l3-bands 0:0`: L3 keeps the refined 32 m surface), so both levels run end to end:

```bash
"${CAP6[@]}" cargo build --release --locked --offline -p topography-cli
O=scratch/hydrology-v3b/t11-chain; rm -rf $O
"${CAP12[@]}" "${PY[@]}" -m topography_training.diffusion.cascade_chain --collage data/worlds/fujix-collage-55 \
    --l3-run data/models/cascade-l3-run-5 --l3-km 2 --l3-centre 22858,38086 --l3-bands 0:0 --out $O 2>&1 | tail -n 15
jq '{passed: .["32m"].passed}' $O/hydrology/run-32m/level.json
jq '{passed, failures, coarse_guide: (.coarse_guide | {hard_failures, lakes_compared, lakes_small})}' $O/hydrology/run-4m/level.json
ls $O/manifest*.json $O/provenance.u8 $O/hydrology/32m/ $O/hydrology/4m/ $O/hydrology/run-4m/review/lake-exits | head -40
```

Expected: both levels ran; `manifest.json` or `manifest-failed.json` with the levels' failures; `hydrology/32m` and `hydrology/4m` hold v3 outputs. A failure here is judged like any check failure (a real defect to fix, never a threshold); since L3 was not diffused, the 4 m level here is on upsampled 32 m ground and says nothing about L3.

- [ ] **Step 5: Commit**

```bash
git add training/src/topography_training/diffusion/cascade_chain.py docs/workflows/terrain-cascade.md
git commit -m "feat(chain): hydrology v3 levels at 32 m and 4 m with provenance; drop the v1 shares

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Expert review** — output: `$O/hydrology/run-*/level.json`, `$O/hydrology/run-4m/review/` (spots, lake exits, overview), `$O/l3.png`.

---

## Task 12: `cascade_l4` only diffuses; the water world comes from `water.run`

Rulings B19, B20; spec "Removed" (the band-by-band driver).

**Files:**
- Modify: `training/src/topography_training/diffusion/cascade_l4.py`
- Create: `training/tests/test_cascade_l4_raw.py`
- Modify: `training/tests/test_water_run.py` (an L4 square world)

**Interfaces:**
- Consumes: a v3 chain world (Task 11).
- Produces: an L4 raw world: `elevation-raw.f32`, `snow-ice.u8`, `vegetation.u8`, `world-input/world.json` (origin offset by the square), `description.*`, `moisture.f32` link, `overview.png`, `detail.png` (hillshades of the raw surface) and `manifest.json` with `"kind": "world_raw"`, `"source_world"`, `"params"` (incl. `window_4m`), `"hydrology_input": {"file": "elevation-raw.f32", "sha256": …, "rerun": …}`, `"layers"` (digests of the three rasters). `water.run world --source <it>` makes the water world (whole or square).
- Removed: `_hydro_band`, `_seam_audit`, `_hydrology_only`, `_pin`, `_code_digest`, `SeamAuditFailed`, `MANIFEST_FAILED`, `BODY_FIELDS`, the seam constants, `--hydrology-only`, `--restart`, `--seam-audit`, `--drop-raw`, and the water layers in `_layers`.
- New flag (B32): `--keep-river-residual` (`action="store_true"`) keeps the diffused residual under 4 m rivers (zeroed under lakes and sea only); default keeps today's mask (residual zeroed under all 4 m water). In the band loop: `land = 1.0 - np.clip(smooth((lakes_sea if args.keep_river_residual else water).astype(np.float32), 2.0), 0, 1)` where `lakes_sea` is the refined mask of 4 m cells whose body is the sea or a lake (from `water-body.u32` and `bodies.json` of the 4 m world, refined like `water`).

- [ ] **Step 1: Write the failing test**

`training/tests/test_cascade_l4_raw.py`:

```python
import json
import tempfile
import unittest
from pathlib import Path

import numpy as np

from topography_training.diffusion import cascade_l4


class RawWorldTest(unittest.TestCase):
    def test_band_hydrology_flags_are_gone(self):
        for flag in ("--hydrology-only", "--seam-audit", "--drop-raw", "--restart"):
            with self.assertRaises(SystemExit):
                cascade_l4.main(["--out", "x", flag])

    def test_the_raw_manifest_names_its_layers_and_source(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            n = 64
            for name, dt in (("elevation-raw.f32", "<f4"), ("snow-ice.u8", np.uint8), ("vegetation.u8", np.uint8)):
                np.ones((n, n), dt).tofile(out / name)
            m = cascade_l4.raw_manifest(out, n, world_input=out / "world-input", source_world=out,
                                        params={"window_4m": [4, 8]}, extra={})
            self.assertEqual(m["kind"], "world_raw")
            self.assertEqual(m["params"]["window_4m"], [4, 8])
            self.assertEqual(set(m["layers"]), {"elevation-raw.f32", "snow-ice.u8", "vegetation.u8"})
            self.assertEqual(m["hydrology_input"]["file"], "elevation-raw.f32")
            self.assertEqual(m["hydrology_input"]["sha256"], m["layers"]["elevation-raw.f32"])
            json.dumps(m)


if __name__ == "__main__":
    unittest.main()
```

Run: `PYTHONPATH=training/src $P -m unittest training/tests/test_cascade_l4_raw.py` — Expected: FAIL (flags still accepted; no `raw_manifest`).

- [ ] **Step 2: Implement**

Delete the functions and flags listed under "Removed". `_layers` keeps `elevation-raw.f32`, `snow-ice.u8` and `vegetation.u8` only (`c.raw`, `c.snow_out`, `c.veg_out`); `DISK_BYTES = 6` (`# raw surface 4 B, snow and vegetation 1 B each`). In the band loop delete `hydro_until`, the pin block and `failed`. Add:

```python
def raw_manifest(out: Path, n: int, world_input: Path, source_world: Path, params: dict, extra: dict) -> dict:
    """The manifest of an L4 raw world: the diffused surface and cover, no water (ruling B20)."""
    digests = {name: hydrology.sha256(out / name) for name in (RAW, "snow-ice.u8", "vegetation.u8")}
    return {
        "schema_version": 1, "kind": "world_raw", "world_input": str(world_input.resolve()),
        "spacing_m": 1.0, "size": [n, n],
        "method": "banded L4 over a cascade chain 4 m world; water: python -m topography_training.water.run world",
        **extra, "source_world": str(source_world.resolve()), "params": params,
        "hydrology_input": {"file": RAW, "sha256": digests[RAW],
                            "rerun": "python -m topography_training.water.run world --source <this world> --out <new world>"},
        "layers": digests,
    }
```

`_finish` becomes:

```python
def _finish(c: SimpleNamespace, extra: dict) -> None:
    """Pictures, world input and the raw manifest of a finished L4 run."""
    from .cascade_chain import _png

    n = c.n
    over = np.concatenate([cascade.block_mean(np.asarray(c.raw[y : y + 1024]), 8) for y in range(0, n, 1024)])
    _png(c.out / "overview.png", over, 8.0, np.zeros(over.shape, bool), 2048)
    m, r = n // 2, min(512, n // 2)
    _png(c.out / "detail.png", np.asarray(c.raw[m - r : m + r, m - r : m + r]), 1.0, np.zeros((2 * r, 2 * r), bool), 1024)
    src = json.loads((Path(c.world["world_input"]) / "world.json").read_text())
    ox, oy = src["origin"][0] + c.col4 * 4.0, src["origin"][1] - c.row4 * 4.0
    world_input = c.out / "world-input"
    world_input.mkdir(exist_ok=True)
    (world_input / "world.json").write_text(json.dumps(src | {"name": f"{src.get('name', 'world')}-l4",
                                                              "origin": [ox, oy]}, indent=1))
    if (c.world_dir / "moisture.f32").exists():
        hydrology.link(c.world_dir, c.out, ("moisture.f32",))
        extra["moisture"] = c.world["moisture"]
    if "description" in c.world:
        shutil.copy(c.world_dir / c.world["description"], c.out / "description.json")
        shutil.copy(c.world_dir / "description.md", c.out / "description.md")
        extra["description"] = "description.json"
    params = c.params | {"window_4m": [c.col4, c.row4]}
    manifest = raw_manifest(c.out, n, world_input, c.world_dir, params, extra | {"class_id": c.class_id,
                                                                               "band_s": c.band_s})
    cascade.save_json(c.out / "manifest.json", manifest)
```

(`params.window_4m` is what `water.run.build_world_grid` reads for a square; `_setup` already computes `col4, row4`.) The one caller at the end of `_main` becomes `_finish(c, extra)` after the band loop, with `extra` holding `peak_vram_gb`, `deep_sea_bands`, `windows` as today; `_drop` keeps only the overlap clean-up. `main`'s "already complete" message drops the `--hydrology-only` hint. `_setup` still reads the 4 m grid's origin and offset from `world["hydrology"]["4m"]/grid.json` (the v3 chain keeps that path).

In `test_water_run.py`, `fake_world` gets a `square` option: the chain manifest says `size` [2n/4…] and the L4 manifest carries `"params": {"window_4m": [col4, row4]}`, plus a chain `provenance.u8` (32 m); add a test that `build_world_grid` writes cropped `classes.u8` and `provenance.u8` of `(n/4)²` bytes for a square and links them for a whole world. Also give the whole-world fixture a chain `manifest.json` (`{"size": [n // 4, n // 4]}`) and `provenance.u8` (`(n // 32)²` bytes).

- [ ] **Step 3: Run the tests**

Run: `PYTHONPATH=training/src $P -m unittest training/tests/test_cascade_l4_raw.py training/tests/test_water_run.py`
Expected: PASS.

- [ ] **Step 4: E2E on a 1 km L4 square over Task 11's chain** (GPU about 1–2 min, under 4 GB VRAM; announce only)

Task 11's chain did not diffuse L3; it is still a valid 4 m world for an L4 test of the plumbing:

```bash
O=scratch/hydrology-v3b/t11-chain; L=scratch/hydrology-v3b/t12-l4; W=scratch/hydrology-v3b/t12-l4-water
rm -rf $L $W
"${CAP12[@]}" "${PY[@]}" -m topography_training.diffusion.cascade_l4 --run data/models/cascade-l4-run-8 \
    --class-runs data/models/cascade-l4-run-8-experts --world $O --out $L --km 1 --centre 22858,38086 2>&1 | tail -n 5
jq '{kind, size, window: .params.window_4m}' $L/manifest.json
"${CAP6[@]}" "${PY[@]}" -m topography_training.water.run world --source $L --out $W --tile 512 --threads 4 | tail -n 3
jq '{passed: .water.passed, failures: .water.failures}' $W/manifest*.json
```

Expected: a `world_raw` manifest, then a water world whose manifest names its checks; `hydrology/grid/classes.u8` and `provenance.u8` are 256² bytes (a 1,024 m square: 256 cells of 4 m).

- [ ] **Step 4b: Kill and resume (review I7)** (GPU a few minutes; announce only)

The 35–41 h run must resume to the same bytes. Run the 1 km square again into a new directory, kill it after its first band, rerun, and compare the raw surface with Step 4's uninterrupted run; then do the same for the water world:

```bash
L2=scratch/hydrology-v3b/t12-l4-resume; rm -rf $L2
( "${CAP12[@]}" "${PY[@]}" -m topography_training.diffusion.cascade_l4 --run data/models/cascade-l4-run-8 \
    --class-runs data/models/cascade-l4-run-8-experts --world $O --out $L2 --km 1 --centre 22858,38086 > $L2.log 2>&1 ) &
until grep -q '^band 1/' $L2.log 2>/dev/null; do sleep 5; done
kill $(pgrep -f '[c]ascade_l4 .*t12-l4-resume' | head -n 1); sleep 5
"${CAP12[@]}" "${PY[@]}" -m topography_training.diffusion.cascade_l4 --run data/models/cascade-l4-run-8 \
    --class-runs data/models/cascade-l4-run-8-experts --world $O --out $L2 --km 1 --centre 22858,38086 | tail -n 2
cmp $L/elevation-raw.f32 $L2/elevation-raw.f32 && echo "raw identical after resume"
W2=scratch/hydrology-v3b/t12-l4-water-resume; rm -rf $W2
( "${CAP6[@]}" "${PY[@]}" -m topography_training.water.run world --source $L --out $W2 --tile 512 --threads 4 > $W2.log 2>&1 ) &
until grep -q 'water: ' $W2.log 2>/dev/null; do sleep 2; done
kill $(pgrep -f '[w]ater.run world .*t12-l4-water-resume' | head -n 1); sleep 3
"${CAP6[@]}" "${PY[@]}" -m topography_training.water.run world --source $L --out $W2 --tile 512 --threads 4 | tail -n 1
cmp $W/elevation.f32 $W2/elevation.f32 && cmp $W/water-body.u32 $W2/water-body.u32 && echo "water identical after resume"
```

(`sleep` here is inside a test step the executor runs in the foreground; with a harness that forbids foreground sleep, poll with a Monitor `until` loop instead.) Expected: both "identical". A difference is a resume bug: fix it before gate 4.

- [ ] **Step 5: Commit**

```bash
git add training/src/topography_training/diffusion/cascade_l4.py training/tests
git commit -m "refactor(l4): diffusion only; water worlds come from water.run world (squares too)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Expert review** — output: `$L/manifest.json`, `$W/hydrology/1m/report.json`, `$W/hydrology/review/spots/E25645-N24400.png` if inside.

---
## Task 13: Comb-cut channel pits on raw flats (gate-2 park)

Ruling B26. A 1 m defect independent of the regeneration; fixed before gate 4 so the L4 squares are judged on the final code.

**Files:**
- Modify: `crates/topography-core/src/world/water/checks.rs` (hard check `carve_pits`)
- Modify: `crates/topography-core/src/world/water/channel.rs` and/or `breach.rs` (the fix the diagnosis names)
- Test: a regression test in the file that holds the fix, built from the reproduction window

**Interfaces:**
- Produces: hard check `carve_pits`: a core cell lowered by more than `CHANGED_M` from raw, dry in the written water (`body == 0`), inside an m3 node (`node >= 2`) and in no m2 node (`node_written == 0`): a pond that only the cutting made.

- [ ] **Step 1: Reproduce** (seconds; announce only). Use superpowers:systematic-debugging.

```bash
"${CAP6[@]}" cargo build --release --locked --offline -p topography-cli
R=scratch/hydrology-v3b/t13-comb
"${CAP6[@]}" "${PY[@]}" -m topography_training.water.run window --source data/worlds/fujix-55-l4v2 \
    --level 1 --centre 41058,56173 --size 1024 --tile 512 --out $R | tail -n 3
"${CAP6[@]}" "${PY[@]}" -c "
from pathlib import Path
from topography_training.water import review
print(review.window_pair(Path('$R'), Path('$R/pair.png')))"
```

Look at `$R/pair.png` at the centre (the window's middle is E41058 N56173, row 9,363 of the world): the comb is a row of small cut pits across a raw flat.

- [ ] **Step 2: The failing check and its test**

In `check_tile`, per core cell (with `n2` = `l.node_written` and `node` = m3's, both already read):

```rust
            if raw[j] - z[j] > CHANGED_M && body[j] == 0 && node[j] >= 2 && n2[j] == 0 {
                t.fail("carve_pits", x, y);
            }
```

add `"carve_pits"` to `report`'s list, and a unit test in `checks.rs` (Plan A's fixture): a flat with one cell lowered 0.3 m, dry, `node` 2 there and `node_written` 0 → `carve_pits` 1; with `node_written` 2 (a pond before the cutting) → 0. Rebuild, rerun Step 1's window (delete `$R/work $R/out $R/review $R/progress.json` first; the binary changed) and confirm `jq '.checks.carve_pits' $R/out/report.json` counts the comb's cells and `report.json` `examples.carve_pits` lists cells at the comb.

- [ ] **Step 3: Diagnose** (no fix yet). For the listed cells, print per cell: raw, breached, carved, m2 level/node, m3 level/node, the owning thalweg cell and its section radius and centre cut (add a `#[ignore]` probe test in `channel.rs` reading `$R/work` through `probe.rs`, as Plan A's probes do). Decide between the two leading hypotheses and record the evidence in the progress file:
  - **H1 (taper below the surface on a flat):** on a raw flat the thalweg's R is the flat (m2 pond) level and the section's centre cut `R − depth` reaches neighbours whose own ground equals R, so each section disc dips below the flat between thalweg cells that the restore rounds lift unevenly, leaving pits.
  - **H2 (wall halving leaves alternate cells):** the wall rounds halve cuts cell by cell in Jacobi rounds; on a flat their 3 × 3 reach leaves every other cell cut, a comb.
  If neither fits the printed cells, form the hypothesis the data shows before changing code.

- [ ] **Step 4: Fix** — scope cap (review M9): if the root cause lies outside `channel.rs`/`breach.rs`, stop and re-plan with the coordinator before gate 4. Otherwise one change at the root cause, with a synthetic regression test reproducing the printed cell pattern (a 32 × 32 flat with a thalweg at the measured Q). The fix may not raise ground and may not touch any check. For H1 the shape of the fix is: a section cell on a flat whose cut would end below every non-section neighbour and is not under the river's surface is not cut (it would be a pit, not a channel). For H2: halve whole section rows together (all cells of one thalweg cell's disc), not single cells.

- [ ] **Step 5: Verify** — the unit tests, then Step 1's window: `carve_pits` 0 and the comb gone in `pair.png`; then Plan A's gate-1 canyon window (`--centre 27169,44546 --size 2048`) still passes every check.

- [ ] **Step 6: Commit** (two commits: the check, then the fix)

```bash
git add crates/topography-core/src/world/water/checks.rs
git commit -m "feat(water): carve_pits, ponds that only the cutting made, as a hard check

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
git add crates/topography-core/src/world/water
git commit -m "fix(water): no comb of cut pits on raw flats

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 7: Expert review** — output: `$R/pair.png` before and after, the diagnosis note, the canyon window's report.

---

## Task 14: Delete the v2 hydrology; documentation

Spec "Removed"; rulings B20, B24.

**Files:**
- Delete: `crates/topography-core/src/world/hydrology/` (the rest of it), `configs/hydrology/water-v1.toml`, `scripts/hydrology_grid.py`
- Modify: `crates/topography-core/src/world/mod.rs` (`pub mod hydrology;` out)
- Modify: `crates/topography-cli/src/main.rs` (the `world hydrology` and `world hydrology-seam` arms, `fn hydrology`, their usage lines)
- Modify: `scripts/check.sh` (line 36 and every v2 line), `scripts/README.md`
- Modify: `training/src/topography_training/diffusion/hydrology.py` (v2 wrappers and the v1 table out)
- Modify: `training/src/topography_training/diffusion/game_layers.py:173` (message)
- Modify: `docs/workflows/terrain-cascade.md`, `docs/workflows/terrain-diffusion.md`, `docs/superpowers/specs/2026-11-05-hydrology-v3-design.md` (Plan B departures)

**Interfaces:**
- Consumes: nothing new.
- Produces: no `world::hydrology`; `diffusion/hydrology.py` keeps only `flood`, `accumulate`, `carve`, `drain` (and their helpers, for L1 and exemplars), `binary`, `use_binary`, `pin_binary`, `prune_pins`, `sha256`, `link`, `world_bodies`, `check_disk`, `WORLD_LAYERS`, `WORLD_DRY_MIN_KM2`, `WATER_FILL`, `Output`, `read`.

- [ ] **Step 1: Delete**

```bash
git rm -r crates/topography-core/src/world/hydrology configs/hydrology/water-v1.toml scripts/hydrology_grid.py
```

Remove `pub mod hydrology;` from `world/mod.rs`; in `main.rs` delete the two match arms (lines ~373–375 and ~381–400), `fn hydrology` (~530–600) and the two usage lines (~46–47); in `check.sh` delete `cargo test … hydrology` (line 36) and any `world hydrology` lines (`grep -n hydrology scripts/check.sh`). In `diffusion/hydrology.py` delete `WATER_TABLE`, `Water`, `load_water`, `WATER`, `write_table`, the `write_grid` re-export, `TIMEOUT_S`, `_report`, `run`, `seam` and the "Rust `topography world hydrology`" comment; `DRY_MIN_DEPTH_M`/`DRY_MAX_ELONGATION` stay (used by `drain`). `game_layers.py:173` says "regenerate it with hydrology v3".

- [ ] **Step 2: Nothing refers to v2 any more**

```bash
rg -n 'world::hydrology|world hydrology|hydrology-seam|water-v1|water-v2\.toml|hydrology_grid|--hydrology-only|hydrology\.run\(|hydrology\.WATER|hydrology\.Water|write_table|seam_audit' \
   --glob '!docs/superpowers/**' --glob '!docs/plans/**' --glob '!scratch/**' --glob '!data/**' --glob '!target/**' --glob '!PLAN*' --glob '!TASKS.md' .
```

Expected: no output (history in `docs/superpowers/` and `docs/plans/` stays as written).

- [ ] **Step 3: Build and test**

Run the task-finish block without the `hydrology` test line (it no longer exists). Expected: all green; `check.sh` exit 0.

- [ ] **Step 4: Documentation**

`docs/workflows/terrain-cascade.md`:
- Delete "## Hydrology v2" and its subsections (lines 642–920 before this task).
- Rename "## Hydrology v3 (1 m)" to "## Hydrology v3" and extend it: one model at 32 m, 4 m and 1 m; the coarse levels' `e` stage (routings, stream power, diffusion, taper, caps, provenance K: link the B rulings of this plan); `world rain`; the level runner (`water.run level`) and its run-directory layout (`grid/ work/ out/ review/ bin/ level.json`); the provenance layer (kinds and bits, the `collage provenance` command); the water table v3 (`hot_dry`, `below_sea_coast_km`, `wall_deg`, `[levels.N.erosion]`); the new checks (`interior_p99`, `interior_max`, `handoff_narrow`, `dam_drops`, `lake_below_sea`, `carve_pits`) in the checks table with one line each, and `seam_lakes`, `interior_change_*` among the figures; the hard coarse guide (min size, what withholds a manifest); the outlet trickle; the coastal water table; the stage table gains `e` (`e<k>.f32`, `e<k>-*` rasters) and the "Rerunning from a stage" table gains `erosion` → first stage `e`.
- "Generating fujix from the collage": the commands become `collage provenance`, `cascade_chain --collage … --out data/worlds/fujix-55-chain-v3`, `cascade_l4 … --out data/worlds/fujix-55-l4v3b-raw`, `water.run world --source …-raw --out data/worlds/fujix-55-l4v3b`, `game_layers`, `export heightmap`; the L4 disk line becomes 6 B a 1 m cell; band hydrology times are gone from the cost list.
- `docs/workflows/terrain-diffusion.md`: fix its one v2 reference the grep in Step 2's sibling (`grep -n 'hydrology' docs/workflows/terrain-diffusion.md`) shows.

Spec: add a section "## Plan B departures" after "## Plan A departures" with one bullet each (filled with the measured numbers at gate 5, Task 18): erosion runs on the CPU (implicit, global; benchmark Task 9); provenance replayed from the collage as a sidecar; the hand-off checks apply at 32 m too; `dam_drops` counts labelled lakes' exits only and uses a knickpoint rule (step then gentle) instead of a raw exception; the coarse guide is hard only for lakes of 0.05 km² or more, deeper than the finer breach cap + 1 m, away from window edges; at coarse levels the all-cell `change_p99` is a figure and `change_max` adds both caps; the coastal water table for basins below sea level (per basin); every full lake starts a river; diffusion only near river cuts; the L3/L4 river conditioning A/B outcome (B32: "ground plus the existing water conditioning"); the L4 test strip is four squares.

Verify every command in the new sections with `--help` and every path with `ls`.

- [ ] **Step 5: Commit**

```bash
git add -A crates scripts configs training docs/workflows docs/superpowers/specs/2026-11-05-hydrology-v3-design.md
git commit -m "refactor!: delete the v2 object hydrology; document hydrology v3 at every level

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Expert review** — output: `git diff --stat`, the rewritten doc section, `check.sh` log.

---

## Task 15: Probe squares and the 4 m calibration (before gate 3)

Rulings B2 (4 m), B9, B14, B15. Real L3 output on two squares checks the 4 m level before the 5–6 h L3 run.

**Files:**
- Modify (only if the calibration moves a value): `configs/hydrology/water-v3.toml` (`[levels.4.erosion]`)
- Output (not committed): `scratch/hydrology-v3b/t15/`

**Interfaces:**
- Consumes: Tasks 1–14.
- Produces: the 4 m `k_dt`; evidence that the 4 m level passes on real L3 at the dam spots and the canyon.

- [ ] **Step 1: Two L3 squares** (GPU about 5 min and 2 min, VRAM about 6 GB, RAM under 8 GB; announce only)

```bash
"${CAP6[@]}" cargo build --release --locked --offline -p topography-cli
T=scratch/hydrology-v3b/t15; mkdir -p $T
for sq in "dams 8 22858,38086" "canyon 4 27169,20990"; do set -- $sq
  "${CAP12[@]}" "${PY[@]}" -m topography_training.diffusion.cascade_chain --collage data/worlds/fujix-collage-55 \
      --l3-run data/models/cascade-l3-run-5 --l3-class-runs data/models/cascade-l3-run-5-experts \
      --guidance 2 --guidance-classes 19 --l3-km $2 --l3-centre $3 --out $T/chain-$1 > $T/chain-$1.log 2>&1
  echo "$1: exit $?"; tail -n 3 $T/chain-$1.log
  jq -c '{passed, failures, guide: (.coarse_guide | {hard_failures, lakes_compared, lakes_small})}' $T/chain-$1/hydrology/run-4m/level.json
  jq -c '.checks | {dam_drops, handoff_narrow, walls, interior_p99, interior_max, lake_below_sea}' $T/chain-$1/hydrology/run-4m/out/report.json
done
```

- [ ] **Step 2: Calibration variants on the dams square** (CPU minutes each)

```bash
for k in 0 4.4e-5 8.75e-5 1.75e-4; do
  sed "/^\[levels.4.erosion\]/,/^interior_cap_m/ s/^k_dt = .*/k_dt = $k/" configs/hydrology/water-v3.toml > $T/water4-k$k.toml
done
awk 'BEGIN{skip=0} /^\[levels.4.erosion\]/{skip=1; next} /^\[/{skip=0} !skip' configs/hydrology/water-v3.toml > $T/water4-off.toml
B=target/release/topography; G=$T/chain-dams/hydrology/run-4m/grid
for v in off k0 k4.4e-5 k8.75e-5 k1.75e-4; do
  R=$T/4m-$v; rm -rf $R; mkdir -p $R
  "${CAP10[@]}" $B world water --grid $G --work $R/work --tile 2048 --threads 4 --water $T/water4-$v.toml > $R/log 2>&1
  "${CAP10[@]}" $B world water-finish --grid $G --work $R/work --ground $R/work/carved.f32 --out $R/out \
      --tile 2048 --threads 4 --water $T/water4-$v.toml >> $R/log 2>&1
  echo "$v: $(jq -c '{passed, failures, seam: .figures.seam_lakes.count, dams: .checks.dam_drops,
        narrow: .checks.handoff_narrow, ip99: .figures.interior_change_p99_m}' $R/out/report.json)"
done
```

- [ ] **Step 3: Apply the 4 m rule (B2)**

Among `k4.4e-5`, `k8.75e-5`, `k1.75e-4` whose hard checks all pass (including `dam_drops`, `handoff_narrow`, the interior budget), choose the **smallest** with `seam_lakes ≤ max(2, 0.2 × seam_lakes(off))`; if none, the passing one with the fewest seam lakes. If no variant passes `dam_drops`, stop: crop each failing exit (`review.crops` on `$R/out`, `failures/`), find whether the step comes from the 32 m ground (look at `chain-dams/hydrology/32m` there) or from L3, and fix that stage (a code fix, never a threshold; ruling B14 changes only with the owner). Commit a changed `k_dt` as `chore(water): calibrate 4 m erosion k_dt on the probe squares`.

If a square's `level.json` has coarse-guide hard failures, read each mismatch figure (`review/coarse-guide/`): a 32 m lake that L3 did not keep (gone) or kept at another level (over 5 m) is a real disagreement between levels; diagnose before gate 3 (the usual cause would be L3 easing the lake bed to the 32 m ground but not its rim).

- [ ] **Step 3b: A/B the L3 river clamp on the dams square (B32, review I1)** (GPU about 5 min; announce only)

```bash
"${CAP12[@]}" "${PY[@]}" -m topography_training.diffusion.cascade_chain --collage data/worlds/fujix-collage-55 \
    --l3-run data/models/cascade-l3-run-5 --l3-class-runs data/models/cascade-l3-run-5-experts \
    --guidance 2 --guidance-classes 19 --l3-km 8 --l3-centre 22858,38086 --no-l3-river-clamp \
    --out $T/chain-dams-noclamp > $T/chain-dams-noclamp.log 2>&1
for v in chain-dams chain-dams-noclamp; do
  echo "$v: $(jq -c '{passed, failures}' $T/$v/hydrology/run-4m/level.json)"
  jq -c '.checks | {dam_drops, thalweg_beside, handoff_narrow}' $T/$v/hydrology/run-4m/out/report.json
  jq -c '.figures | {straight_share, parallel_rivers_count, river_km, lakes}' $T/$v/hydrology/run-4m/out/report.json
done
```

Keep the variant with fewer hard-check failures; on a tie, the one whose spot crops (`review/spots/` at the three dam spots) show rivers in their own valleys rather than in 32 m-wide strips, as the expert judges. If removing the clamp changes the look visibly either way, show the owner both crops for a Yes/No. Make the winner the chain's default (flip the flag's default in `cascade_chain.py` if the no-clamp variant wins; commit `feat(chain): …`) and record the result as a Plan B departure.

- [ ] **Step 4: Expert review and owner look**

Expert review with: both squares' `level.json`, `review/spots/` crops at E25645 N24400, E25307 N24585, E20071 N30501 and E27169 N44546, the 4 m `review/lake-exits/` sheets and the calibration table. Show the owner the four spot crops (optional look; record comments). Do not start gate 3 until the reviewer reports no blocking problem.

If Task 7's negative controls or these squares left `dam_drops` firings that look like natural waterfalls, ask owner question 3 now with those crops (before gate 4, never first at gate 5).

---
## Task 16: Gate 3 — regenerate 32 m, L3 (raw kept) and 4 m

Spec staging 3. Gate: every hard check on the whole 4 m world, the hand-off checks (`walls` at 35°, `handoff_narrow`), `dam_drops`, the hard coarse guide, and the lake-exit contact sheet.

**Files:**
- Output only: `data/worlds/fujix-55-chain-v3/`, `scratch/hydrology-v3b/gate3/`
- Record: `TASKS.md` (gate note; not committed), progress file

**Interfaces:**
- Consumes: Tasks 1–15. Produces: the v3 chain world that L4 reads.

- [ ] **Step 1: HEAVY — warn the owner**

Tell the owner: the fujix chain from the collage: the 32 m water (minutes), then **L3 over the whole map** (18,796 windows, 6,522 deep-sea skipped, guidance 2 on class 19: about 5–6 h of GPU from the 2026-10-28 per-window rates, ±25 %), then the 4 m water on 268 M cells (two erosion routings and the model runs: 40–90 min of CPU, uncertain until Task 9's timing; Task 9's measured figure goes here), then layers and pictures (about 10 min). Peak RAM about 8 GB (L3) and about 7–9 GB (4 m water with the Python process), under `MemoryMax=12G`; VRAM about 6 GB; disk about 35 GB new under `data/worlds/fujix-55-chain-v3` (free space is checked first). Nothing existing is written. Close other GPU and memory-heavy applications. Wait for the go.

- [ ] **Step 2: Launch**

```bash
RUN=scratch/hydrology-v3b/gate3; mkdir -p $RUN; rm -f $RUN/exit_code
( systemd-run --user --scope -q -p MemoryMax=12G env PYTHONPATH=training/src $P \
    -m topography_training.diffusion.cascade_chain --collage data/worlds/fujix-collage-55 \
    --l3-run data/models/cascade-l3-run-5 --l3-class-runs data/models/cascade-l3-run-5-experts \
    --guidance 2 --guidance-classes 19 --out data/worlds/fujix-55-chain-v3 >> $RUN/log 2>&1; echo $? > $RUN/exit_code ) &
echo $! > $RUN/pid
```

Watch per the heavy-run protocol (`L3 band`, `water: `, `erosion: `, `level `, `Traceback|Error|OOM|Killed|FAILED`), hourly heartbeat with bands done, rate and remaining time. A kill or crash: rerun the same command; it resumes after the last L3 band (the 32 m level and the 4 m level resume from their caches).

- [ ] **Step 3: Read the result**

```bash
W=data/worlds/fujix-55-chain-v3; cat $RUN/exit_code
ls $W/manifest*.json $W/l3-raw.f32
for l in 32m 4m; do jq -c '{passed, failures, guide: .coarse_guide}' $W/hydrology/run-$l/level.json; done
jq '{checks, f: (.figures | {lakes, rivers, seam_lakes: .seam_lakes.count, interior_change_p99_m,
     interior_change_max_m, max_discharge_km2, dam_drops_any: .dam_drops_any.count})}' $W/hydrology/run-4m/out/report.json
jq '.erosion | {routings, lowered_cells, max_lowering_m, sills_lowered, walls_left}' $W/hydrology/run-4m/work/stages.json
ls $W/hydrology/run-4m/review/lake-exits/ | head
```

Gate 3 passes when: exit 0 and `manifest.json` exists; both `level.json` `passed`; every check in the 4 m `report.json` is 0 (straight and parallel runs are figures); the coarse guide has no hard failure; the expert, reading every lake-exit sheet page, sees no dam-like step (a lake held by a rim with a drop right below) and no lake sitting on a slope. On a failure: crop it (`review/failures/`), diagnose on the real cells, fix the code with a test, then rerun **the 4 m level only** from the kept raw L3:

```bash
H=$W/hydrology/run-4m
rm -rf $H/work $H/out $H/review $H/bin $H/progress.json $H/level.json
jq '.hydrology_4m_done = false' $W/progress.json > $W/progress.tmp && mv $W/progress.tmp $W/progress.json
rm -f $W/manifest-failed.json
# then relaunch Step 2's command: it resumes after L3 (l2.npz is kept until manifest.json is written)
```

(Task 11 keeps `l2.npz` while the world is failed; check `ls $W/l2.npz` before deleting anything. A fix to the 32 m stage means rerunning L3, which the owner must agree to again.)

- [ ] **Step 4: Gate note and owner look**

Write in `TASKS.md` "Hydrology v3 Plan B — gate 3": time per stage, peaks (RAM, VRAM), disk; erosion figures; lakes, rivers, seam lakes; interior change p99/max; coarse-guide numbers (compared, small, hard); every check count; and the spots (E25645 N24400, E25307 N24585, E20071 N30501, E43966 N21633, E27169 N44546) at 4 m from `review/spots/`. Show the owner the dam spots and two lake-exit sheet pages (their Yes is not required by the spec at gate 3; record comments).

- [ ] **Step 5: Expert review (gate 3)** — with the reports, `level.json`s, the lake-exit sheets, spot crops and the gate note. Gate 3 passes on all hard checks and no blocking finding. Record the verdict.

---

## Task 17: Gate 4 — the L4 test squares and their 1 m water

Spec staging 4; ruling B19. Gate: every hard check (incl. `dam_drops`, `carve_pits`, the coarse guide against the v3 4 m level) and the owner's Yes on the crops.

**Files:**
- Output only: `data/worlds/fujix-55-l4v3b-{dams,canyon,e43966,alpine}` and `…-water`, `scratch/hydrology-v3b/gate4/`

- [ ] **Step 0: Pick the alpine square (B19, review I8)** — the 4 km block of the gate-3 4 m world holding the most labelled lakes in classes 3, 6 and 10 (fjords, alpine, glaciers), so the 1 m hard checks meet alpine and glacial lake exits before gate 5:

```bash
ALP=$("${CAP6[@]}" "${PY[@]}" -c "
import json, numpy as np
w = 'data/worlds/fujix-55-chain-v3'; n = 16384
body = np.memmap(f'{w}/water-body.u32', '<u4', 'r', shape=(n, n))
cls = np.memmap(f'{w}/classes.u8', np.uint8, 'r', shape=(n, n))
lakes = {b['id'] for b in json.load(open(f'{w}/bodies.json'))['bodies'] if b['kind'] == 'lake'}
k = 1024  # 4 km blocks of 4 m cells
score = np.zeros((n // k, n // k), np.int64)
for by in range(n // k):
    b = np.asarray(body[by * k:(by + 1) * k]); c = np.asarray(cls[by * k:(by + 1) * k])
    for bx in range(n // k):
        bb, cc = b[:, bx * k:(bx + 1) * k], c[:, bx * k:(bx + 1) * k]
        ids = np.unique(bb[np.isin(cc, (3, 6, 10)) & (bb > 1)])
        score[by, bx] = sum(int(i) in lakes for i in ids)
by, bx = np.unravel_index(score.argmax(), score.shape)
print(f'{bx * 4096 + 2048},{by * 4096 + 2048}')")
echo "alpine square centre (x, y from NW): $ALP"
```

Record the centre and its lake count in the progress file.

- [ ] **Step 1: HEAVY — warn the owner**

Tell the owner: four L4 squares over the v3 chain: dams 8 km (about 2,600 windows; 55–100 min of GPU), canyon 4 km with guidance on class 19 (about 650 windows, 25–40 min plus a one-off kernel search of a few minutes), E43966 4 km (15–25 min), alpine 4 km (15–25 min); then the 1 m water on each (CPU, about 6, 2, 2 and 2 min). Total about 1.9–3.2 h. RAM under 10 GB (`MemoryMax=12G`), VRAM about 6–8 GB, disk about 8 GB. Optionally (B32) one more dams square with `--keep-river-residual` (about 1 h) if the L3 A/B in Task 15 showed the river clamp mattered. Wait for the go.

- [ ] **Step 2: Launch the squares, one after the other, then their water**

```bash
RUN=scratch/hydrology-v3b/gate4; mkdir -p $RUN; rm -f $RUN/exit_code
cat > $RUN/run.sh <<'SH'
set -e
P=data/models/diffusion-env-v1/.venv/bin/python
for sq in "dams 8 22858,38086" "canyon 4 27169,20990" "e43966 4 43966,43903" "alpine 4 $ALP"; do set -- $sq
  env PYTHONPATH=training/src $P -m topography_training.diffusion.cascade_l4 --run data/models/cascade-l4-run-8 \
      --class-runs data/models/cascade-l4-run-8-experts --guidance 2 --guidance-classes 19 \
      --world data/worlds/fujix-55-chain-v3 --out data/worlds/fujix-55-l4v3b-$1 --km $2 --centre $3
  env PYTHONPATH=training/src $P -m topography_training.water.run world --source data/worlds/fujix-55-l4v3b-$1 \
      --out data/worlds/fujix-55-l4v3b-$1-water --tile 2048 --threads 4 || true
done
SH
( systemd-run --user --scope -q -p MemoryMax=12G env ALP=$ALP bash $RUN/run.sh >> $RUN/log 2>&1; echo $? > $RUN/exit_code ) &
echo $! > $RUN/pid
```

(`|| true` keeps the next square going when one water world fails its checks; the result is read per square below. A rerun of `run.sh` resumes every step.)

- [ ] **Step 3: Read the results**

```bash
for s in dams canyon e43966 alpine; do W=data/worlds/fujix-55-l4v3b-$s-water
  echo "$s: $(jq -c '{passed: .water.passed, failures: .water.failures,
        guide: (.water.coarse_guide | {hard_failures, lakes_compared, lakes_small})}' $W/manifest*.json)"
  jq -c '.checks | with_entries(select(.value != 0))' $W/hydrology/1m/report.json
  jq -c '.figures | {lakes, rivers, bank: .bank_steps_over_0_5m.count, dam_drops_any: .dam_drops_any.count,
        dam_drops_at_spots: [.dam_drops_at_spots[]? | {cell, count}]}' $W/hydrology/1m/report.json
done
```

Gate 4 hard part: every square's `report.json` has every check 0 and its coarse guide no hard failure. `dam_drops_at_spots` at E25645, E25307, E20071 (and E43966) should now show no step-then-gentle drop (B14's knickpoint rule); compare with Plan A's 71 / 12 / 7 / 48. Bank steps (B27): density = `bank_steps_over_0_5m.count` ÷ the square's land km²; over 24 per km² is a flag for the owner question on the relaxation.

- [ ] **Step 4: Owner Yes/No**

Show the owner, per square, `hydrology/review/spots/` for E25645 N24400, E25307 N24585, E20071 N30501 (dams), E27169 N44546 (canyon: one river on the floor; Q is window-limited here, so the trunk is probably not a main river and the doubled-river test waits for gate 5, review M5) and E43966 N21633, the dam flow view (`review/dam/`) and the alpine square's `review/lake-exits/` pages. If `dam_drops` fired on what looks like a natural waterfall and owner question 3 was not yet answered, ask it here with those crops. Ask Yes/No for the terrain. A No names what is wrong; diagnose, fix, rerun the affected stage (1 m water only, from the kept raw; or the 4 m level and the squares, which needs the owner's go again).

- [ ] **Step 5: Expert review (gate 4)** — reports, crops, the owner's answer. Gate 4 passes on all hard checks, the owner's Yes and no blocking finding. Record the verdict and the gate note in `TASKS.md`.

---

## Task 18: Gate 5 — full L4, 1 m water, game layers and export

Spec staging 5. Gate: every hard check on the whole 1 m world, the export's validation, and the review crops.

**Files:**
- Output only: `data/worlds/fujix-55-l4v3b-raw`, `data/worlds/fujix-55-l4v3b`, `data/exports/fujix-55-l4v3b`, `scratch/hydrology-v3b/gate5/`
- Modify: `docs/workflows/terrain-cascade.md` (measured cost), the spec's "Plan B departures" (numbers); commit both

- [ ] **Step 1: HEAVY — warn the owner (L4)**

Tell the owner: the full L4 diffusion over the v3 chain: 173,910 windows, 72,597 deep-sea skipped, 16,136 guided: **35–41 h of GPU** (the 2026-10-28 rates; ±15 %; no band hydrology any more), RAM about 9.4 GB under `MemoryMax=12G`, VRAM about 6–8 GB, disk about 26 GB. Hourly progress reports. Resumable after a crash or reboot. Then, separately, the 1 m water (Step 4). Wait for the go.

- [ ] **Step 2: Launch L4**

```bash
RUN=scratch/hydrology-v3b/gate5; mkdir -p $RUN; rm -f $RUN/l4.exit
( systemd-run --user --scope -q -p MemoryMax=12G env PYTHONPATH=training/src $P \
    -m topography_training.diffusion.cascade_l4 --run data/models/cascade-l4-run-8 \
    --class-runs data/models/cascade-l4-run-8-experts --guidance 2 --guidance-classes 19 \
    --world data/worlds/fujix-55-chain-v3 --out data/worlds/fujix-55-l4v3b-raw >> $RUN/l4.log 2>&1; echo $? > $RUN/l4.exit ) &
echo $! > $RUN/l4.pid
```

Watch: Monitor on `band [0-9]+/[0-9]+|Traceback|Error|OOM|Killed|nan|FAILED`, hourly heartbeat (`timeout 3600 tail --pid=$(cat $RUN/l4.pid) -f /dev/null`) reporting bands done, s per band and hours left. On a crash, relaunch the same command.

- [ ] **Step 3: Disk check before the water**

```bash
df -B1 --output=avail /home | tail -n 1
```

The 1 m water needs about 290 GB (work 232 GB at 54 B a cell, published layers about 55 GB) plus the driver's 10 GB spare. If less is free, stop and ask the owner (owner question 1: may `data/worlds/fujix-55-l4v3-water/hydrology/work`, 196 GB of recomputable CPU caches, be deleted?). Never delete without that answer.

- [ ] **Step 4: HEAVY — warn the owner (1 m water), then launch**

Tell the owner: the whole 1 m water world (4.3 G cells) in a 10 GB cap: about 4.9 h of CPU from fresh (gate 2's measured 17,568 s; ±20 %), Rust about 4.4 GB and the driver about 8 GB peak, disk as Step 3. Wait for the go.

```bash
( systemd-run --user --scope -q -p MemoryMax=10G env PYTHONPATH=training/src $P -m topography_training.water.run world \
    --source data/worlds/fujix-55-l4v3b-raw --out data/worlds/fujix-55-l4v3b --tile 2048 --threads 4 \
    >> $RUN/water.log 2>&1; echo $? > $RUN/water.exit ) &
echo $! > $RUN/water.pid
```

Watch per the heavy-run protocol (`water: |erosion: |review: |done|Traceback|Error|OOM|Killed|FAILED`), hourly heartbeat.

- [ ] **Step 5: Read the result**

```bash
W=data/worlds/fujix-55-l4v3b; cat $RUN/water.exit
jq '{passed: .water.passed, failures: .water.failures, guide: (.water.coarse_guide | {hard_failures, lakes_compared, lakes_small})}' $W/manifest*.json
jq -c '.checks | with_entries(select(.value != 0))' $W/hydrology/1m/report.json
jq '.figures | {lakes, rivers, river_km, sea_km2, label_agreement, max_discharge_km2,
     bank: .bank_steps_over_0_5m.count, dam_drops_any: .dam_drops_any.count, dam_drops_at_spots: [.dam_drops_at_spots[]? | {cell, count}],
     straight_share, parallel_rivers_count}' $W/hydrology/1m/report.json
jq '[.bodies[] | select(.kind == "lake" and .level_m < 0)] | length' $W/bodies.json
```

Exit 0 and every check 0: go on. Exit 1: list each failed check with its examples and crops; a failure is a bug unless it is a terrain call the rulings send to the owner (B14 natural cascades); fix with a test and rerun from the stage the fix touches (docs "Rerunning from a stage"; the raw surface is kept, no GPU rerun).

- [ ] **Step 6: Game layers and export** (CPU about 55 min and about 1 h; covered by the owner's go for gate 5 — announce)

```bash
( systemd-run --user --scope -q -p MemoryMax=10G env PYTHONPATH=training/src $P \
    -m topography_training.diffusion.game_layers --world data/worlds/fujix-55-l4v3b --layout data/worlds/fujix-collage-55 \
    >> $RUN/game.log 2>&1; echo $? > $RUN/game.exit ) &
echo $! > $RUN/game.pid
# when game.exit is 0:
( systemd-run --user --scope -q -p MemoryMax=10G target/release/topography export heightmap \
    data/worlds/fujix-55-l4v3b data/exports/fujix-55-l4v3b >> $RUN/export.log 2>&1; echo $? > $RUN/export.exit ) &
echo $! > $RUN/export.pid
du -sh data/exports/fujix-55-l4v3b
```

Expected: both exit 0; the export's validation passes on every check; its size is under 10 GB (Plan A's was 3.5 GB).

- [ ] **Step 7: Gate note, docs and owner look**

Gate note in `TASKS.md` ("Hydrology v3 Plan B — gate 5"): every stage's time, peaks and disk; bodies (lakes, rivers, sea, ponds) against the 50,000 budget; coarse guide; below-sea lakes (expected none outside hot-dry, B13); `dam_drops_at_spots` against Plan A's 71 / 12 / 7 / 48 / 25; bank-step density; export size. Update `docs/workflows/terrain-cascade.md`'s cost table with the measured numbers and the spec's "Plan B departures" with them; commit:

```bash
git add docs/workflows/terrain-cascade.md docs/superpowers/specs/2026-11-05-hydrology-v3-design.md
git commit -m "docs(water): measured cost of the v3 regeneration and the Plan B departures

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

Show the owner the eleven spot crops (`$W/hydrology/review/spots/`) and the overview, and ask Yes/No on the terrain.

- [ ] **Step 8: Expert review (gate 5)** — the report, guide, crops, gate note and export log. Gate 5 passes on every hard check, a passing export, no blocking finding and the owner's Yes. Then hand the branch to the owner (superpowers:finishing-a-development-branch); merging is theirs.

---

## Self-review

**Spec coverage** (spec section → task):

| Spec item | Task |
| --- | --- |
| Ground never raised; lowered only by erosion (32 m, 4 m), breaching, carving | 5 (`v.min(zc)`, lowering-only diffusion and taper), 6 (test: nothing rises), 7 (`change_max`, walls) |
| Erosion: stream power + hillslope diffusion, fixed budget per level, stated K·dt | 2 (`[levels.N.erosion]`), 5, 6, B2, B6 |
| Lowering cap per cell (30 m / 10 m), breach gorges excepted | 2, 5 (floor), 6 (floor = breached − cap), B4 |
| Lake-aware routing, sills incise, outlets grade | 5 (`stream_power` sills and levels), 6, B1 |
| Solver chosen by benchmark | 9 (decision rule), 9b (contingency), B1 |
| Provenance layer from the collage build | 3 (`surface`, `provenance`, sidecar), B10 |
| Real interiors low K + breach-or-fill; blends, painted, fills full K | 2 (`interior_k`), 6 (kinds → k, floor), B3 |
| Hand-off: ground only; no carved channel < 3 cells, no cut > 35° | 5 (taper, wall rounds), 7 (`handoff_narrow`, walls 35°), B7–B9 |
| Outlets stay smooth valleys several cells wide | 5 (taper ≥ 3 cells), 15 and 16 (lake-exit sheets) |
| Raw L3 kept, 4 m hydrology redoable without L3 | 11 (grid links `l3-raw.f32`), 16 (rerun the 4 m level only) |
| Change budget at 32 m and 4 m for real interiors | 7 (`interior_p99`, `interior_max`) |
| Coarse guide: hard on gross mismatch | 10 (hard in the runner and the world driver), B15 |
| Contact sheet of every 4 m lake exit | 10 (review in `run_level`), 15, 16 |
| Dams hard (R31 `dam_drops_any`) | 7 (`dam_drops` knickpoint rule; positive and negative controls on Plan A's raw at the four dam spots), B14 |
| Change budget only on interiors at coarse levels; `change_max` with both caps | 2 (`change_max_m`), 7 (`change_p99` a figure at coarse levels), B4 |
| Coarse water conditioning of L3/L4 (not named by the spec) | 11 (`--no-l3-river-clamp`), 12 (`--keep-river-residual`), 15 Step 3b (A/B), B32 |
| No pits made by erosion; diffusion only near cuts | 5 (bed grade, footprint), 6 (figures), 7 (`erosion_new_pits`), B6, B33 |
| Resume of the long L4 run | 12 Step 4b (kill and resume reproduces the raw sha) |
| Below sea level (lagoons) and the owner's ruling | 4 (coastal water table, `lake_below_sea`), B13 |
| Moisture from the 32 m Smith–Barstad stage | 1 (`world rain`, byte identity), B12 |
| Outputs in today's formats | 10 (finish publishes moisture too), 11 (world layers unchanged), 12 |
| No provenance scaling at 1 m | B11 (1 m reads only the flags), 6 (no `e` stage at 1 m) |
| Removed: v2 module, share budgets, slots, lips, imposition, seams, `widen_trenches`; reusable parts moved; `--hydrology-only` replaced | 1 (grid, moisture moved), 12 (band driver gone), 14 (module, CLI, table, wrappers deleted) |
| Staging 3, 4, 5 | 16, 17, 18 |
| Every stage keeps its raw output and is resumable; long-job rules | 6 (routings cached), 10 (`run_level` resumes), 11, 12, Heavy runs protocol, 16–18 |
| Expert review of each task's diff with real-terrain output | every task's last step |
| Parked items: comb-cut pits, coarse-guide mismatches, lakes without outflow, bank steps, lake 22 | 13 (B26), B28, 8 (B16), B27 and 17 Step 3, 4 and 18 Step 5 (B13) |

**Review items** (`.superpowers/sdd/2026-11-07-hydrology-v3-b/plan-review.md`): C1 → B14, Task 7 (rule, tests, Step 4b controls), Task 9/15 readouts; C2 → B9, Task 7 (criterion, V and disc-trench tests); C3 → B4, Task 2 (`change_max_m`), Task 7 (`change_p99` figure); I1 → B32, Tasks 11, 12, 15 Step 3b, 17 Step 1; I2 → B33, Task 5 (bed grade, test), Task 6 (`new_pits`), Task 7 (`erosion_new_pits`); I3 → B6, Task 5 (`footprint`), Task 6 (figures), Task 9 readout; I4 → B13, Task 4 (per-basin floor, mixed-basin test); I5 → B15, Task 10 (size, depth, edge rules and tests); I6 → B10, Task 3 (agreement tolerances, stored-l2 flags); I7 → Task 12 Step 4b; I8 → B19, Task 17 Step 0. M1–M9 → B34 (none declined).

**Placeholder scan:** every code step carries code. Two steps cannot carry their final code by nature and say so: Task 13's fix (a diagnosis task, with the reproduction, the hard check, the test and the two candidate fixes stated) and Task 9b (a contingency whose Rust side is described by its file protocol).

**Type and name consistency:** `Erosion`, `Level` (with `wall_deg`, `erosion`), `Params.wall_deg/erosion`, `is_wall(slope, raw, wall_deg)` (Task 2) are used verbatim in Tasks 5–7. `Setup.provenance`, `Setup::flags`, `Setup::below_sea(core, ground)`, `balance::coastal_floor`, `BELOW_SEA_OK`, `STEEP`, `KIND_MASK`, `INTERIOR` (Task 4) are used in Tasks 6 and 7. `Bits`, `Sill`, `Routing`, `Figures` (with `new_pits`, `hillslope_*`), `receiver`, `stream_power`, `footprint`, `diffuse(r, z, alpha, near)`, `taper`, `new_pits`, `erode` (Tasks 5–6) match. `pre_channel`, `CheckLayers.pre_channel` and `.breached` (Task 6) are what Task 7 reads. `grids.write_grid/add_moisture/provenance_window`, `run.run_level`, `level.json` keys (Task 10) are what Tasks 11, 12, 15 and 16 read. The provenance bits (Task 3 Python, Task 4 Rust) are the same values (1 interior, 2–4 blend/painted/fill, 64, 128).

**Known risks to watch:**
- The 4 m erosion's memory (about 17–21 B per cell plus the 8 B sort keys) is measured in Task 9; if it fails the rule, Task 9b's GPU path adds plumbing.
- `order_ties` > 0 would mean R12's strict dry receivers do not hold on eroded coarse ground; Task 9 Step 6 checks it.
- The coarse guide's 5 m rule between 32 m and 4 m depends on L3 keeping lake rims; Task 15 measures it on real L3 before the long run.
- `dam_drops`' knickpoint factor 3 is calibrated on Task 7's controls; a sheer natural waterfall right at an outlet still fails and goes to the owner before gate 4 (owner question 3), never to a code exemption.
- The per-basin coastal floor (B13) adds two integer sums to the hierarchy's supply plumbing; Task 4's tiled-exactness tests must cover them.
- Disk: the 1 m water's work directory is the largest single cost (B21); gate 5 checks free space first.
