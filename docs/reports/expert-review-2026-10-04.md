# Expert review, 2026-10-04

Two read-only reviews of process, results and areas for improvement, made after
L1 run 4 and L2 run 5 and before L3 run 2:

- **Machine learning:** Claude Opus 5.5.
- **Geography and geomorphology:** Codex `gpt-6-astra`.

Only the most important points are kept. File references are to commit 366e2c0.

## Main findings

### 1. The class recipe trains a mode the chain never uses (ML)

`cascade_chain._sample` always passes the coarse input and uses no guidance. Class
learned with the coarse input hidden therefore only counts if it carries over to the
coarse-shown branch, and L2 run 5 shows it does not: with the coarse input shown,
roughness is 8.5–8.8 m for every class.

The levels should have different jobs:

- **L1** (painted input) must set the class amplitude.
- **L2–L4** should add class form (spectrum, anisotropy, steps) on a coarse surface
  that already matches the class.

Before training more, measure on real data how much class explains once coarse local
relief is known. That is the most any shown-coarse gate can expect.

### 2. Fixes to try, cheapest first (ML)

1. **Borrow the class direction from the hidden branch; no retraining needed.** Sample
   with `D(c,k) + w·[D(c_hidden,k) − D(c_hidden,∅)]` on L2 run 5 and L1 run 4. This
   needs a 3-call extension of `denoise_field` (`model.py:204`).
2. **Use FiLM-style class conditioning.** Today the class is mean-pooled into a bias on
   the time embedding (`model.py:61`, diffusers `resnet_time_scale_shift="default"`);
   switch to `"scale_shift"` (AdaGN).
3. **Replace binary coarse dropout with conditioning augmentation** (Ho et al. 2021).
   Blur or add noise to the coarse input at a random strength, and give the model that
   strength as an input. This matches training to the over-smooth painted and upstream
   inputs. Replace the all-zero "hidden" sentinel (`model.py:167`) with an explicit
   flag, so a flat 0 m painted window is not read as hidden.
4. **Add an explicit roughness channel at L2–L4**, like L1's relief channel. In
   training, take it from the truth (with dropout); at generation, from a per-class
   table or the painter.
5. **Leave per-class fine-tunes (`ClassMixture`) as the last resort.** They memorise
   5–6 regions, and running up to 13 models per window is unaffordable at L3/L4 scale.
   Prefer per-class adapters or LoRA if this is needed.

### 3. Runs are undertrained (ML)

Every run stops at about 6k steps with validation loss still falling. The model is
23.6M parameters and uses 5.8 of 16 GB VRAM. Profile the synchronous batch build
(`train.py:256`), try a larger batch without gradient checkpointing, and run longer.
The compressed per-class roughness range fits underfitting.

### 4. Gates are noisy and leak (ML and geography)

- **Noise:**
  - The training-time roughness rank comes from 1 window × 1 seed × 8 steps, and it
    swings between 0.42 and 0.80 across checkpoints. Early stopping on it is
    unreliable.
  - Use at least 8 windows × 2 seeds, the production step count, and bootstrap
    confidence intervals over regions.
- **Leakage:**
  - `class_gates` pools training and validation windows (`class_gates.py:147`).
  - In context-v3, `waimakariri-upper` (validation, braided rivers) overlaps
    `rakaia-upper` (training) by about 16%.
  - Validation selection hashes region IDs with no geographic grouping
    (`release.rs:82`). Split by landform or survey cluster with spatial buffers.

### 5. Roughness is not landform identity (geography)

Recognition still fails at L1 and L2. Painted Fujion reads as contrasting relief, not
as twelve terrain families.

- **Judge the classes by scale:**
  - basins and islands at 128 m
  - valley and ridge systems at 32 m
  - local forms at 4 m
  - surface texture at 1 m

  A 1 km L3 window cannot show a whole caldera or fjord.
- **Labels are region-wide** (`cascade.py:504`). Valley walls around braided rivers
  and hills around karst count as positive examples. Use cover layers or tighter
  footprints for feature-focused sampling.
