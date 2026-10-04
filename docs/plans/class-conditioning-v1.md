# Class conditioning v1

Status: planned 2026-10-03. Retrain every cascade level (L1–L4) so that terrain class
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

Current independent training regions per class (train / validation):

| Class | L1/L2 GLO-30 | L3 lidar | L4 1 m tiles |
| --- | --- | --- | --- |
| 1 Karst | 5 / 1 | 2 / 0 | 7 |
| 2 Gorges and canyons | 5 / 1 | 3 / 1 | 15 |
| 3 Fjords | 7 / 1 | 3 / 0 | 12 |
| 4 Calderas and craters | 5 / 1 | 3 / 1 | 14 |
| 5 Archipelago | 3 / 0 | 1 / 0 | 10 |
| 6 Alpine | 5 / 1 | 3 / 1 | 14 |
| 7 Badlands and mesas | 5 / 0 | 3 / 1 | 9 |
| 8 Dunes | 5 / 0 | 3 / 0 | 10 |
| 9 Braided rivers | 3 / 0 | 1 / 0 | 7 |
| 10 Glaciers | 5 / 1 | 3 / 0 | 10 |
| 11 Cones and lava | 5 / 1 | 3 / 1 | 16 |
| 12 Coastal cliffs | 4 / 0 | 3 / 0 | 8 |

Targets per class: L1/L2 at least 8 training and 2 validation GLO-30 regions; L3 at
least 5 training and 1 validation lidar regions; L4 at least 15 tiles from at least 4
independent areas, 1 held out. Classes without a validation region cannot pass gate
2 and get one first. More public data is acquired where a class falls short, through
the existing catalog plans and the gallery Yes/No.

## Steps

1. **Instrument.** Add the class-swap and class-recognition gates to `cascade_eval`
   for all levels, plus the class grid page. Record the failing baseline of the
   current runs.
2. **Data.** Catalog plans for the missing GLO-30 and lidar regions; gallery votes;
   new releases (`context-v3`, `lidar-v2`, L4 tiles).
3. **Pilot at L2** (32 m, about an hour). Class embedding, coarse dropout, balanced
   and mixed windows. Watch the gates at each checkpoint; stop when the class effect
   is absent after a fixed budget and revise the design.
4. **Retrain L1, L3 and L4** with the design that passed, each through its gates and
   an owner review.
5. **Regenerate Fujion** from the painted layout and review L2, L3, then L4.

Warn the owner before each training run (expected time, VRAM, disk), as for all
heavy work.
