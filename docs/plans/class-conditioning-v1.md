# Class conditioning v1

Status: in progress; next steps revised after the expert review of 2026-10-04.
Retrain every cascade level (L1–L4) so that terrain class
changes what the models generate. Supersedes the class handling in the
[terrain cascade plan](terrain-cascade-v1.md); its levels, data and water handling stay.

## What went wrong

All four cascade models ignore the terrain class. Measured on 2026-10-03 with the
class-swap test below (same coarse surface, same seed, only the class changed):

| Level | Run | Correlation between classes | Same class, other seed |
| --- | --- | --- | --- |
| L1 128 m | `cascade-l1-run-3` | 0.99 | 0.33 |
| L2 32 m | `cascade-l2-run-2` | 1.00 | 0.67 |
| L3 4 m | `cascade-l3-run-1` | 1.00 | 0.50 |
| L4 1 m | `cascade-l4-run-5` | 1.00 | 0.59 |

The seed changes the output more than switching between karst, canyon, alpine and
dunes. On the painted Fujion world every biome, the volcano and the seabed came out
with the same crumpled texture.

Every model passed its blind review (L1 7/7, L2 32/32, L3 32/32, L4 32/32) and none of
the reviews could catch this. Each review generated from the real coarse surface of
a held-out region. That surface already carries the region's character, so the
model only continued it. Training rewarded the same shortcut: within one region the
class is constant and the coarse surface predicts the residual, so the one-hot class
channels added nothing to the loss. With 3–7 regions per class the class was little
more than a region label the coarse surface already implied.

Coarse elevation and terrain class are the two most important inputs at every
level. A model that ignores either does not do its job, whatever its loss or review.

## Requirements

1. At every level, changing the class changes the output in that class's direction:
   generated terrain is recognised as the class it was asked for.
2. Class works per cell, so painted layouts with several classes and class borders
   generate each class in place and blend at the border.
3. The coarse input still sets position and large shape. Class sets the character
   the coarse input does not determine: dune ridges, mesa steps, karst towers,
   gullies, cirques, crater rims.
4. The other conditioning inputs (water, snow/ice, vegetation, landmarks, relief)
   are tested the same way. An input that has not been shown to change the output
   counts as ignored.

## Design

To be confirmed by a pilot (step 3) before full runs.

- **A class signal in every block.** Keep the per-cell one-hot channels, and add a
  learned class embedding that modulates every resolution of the U-Net. The
  embedding of each window is its cells' class shares weighted over learned class
  vectors, added to the timestep embedding, so mixed windows mix their classes.
  If the pilot shows the global embedding is not enough at class borders, move to
  per-cell modulation (SPADE-style scale and shift from the class map at each
  resolution).
- **Make the class necessary.** Raise coarse dropout to about 25% at L2–L4 and add
  envelope dropout at L1, so a share of windows must be generated from class alone.
  Keep class dropout at 10% for guidance. Sample training windows class-balanced by
  independent region, not by area.
- **Mixed-class windows.** Add training windows that join two regions of different
  classes along a soft random border, with the coarse input and the class map joined
  the same way, so the model sees class borders it will meet in painted layouts.
- **Guidance at generation.** Classifier-free guidance on the class (scale above 1
  where the swap test shows a weak class effect), using the class-dropout training.

## Gates

Each run is checked automatically before any owner review. A failed gate stops the
level; it is not overridden by a good blind review.

1. **Class-swap test.** Fixed coarse windows from invented and real inputs; generate
   every class with the same seed. The mean correlation between classes must be
   below the same-class, other-seed correlation, and the residual statistics (slope
   distribution, curvature, relief spectrum) must differ by class.
2. **Class recognition.** A simple terrain classifier trained on real held-out
   patches of each level (slope histogram, curvature, spectrum and relief features)
   must recognise generated patches as their requested class at no less than about
   70% of its accuracy on real patches. Report a confusion matrix per level.
3. **Invented inputs.** Blind and visual reviews include generations from painted or
   invented coarse inputs, not only real held-out coarse surfaces, and a class grid:
   one input surface rendered in several classes side by side, for an owner Yes/No
   on whether each looks like its type.
