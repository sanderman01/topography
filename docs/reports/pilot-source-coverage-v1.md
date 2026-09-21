# Pilot source coverage

As of 2026-09-21, the library has **20 accepted, prepared patches**: twelve
512 × 512 m crops and two 1,024 × 1,024 m contexts at native 0.5 m spacing,
plus three Banks Peninsula and one Grand Canyon crop at 1,536 × 1,536 m,
and two Mount St. Helens crops at 2,048 × 2,048 m, all native 1 m.
Two rejected tiles remain preserved and excluded. The selected St. Helens crater
rim is separately blocked from ground preparation because it contains glacier
surface. Five additional Grand Canyon/Zion crops now have owner Yes decisions;
their technical qualification remains pending. See the
[vote recovery checkpoint](gallery-vote-recovery-v1.md).

Nine named areas now span Puerto Rico, Banks Peninsula, Grand Canyon and Mount
St. Helens. The two US mainland survey projects supplement the Puerto Rico
collection and Banks survey. Multiple crops, survey subareas and metadata-only
successor records do not establish independent projects or extra terrain patches.

| Broad pilot family | Areas | Accepted patches |
| --- | --- | ---: |
| Karst | Río Abajo, Camuy | 6 |
| Gorge | Central gorge, Guajataca, Grand Canyon | 6 |
| River/floodplain | Southern floodplain, Arecibo | 3 |
| Eroded volcanic terrain | Banks Peninsula, Mount St. Helens flanks | 5 |

The original twelve-crop numeric inventory and geographic roles are retained in
`data/review/numeric-evaluation-v1/selection.json`. Its evaluation-only release is
`6b7e2049f45d94bd87216ac07d96c38ece2885e8f22a00fb9d5e0686d6f07411`.
Different crop files are not necessarily independent landscapes or surveys.
The two wider contexts are recorded in
`data/review/connected-context-qualification-v1/records.json` and are excluded from
that existing release. Their gallery selections, masks and unchanged native
heights are verified in the [qualification report](connected-context-qualification-v1.md).
The [Banks qualification](banks-peninsula-qualification-v1.md) records the three
new native regions; those also remain outside the existing numeric release.
The [landmark continuation](landmark-continuation-v1.md) adds three qualified
regions without extending that release. Its `inventory.json` records exact grids
and survey descriptions under `data/review/landmark-continuation-v1/`.

## What this collection establishes

It provides real examples for acquisition, masks, provenance and numeric
loading/precision checks. The bounded codec experiments measure those particular
representations and runs; they do not establish broader terrain-model feasibility
or adequate geographic generalization.

The library is too small and geographically narrow for a representative
world-quality pilot. Its unique prepared footprint is now 22.232003 km²: the
4.406211 km² Puerto Rico window union, 7.077888 km² of disjoint Banks crops,
2.359296 km² from Grand Canyon and 8.388608 km² from St. Helens. This includes
masked cells; it is not an independently measured ground-area claim. Summing
overlapping crop areas would overstate the footprint as 23.068672 km².
The 4,096 m square pilot covers 16.777216 km².
At the original 512 m patch scale, direct patch assembly
would require 64 non-overlapping patch-equivalents just to cover that footprint,
before blending overlaps. That is a geometric comparison, not a one-to-one data
requirement for learned generation. The rejected world proposal actually repeated
only six patches across 121 overlapping placements.

## Next priority

Follow [master-plan §§6–7](../plans/terrain-design-master-plan.md): expand to
several independent areas per pilot family, retain larger connected landscapes
and real transitions, and separate geography before extracting overlapping
crops. The existing numeric split permits only 512 m context and has no untouched
test set; broader contexts need a fresh geographic split check.

Use [the example locations](../locations.md) as the explicit discovery shortlist,
recording actual native coverage before choosing downloads. The preceding Banks
batch was an opportunistic public-source expansion, not a ranking of that list.
Grand Canyon and Zion Narrows now have completed USGS downloads and a five-tile
review batch. The owner explicitly liked Grand Canyon; keep adding varied canyon
contexts in later batches while broadening the other families. Two LINZ Milford
Sound-area source targets are queued for a future batch under
`data/staging/fjord-shortlist-v1/`; actual valid-ground coverage still needs inspection.

The final library must cover all finalized terrain families. Hundreds or
thousands of crops are a plausible result, not an established sufficiency target.
Report independent areas, source projects and surveyed coverage alongside crop
counts, and assess variety in composed results. Many overlapping crops from the
same valley do not supply the same diversity as different landscapes.

Use publicly downloadable sources with documented permission and the existing
supplier-QA policy. Review source tiles in batches with whole-tile yes/no votes;
do not reopen accepted tiles or request individual-feature selection. Source
expansion is a prerequisite for the next world-quality review, while better
layout and blending remain separate composition work. The authored v3 experiment
is retained internally and is not another owner review.
