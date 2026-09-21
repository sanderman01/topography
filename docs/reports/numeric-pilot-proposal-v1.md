# First numeric pilot proposal

Prepared on 2026-09-21. **Proposed, not approved or released.** The owner has
selected 12 source tiles; one prior tile remains rejected. All selected tiles are
qualified and prepared under [supplier_qa_pilot_v1](../design/source-quality-pilot-v1.md).
The next checkpoint is approval of the collection and geographic grouping for
numeric evaluation, not another visual review of those tiles.

Open the local [proposal page](../../data/review/pilot-dataset-proposal-v1/review.html).
The exact candidate is `data/review/pilot-dataset-proposal-v1/proposal.json`, SHA-256
`72687a180799c3796c7c95ad8543ce047db8561d19e276647b79e4415a9f01ed`.
`review-identity.json` pins the page and probe as well; its decision is unset.

## Collection and geographic roles

| Area | Broad candidate family | Role | Selected tiles |
| --- | --- | --- | ---: |
| Río Abajo | Karst, class 1 | Calibration | 3 |
| Central gorge | Gorge, class 2 | Calibration | 2 |
| Southern floodplain | River/floodplain, class 9 | Calibration | 1 |
| Camuy | Karst, class 1 | Validation | 2 |
| Guajataca | Gorge, class 2 | Validation | 2 |
| Arecibo | River/floodplain, class 9 | Validation | 2 |

Keep all crops, later augmentations and alternate resolutions of one area in its
assigned role. The two sides contain six tiles each, representing all three
families. No overlapping training windows have been extracted. The local proposal
pins every current source decision, prepared manifest, native grid and footprint.

The 12 disjoint 512 × 512 m footprints total 3.145728 km², including 3.0557355 km²
of classified usable land. They cover six geographically separate areas, all
within one Post-Maria provider project; shared acquisition/processing remains a
source of correlation. Crop counts do not establish independent survey diversity.

The minimum calibration/validation footprint-edge distance is 8,042.645 m in
EPSG:6566. Every opposing pair exceeds the proposed 512 m buffer for a maximum
512 m physical evaluation context. The exact pairwise measurements are retained
in `geographic-separation.json`. Larger contexts require recalculation: this
grouping would fail an 8,192 m buffer for the planned coarse training level.

This is a small **numeric evaluation pilot**, not a full train/validation/test
release. Two areas per family support calibration and validation; there is no
untouched final test set, no trained model and no generalization claim. The small
native regions also cannot supply full 8,192 m coarse training windows. Address
those needs when a concrete model experiment requires them; do not expand source
review indefinitely before implementing the numeric consumer.

## Preliminary representation measurements

A local CPU probe read all prepared layers with GDAL and used NumPy for scalar
arithmetic. Settings were written before the measurement. Normalization is the
fixed experimental transform `encoded = height_m / 2048`, with inverse
`height_m = encoded * 2048`, over a declared −2,048 to +2,048 m range. It was not
fitted separately to tiles, and it does not choose the final world/export encoding.

Only valid terrestrial samples enter height metrics. Slope-component and terracing
metrics require both neighboring cells to be valid; masked water fill is excluded.
All real values were within the experimental range, without clipping. Synthetic
64² ramps, flats and sharp/narrow ridges supplemented the real regions.

| Representation | Observed result |
| --- | --- |
| Float32 scalar loading and normalized storage | All 12,222,942 valid real samples and every synthetic sample round-trip bit for bit; zero height/slope error and no newly flat adjacent pairs |
| Float16 normalized storage | Up to 0.25 m absolute height error on real terrain; measurable slope changes and newly flat neighboring samples |
| Float16 synthetic narrow ridge | A one-cell-wide 0.125 m ridge at 1,800 m disappears; its 128 previously nonflat neighboring pairs become flat |

Retain Float32 for authoritative heights and the next loader evaluation. These
measurements do not reject every use of mixed precision inside a future model;
they demonstrate loss when normalized absolute heights themselves are stored in
Float16. No autoencoder or learned model was tested. Numeric acceptance tolerances
and vehicle-clearance requirements remain unset.

The run took 9.1 seconds, used approximately 146 MiB peak Python-process RSS and
no GPU. That memory figure excludes the child-process tree. Reproduction from the
repository root uses the retained local probe:

```sh
python3 data/review/pilot-dataset-proposal-v1/measure.py
```

It requires the existing GDAL CLI and NumPy (this run: GDAL 3.13.3, NumPy 2.5.3,
Python 3.14), reads the pinned prepared artifacts and writes local descriptive
reports. Preserve the current reports before another run because this temporary
measurement script is not an immutable job runner. The code and helper hashes,
software versions, settings and exact results are retained in `numeric-probe.json`.
This local probe does not implement or complete the planned production Python
loader/batch interface. No ML environment or model dependencies were installed.

## Decision and next implementation

The [master plan, section 9](../plans/terrain-design-master-plan.md#9-human-editing-and-partial-reruns)
retains a separate dataset-release checkpoint. The requested decision is whether
to use this exact collection and grouping for the first numeric evaluation pilot.
An approval will be captured against the pinned proposal/page identities. It will
not invent approval of training, a model, a gameplay tolerance or generated terrain.

Dataset release tooling and the production numeric job remain unimplemented.
After this checkpoint, add their versioned producer/consumer boundary, rechecking
current prepared parents, geographic roles, paired layers, masks, fixed
normalization and complete output publication. Use the local probe as measured
reference. Source selections remain valid without repeated yes/no questions.
