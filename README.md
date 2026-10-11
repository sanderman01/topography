# Topography

A terrain workshop for a dirigible game: collect detailed Earth elevation, curate
it with whole-tile Yes/No review, train coarse-to-fine diffusion models on it, and
generate and export 1 m heightmap worlds.

The [master plan](docs/plans/terrain-design-master-plan.md) defines the project;
[progress](docs/progress.md) lists what is done and what comes next.

## What works

- **Source library:** a Rust CLI discovers, downloads and imports public USGS and
  LINZ DEMs as raw Float32 GeoTIFFs with checksums. A local review gallery shows
  native crops for Yes/No votes; a Yes queues preparation, technical checks and
  game cover (snow/ice, vegetation, water, land cover) automatically.
- **Context regions:** 32 m Copernicus GLO-30 regions with a GEBCO seabed and
  4 m regions averaged from public lidar, each voted on, train the coarse levels.
- **Authored layouts:** a written brief and `layout.toml` render to a 128 m layout
  (`topography world layout`), which is then painted as layers of one GIMP file:
  height, roughness and terrain class. Real exemplars fill painted craters,
  calderas, cinder cones and drowned karst; a painted volcano can be replaced by a
  real one.
- **Terrain cascade:** four residual diffusion models (128 m, 32 m, 4 m, 1 m)
  refine a layout level by level. They follow the per-cell terrain class (legend v3,
  18 classes) and its form channels (roughness, flats, peaks, hollows, direction); the
  current models pass their class gates. Drainage, lakes, tarns and streams come
  from the generated terrain; terrain continues under the sea. Snow and tree lines
  follow aspect, slope and gullies. A 32 km world generates at 4 m in about an hour
  and at 1 m in about 17 hours of bands.
- **Export:** 16-bit heightmap tiles (default 257² `shared_edge`) with quadtree LOD
  levels, mask, land-cover and biome tiles, SHA-256 checksums and optional Float32
  COG masters, plus a local 3D viewer.

Processing success never approves terrain: source tiles, regions, models and worlds
each pass an explicit human review. The latest 32 km world at 1 m (Fujion, from the
current models) awaits the owner's review; regional reruns and editing are planned.

## Workflows

| Task | Guide |
| --- | --- |
| Add source tiles and vote on them | [Batch download and gallery](docs/workflows/batch-source-review.md) |
| Acquire or import a single DEM | [DEM acquisition](docs/workflows/dem-acquisition.md) |
| Build and vote on context regions | [Context regions](docs/workflows/context-regions.md) |
| Paint a layout, train cascade levels, generate and export a world | [Terrain cascade](docs/workflows/terrain-cascade.md) |
| Build the 1 m training release | [Terrain diffusion](docs/workflows/terrain-diffusion.md) |
| Draft cover masks for one crop | [Cover acquisition](docs/workflows/cover-acquisition.md) |

Curation, regional quality, aligned preparation, numeric evaluation and the
codec experiments have their own guides in [docs/workflows](docs/workflows/);
findings are in [docs/reports](docs/reports/).

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
  context/               GLO-30 and 4 m lidar context regions with votes
  datasets/              Versioned dataset releases
  models/                Training runs, checkpoints, and evaluations
  worlds/                World revisions, controls, edits, and locks
  exports/               Packaged height tiles, masks, and manifests
  cache/                 Rebuildable cached artifacts
  staging/               In-progress downloads and temporary processing files
docs/                    Design, implementation plan, and reference notes
examples/manifests/      Synthetic source record; future batch examples
schemas/                 Versioned metadata and Rust/Python batch contracts
scripts/                 Small Bash orchestration and development checks
tests/fixtures/          Small synthetic fixtures shared across components
training/                Python numeric evaluation and diffusion models (GPU)
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
cargo run --locked --offline -p topography-cli -- validate legend catalog/labels/terrain-v3.toml
cargo run --locked --offline -p topography-cli -- validate source examples/manifests/source-v1.toml
bash scripts/check.sh
```

Fetch locked dependencies once with network access; checks retain locked offline
resolution. Serde and TOML provide strict typed decoding. The check script builds
the workspace, checks formatting, runs Clippy, and exercises valid/invalid CLI
inputs. When Python 3.11+ and GDAL tools are available it also runs small local
import/review, curation, quality, preparation and dataset checks. The installed
training environment enables small CPU numeric worker checks too.
Set `CARGO_BUILD_JOBS=1` to limit build
concurrency. See [training](training/README.md) for the separate Python package.

CLI exit codes: 0 for successful metadata validation, 1 for invalid/unreadable
inputs, and 2 for unsupported arguments. An excluded source record can be valid
metadata; success does not establish training permission, data quality or human
acceptance. No files or directories are created by validation commands; acquisition,
import, review and curation commands publish local artifacts.

See [AGENTS.md](AGENTS.md) for repository working instructions and the topic
guides in [docs/best-practices](docs/best-practices/).

Acquisition/review requires curl and GDAL command-line tools; they are not needed
to build Rust or validate contracts. LINZ (New Zealand) DEMs are LERC-compressed,
which needs a GDAL built with internal libtiff and LERC. If your distribution's
GDAL lacks it (`gdalinfo` reports "missing codec LERC"), build GDAL from source with
`cmake -DGDAL_USE_LERC_INTERNAL=ON -DGDAL_USE_TIFF_INTERNAL=ON
-DGDAL_USE_GEOTIFF_INTERNAL=ON -DCMAKE_INSTALL_PREFIX=$HOME/.local/opt/gdal-lerc
-DCMAKE_INSTALL_RPATH=$HOME/.local/opt/gdal-lerc/lib`, install it, and run
`topography` with `PATH=$HOME/.local/opt/gdal-lerc/bin:$PATH`. Use QGIS to inspect review rasters.
The optional ground-support probe uses PDAL. Model training and generation need
a GPU PyTorch environment within the 16 GB VRAM limit; see
[training/README.md](training/README.md#terrain-diffusion).
