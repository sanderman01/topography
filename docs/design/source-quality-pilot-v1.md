# Pilot source quality: supplier_qa_pilot_v1

Adopted 2026-09-21 under the owner's instruction to use implementer judgment and
keep qualification simple. This is the evidence basis for classifying released
native bare-earth DEMs during the pilot. `native_quality_v2` remains the separate
implemented mask/grid/coverage validator; no CLI gate or numeric threshold changes.

Use the provider's published product QA and documented processing as the baseline.
For a traceable native 0.5–1 m bare-earth product with known metre units and datum,
classify finite, provider-valid terrestrial cells as supported unless an inspected
source document or local check identifies a concrete defect. Retain known defect
exclusions in the quality mask. Require the existing 99.5% usable terrestrial
coverage, confirmed dataset permission and a whole-tile human yes.

Routine supplier interpolation beneath canopy is acceptable. Sparse or empty
LiDAR count bins alone are not defects and do not require additional measurements,
confidence thresholds or human review. Retain those diagnostics and the supplier's
limitations as provenance. Do not claim that nominal grid spacing or this policy
proves measured fine-feature accuracy. Demonstrated large voids, incorrect surface
type, artifacts, invalid values or incompatible coordinates still require exclusion
or a different source. Missing supplier QA polygons are a recorded limitation,
not a mandatory acquisition or correspondence task.

The implementer supplies technical findings and masks. A whole-tile yes remains
selection by the owner; it is not authorship of the implementer's technical findings.
Use an empty feature mask when no optional annotations were requested. Zero project
repairs means the project preserved native elevations, not that the supplier never
interpolated them. Water classification remains separate from elevation and quality.

Capture this policy's exact bytes, the delegation, source QA references and the
per-crop qualification with each current curation record. Prepared manifests pin
those records and the exact assessments; retain their portable evidence exports.
Future dataset releases must carry that evidence basis and apply geographic splits.
Preparation alone does not release training data or approve generated terrain.

For the initial Río Abajo crops, use the existing reviewed native terrain,
documented USGS/NOAA release, complete sampling diagnostics and supplier report.
No concrete local defect requiring exclusion has been identified in those checks.
Keep all three selections and their native Float32 values; no new point-cloud
measurement or point/DEM height comparison is required.
