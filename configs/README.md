# Configuration inputs

These TOML files are draft examples derived from the master plan, not a frozen
configuration contract. The current CLI does not parse or validate them.

- `runtime.example.toml`: local storage paths and the fixed pilot GPU budget.
- `worlds/pilot.toml`: the 4,096 m pilot world.
- `worlds/standard.toml`: the recommended 32,768 m world.
- `worlds/exact-32km.toml`: the alternative 32,000 m world.
- `local/`: ignored machine-specific overrides and local paths.

Example paths are relative to the repository root. Future configuration loading
must define path resolution explicitly and allow storage outside the checkout.
Use environment variables or an appropriate credential store for account secrets.

World extent is derived from cell count multiplied by spacing; do not accept
independent contradictory values. Training windows, inference windows, and export
tile sizes have separate purposes. The 512-cell layout is only a painting sketch;
the authoritative ownership map needs finer boundaries.

Export tiles cover 1,024 cell intervals and contain 1,025 edge-inclusive vertices
per full side. The exact 32 km preset has a partial final tile (256 intervals);
edge-tile handling must be specified and validated when export is implemented.
The inference window sizes are proposed settings to benchmark within 16 GB VRAM.

Vehicle dimensions, clearance margin, flight speed, turning space, climb rate,
height encoding range, biome precedence, and numerical acceptance tolerances
still need pilot decisions. They are deliberately not assigned invented values
in these examples.