4. **The other inputs.** The swap test is repeated for water, snow/ice, vegetation
   and landmark channels where the level has them.

Baseline of the current runs (`class_gates`, 2026-10-03; recognition is the share of
generated windows recognised as their requested class, against the classifier's
leave-one-region-out accuracy on real windows):

| Level | Between classes | Other seed | Real | Swapped | Own | Result |
| --- | --- | --- | --- | --- | --- | --- |
| L1 run 3 | 0.99 | 0.43 | 0.42 | 0.19 | 0.42 | fail |
| L2 run 2 | 1.00 | 0.69 | 0.24 | 0.00 | 0.42 | fail |
| L3 run 1 | 1.00 | 0.71 | 0.17 | 0.08 | 0.35 | fail |
| L4 run 5 | 1.00 | 0.71 | 0.32 | 0.12 | 0.38 | fail |

The classifier's real accuracy (chance is 0.08) is low with 3–7 regions per class; it
must rise with the new data before the recognition gate means much.

### L2 pilot (2026-10-04)

- Run 3 hid shape and slopes but left absolute height, which carries the whole coarse
  surface; it learned no class (swap 0.98 at step 1,000). Coarse dropout now hides all
  four channels.
- Run 4 (class embedding, balanced and mixed windows, 25% coarse dropout): swap 0.97–0.98
  through step 2,000. The true class lowered the denoising error by only 1–2% (L1 run 3:
  3–5%), and class guidance up to 6 barely changed samples.
- Root cause: real residual roughness differs almost 3x by class (dunes 4 m, fjords 11 m
  at L2), but generated roughness is the same 10.4 m for every class with the coarse
  input shown. In real data the coarse surface always reveals the class, so the model
  reads style from it; a painted layout's smooth coarse surface carries none. With the
  coarse input hidden, run 4 had not yet learned per-class roughness.
- Correlation is blind to amplitude, so a roughness gate was added: generated per-class
  residual roughness must follow the real per-class roughness (rank correlation 0.7),
  with the coarse input shown and hidden; training logs it at every validation.
- Next: coarse dropout 50% and longer training. If class style still does not appear,
  fine-tune one model per class, which cannot ignore its class by construction.

### L2 run 5 (2026-10-04)

50% coarse dropout, 3 hours (5,946 steps). A sampling bug hid the result at first: the
sampler normalised a hidden (all-zero) shape channel to -1, while training kept it at 0,
so every "coarse hidden" measurement, including the training-time roughness rank, ran on
inputs the model never saw. Fixed in `model._shape`. With the fix:

| Class | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Real roughness (m) | 10.7 | 9.5 | 10.7 | 5.9 | 7.4 | 11.0 | 4.9 | 4.4 | 9.3 | 10.1 | 4.5 | 5.6 |
| Coarse hidden | 6.1 | 6.1 | 5.5 | 4.3 | 4.7 | 6.7 | 4.4 | 4.6 | 6.4 | 5.8 | 3.5 | 3.1 |
| Coarse shown | 8.8 | 8.6 | 8.5 | 8.5 | 8.5 | 8.5 | 8.8 | 8.7 | 8.5 | 8.6 | 8.6 | 8.5 |

With the coarse input hidden the class orders roughness correctly (rank 0.76) but with a
compressed range; with it shown the model still follows the coarse input. Swap 1.00 and
recognition 0.10 still fail. Next: L1 and L3 with the same recipe, then the whole
cascade on the painted Fujion layout, where L1 sees a featureless envelope. Per-class
fine-tunes (`--only-class`, `--init-from`, `ClassMixture`) are ready as the fallback.

### L1 run 4 (2026-10-04)