- **Diversity is thinner than the counts suggest:**
  - canyons, badlands and dunes are mostly from the western US
  - archipelago and braided rivers are mostly from New Zealand

### 6. Hydrology breaks real geomorphology (geography)

- **Streams fail to connect:**
  - Stream thresholds are applied per cell, so channels vanish across class borders
    (`hydrology.py:106`).
  - Lakes are terminal sinks with no spill outlet.
- **Carving erases legitimate basins:** every depression beyond the lake allowance is
  carved open (`hydrology.py:199`). This loses karst dolines, interdune basins and
  craters.
- **Braided rivers cannot form:** single-parent flow routing cannot branch and rejoin.
- **Water is not enforced through refinement:**
  - River profiles and lake IDs are not enforced after L2.
  - Lake-level propagation takes the maximum nearby level (`cascade_chain.py:90`).

### 7. Data mismatches (geography)

- **Source mismatch at L3:** L3 trains on a coarse surface averaged from bare-earth
  lidar, but in the chain it receives L2 output learned from GLO-30, a surface model
  that includes trees and buildings. Evaluate L3 on real generated L2 input, not only
  on its own averaged targets.
- **Averaging depends on the supplier:** lidar is averaged with `-ovr AUTO-1`
  (`lidar.rs:105`), so the result depends on how each supplier built its overviews.
  Spot-check cliff edges and dune crests against a native-resolution average.
- **The seabed join oversteepens shallow coasts:** it suits fjords but overstates
  slopes at beaches and shallow shelves. Exclude extrapolated seabed from land class
  statistics.

## Implications for the pending L3 run 2

1. **Use lidar-v2, not lidar-v1.**
   - lidar-v1 has 1 training region for archipelago and for braided rivers, and no
     validation region for 7 classes. Balanced sampling would draw 1/12 of all
     windows from a single region, and recognition cannot be computed.
   - lidar-v2 has 80 regions, with 5–6 training and 1 validation region per class,
     and its fields are already cached.
   - The data table in `class-conditioning-v1.md` is stale.
2. **Qualify lidar-v2 first.**
   - 44 of its 80 regions have no owner vote, including 6 of the 12 validation
     regions: bisti, cape-kidnappers, eureka-dunes, matukituki, palisades,
     punakaiki.
   - The braided-river validation region (matukituki) has only about 9k valid cells,
     not enough for one 256² crop. Replace it.
3. **Fix the gates before launching.** Use validation windows only and at least
   8 windows × 2 seeds.
4. **Lower the dropout and use a fresh validation set.**
   - L3 always receives L2's coarse output in the chain, so 50% dropout mostly halves
     useful training. Prefer 25%, plus `scale_shift` if it lands first.
   - Do not `--init-from` run 1, and do not compare run 1 on the lidar-v2 validation
     set: three of its validation regions were run 1 training regions.
   - Budget 4–6 h; loss was still falling at 6k steps in earlier runs.
5. **Check memory early in the run.** The ML review read 366e2c0 as correct:
   memory-mapped fields are read-only, and the cache key and atomic write are sound.
   The lidar-v2 field build peaked at 4.96 GB RSS. Remaining risks:
   - The training loader's peak memory is unmeasured; check it early in the run.
   - Changes to `refine_fields`, `soft_u8` or `join_seabed` need a manual
     `FIELDS_VERSION` bump.
6. **Success criteria:**
   - On validation windows only: roughness rank ≥ 0.7 with the coarse input hidden,
     and recognition ≥ 70% of the real classifier accuracy.
   - A Fujion chain test (L1 run 4 → L2 run 5 → L3): per-class residual roughness
     rank-correlates ≥ 0.7 with real and falls within about ±30% of it.
   - No regression in slope, curvature or memorisation distance, and the owner's
     blind review and class grid.
   - Report shown-coarse swap correlation, but do not gate on it.

**Verdict:**

- **No-go on lidar-v1.**
- **Go on lidar-v2** after the validation-region votes and the gate fixes.
- **Run the guidance experiment first** (finding 2, item 1). It needs no training and
  may change the dropout choice.
