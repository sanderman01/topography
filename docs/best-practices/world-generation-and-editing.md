# World generation and editing

Applies to `world` and `generation` in `topography-core` and their future Python
jobs. Follow the [master plan](../plans/terrain-design-master-plan.md), sections
8–10. Curated real patches, sculpting, and blending must remain a usable baseline
and fallback alongside diffusion experiments.

## Compose in shared physical coordinates

- Establish large ridges, drainage, coasts, and routes in an editable base
  surface. Retain each real-patch crop, translation, rotation, physical scale,
  elevation offset, blend mask, and source identity. Keep Earth coordinates in
  the source library and record transforms into the fictional world.
- Use the proposed 16 m, 4 m, and 1 m scale pyramid as configurable, versioned
  settings. Review each level before refining it. Keep coarse elevation and
  anchors as constraints so local detail cannot freely move a mountain range.
- Keep the authoritative ownership map finer than the 512² painting overview.
  Use stable integer classes and versioned precedence for the primary class;
  represent secondary terrain characteristics as additional controls. Water and
  validity remain separate masks, and the provisional nine-class count may change
  through a versioned migration.
- Require every finalized class to meet configured area requirements, then
  inspect whether the terrain actually exhibits the intended forms. A correctly
  colored map is insufficient evidence of successful generation.

## Coordinate overlapping windows

- Share world-coordinate controls and spatially consistent noise across windows.
  Include context halos, normalized overlap weights, outer-edge handling, and
  four-window corner handling; keep window and stride settings explicit.
- When implementing MultiDiffusion-style inference, combine overlapping
  predictions into a shared update at each denoising step. Independently finishing
  images and feathering their borders implements a different algorithm. The
  [MultiDiffusion method](https://multidiffusion.github.io/) demonstrates image
  coordination; terrain still needs height, slope, drainage, and route checks.
- Validate seams over the whole overlap, including tiled latent decoding.
  Restrict any subsequent join repair to a declared writable band and preserve
  locked heights and landmarks. Keep elevation in common metres without
  per-window contrast adjustment.
- Use windowed storage and bound concurrent inference. Measure representative
  memory, runtime, and I/O within 16 GB VRAM before extrapolating to full extent.
  Account for shared accumulation buffers as well as model windows.

## Keep water and edits explicit

- Represent water by masks and authored surface functions: level lakes/ocean and
  downstream river profiles with explicit designed waterfall drops. Underwater
  raster cells contain deterministic masked fill, excluded from terrain controls
  and metrics. Do not generate or infer bathymetry.
- Store edits as ordered, reversible layers with an extent and encoding: height
  deltas, replacements, landmarks, water constraints, ownership edits, and locks.
  Retain both the generated proposal and the accepted edited revision. External
  raster edits enter as explicit imports with known transforms.
- For a regional rerun, record the selected region, model, controls, scale, seed,
  read-only context halo, writable transition band, and protected regions. Show
  the dependency-derived affected area before execution; a river change may
  invalidate downstream terrain beyond the selected polygon.
- Preserve the original revision. After generation, copy original values back
  outside the permitted edit domain and inside protected regions, then verify
  exact equality there. Check the transition boundary and connected features.
- Save a rerun as a separate candidate until accepted. Changes to a dataset or
  model must not silently overwrite accepted regions. Keep dependencies and
  retained outputs sufficient to explain and restore a revision; fixed seeds do
  not guarantee bitwise GPU regeneration across environments.

## Verify

Before scaling up, demonstrate the 4,096 m pilot with several class boundaries,
connected water, a protected landmark, and a local rerun. Compare unchanged
regions exactly, inspect height/slope joins and water connections, and review
repeatable low-altitude routes. Use [validation and review](validation-and-review.md)
and [terrain export](terrain-export.md) at their respective boundaries.
