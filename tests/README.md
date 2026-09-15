# Verification layout

- Rust integration tests belong in each crate's `tests/` directory.
- Python tests belong in `training/tests/`.
- `fixtures/` holds small synthetic inputs shared across components, such as
  elevation ramps, masks, and manifest examples. Keep real survey data in `data/`.

No formal test suite is configured. Current verification builds and runs the CLI
through `bash scripts/check.sh`, including valid and invalid contract fixtures in
`fixtures/contracts/`. These tiny inputs do not represent real terrain or verify
source admission. `fixtures/acquisition/` adds a synthetic ramp with a missing
cell, source metadata and a simulated TNM response for local/HTTPS workflow
verification. See [the script documentation](../scripts/README.md).

As functionality is implemented, verify complete user workflows and the master
plan's physical invariants: preserved metres and masks, consistent coordinates,
geographic split isolation, bit-identical shared export borders, unchanged cells
outside edit domains, and reproducible artifact lineage.
