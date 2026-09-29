# Terrain diffusion pilot v1

Status: planned 2026-09-25. Supersedes the "continue without a model" decision in
the [scalar fitting report](../reports/scalar-codec-fit-v1.md) and
[image-codec report](../reports/image-codec-pilot-v1.md). Curated patches and
blending stay the fallback (master plan §7).

## Why the earlier attempts do not settle the question

No generative model was trained. Two representation checks ran on CPU:

- A frozen Stable Diffusion VAE got absolute height ÷ 2,048 m. A 128 m window
  with ~20 m relief then spans ~1% of the input range, below what an 8-bit-image
  VAE resolves. Removing each window's mean bias cut its error from 4–28 m to
  0.7–7.7 m, so the scaling caused most of the loss.
- A scalar autoencoder trained for 200 steps at batch 1 (75 s on two CPU threads),
  also on absolute ÷ 2,048 m heights. That budget can't show whether the
  architecture works.

This plan removes both causes: no autoencoder, and the model only sees heights
relative to a coarse surface at a fixed physical scale.

## Approach

**Pixel-space conditional diffusion of fine residuals.** The model generates the
1 m detail that a coarse surface lacks, for given game cover (conditioning
layout 2, run 4 onward; runs 1–3 used layout 1 and predicted the cover masks as
extra target channels, so they never knew where ice or forest was):

- Coarse surface `C`: ground elevation block-averaged to 32 m (valid cells only),
  then bicubically upsampled to 1 m. This matches both a low-passed survey and a
  public 30 m DEM, so the same model works on real training tiles and on a world
  base built from Copernicus GLO-30 or sculpting.
- Target (1 m, 256² training crops): residual `(H − C) / s_r`, with `s_r` a
  fixed metre scale fitted on training cells and then frozen.
- Conditioning channels, concatenated to the input:
  - `(C − mean(C over crop)) / 256 m`: local coarse shape;
  - `C / 4,096 m`: absolute elevation, which snow depends on;
  - water mask (authored in worlds; water cells are not generated terrain);
  - snow/ice and vegetation coverage, blurred with a 6 m Gaussian and scaled to
    ±1, so the 10 m land-cover steps do not print as squares into the terrain;
  - per-pixel terrain class as a one-hot map over the nine legend classes plus
    "unknown". 10% class dropout during training enables classifier-free guidance.
- Loss: EDM denoising loss (Karras et al. 2022) over valid, non-water cells,
  including glacier surfaces.
- Network: a U-Net (diffusers `UNet2DModel`) of about 50–100 M parameters, with
  attention only at the two coarsest levels so it stays convolutional and works on
  any window size.
- Precision: FP32 master weights with BF16 autocast. Heights stay FP32 at
  input/output and are converted to metres in FP32. The 16-bit conversion happens
  only at export, with one world-wide offset/scale (0.0625 m steps over 4,096 m).
- Augmentation: the eight rotations/reflections applied to all layers together.
  No resizing.

Inference for large areas uses MultiDiffusion: overlapping windows share
spatially consistent noise, and their predictions are averaged at every
denoising step. The world's cover masks pass through to the output. Under water, the
elevation gets the deterministic fill rule from the world contract.

## Data

All accepted **Ready** gallery tiles (currently 33, about 82 km², seven classes:
karst, gorges, fjords, volcanic, badlands, dunes and braided rivers). A Rust
command reads the gallery and snapshots each tile's current prepared product into
a training release:

- 0.5 m sources are average-resampled to 1 m by GDAL;
- validity, water, snow/ice and vegetation masks follow by nearest/majority rules;
- the output is raw little-endian arrays plus a manifest with hashes, CRS, class
  and provenance.

Python reads only the release. Adding or accepting tiles later means rebuilding the
release; there is no per-tile hand work.

**Source artifacts (added after run 1).** Run 1 reproduced defects that were in the
sources but too small to see in the library's 3 m/pixel overview: interpolated
triangle facets (New Zealand LINZ and parts of Puerto Rico), periodic scan striping
(St. Helens 2 m, Great Sand Dunes 5 m) and straight seams. Automatic checks
(`artifacts_v1`, run at about 1 m) now handle them without rejecting tiles:

