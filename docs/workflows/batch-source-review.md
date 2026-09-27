# Batch downloads and terrain gallery

Put target sources and native crop windows in a JSON queue. A small structural
example is [batch-queue-v1.json](../../examples/manifests/batch-queue-v1.json);
replace its source with a discovered or reviewed bare-earth source declaration.
The [contract](../../schemas/batch-gallery-v1.md) defines paths and bounds.

```sh
target/debug/topography batch validate data/staging/targets.json
target/debug/topography batch download data/staging/targets.json configs/runtime.example.toml data/review/source-batch
target/debug/topography batch review data/staging/targets.json configs/runtime.example.toml data/review/source-batch
target/debug/topography gallery build data/review/source-batch/gallery-input.json catalog/labels/terrain-v1.toml data/review/source-gallery
target/debug/topography gallery serve configs/runtime.example.toml data/review/source-gallery/gallery.json 8765
```

A plan TOML (see `catalog/plans/`) names areas with a provider, bounding box,
terrain class and crop count. `batch plan` discovers sources and writes the queue,
skipping ground that already has a curation decision, accepted or rejected:

```sh
target/release/topography batch plan catalog/plans/expansion-4.toml configs/runtime.example.toml data/review/expansion-5
```

`batch review` leaves out crops with under 50% valid cells, under 10 m relief, or
over 88% of the ground under 3° slope at 4 m (floodplains, water, flat fields); they
are listed in `skipped-crops.json`.

For real source batches, build the optimized CLI once with
`cargo build --release --locked --offline` and use `target/release/topography`
in these commands. This substantially reduces repeated source hashing time.

Open the printed local URL. Select a thumbnail to inspect overhead, profile or
perspective views. **Yes / Y** and **No / N** save directly, then advance. Arrow
keys browse without voting. Filtering shows pending, accepted, rejected or all
tiles. Choices survive browser reloads and server restarts. Existing selections
are shown without requiring another vote. An error is shown if the catalog has
changed since the page loaded; no choice is silently overwritten or retried.

Restarts are quick. File digests are reused while a file's inode, size, mtime and
ctime are unchanged (`<review>/file-digests.json`). Artifact checks are cached per
review and detector version (`<review>/artifact-checks/`). A tile saved as Ready stays
Ready without re-preparation unless its decision, profile, registered product, cover
policy or parent records changed. `curate check` still re-reads every byte.

The service runs until Ctrl-C and is available only on this machine. To resume,
run the same `gallery serve` command. Nothing is uploaded. Only a deliberate
Yes/No selection appends a human decision; terrain inspection never does.

Rerun the same batch command to resume. Valid completed downloads and crops are
verified and reused; failures are listed in the JSON summary and produce exit 1.
The underlying downloader handles HTTP retries/ranges. The normal workflow does
not need per-file curl calls or conversational vote transcription. A partially
successful review batch can already produce a gallery of its successful tiles.
Use a new gallery directory for a new snapshot; prior pages/decisions stay intact.

Download and crop processing are separate. Estimate crop workload before running
large lists and obtain readiness under AGENTS.md for substantial raster work.
Per-source/aggregate byte caps bound assets; staging chunks and review/gallery
copies use additional disk. The first version runs sequentially and requires
curl, GDAL and the existing Rust CLI; it adds no web framework or model environment.
The local server currently reads session entropy from `/dev/urandom` (Unix).

Source discovery, dataset-specific permission/QA interpretation and geographic
split planning still require technical judgment. They feed the target list;
visual approval does not skip those gates or create prepared/training admission.
For a first gallery choice, a source's declared confirmed permission remains
unconfirmed in curation until its supporting evidence is attached. The raw
declaration and license details remain intact, as do any declared restrictions
or prohibitions. Later visual choices retain existing curation evidence and
permission findings.
When published evidence is already available, complete survey/date scope, actual
CRS, units, datum and processing declarations before publishing review crops.
This avoids unnecessary metadata-only successors after selection. Preserve
unknowns when evidence is missing; never infer acquisition dates from a title.

## Automatic technical preparation

Add an optional `preparation_profile` path to each queue source. The
[profile contract](../design/automatic-preparation-v1.md) captures a reusable
supplier-QA assessment, source/asset identities, permission evidence and an
explicit terrestrial footprint or source-grid water mask. An assessment needs
technical judgment once; it is not another human tile-selection task.

`batch review` checks configured profiles, stages immutable masks and reports
technical exceptions in `preparation-status.json`. A technical exception leaves
the terrain available for Yes/No review. The profile digest is pinned through
batch and gallery publication; changed assessments need a new queue/gallery.

While the gallery service runs, one background worker checks pending tiles and
prepares accepted tiles sequentially. **Yes saves immediately**. The gallery
shows Checking, Awaiting acceptance, Preparing, Ready or Needs technical
attention. No shows Not selected. Ready means verified native prepared terrain,
not training admission. Existing prepared regions are recognized without profiles;
accepted tiles with older products gain game water, snow/ice and vegetation
masks automatically (`game_cover_v5`, see [cover acquisition](cover-acquisition.md))
and become Ready. A tile whose raw receipt has a metadata successor for the same
asset is shown on the successor crop and keeps its vote.

Tile details show the WGS84 centre and corners (5 decimals) and an
*Open in Google Earth* link (new tab, range about 2.5 tile widths). `/api/tiles`
includes the same data as `location`.

A Ready tile (v2 product) offers optional **Water**, **Snow/ice** and
**Vegetation** overlays on the overhead view. Solid colour marks present cells,
grey hatching marks unknown cells and clear cells are assessed absent. The page
loads them lazily from `/api/overlay/N`, which verifies the product manifest and
mask hashes before cropping the tile's native window. Overlays are read-only; they
never vote, write the catalog or ask for mask authoring. Cover sources that
cannot be reached leave the tile needing attention until the next service start.

No additional vote is needed for an unchanged tile. Missing source assessments,
unknown water, known defects and integrity failures remain technical exceptions;
they never turn a Yes into a No or erase a failed finding. Existing permission
restrictions cannot be upgraded automatically. Large new raster batches still
require workload readiness before starting the processing service.

Job artifacts live under the runtime review root's `automatic-preparation/`.
Restarting the same service verifies and reuses completed artifacts and resumes
interrupted work. Failed jobs retry once per restart or changed decision, rather
than retrying endlessly. The saved status is advisory; actual artifacts and
current catalog decisions determine readiness. Rejection preserves old products
and excludes them from use.