Same recipe at L1 (envelope dropout hides mid surface and relief), 3 hours (5,717 steps).
Roughness gate passes with the envelope shown and hidden (rank 0.91 both); generated
fine relief follows the real per-class order with a compressed range (108–203 m against
42–307 m). Recognition 0.19 (gate 0.25). The swap gate (0.98 between classes, 0.10 for
another seed) is the wrong test at L1: a whole-field L1 changes completely with the seed,
and the class shows as amplitude, which correlation ignores; roughness is the measure.
On the painted Fujion envelope the old L1 gave every class about 150 m of fine relief
(2–6x real); run 4 gives 37–66 m, ordered by class. Fujion L1–L2 with L1 run 4 and L2
run 5: rugged fjord and alpine coasts, a smooth cone, gentle lee plateau
(`data/worlds/fujion-painted-chain-4`).

Checkpoints are tested during training (at each validation step), so a run that is
not learning the class is stopped early instead of finishing its full budget.

## Data

Releases `context-v3` (L1/L2) and `lidar-v2` (L3) exist and meet the per-class
counts; their owner votes are not complete (2026-10-04):

| Class | context-v3 train / val | lidar-v2 train / val | L4 1 m tiles |
| --- | --- | --- | --- |
| 1 Karst | 9 / 2 | 6 / 1 | 7 |
| 2 Gorges and canyons | 9 / 2 | 6 / 1 | 15 |
| 3 Fjords | 9 / 2 | 5 / 1 | 12 |
| 4 Calderas and craters | 9 / 2 | 5 / 1 | 14 |
| 5 Archipelago | 9 / 2 | 5 / 1 | 10 |
| 6 Alpine | 8 / 2 | 6 / 1 | 14 |
| 7 Badlands and mesas | 9 / 2 | 6 / 1 | 9 |
| 8 Dunes | 8 / 2 | 6 / 1 | 10 |
| 9 Braided rivers | 8 / 2 | 5 / 1 | 7 |
| 10 Glaciers | 9 / 2 | 6 / 1 | 10 |
| 11 Cones and lava | 9 / 2 | 6 / 1 | 16 |
| 12 Coastal cliffs | 8 / 2 | 6 / 1 | 8 |

- `context-v3`: 64 of its regions are unreviewed, including 11 validation regions.
  Validation `waimakariri-upper` overlaps training `rakaia-upper` (about 16% of the
  region); fix this in the next release.
- `lidar-v2`: 44 of 80 regions are unreviewed, including 6 of the 12 validation
  regions: bisti, cape-kidnappers, eureka-dunes, matukituki, palisades and
  punakaiki. Matukituki (braided rivers) has about 9k valid cells, too few for one
  256² crop, and must be replaced. Do not train L3 on `lidar-v1`: it has 1 training
  region for some classes and no validation region for 7 classes.

Targets per class: L1/L2 at least 8 training and 2 validation GLO-30 regions; L3 at
least 5 training and 1 validation lidar regions; L4 at least 15 tiles from at least 4
independent areas, 1 held out. More public data is acquired where a class falls short,
through the existing catalog plans and the gallery Yes/No.

## Expert review (2026-10-04)

The [expert review](../reports/expert-review-2026-10-04.md) changes the next steps:

- **The recipe trains a mode the chain never uses.** The chain always passes the coarse
  input and uses no guidance (`cascade_chain._sample`). With the coarse input shown,
  L2 run 5 gives every class the same roughness, so the class only works when the
  coarse input is hidden, which never happens in a real world.
- **Split the jobs by level.** L1 (painted input) sets each class's amplitude. L2–L4
  add each class's form (spectrum, anisotropy, steps, crests) on a coarse surface that
  already matches the class.
- **Shown-coarse swap correlation is reported, not gated.** Roughness and recognition
  are the gates.
- **The gates are noisy and leak.** They mix training and validation windows
  (`class_gates.py:147`). The training-time roughness rank (1 window × 1 seed) swings
  0.42–0.80 between checkpoints, so early stopping on it is unreliable.
- **Runs are undertrained.** Validation loss was still falling at about 6k steps, and
  VRAM use is 5.8 of 16 GB.
- **Region-wide class labels.** Every window takes its region's class, so background
  ground around the feature also counts as that class (`cascade.py:504`).

## Steps

