# Topography

A terrain workshop for a dirigible game: collect detailed Earth elevation,
curate terrain, and compose an editable world with human review throughout.

The [master plan](docs/plans/terrain-design-master-plan.md) defines the project.
See the [progress summary](docs/progress.md) for completed work, verification and
the remaining implementation order.
The Rust CLI validates presets and metadata, discovers USGS DEM candidates,
acquires/imports raw Float32 GeoTIFFs, verifies stored bytes, and creates small
native-grid review crops through GDAL. SQLite curation records crop labels, evidence
and explicit human decisions. Regional mask assessment and preparation eligibility
checks compare measured coverage with pinned decisions. Eligible areas from the
same survey can be composed on an exact native grid into immutable prepared
regions. Dataset releases, training, generation and export remain planned.
Processing success never approves terrain.
Review crops include a self-contained browser preview with multiple hillshades,
cross-sections and metric low-altitude views where the source metadata supports them.

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
inputs. When Python 3.11+ and GDAL tools are available it also runs small local
import/review, curation, quality and preparation checks. Set `CARGO_BUILD_JOBS=1` to limit build
concurrency. See [training](training/README.md) for the separate Python package.

CLI exit codes: 0 for successful metadata validation, 1 for invalid/unreadable
inputs, and 2 for unsupported arguments. An excluded source record can be valid
metadata; success does not establish training permission, data quality or human
acceptance. No files or directories are created by validation commands; acquisition,
import, review and curation commands publish local artifacts.

See [AGENTS.md](AGENTS.md) for repository working instructions and the topic
guides in [docs/best-practices](docs/best-practices/).

Acquisition/review requires curl and GDAL command-line tools; they are not needed
to build Rust or validate contracts. Use QGIS to inspect review rasters. The
optional ground-support probe uses PDAL. Training dependencies and GPU settings will be selected during the feasibility pilot,
within the plan's 16 GB VRAM limit.

The [configuration contract](configs/README.md),
[candidate legend](schemas/legend-v1.md), [source record](schemas/source-v1.md)
and [initial acceptance criteria](docs/design/pilot-acceptance-v1.md) start the
first plan item. Vehicle dimensions remain explicitly unset for the pilot.
Follow the [acquisition and import workflow](docs/workflows/dem-acquisition.md)
for discovery, bounded downloads, integrity checks and review crops. A real USGS
coastal DEM has been imported locally and the native-grid crop path is verified;
the live TNM search API returned HTTP 504 during development. The
[early karst trial](docs/reports/rio-abajo-karst-trial.md) now provides three
native 0.5 m crops from the Río Abajo area. The
[curation workflow](docs/workflows/source-curation.md) now preserves crop decisions
and evidence. The [regional quality workflow](docs/workflows/regional-quality.md)
measures whole-crop masks and checks current preparation eligibility.
[Aligned native preparation](docs/workflows/aligned-preparation.md) now preserves
eligible regions without resampling or cross-survey joins. Twelve accepted terrain
regions are prepared under the [simple supplier-QA pilot policy](docs/design/source-quality-pilot-v1.md),
with exact native ground elevation preserved and water masked separately.
[Batch 3](docs/reports/pilot-tile-batch-3.md) is accepted and prepared. A
[numeric pilot proposal](docs/reports/numeric-pilot-proposal-v1.md) groups the six
source areas for evaluation; the dataset checkpoint is awaiting approval.
A small arithmetic probe preserved Float32 values and measured Float16 losses.
Dataset release tooling and the production loader/autoencoder evaluation remain pending.
