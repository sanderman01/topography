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

Open the printed local URL. Select a thumbnail to inspect overhead, profile or
perspective views. **Yes / Y** and **No / N** save directly, then advance. Arrow
keys browse without voting. Filtering shows pending, accepted, rejected or all
tiles. Choices survive browser reloads and server restarts. Existing selections
are shown without requiring another vote. An error is shown if the catalog has
changed since the page loaded; no choice is silently overwritten or retried.

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
