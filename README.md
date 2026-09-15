# Topography

A terrain workshop for a dirigible game: collect detailed Earth elevation,
curate terrain, and compose an editable world with human review throughout.

The [master plan](docs/plans/terrain-design-master-plan.md) defines the project.
The Rust CLI validates world/runtime presets, candidate class legends and source
metadata. Acquisition, raster processing, training, generation and export are
still to be implemented. Validation reads metadata only; it does not approve terrain.

## Directory layout

```text
catalog/                 Versioned provider descriptions, areas, and label legends
configs/                 Loadable draft runtime/world presets; local machine copies
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
examples/manifests/      Synthetic source record; future batch examples
schemas/                 Contract documentation; future shared batch schemas
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
cargo fetch --locked
cargo run --locked --offline -p topography-cli -- --help
cargo run --locked --offline -p topography-cli -- --version
cargo run --locked --offline -p topography-cli -- validate world configs/worlds/pilot.toml
cargo run --locked --offline -p topography-cli -- validate runtime configs/runtime.example.toml
cargo run --locked --offline -p topography-cli -- validate legend catalog/labels/terrain-v1.toml
cargo run --locked --offline -p topography-cli -- validate source examples/manifests/source-v1.toml
bash scripts/check.sh
```

Fetch locked dependencies once with network access; checks retain locked offline
resolution. Serde and TOML provide strict typed decoding. The check script builds
the workspace, checks formatting, runs Clippy, and exercises valid/invalid CLI
inputs. Set `CARGO_BUILD_JOBS=1` to limit build concurrency. See
[training](training/README.md) for the separate Python package.

CLI exit codes: 0 for successful metadata validation, 1 for invalid/unreadable
inputs, and 2 for unsupported arguments. An excluded source record can be valid
metadata; success does not establish training permission, data quality or human
acceptance. No files or directories are created by validation commands.

See [AGENTS.md](AGENTS.md) for repository working instructions and the topic
guides in [docs/best-practices](docs/best-practices/).

Later processing stages will use GDAL/PROJ, PDAL where point clouds are involved,
and QGIS for human review. They are not required to build this scaffold. Training
dependencies and GPU settings will be selected during the feasibility pilot,
within the plan's 16 GB VRAM limit.

The [configuration contract](configs/README.md),
[candidate legend](schemas/legend-v1.md), [source record](schemas/source-v1.md)
and [initial acceptance criteria](docs/design/pilot-acceptance-v1.md) start the
first plan item. Vehicle dimensions remain explicitly unset for the pilot.
The next implementation step is one numeric DEM acquisition path and manual
import. The first terrain milestone remains reviewable real terrain and a
numeric precision check; see the master plan's implementation sequence.
