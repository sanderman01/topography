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
1 m detail that a coarse surface lacks, plus game cover masks:

- Coarse surface `C`: ground elevation block-averaged to 32 m (valid cells only),
  then bicubically upsampled to 1 m. This matches both a low-passed survey and a
  public 30 m DEM, so the same model works on real training tiles and on a world
  base built from Copernicus GLO-30 or sculpting.
- Target channels (1 m, 256² training crops):
  1. residual `(H − C) / s_r`, with `s_r` a fixed metre scale fitted on training
     cells and then frozen (initial guess 16 m; clip rate recorded);
  2. snow/ice present (±1);
  3. vegetation present (±1).
- Conditioning channels, concatenated to the input:
  - `(C − mean(C over crop)) / 256 m`: local coarse shape;
  - `C / 4,096 m`: absolute elevation, which snow depends on;
  - water mask (authored in worlds; water cells are not generated terrain);
  - per-pixel terrain class as a one-hot map over the nine legend classes plus
    "unknown". 10% class dropout during training enables classifier-free guidance.
- Loss: EDM denoising loss (Karras et al. 2022). Height loss covers only valid,
  non-water, non-snow-surface cells. Each mask channel's loss covers only cells
  where that mask is known.
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
denoising step. Snow/ice and vegetation are thresholded at 0. Under water, the
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
  each, choosing between them on validation loss and samples.
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
- [x] **Pilot world inputs:** `topography world input configs/diffusion/pilot-1.toml <dir>`; a 4,096 m world config with a coarse base (a chosen
  GLO-30 region at 32 m or a sculpted surface), a coarse class map and authored
  water.
- [ ] **Pilot world generation:** 1 m MultiDiffusion over 4,096² (about 1,000 windows
  of 256² at stride 128), writing Float32 elevation and masks with provenance. Then
  a whole-world visual review.
- [x] **16-bit export:** `topography export heightmap <world-output> <dir>`; 257² `shared_edge` tiles and quadtree LOD levels, per master
  plan §5 and §10.

## Resource expectations

- Training: 256² crops, batch 8–16, estimated at under 12 GB VRAM. The smoke run
  measures the real figure; if a configuration exceeds 16 GB, reduce batch size or
  width, and never lower output precision.
- Release: under 1 GB on disk.
- Checkpoints: roughly 0.5–1 GB per run.
- Generation time: estimated from smoke-run throughput, not assumed.

## Open pilot choices

- `s_r`, U-Net width, noise schedule and step counts are starting values to be
  tuned, not requirements.
- The GLO-30 region for the pilot base is chosen at the pilot-world step.
  GLO-30 is a surface model (it includes canopy). At 32 m averaging this is
  tolerated for a pilot; sculpted bases avoid it entirely.
- Classes without accepted tiles (archipelagos, alpine) have no training data.
  Requesting them falls back to "unknown" until tiles are accepted.
