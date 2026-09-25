# Working on Topography

## Project context

Topography is a human-directed terrain workshop for a dirigible game. It will
acquire and curate Earth elevation, compose editable worlds, evaluate optional
diffusion models, and export terrain with reproducible provenance.

The Rust CLI validates presets/metadata, discovers USGS DEM candidates, acquires
and imports raw Float32 GeoTIFFs, verifies raw storage and creates small native-grid
review crops through GDAL. Eligible sources now support aligned native regional
preparation with immutable outputs, scoped dataset releases, and bounded CPU
numeric/model experiments. Batch acquisition, a persistent review gallery and
profile-driven automatic preparation after acceptance are implemented. Production
training, reusable world generation and export remain planned.
Preset values remain draft pilot choices; processing does not create human acceptance.

Read [README.md](README.md) for setup and the
[master plan](docs/plans/terrain-design-master-plan.md) for scope, physical
contracts, and implementation order. The master plan refines the older initial
specification and research notes. Distinguish implemented behavior, planned
requirements, and unresolved pilot decisions in code and documentation.

## Where work belongs

| Location | Responsibility |
| --- | --- |
| `crates/topography-cli/` | Arguments, user-facing output, process exit status |
| `crates/topography-core/` | Configuration, catalog, providers, preparation, review, datasets, generation, world editing, export |
| `training/` | Python numeric evaluation and model jobs behind a planned versioned batch interface |
| `scripts/` | Small Bash orchestration and development checks |
| `catalog/`, `configs/` | Small reviewed inputs and portable configuration examples |
| `schemas/`, `examples/manifests/` | Versioned contracts and tiny examples as their consumers are implemented |
| `data/` | Ignored local databases, source data, derived artifacts, runs, and world revisions |
| `tests/fixtures/` | Small synthetic inputs shared across components |

## Best-practices guides

Read the guides relevant to the files and behavior being changed. They apply the
master plan to implementation; they do not imply that future tooling already
exists. Consult upstream documentation for the versions actually selected.

| Topic | Guide |
| --- | --- |
| Component boundaries, configuration, and manifests | [Architecture and contracts](docs/best-practices/architecture-and-contracts.md) |
| Rust library and CLI development | [Rust](docs/best-practices/rust.md) |
| Python packaging and batch jobs | [Python](docs/best-practices/python.md) |
| Bash and external command orchestration | [Shell scripting](docs/best-practices/shell-scripting.md) |
| Acquisition, catalog, storage, and provenance | [Data management](docs/best-practices/data-management.md) |
| DEMs, LiDAR, coordinates, masks, and raster processing | [Geospatial processing](docs/best-practices/geospatial.md) |
| Dataset splits, numeric fidelity, training, and evaluation | [Machine learning](docs/best-practices/machine-learning.md) |
| Coarse-to-fine composition and reversible regional edits | [World generation and editing](docs/best-practices/world-generation-and-editing.md) |
| Cell-to-vertex conversion and height encoding | [Terrain export](docs/best-practices/terrain-export.md) |
| Workflow verification and human acceptance | [Validation and review](docs/best-practices/validation-and-review.md) |

## Essential constraints

- This is terrain for a game, not science. Aim for plausible, good-looking 1 m
  terrain. Prefer tool defaults and judgment calls over evidence hunting,
  forensic analysis or sub-metre accuracy work. Ask the user only for terrain
  Yes/No and genuine direction changes.
- Use only source data publicly available for download. Do not request private
  deliveries or commission measurements. Never send emails or prepare email
  drafts, and do not ask the user to contact people or institutions for data.
- Human source review is a yes/no decision for each whole terrain tile, presented
  in batches. Do not require feature selection, mask authoring or repeated visual
  approval of an unchanged tile. Handle technical checks separately; individual
  feature annotations are optional and only requested by the user.
- Preserve authoritative elevation as Float32 metres with explicit masks and
  coordinate metadata. Image appearance and nominal grid spacing do not establish
  numeric accuracy or measured feature resolution.
- Keep raw assets immutable, derived artifacts traceable, and human acceptance
  separate from successful processing. Preserve accepted outputs and edits.
- Keep water masks and authored surface levels separate from ground elevation.
  Underwater cells use deterministic masked fill; bathymetry is outside scope.
