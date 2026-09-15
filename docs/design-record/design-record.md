# Design record

## 2026-09-15 — Repository scaffold

Use a Rust workspace with a CLI crate and a core library. Reserve the planned
pipeline boundaries as documented modules within the core crate; separate
provider crates and model integrations can be introduced when implementation
requires them.

Keep Python training/evaluation in `training/` behind the planned manifest batch
boundary. The scaffold adds no model or geospatial dependencies.

Separate small versioned catalog inputs, configuration, and future schemas from
ignored local artifacts under `data/`. Track placeholders so fresh checkouts
retain the storage layout. Draft world presets reproduce the master plan's
pilot, recommended extent, and exact 32 km alternative; configuration validation
and the remaining physical contracts are still implementation work.

The existing master plan and research notes remain the project references.
