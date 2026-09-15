# Verification layout

- Rust integration tests belong in each crate's `tests/` directory.
- Python tests belong in `training/tests/`.
- `fixtures/` holds small synthetic inputs shared across components, such as
  elevation ramps, masks, and manifest examples. Keep real survey data in `data/`.

No formal tests are included in the scaffold. Current verification builds and
runs the CLI through `bash scripts/check.sh`.

As functionality is implemented, verify complete user workflows and the master
plan's physical invariants: preserved metres and masks, consistent coordinates,
geographic split isolation, bit-identical shared export borders, unchanged cells
outside edit domains, and reproducible artifact lineage.
