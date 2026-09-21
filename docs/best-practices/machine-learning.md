# Machine learning

Applies to dataset releases, numeric evaluation, training, and inference.
The scalar worker is implemented; the optional codec experiment uses a separate
CPU-only Torch/Diffusers environment. Model training is not implemented. Follow sections 6–7 of the
[master plan](../plans/terrain-design-master-plan.md): prove numeric fidelity and
feasibility before committing to a diffusion architecture or a large training run.

## Prevent leakage and preserve labels

- Train on approved, traceable source data only. Establish geographic
  train/validation/test splits before extracting overlapping crops. Keep nearby
  crops, repeated surveys, alternate resolutions, and augmentations of the same
  landform in one split; buffer separation by the largest physical context window.
- Report independent source regions and surveyed area alongside crop counts.
  Balance terrain classes by independent area and limit domination by one large
  survey. Reserve held-out evaluation data from normalization fitting and model
  tuning; overlapping crops are not independent evidence.
- Version integer class IDs and their display legend; derive RGB controls from
  labels. Use nearest-neighbor resizing for categorical labels and transform all
  paired layers together. Mask uncertain labels out of supervised label loss.
- Include mixed-class regions and controls resembling the human's coarse layout.
  Apply geometric augmentations consistently to heights, masks, and structural
  controls. Any deliberate physical scaling must update spacing and metadata.
- Keep approved secondary 2 m sources out of 1 m supervision. Under this plan
  they may supervise only the 4 m and 16 m levels; upsampling is not fine-detail
  ground truth. See [geospatial processing](geospatial.md).

## Measure numeric fidelity first

- Load DEMs as scalar physical values. Use fixed normalization per model level,
  fitted on training data and frozen for validation. Record inverse transforms,
  clipping rates, and the physical scale of residual targets. Never independently
  min/max normalize each crop or inherit RGB/gamma preprocessing.
- Before expensive training, pass held-out ramps, flats, sharp ridges, narrow
  features, and real DEM crops through the loader and any encoder/decoder.
  Measure height error in metres, slope changes, terracing, and feature survival.
- Treat LoRA and ControlNet as experiments. LoRA cannot restore detail already
  lost by a loader or autoencoder. ControlNet needs paired terrain training and
  does not guarantee exact placement; ownership constraints and review remain
  authoritative, as specified by the master plan.
- Exclude invalid terrain and deterministic underwater fill from elevation
  losses and structural metrics. Exclude uncertain labels from label losses.
  Reject or explicitly handle all-masked batches to avoid undefined reductions.

## Stay within the feasibility budget

- Keep training and generation within 16 GB VRAM. Start from the plan's 512²
  training crops; larger sizes require measured evidence. Account separately for
  model weights, control branches, optimizer state, activations, decoding, and
  buffers. Also report host RAM, disk use, and runtime.
- Record precision at each stage. Benchmark mixed precision, checkpointing,
  accumulation, and optimizer quantization against both memory and error in
  metres. A run fitting in memory does not prove elevation precision. Consult
  [PyTorch's AMP examples](https://docs.pytorch.org/docs/main/notes/amp_examples.html)
  for operation-specific precision behavior, using the chosen release's APIs.
- Begin with standard diffusion losses; add terrain-aware objectives only for
  measured failures. A blanket smoothness loss can erase desired pinnacles and
  narrow passages.
- Compare against curated real patches and blending on the same review layouts.
  Set a bounded experiment budget before long runs. If reconstruction, memory,
  or human terrain quality fails the gate, proceed with the patch-based fallback.

## Make evaluation and runs reproducible

- Record dataset/model/legend versions, controls, normalization, seeds, scheduler,
  training settings, precision, software/hardware versions, peak GPU memory,
  runtime, metrics, and checkpoints. Seed every random source used, including
  data-loader workers, and document nondeterministic operations. Fixed seeds
  alone do not guarantee identical results across environments; see
  [PyTorch reproducibility](https://docs.pytorch.org/docs/main/notes/randomness.html).
- Use physical error for reconstruction tasks. For novel terrain, evaluate
  landform structure, control adherence, seams, repetition, possible source
  memorization, and human low-altitude review; there is no single reference DEM.
- Promote a model only after numeric, structural, resource, and human checks.
  Retain accepted output files and complete run manifests in `data/models/`.
  Follow [Python batch-job practices](python.md) for completion and resume behavior.
