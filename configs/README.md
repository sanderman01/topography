# Configuration inputs

The CLI reads and validates these version-one TOML presets. Their values remain
draft pilot proposals; validation does not approve a terrain build.

```sh
cargo run --locked --offline -p topography-cli -- validate world configs/worlds/pilot.toml
cargo run --locked --offline -p topography-cli -- validate runtime configs/runtime.example.toml
```

- `runtime.example.toml`: local storage paths and the fixed pilot GPU budget.
- `worlds/pilot.toml`: the 4,096 m pilot world.
- `worlds/standard.toml`: the recommended 32,768 m world.
- `worlds/exact-32km.toml`: the alternative 32,000 m world.
- `local/`: ignored machine-specific overrides and local paths.

## Loading and paths

Supply one file explicitly. No defaults, environment overrides, includes, or
automatic local overrides apply. Unknown fields at every table level, missing
required fields, unsupported versions, and invalid numeric values fail. Inputs
must be UTF-8 TOML no larger than 1 MiB. Commands are read-only. The executable
contracts are the Rust types and checks in
[`topography-core/src/config/`](../crates/topography-core/src/config/).

World and runtime files require `schema_version = 1` and `status = "draft"`.
Status describes the preset, never processing completion or human acceptance.

`paths.base_dir` resolves relative to the supplied runtime file's parent; all
other storage paths resolve relative to that base. Absolute paths stay absolute.
The example's `base_dir = ".."` selects the repository root regardless of the
caller's working directory. Paths follow the supplied filename without resolving
symlinks, may retain `..`, and receive no tilde/environment expansion. They must
be nonempty. Validation does not create directories, probe capacity, check
writability or detect overlapping storage roots; future writers need those checks.

Storage outside the checkout is supported. Adjust `base_dir` when moving a copy
into ignored `configs/local/`. Use an environment or credential store for secrets.
`hardware.gpu_vram_budget_gb` is an integer in 1–16: the budget may be reduced,
but cannot exceed the plan's 16 GB ceiling.

## World geometry

`name` is nonempty. `grid.cells` contains two positive UInt32 counts in x, y order;
`grid.spacing_m` is positive finite square-cell spacing in metres. Extent is their
product, with no independently configurable extent. Rectangles and fractional
spacing are supported. The size of one Float32 layer must fit a UInt64 byte count;
its reported size is not a working-set estimate. Samples are cell centers. CRS,
origin and row orientation belong to future raster/world-revision contracts.

- `training.tile_cells` is an independent positive UInt32 crop size.
- `generation.window_cells` and `stride_cells` are positive UInt32 counts; stride
  cannot exceed window size. They need not divide the world or fit its coarsest
  level; context padding belongs to future generation.
- `generation.scales_m` must be positive, finite, strictly descending, end at
  `grid.spacing_m`, and divide both extents into whole cells. The integral-cell
  check allows eight Float64 epsilons scaled by cell count for arithmetic rounding,
  not as a terrain quality tolerance.
- `layout.sketch_cells` contains two positive UInt32 counts. The sketch is only
  a painting overview; authoritative ownership needs finer boundaries.

`export.tile_cells` is exactly 1,024 in v1 and `sampling` must be
`"shared_edge_vertices"`. Full sides contain 1,025 vertices; a partial side with
N intervals contains N + 1. Only dimensions are calculated, not raster sampling.

| Preset | Extent at 1 m spacing | Export tile grid | Final corner vertices |
| --- | --- | --- | --- |
| Pilot | 4,096 × 4,096 m | 4 × 4 | 1,025 × 1,025 |
| Standard | 32,768 × 32,768 m | 32 × 32 | 1,025 × 1,025 |
| Exact 32 km | 32,000 × 32,000 m | 32 × 32 | 257 × 257 |

## Review settings

`review.clearances_m` is a nonempty list of finite positive clearances above
terrain, initially 32, 64 and 128 m. `review.vehicle` is optional and deliberately
absent from the pilot presets. When supplied, the whole table must contain:

| Field | Meaning and constraint |
| --- | --- |
| `width_m`, `length_m` | Positive finite vehicle dimensions |
| `clearance_margin_m` | Finite nonnegative margin |
| `speed_m_s` | Positive finite review flight speed |
| `minimum_turning_diameter_m` | Positive finite diameter of required turning space |
| `climb_rate_m_s` | Positive finite climb rate |
| `ceiling_elevation_m` | Optional finite ceiling relative to the world's elevation zero |

Vehicle dimensions, clearance margin, flight speed, turning space, climb rate,
height encoding range, biome precedence, and numerical acceptance tolerances
still need pilot decisions. See the
[initial acceptance criteria](../docs/design/pilot-acceptance-v1.md). Providing
an envelope does not itself establish flight suitability.
