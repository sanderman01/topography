# Versioned catalog inputs

Keep small, reviewed descriptions here. The local catalog database belongs in
`data/catalog/`; rasters and original provider responses belong in `data/raw/`.

- `providers/`: provider capabilities, collection identifiers, access methods,
  and metadata mappings. Add adapters only as needed for the pilot.
- `areas/`: area-of-interest polygons, terrain family/subtype, feature dimensions,
  coverage findings, flight value, and accepted crop/label decisions. GeoJSON is
  suitable for geographic polygons; make coordinate conventions explicit.
- `labels/`: versioned integer class legends, colors, and precedence rules.
  Start from the master plan's nine provisional classes. Never reuse retired IDs;
  keep water and validity separate from terrain classes.
- `candidates/`: source descriptions used for bounded acquisition trials, with
  explicit unknowns. Copy them beside the retained metadata before acquisition.

The initial [candidate legend](labels/terrain-v1.toml) is now loadable with
`topography validate legend`; see the [v1 contract](../schemas/legend-v1.md).
Class precedence remains unset. Source descriptions have a
[v1 contract](../schemas/source-v1.md) and read-only validator. The
[acquisition workflow](../docs/workflows/dem-acquisition.md) writes discovery
candidates and immutable raw receipts. The
[curation workflow](../docs/workflows/source-curation.md) persists crop decisions,
labels and evidence in SQLite. Native regional preparation is implemented;
training releases remain pending. Discovery bounds do not establish valid-ground
coverage.

Candidate locations in `docs/locations.md` and the master plan remain research
leads until their exact source footprints and quality have been checked. Do not
mark a provider or dataset as verified simply by adding a description here.

[Río Abajo crop envelopes](areas/rio-abajo-crops.geojson) record the three
native-grid windows from the [karst trial](../docs/reports/rio-abajo-karst-trial.md).
They are longitude/latitude GeoJSON raster footprints, not accepted crop labels
or measured terrestrial coverage. Their terrain class is a candidate annotation.

[Batch 2 crop envelopes](areas/pilot-batch-2-crops.geojson) and the source examples
under `candidates/pilot-batch-2/` describe six additional whole-tile candidates.
See the [batch report](../docs/reports/pilot-tile-batch-2.md) for previews and
reproduction. Tiles 1–5 are now selected and prepared; tile 6 is excluded without
deletion. [Batch 3](../docs/reports/pilot-tile-batch-3.md) adds four new candidates
under `candidates/pilot-batch-3/` with [crop envelopes](areas/pilot-batch-3-crops.geojson)
and four human acceptances, all qualified and prepared. The
[numeric pilot proposal](../docs/reports/numeric-pilot-proposal-v1.md) keeps the
six source areas separate before any training-window extraction; it is not an
approved training release.

Dataset admission must record confirmed training permission and its supporting
clause or confirmation, as required by the master plan. Credentials never belong
in these files. Portable catalog exports should reference artifact checksums and
preserve approval decisions and lineage.
