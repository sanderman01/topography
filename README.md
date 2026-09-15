# Topography

A terrain workshop for a dirigible game: collect detailed Earth elevation,
curate terrain, and compose an editable world with human review throughout.

The [master plan](docs/plans/terrain-design-master-plan.md) defines the project.
This repository currently contains the directory scaffold and a minimal Rust
CLI supporting `--help` and `--version`. Acquisition, raster processing, training,
generation, and export are still to be implemented.

## Directory layout

```text
catalog/                 Versioned provider descriptions, areas, and label legends
configs/                 Draft runtime and world presets; local machine overrides
crates/
  topography-cli/        Rust command-line entry point
  topography-core/       Catalog, providers, processing, and orchestration modules
data/
  catalog/               Local catalog database
  raw/                   Immutable source files and original metadata
  prepared/              Regional elevation rasters and masks
  review/                Previews, metrics, and working annotations
  datasets/              Versioned dataset releases
  models/                Training runs, checkpoints, and evaluations
  worlds/                World revisions, controls, edits, and locks
  exports/               Packaged height tiles, masks, and manifests
  cache/                 Rebuildable cached artifacts
  staging/               In-progress downloads and temporary processing files
docs/                    Design, implementation plan, and reference notes
examples/manifests/      Small example manifests for future batch interfaces
schemas/                 Future versioned manifest and configuration schemas
scripts/                 Small Bash orchestration and development checks
tests/fixtures/          Small synthetic fixtures shared across components
training/                Python package for future training and evaluation jobs
```

Each directory includes a README or a tracked placeholder so a fresh checkout
retains the structure. Large artifacts, local configuration, and credentials are
ignored by Git. See [data storage](data/README.md) before adding source files.

## Development

Use Rust 1.85 or newer with Cargo, rustfmt, and Clippy. From the repository root:

```sh
cargo run --offline -p topography-cli -- --help
cargo run --offline -p topography-cli -- --version
bash scripts/check.sh
```

The Rust scaffold has no external dependencies. The check script builds the
workspace, checks formatting, runs Clippy, and exercises the CLI entry points.
See [training](training/README.md) for the separate Python package.

Later processing stages will use GDAL/PROJ, PDAL where point clouds are involved,
and QGIS for human review. They are not required to build this scaffold. Training
dependencies and GPU settings will be selected during the feasibility pilot,
within the plan's 16 GB VRAM limit.

The [configuration examples](configs/README.md) record draft defaults from the
plan. The CLI does not load them yet. The first implementation milestone is
reviewable real terrain and a numeric precision check; see the master plan's
implementation sequence.