- Snow/ice, vegetation and water masks are game layers from coarse public land
  cover (about 10 m is enough). Mark snow/ice where the area commonly has it;
  ignore seasonality and survey dates. Missing or ambiguous cover is resolved by
  a recorded judgment call, never left as a blocker.
- Split training data geographically before overlapping crops. Admit source data
  only under the master plan's quality and documented training-permission gates.
- Keep the 16 GB VRAM ceiling. Establish real-terrain and numeric-precision
  baselines before expensive model work; curated patches and blending remain a
  supported fallback if diffusion fails its feasibility gate.
- Keep training windows, inference windows, and export tiles independent. Full
  export tiles span 1,024 intervals and contain 1,025 shared-edge vertices per side.
- Leave Bevy integration, QuadTrees, vegetation rendering/placement/growth,
  weather, a full flight model, and underwater terrain outside the current plan.

## Before heavy workloads

Before starting any task or process expected to require substantial CPU/GPU
compute, RAM, or VRAM, pause and warn the user so they can close other
applications. Describe the workload and expected resource use and duration when
known; state uncertainty rather than inventing estimates. Wait for the user to
confirm they are ready before launching, unless they have already explicitly
confirmed readiness for that run. This applies to training, inference, large
raster jobs, builds, benchmarks, and any other resource-heavy work.

(Rust builds and file downloads are fine. Do not pause for these)

## Development and verification

Use Rust 1.85 or newer (edition 2024), Cargo, rustfmt, and Clippy. Python work
targets 3.11 or newer. Run the existing check from the repository root:

```sh
bash scripts/check.sh
```

It checks formatting, builds the workspace, runs Clippy with warnings denied,
and exercises CLI help/version and valid/invalid contract inputs using locked
offline Cargo resolution. With GDAL and Python 3.11+ installed it also exercises
small local import/review workflows. Fetch Rust dependencies with `cargo fetch --locked`.
Full acquisition checks (including a loopback HTTPS server) are documented in
[scripts/README.md](scripts/README.md). No formal test runner or ML environment is
configured; geospatial tools are not required for contract validation.
See [training setup](training/README.md) when implementing Python functionality.

Reproduce bugs through the user's workflow before fixing them. Run the relevant
workflow after changes and report what was actually verified. Add formal tests
when requested; follow [the verification guide](docs/best-practices/validation-and-review.md)
for their placement and terrain invariants. For documentation, verify links,
commands, and consistency with implementation and the master plan.

## Efficiency

- Per-tile work belongs in the tooling. If a tile needs hand-written scripts,
  profiles or manual CLI chains, fix the tool instead. Accepted tiles should
  reach Ready through the gallery worker without agent involvement.
- Read only the plan sections and guides relevant to the change.
- Keep verification proportional: `bash scripts/check.sh`, the real workflow
  and `topography curate check`. Skip byte-level preservation proofs, catalog
  diffs and independent raster re-verification unless investigating a bug.
- Use few subagents with tight scopes; don't have several agents rediscover
  the same workflow.


- Delegate suitable independent tasks to Sol subagents (`gpt-5.6-sol`) by default.
  Give each agent a bounded scope and distinct file ownership; keep dependent
  work and integration coordinated by the primary agent. Verify returned work
  before relying on it. Serialize shared catalog writes and integrity scans.
  Do not use Git worktrees.
- Prefer simple, robust, maintainable implementations. Keep domain logic in the
  core library; use GDAL/PROJ and PDAL for established geospatial algorithms.
- Add dependencies and abstractions for concrete consumers. Do not preselect an
  ML stack, build every provider adapter, or invent unresolved pilot tolerances.
- Follow `.editorconfig`, rustfmt, and existing conventions. Keep comments short
  and explain non-obvious reasons. Update nearby documentation with behavior.
- Inspect Git status before editing and preserve unrelated user changes. Use a
  branch for a coherent change and small Conventional Commits; leave merging to
  the user unless instructed otherwise.
- Keep local progress in root `TASKS.md`, marking completion after verification.
  Keep `PLAN.md` and task notes out of commits unless requested. Confirm changes
  to `PLAN.md` or departures from the agreed plan unless already authorized.
- Keep credentials, machine-local settings, large rasters, databases, and model
  artifacts out of Git. Follow [data storage](data/README.md); rejection records
  an exclusion, while deletion is a separate explicit housekeeping operation.
