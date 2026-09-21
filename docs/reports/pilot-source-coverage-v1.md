# Pilot source coverage

As of 2026-09-21, the library has **17 accepted, prepared patches**: twelve
512 × 512 m crops and two 1,024 × 1,024 m contexts at native 0.5 m spacing,
plus three 1,536 × 1,536 m Banks Peninsula crops at native 1 m spacing.
Two rejected tiles remain preserved and excluded. Six Puerto Rico geographic
areas and Banks Peninsula span two survey collections; the wider Puerto Rico
contexts overlap existing selections. Three Banks crops do not represent three
independent survey projects.

| Broad pilot family | Areas | Accepted patches |
| --- | --- | ---: |
| Karst | Río Abajo, Camuy | 6 |
| Gorge | Central gorge, Guajataca | 5 |
| River/floodplain | Southern floodplain, Arecibo | 3 |
| Eroded volcanic terrain | Banks Peninsula, New Zealand | 3 |

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

## What this collection establishes

It provides real examples for acquisition, masks, provenance and numeric
loading/precision checks. The bounded codec experiments measure those particular
representations and runs; they do not establish broader terrain-model feasibility
or adequate geographic generalization.

The library is too small and geographically narrow for a representative
world-quality pilot. Its unique selected footprint is now 11.484099 km²: the
4.406211 km² Puerto Rico window union plus 7.077888 km² of disjoint Banks crops.
Summing overlapping crop areas would overstate it as 12.320768 km².
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
Grand Canyon and Zion Narrows now have retained USGS discovery results and a
bounded two-source download queue under `data/staging/location-shortlist-v1/`.

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
