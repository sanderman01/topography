# Regional quality and preparation eligibility

Use this after [native review](dem-acquisition.md) and alongside
[crop curation](source-curation.md). The CLI measures classifications over an
entire native review footprint and checks the current preparation gates. Assessment does
not resample, repair, mosaic, publish prepared data or release training data.
Eligibility is a snapshot; [aligned preparation](aligned-preparation.md) repeats
the check when it runs and before registering its output.

## Pin and classify the crop

Start with the raw receipt and the exact crop you inspected:

```sh
topography quality draft data/raw/records/RECEIPT_SHA256.json data/review/candidate/review.json data/review/candidate/quality.json
```

The draft pins both inputs. Its four masks start as `null`. Supply one
single-band Byte GeoTIFF per mask, covering the entire review grid:

| Mask | Values | Meaning |
| --- | --- | --- |
| `water` | 0 / 1 / 2 | Unclassified / terrestrial / water |
| `quality` | 0 / 1 / 2 | Unknown / supported ground / unsupported ground |
| `repairs` | 0 / 1 | Unrepaired / repaired |
| `features` | 0 / 1 | Outside / inside a defining landmark or passage |

These classifications require an explicit evidence basis. The pilot uses
[supplier_qa_pilot_v1](../design/source-quality-pilot-v1.md): published product QA
supports finite provider-valid ground unless a concrete defect is identified.
Provider validity or occupied LiDAR bins alone are not that evidence basis.
Keep unknown water as 0;
classifying uncertain ground as water conceals it from the terrestrial denominator.
Feature annotations are optional. Use an all-zero feature layer when none were
requested; do not ask the owner to select individual ridges or passages. If the
owner requests annotations, cover each full feature rather than a representative
point. The implementer supplies masks and evidence; human tile review stays yes/no.

A crop with no identified water can have an entirely terrestrial water mask;
record the evidence and review supporting that classification. The presence of
a required water mask does not imply that the crop contains water or requires
hydrographic vector downloads. Source QA polygons are optional evidence when
publicly downloadable and relevant to the crop. Empty ground-return bins alone
identify neither water nor invalid DEM elevations.

Author masks in a GIS using the review's exact dimensions, transform and CRS.
Use nearest-neighbour operations for categorical data. Remove nodata and hidden
validity masks: every cell must carry an explicit category, including zero. The
CLI rejects shifted, differently sized, differently referenced, scaled, non-Byte
or partially masked inputs. It does not align them automatically.

Replace each corresponding `null` in `quality.json` with an object:

```json
{"path": "water.tif", "sha256": "COPY_THE_64_CHARACTER_SHA256_OF_WATER_TIF"}
```

Use `sha256sum data/review/candidate/water.tif` for the actual digest. Paths
resolve beside `quality.json`. Keep masks and supporting evidence with the crop;
the small curation export does not contain raster files. Back them up separately.

## Measure and review the evidence

```sh
topography quality assess data/review/candidate/quality.json data/review/candidate/quality-report.json
```

A completed assessment can report blockers. Missing masks yield `counts: null`.
When all four masks exist, a valid terrestrial cell must be land, supported by
the quality classification, provider-valid and finite. At least 99.5% of land
must meet those conditions. All water must be classified, and every marked
feature cell must be valid terrestrial ground. Zero marked features is valid
under `native_quality_v2`. Nonfinite provider-valid land blocks eligibility; water heights
are excluded. No elevation bytes are changed.

Any marked repairs block this first version. Repaired elevation needs before/after
lineage and a reviewed repair policy, which are not implemented. This is not a
zero-repair policy for the finished project.

The report contains input identities, mask hashes, grid, counts, blockers and
tool versions. It has no timestamp or local locator paths: unchanged inputs and
tool versions produce identical report bytes. Choose a new output filename for
each changed assessment; existing reports are preserved.

Add the report as captured evidence in a fresh curation draft. Reference its
evidence ID in `ground_support`, `artifacts_and_repairs`, `water_classification`,
`unsupported_feature_gaps` and `coverage`. Copy the measured terrestrial counts
into the coverage claim. Include supporting point-cloud/reference inspections,
coordinate evidence and dataset-specific permission documents as well. The report
checks classifications; it cannot establish their truth or the legal meaning of
permission documents. A human must review and record their own decision.

## Check the current decision

Use the immutable record ID printed by `curate record`, not the review hash:

```sh
topography prepare check configs/runtime.example.toml data/review/candidate/quality.json CURATION_RECORD_SHA256
```

This recomputes the assessment, verifies the curation history, requires the exact
current record, and checks that the decision references the resulting report
hash. The current human decision must accept the crop, assign an active class,
pass all six findings, confirm dataset-specific training permission with captured
evidence and obligations, and agree with measured coverage. Source provenance,
bare-earth meaning, units, datum and native 0.5–1 m metric spacing are required.
The source's declared `EPSG:code` must match the raster's horizontal EPSG identity;
other CRS declarations are conservatively unsupported in this first consumer.
Synthetic sources remain excluded. No secondary 2 m exception is implemented.

JSON goes to stdout. Exit 0 means `eligible: true`; exit 1 with JSON means blocked.
Malformed, corrupt or stale inputs return exit 1 with a diagnostic on stderr.
Every result retains `admission: not admitted`. A later annotation or rejection
supersedes acceptance. No catalog decision or prepared raster is created.
Historical curation v1 blockers remain unchanged in old records.

## Río Abajo and workload bounds

All three current Río Abajo decisions retain the owner's whole-tile yes and
confirmed dataset permission. They now pass all preparation gates under the
adopted supplier-QA pilot rule; see the
[qualification checkpoint](../reports/rio-abajo-source-qualification.md).
Latest complete drafts and assessments are under
`data/review/rio-abajo-supplier-qa-pilot-v1/{north-hollows,east-ridges,south-saddles}/`.
Each has 1,048,576 usable terrestrial cells, all-terrestrial water classification,
zero optional feature annotations and zero project repairs. All three regions
are prepared; no training dataset is released. The underlying supplier
interpolation uncertainty is retained in captured provenance and does not require
another measurement or confidence decision from the owner.

Before whole-crop LiDAR processing, specify the actual footprint, source identity,
buffer, concurrency and memory/storage bounds, then obtain workload readiness
under [AGENTS.md](../../AGENTS.md). Prior sample readiness does not cover that run.
Use the LiDAR header's horizontal CRS; do not compare point and DEM heights until
the GEOID18/GEOID12B difference is resolved.

Assessment is limited to existing reviews, at most 2,048 cells per side and
128 MiB per input mask. The implementation reads bounded native bands and runs
GDAL sequentially with a 64 MiB cache, one worker and temporary ENVI files. These
are component bounds, not a guaranteed peak RSS. No point-cloud or GPU job runs.
Raster inspection and decoding follow the GDAL
[gdalinfo](https://gdal.org/en/stable/programs/gdalinfo.html) and
[ENVI driver](https://gdal.org/en/stable/drivers/raster/envi.html) interfaces.

See the [v1 contract](../../schemas/regional-quality-v1.md) and
[verification commands](../../scripts/README.md). The
[aligned preparation workflow](aligned-preparation.md) now publishes bounded
same-survey native regions after repeating these checks. The
[whole-crop ground-support probe](whole-crop-ground-support.md) completed the
approved Río Abajo measurements. Repaired revisions,
cross-survey alignment and authored water surfaces remain future processing work.
