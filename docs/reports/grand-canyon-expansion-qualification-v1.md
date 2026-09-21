# Grand Canyon expansion qualification

2026-09-21. The three recovered owner-accepted Grand Canyon expansion crops are
technically qualified and prepared under `supplier_qa_pilot_v1`. The existing
owner Yes records remain the human selection evidence; Codex authors the
technical findings, masks and qualification requests. No feature annotations,
LiDAR confidence review, project repairs or ground-elevation changes were introduced.

All crops use raw receipt `11afe2ce...e7fc4e7f`, immutable asset
`84b35005...ed92`, native 1 m EPSG:26912 Float32 elevations and NAVD88 metre
heights. Existing comparison evidence verifies all 8,912,896 source/review
samples and provider-validity cells exactly. The already-qualified adjacent tile
supplies the same work-unit QA and public-domain evidence; source identity was
updated to `USGS_1M_12_x40y400_AZ_GrandCanyonNP_2019_B19`.

## Water classification

The official USGS National Hydrography Dataset service was queried once for the
combined crop envelope. The retained service description says the data are open,
non-proprietary and public domain, with no use constraints, and records a July
2026 refresh. Its large-scale polygon layer identifies the perennial Colorado
River (`FTYPE=460`, `FCODE=46006`) and mapped rapids (`FTYPE=431`) within the two
river crops. The polygon agrees closely with the flat river surface visible in
the native hillshades.

Only polygonal water corroborated by the terrain is assigned category 2. NHD
flowlines, intermittent channels and uncertain dry terrain remain land rather
than receiving an invented width. NHD's mapping scale and temporal limits mean
the mask is a defensible categorical surface-water classification, not a
one-metre shoreline measurement or bathymetry.

| Crop | Grid | Terrestrial / valid | Water | Finding |
| --- | ---: | ---: | ---: | --- |
| Northwest layered cliffs | 1,536² | 2,359,296 / 2,359,296 | 0 | No polygonal or visible open water |
| Central river corridor | 2,048² | 4,115,267 / 4,115,267 | 79,037 | Colorado River polygon |
| Southeast river bend | 1,536² | 2,244,906 / 2,244,906 | 114,390 | Colorado River and contained rapids polygons |

All assessments have zero unclassified-water, unsupported-terrestrial,
provider-invalid-terrestrial, nonfinite-terrestrial, repaired, defining-feature
and unsupported-feature cells, and no blockers. Quality is supported for every
terrestrial cell under the adopted policy. Feature and repair masks are empty.

The successor curation records are `f7a38fca...908bb7` (northwest),
`baf8d810...010416` (central) and `1797c2bd...dfecb8` (southeast). Their prepared
manifests are `ba59c376...946f3`, `22c7fc52...66a6b` and
`a66f7114...816c`. Targeted public-CLI verification reports every manifest
current, registered and integrity-verified; preparation alone leaves training
admission unset.

Independent full-sample decoding compared all prepared outputs. Every one of
8,719,469 terrestrial Float32 cells is bit-identical to its native review value;
all 193,427 water cells contain the deterministic zero fill. Prepared validity,
provider-validity, water, quality, repairs and feature masks exactly match their
expected values for every cell.

## Retained staging and resource bounds

Evidence, clipped vectors, pinned masks, quality assessments, curation requests,
prepare templates and the two-phase script are under
`data/review/grand-canyon-expansion-qualification-v1/`. The source query retained
about 0.86 MB of official service responses; the complete qualification directory
is about 9.2 MB. Raster work was sequential, limited to one 1,536² or 2,048² crop at
a time, with `GDAL_CACHEMAX=64`, `GDAL_NUM_THREADS=1` and no full-tile processing.

Staging is reproducible without catalog writes:

```sh
python3 data/review/grand-canyon-expansion-qualification-v1/qualify.py stage
```

The serialized apply phase records the three successor curations, checks their
eligibility, prepares them, verifies the manifests and exports portable evidence:

```sh
python3 data/review/grand-canyon-expansion-qualification-v1/qualify.py apply
```

The filesystem-only full-sample verification is reproducible with:

```sh
python3 data/review/grand-canyon-expansion-qualification-v1/qualify.py verify-numeric
```

Preparation does not create a training release or approve generated terrain.
