# Candidate legend contract v1

Validate [the initial legend](../catalog/labels/terrain-v1.toml) with:

```sh
cargo run --locked --offline -p topography-cli -- validate legend catalog/labels/terrain-v1.toml
```

The executable TOML schema and semantic checks are in
[`catalog/legend.rs`](../crates/topography-core/src/catalog/legend.rs). All fields
are required except `precedence`. Unknown fields fail at every level.

| Field | Constraint |
| --- | --- |
| `schema_version` | Exactly 1; describes file shape |
| `legend_version` | Positive UInt32; identifies the vocabulary/color revision |
| `status` | `candidate`; this consumer does not freeze a production vocabulary |
| `retired_ids` | Unique positive UInt16 IDs, retained permanently |
| `precedence` | Optional list of all active IDs exactly once, highest priority first |
| `classes` | Nonempty array of tables with the fields below |
| `classes.id` | Unique positive UInt16, absent from retired IDs |
| `classes.name` | Nonempty and unique after trimming and lowercasing |
| `classes.color_rgb` | Unique triple of UInt8 display values |
| `classes.flight_experience` | Nonempty description of the experience to seek |

IDs 1–9 follow the master plan. RGB values make its provisional color names
concrete for display; integer IDs carry labels. Water and validity are separate
masks. Zero is unassigned, never an implicit water class. Secondary character
belongs in separate controls.

Precedence stays unset pending overlap review; future labeling must resolve it
before assigning exclusive classes. Candidate locations remain acquisition leads.
The legend makes no coverage claims.

Never recycle retired IDs. Carry forward the retirement list, increment the
legend version for vocabulary/color/precedence changes, and pin that version in
future dataset, model and world revisions. Compare prior versions during review;
this single-file validator detects current collisions but cannot prove historical
non-reuse. Migrations must preserve accepted past outputs.
