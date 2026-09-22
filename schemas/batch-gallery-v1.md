# Acquisition queue and gallery v1

Both inputs are strict JSON: unknown fields and unsupported versions fail.
Paths in inputs resolve relative to the input file. See the
[workflow](../docs/workflows/batch-source-review.md) for commands.

## Acquisition queue

`schema_version: 1`, `max_total_bytes` and `sources` are required. There are
1–256 sources and at most 2,048 crops in a queue. Each source has:

- `id`: unique, 1–80 ASCII letters/digits, hyphens or underscores.
- `source`: validated source TOML path; original metadata paths retain the source
  contract's resolution relative to that TOML.
- `max_bytes`: positive per-source asset limit. The sum of source limits must fit
  `max_total_bytes`; this is an asset-byte budget, not a total disk quota. Retained
  range chunks, immutable raw objects, previews and copied gallery pages use
  additional disk. Metadata retrieval is separately bounded to 1 MiB per source.
- Optional `metadata_url`: public HTTPS URL for missing original metadata.
  Existing metadata is preserved. Credentials/query strings/fragments are refused.
- Optional `existing_receipt`: reuse an imported/acquired source after checking
  all raw objects, source/metadata identities and the byte limit.
- Optional `preparation_profile`: source assessment JSON path. Batch publication
  pins its digest. The [profile contract](../docs/design/automatic-preparation-v1.md)
  defines evidence and scope; no profile means no automatic qualification.
- `crops`: zero or more objects with unique `id` within that source, `title`
  (1–240 UTF-8 bytes), `window_cells: [x,y,width,height]` and `primary_class_id`.
  Windows are native cell offsets, with sides 1–2,048. Exact source-grid bounds
  are checked during review; active class IDs are checked during gallery build.

`batch validate` reads declarations without downloads. `batch download` pins the
queue bytes, absolute source paths, source hashes and runtime identity in
`request.json`. Reusing that run directory with changed inputs fails.
Original metadata is pinned before an asset attempt, including failed attempts;
metadata fetched for initially absent inputs gets its own immutable identity.
`downloads/<source-id>.json` pins a completed raw receipt. Verified completions
skip acquisition; interrupted transfers reuse the downloader's checked chunks.
Independent failures are reported while other sources continue.

`batch review` consumes completed downloads, verifies or creates each crop, and
writes `gallery-input.json`. It includes all successfully verified crops, even
when another fails. A partial run exits 1. `download-status.json`,
`review-status.json` and `gallery-input.json` are atomically replaced operational
summaries; source receipts, review bundles and published galleries are immutable.
Resume preserves completed crops. A damaged completed crop is reported and never
silently overwritten. A new run directory expresses an intentional queue revision.
Configured profiles also stage automatic technical checks. Their exceptions are
reported in `preparation-status.json` without discarding successful review crops.

## Gallery

Input requires `schema_version: 1`, `title`, and `tiles`. Each tile requires `id`,
`title`, `raw` receipt path, `review` manifest path and `primary_class_id`. Duplicate
IDs or review identities fail. The current metadata reader limits a manifest to
1 MiB; use smaller batches if a very large input exceeds that limit.
An optional `preparation_profile` accepts a path or `{path,sha256}`. Batch output
uses the pinned form; build rejects a changed profile. Published tiles retain
the pinned form. Old galleries without this field remain supported.

`gallery build` verifies review bundles and publishes `gallery.json`, the pinned
legend, exact copies of previews and small derived hillshade PNGs. The gallery
pins raw/review identities, preview hashes/sizes and thumbnail hashes. Display
thumbnails are not authoritative elevation. Existing source bytes are unchanged.

`gallery serve` checks the gallery, binds IPv4 loopback, prints its URL and runs
until interrupted. Port 0 requests an available port. The HTTP surface consists
of `/`, `/api/tiles`, `/preview/<index>`, `/thumb/<index>`, and JSON POST
`/api/decision`. Other paths are not filesystem routes. Writes require exact Host,
Origin, content type and a per-session token; no CORS access is offered. Request
headers/body are bounded to 8/4 KiB, with read/deadline limits. Preview iframes
allow scripts but have an opaque origin. A small served bridge enables shortcuts
and reports the displayed identity before voting is enabled.

Startup verifies each distinct raw receipt once and retains verified crop
snapshots. Browsing and votes verify the journal and its captured evidence against
those snapshots, without rereading complete raw rasters on each click. Source
integrity is checked again on restart and by the unchanged numeric admission and
preparation consumers. A visual vote concerns the captured display; it is not a
new claim that raw storage remains intact throughout the session.

A vote supplies gallery/review SHA-256, tile index, expected previous curation
head (or null), and `accept` or `reject`. The server verifies the displayed
artifacts and appends through the existing curation consumer. It retains prior
technical claims, class and evidence. Repeating the current choice does not add
another record. A stale head fails with HTTP 409; refresh before deliberately
retrying. Human identity is a local declaration, not remote authentication.
An automatic technical successor of the exact displayed human head is recognized
when saving a vote; competing human decisions still conflict.
Selection is neither technical qualification nor training admission. No deletion
or automatic acceptance occurs.

The service starts one sequential preparation worker. `/api/tiles` adds
`preparation: {state,message,manifest}` to each tile. States are `checking`,
`awaiting_acceptance`, `preparing`, `ready`, `needs_attention`, `not_selected`.
`manifest` is a local prepared-manifest path only when ready. Votes are independent
of raster work; the page polls status without reloading the selected preview.
Readiness requires verified products and current curation parents, including
parents outside this gallery. Restart rechecks artifacts and retries exceptions
once; operational status never authorizes admission.