- **Facets:** exactly planar cells on sloping ground, spread over 8–16 m. The
  training copy smooths them (a local mean, sigma about 2.5 m, at most 2 m change
  per cell) so the triangle edges vanish. Forest-covered slopes such as Milford
  are triangulated almost everywhere, and leaving them out halved the fjord data.
- **Stripes:** one strong periodic peak per axis between 2 and 8 m. The training
  copy has that frequency removed with a narrow notch filter.
- **Seams:** cross-axis curvature that keeps its sign along a grid line for at
  least 192 m. These are left out of training.

The library shows the same checks before a vote (an Artifacts overlay, the shares
affected, and a 1:1 detail view). Each release writes an `artifacts.u8` layer and a
`report.html` with the worst close-up per tile, to glance at before training.

Split before cropping, by world coordinates: cells in the southernmost 512 m of
every 2,048 m northing band are validation; the 128 m on each side of that band is
unused buffer; everything else is training. Crops never cross split or buffer
boundaries. Tiles that fall entirely in a buffer are unused. This keeps every class
in both splits, which holding out whole surveys can't do with one or two surveys
per class. It's weaker than independent regions, so memorization is checked
explicitly (below).

## Steps

- [ ] **Environment:** ROCm PyTorch in `data/models/diffusion-env-v1/.venv`, plus
  the `diffusion` extra. A GPU matmul check confirms the RX 9060 XT (gfx1200) runs.
- [x] **Release:** `topography dataset diffusion <runtime.toml> <gallery.json> <new-output-dir>`
  builds the training release from current Ready tiles.
- [ ] **Training code:** data loading, split and crops, the model, EDM training with
  checkpoints/resume, a VRAM and throughput log, and a sampler that supports
  MultiDiffusion.
- [ ] **Smoke run** (minutes, needs readiness): fit a few hundred steps on a few
  crops. Loss falls, VRAM peak and steps/s are recorded, and the recorded speed sets
  the real run's time.
- [ ] **Pilot training** (hours, needs readiness): at most three runs of about 8 h
  each, choosing between them on validation loss and samples. Run 1: 11/16
  generated patches got Yes, but grid artifacts (fixed coarse blocks), blocky water
  and learned source artifacts showed. Run 2 uses a smoothed coarse surface with a
  random block offset, smoothed water and the artifact checks above. Run 2's
  blind review showed real patches with grey 10 m blocks where glaciers and raw
  WorldCover water were masked out. Run 3 trains on `game_cover_v5` products:
  glaciers are ground and water edges are smoothed at the source, so the
  release, the loader and world inputs share one outline. Run 3 still made
  glaciers rocky (it was never told where ice is) and flats lumpy. Run 4 uses
  conditioning layout 2 on `game_cover_v7` products (10 m water smoothing that
  also removes small square WorldCover fragments). It is a fourth run beyond
  the three planned, on the owner's direction (2026-09-28).
- [ ] **Evaluation:**
  - generate from the coarse surface, class and water of held-out validation
    windows;
  - report per-class slope and roughness distributions against real terrain,
    plus the nearest training-crop distance, to spot copying;
  - generate a 1 km MultiDiffusion strip and measure the height and slope steps
    across seams.
- [ ] **Human gate:** a blind gallery batch of about 32 hillshaded 256 m patches,
  half real and half generated, in random order. The owner gives each patch a
  Yes/No. The gate passes when at least half the generated patches get Yes and the
  seam strip shows no visible joins. Otherwise try the next run, or fall back after
  three.
- [x] **Pilot world inputs:** `topography world input configs/runtime.example.toml configs/diffusion/pilot-1.toml <dir>`; a 4,096 m world config with a coarse base (a chosen
  GLO-30 region at 32 m or a sculpted surface), a coarse class map and authored
  water.
- [ ] **Pilot world generation:** 1 m MultiDiffusion over 4,096² (about 1,000 windows
  of 256² at stride 128), writing Float32 elevation and masks with provenance. Then
  a whole-world visual review. Check the mid scale specifically (see watch point
  below); 256 m gate patches are too small to show it.
