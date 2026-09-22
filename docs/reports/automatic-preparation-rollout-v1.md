# Automatic preparation rollout

2026-09-22. The batch tool now stages technical checks from reusable, pinned source
profiles. The gallery saves Yes immediately and runs native preparation on one
background worker. It reports progress, verified readiness or a concrete technical
exception without asking for another selection. No preserves the source and
excludes prepared products from use. Training admission remains separate.

Gallery v10 contains the same 35 terrain tiles and retains **33 Yes and 2 No**.
**30 tiles are ready**. Both Badlands crops prepared automatically from their
existing Yes decisions. The two Great Sand Dunes tiles retain unresolved slope
striping; the previously blocked St. Helens glacier tile has no automatic source
assessment. Those three accepted tiles remain available for inspection and need
technical attention, not another human vote.

The Badlands profile captures the public USGS TNM metadata, complete extracted
Block 5 supplier mapping report, delegated pilot policy, previous exact native
comparison and a scoped terrestrial assessment. It preserves native Float32
elevation, the EPSG:26913 grid and NAVD88/GEOID12B heights. All 8,388,608 selected
cells remain provider-valid. Quality masks use the supplier-QA pilot rule; project
repair and optional feature masks are zero. The two terrestrial windows are
explicitly bounded. No smoothing, resampling or new point-cloud check was applied.
The dunes profile records the known striping as a blocker rather than passing it.

The rollout preserves all **75 original curation records** and all **28 original
prepared entries**. The same **38 review identities** remain in the catalog; only
the two Badlands heads advance. Two separately attributed
automatic successors carry forward the exact human actor, Yes, reason and class;
the catalog now has 77 records and 30 prepared entries. Original records remain
byte-identical. The twelve-crop numeric release is unchanged.

Local evidence is retained under `data/review/automatic-preparation-rollout-v1/`:

- `evidence-downloads.json` and its status pin the two public report downloads.
- Source subdirectories contain profiles and captured evidence.
- `catalog-before.sqlite3`, `before.json` and `preservation.json` retain the audit.
- `gallery-state.json` and `gallery.png` capture the deployed result.
- `check.log`, `automatic-final-check.log` and `browser-final-check.log` record
  implementation verification; `restart-check.json` confirms real output reuse.

`bash scripts/check.sh` passed, including formatting, build, Clippy and the local
contract/raster/data workflows. The focused automatic-preparation workflow also
checks rejection during publication, preserved permission restrictions and failed
findings, stale human decisions and restart reuse. Chromium exercised the synthetic
gallery's vote controls and inspected the live gallery without voting. The live
curation integrity check found no unreferenced objects or unregistered records.
Restarting the deployed gallery reused all 30 products without adding records.

The gallery manifest is `data/review/library-gallery-v10/gallery.json`, SHA-256
`ef6d699548bd2294163a14dc98326ec1b0a30333ca35168b16c9c1b1856a0379`.
Restart with:

```sh
target/release/topography gallery serve configs/runtime.example.toml data/review/library-gallery-v10/gallery.json 8765
```

The [design](../design/automatic-preparation-v1.md) and
[workflow](../workflows/batch-source-review.md) describe profile scope, failure
handling and recovery. Source QA interpretation remains an implementer task;
each additional tile can reuse an applicable assessment. Unknown or restricted
permission, unknown water and failed findings cannot be promoted automatically.