1. **Instrument.** Done: the gates and class grid; failing baseline recorded.
2. **Data.** Done: `context-v3` and `lidar-v2` released. Owner votes are open (see
   Data).
3. **Pilot at L2.** Done: runs 3–5, and L1 run 4. Class orders roughness only with the
   coarse input hidden.
4. **Next, in order:**
   1. **Fix the gates** (`class_gates.py`, `train.py`):
      - Use validation windows only.
      - Use at least 8 windows × 2 seeds at the production step count, with bootstrap
         confidence intervals over regions.
      - Make the training-time roughness rank use several windows and seeds.
   2. **Guidance experiment (no training).** Sample with
      `D(c,k) + w·[D(c_hidden,k) − D(c_hidden,∅)]`: the class direction learned with
      the coarse input hidden, added while it is shown. This needs a 3-call
      extension of `denoise_field` (`model.py:204`). Measure the shown-coarse
      roughness gate on L2 run 5 and L1 run 4 for a few `w`, and run Fujion L1–L2
      with the best `w`. Also try class guidance with the coarse input hidden in both
      terms, to widen the compressed range.
   3. **Owner votes on the lidar-v2 validation regions:**
      - Vote on the 6 unreviewed validation regions.
      - Replace matukituki.
      - Rebuild the release, keeping its geographic split.
   4. **Model changes before the L3 run:**
      - Switch class conditioning to `resnet_time_scale_shift="scale_shift"`
        (AdaGN/FiLM).
      - Replace the all-zero "hidden" sentinel (`model.py:167`) with an explicit
        flag.
      - If the guidance experiment is not enough, replace binary coarse dropout with
        conditioning augmentation (Ho et al. 2021): blur and noise the coarse input
        at a random strength, and give the strength as an input.
   5. **L3 run 2** (see below).
   6. **Retrain L2, then L1 and L4,** with whatever passed, using longer runs and a
      larger batch where VRAM allows. Then regenerate Fujion and review L2, L3,
      then L4.
5. **Fallbacks**, in order:
   - **Explicit roughness channel at L2–L4.** From the truth in training (with
     dropout); from a per-class table or the painter at generation.
   - **Per-cell class modulation (SPADE)** for class borders.
   - **Per-class adapters or LoRA.** `ClassMixture` with full per-class models is
     the last resort: it memorises 5–6 regions and runs up to 13 models per window,
     which is unaffordable at L3/L4 world scale.

### L3 run 2

- **Data and model:**
  - `lidar-v2` after step 4.3. Fields are cached at
    `data/cache/cascade-fields/lidar-v2`; the build peaked at 4.96 GB RSS.
  - Fresh weights: no `--init-from` run 1.
  - Class embedding with `scale_shift`, balanced and mixed windows.
- **Coarse handling:** 25% coarse dropout (L3 always receives L2's coarse output in
  the chain), or conditioning augmentation if step 4.4 adopted it.
- **Budget:** 4–6 h. Warn the owner first; check host memory in the first minutes.
- **Validation regions:** do not compare run 1 on the `lidar-v2` validation set.
  glacier-np, glacier-peak and marlborough-sounds were run 1 training regions.
- **Cache key:** a change to `refine_fields`, `soft_u8` or `join_seabed` needs a
  `FIELDS_VERSION` bump (`cascade.py:138`), because the cache key does not cover them.
- **Success criteria:**
  - On validation windows only: roughness rank ≥ 0.7 with the coarse input hidden,
    and recognition ≥ 70% of the real classifier's accuracy.
  - Fujion chain (L1 run 4 → L2 run 5 → L3): per-class residual roughness and
    spectral slope rank-correlate ≥ 0.7 with real, and roughness falls within about
    ±30% of real.
  - L3 evaluated on generated L2 input, not only on averaged lidar: L2 learned from
    GLO-30, which includes trees and buildings, while L3 trains on bare-earth lidar.
  - No regression in slope, curvature, cross-window correlation or
    nearest-training-window distance; blind review and class-grid Yes/No.

Warn the owner before each training run (expected time, VRAM, disk), as for all
heavy work.