- [x] **16-bit export:** `topography export heightmap <world-output> <dir>`; 257² `shared_edge` tiles and quadtree LOD levels, per master
  plan §5 and §10.

## Resource expectations

- Training: 256² crops, batch 8–16, estimated at under 12 GB VRAM. The smoke run
  measures the real figure; if a configuration exceeds 16 GB, reduce batch size or
  width, and never lower output precision.
- Release: under 1 GB on disk.
- Checkpoints: roughly 0.5–1 GB per run.
- Generation time: estimated from smoke-run throughput, not assumed.

## Follow-on: coarse structure model

This pilot only adds 1 m detail to a given base. Original worlds need their
large-scale structure generated too: a second model trained on global GLO-30 at
about 32 m (master plan §8). It is planned after this pilot's gate.

## Watch point: the 32× scale gap

One step from 32 m to 1 m is aggressive; cascades usually step 4–8× per stage. The
1 m model must invent features of roughly 60–300 m wavelength (gullies, spurs, small
ridges) with little guidance from the coarse surface. During review, look for detail
that looks right close up but lacks structure at that scale: texture without
drainage or ridge organisation between the coarse landforms. If it shows, add an
intermediate level (e.g. 32 → 4 → 1 m) trained on the planned 4 m pyramid examples
(master plan §5) instead of adding runs to this model.

## Known issues after run 4 (2026-09-29)

Run 4 passed the blind review (16/16 generated Yes) and halved the flat-terrain
overshoot, but paired evaluation and a guidance test found the following. Work
on them is paused while the pilot world is generated and exported.

1. **Local shape is mostly ignored (the watch point above).** With one seed, four
   different glacier windows produced outputs with 0.99 residual correlation;
   guidance against a flattened coarse surface only lowered that to 0.86 at ×4.
   Detail does not line up with slopes and landforms. Likely cause: the
   coarse-relative channel is scaled by 256 m, so it varies only about ±0.1
   inside a window, and there is no slope input. Proposed run 5: scale the
   coarse input to unit range, add east/north slope channels, and drop
   cover/coarse inputs occasionally in training. If that is not enough, add
   the intermediate 4 m level above.
2. **Glaciers too smooth.** Real glaciers here are crevasse fields (curvature std
   0.65–1.57 m); run 4 gives 0.42. Guidance ×2 against an ice-absent condition
   brings it into range (1.37) without retraining; ×4 overshoots. Used as a
   stopgap for the pilot world. Cover dropout in run 5 would make this guidance
   well-founded.
3. **Flats still hummocky.** Canyonlands 2.1° real vs 7.5° (run 3: 14.3°).
   Class guidance helps some flat windows and hurts others; expected to improve
   with item 1.
4. **Seams slightly worse than run 3** (join/overall slope 1.07 vs 1.02), no
   visible joins. Recheck on the pilot world.
5. **Twin review patches.** Fixed: the evaluator shows real and generated
   patches from different windows.
6. **Water pools on steep slopes** (found in the pilot world, 2026-09-29). Water
   has to flow downhill: a slope can carry a stream, but never a pool. The
   pools are probably WorldCover water false positives (dark terrain shadow on
   steep faces is a known confusion), which neither the cover policy nor
   generation checks against the terrain. Proposed fix, shared by training
   releases and world inputs:
   - **Cover policy:** drop standing-water patches (not OSM waterways) whose
     ground-elevation range or mean slope exceeds a small limit, e.g. more than
     a few metres of spread across the patch. Record the rule in the policy
     version.
   - **Generation:** give each remaining lake one flat surface level (its
     lowest shore) and keep stream beds descending. Terrain around them is
     generated to fit, instead of treating water as an unrelated mask.

## Open pilot choices

- `s_r`, U-Net width, noise schedule and step counts are starting values to be
  tuned, not requirements.
- The GLO-30 region for the pilot base is chosen at the pilot-world step.
  GLO-30 is a surface model (it includes canopy). At 32 m averaging this is
  tolerated for a pilot; sculpted bases avoid it entirely.
- Classes without accepted tiles (archipelagos, alpine) have no training data.
  Requesting them falls back to "unknown" until tiles are accepted.
