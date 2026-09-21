# Batch source workflow v1

The owner requested target lists instead of per-file agent downloads, and one
fast gallery instead of separate review links. Minimize repeated orchestration
and human effort while preserving source identities and earlier decisions.

## Chosen approach

Extend the existing Rust acquisition and curation consumers. A versioned JSON
queue names source TOMLs, optional public metadata URLs, byte limits and native
crop windows. One command processes the queue sequentially, resumes verified
successes, continues independent failures and prints a compact summary. The
existing downloader may invoke curl internally; the agent does not drive each
HTTP request. No new provider abstraction or parallel download scheduler.

A generated gallery manifest pins raw receipts, review manifests, previews,
thumbnail images and the legend. One loopback-only Rust service presents a
thumbnail grid and an inspector with the existing terrain preview. Yes/No buttons
and Y/N shortcuts save directly through the curation journal, then advance.
Arrow keys navigate; filters show pending, accepted, rejected or all tiles.
Existing votes are shown without requesting another vote. A reload retains votes.

A static gallery with downloaded vote files would require a manual import step;
a hosted app would add deployment and account management. The local service
keeps the requested interaction simple and uses the existing on-disk catalog.

## Boundaries and recovery

Queue paths resolve relative to its JSON. Queue identity pins source TOML and
metadata input identities; changed inputs cannot reuse an old run directory.
Per-item results are immutable. Reuse verifies raw/review integrity and exact
crop windows; failure does not erase other work. Downloads are sequential and
bounded by per-source limits and a declared aggregate source-byte budget. Crop
processing remains a separate command so downloads do not implicitly authorize
large raster jobs. Optional metadata retrieval is bounded to 1 MiB per source.

Only an explicit browser Yes/No request records a human selection. Processing,
opening a tile or navigating never accepts it. Decisions include exact gallery
and review identities, use the expected current catalog head, and retain technical
evidence. A stale decision fails visibly instead of overwriting newer work.
Rejection preserves source bytes. Visual selection never grants technical or
training admission. Existing source metadata, supplier QA and permission gates
continue to apply.

Verify each distinct raw source once when opening a gallery session. Retain the
verified crop metadata and display snapshots; browsing and voting check journal
and evidence integrity without rereading complete raw rasters. Votes certify only
the inspected snapshot. Numeric qualification/preparation still perform their
full source-integrity checks. Restarting the gallery verifies sources again.

The server binds 127.0.0.1 only, serves an explicit allowlist, requires same-origin
JSON writes with a session token, and bounds request size/time. Terrain previews
run in a sandboxed iframe; metadata is escaped. No arbitrary filesystem routes,
remote services, email, account, new JavaScript framework or GPU dependency.

## Delivery

Implement queue validation/download/resume, batch crop preparation and gallery
publication, then the gallery interface and durable decisions. Exercise synthetic
success/failure/resume/stale-input cases through the public CLI and HTTP boundary.
Use the tooling on retained real terrain without changing its existing votes,
then use the same queue/gallery path for subsequent source expansion. Preserve
whole-tile review and ask readiness only for new substantial workloads.
