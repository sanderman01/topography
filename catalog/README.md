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

The initial [candidate legend](labels/terrain-v1.toml) is now loadable with
`topography validate legend`; see the [v1 contract](../schemas/legend-v1.md).
Class precedence remains unset. Source descriptions have a
[v1 contract](../schemas/source-v1.md) and read-only validator. Catalog persistence,
coverage discovery and source admission are not implemented yet.

Candidate locations in `docs/locations.md` and the master plan remain research
leads until their exact source footprints and quality have been checked. Do not
mark a provider or dataset as verified simply by adding a description here.

Dataset admission must record confirmed training permission and its supporting
clause or confirmation, as required by the master plan. Credentials never belong
in these files. Portable catalog exports should reference artifact checksums and
preserve approval decisions and lineage.
