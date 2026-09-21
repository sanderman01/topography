# Pilot source coverage

As of 2026-09-21, the library has **12 accepted, prepared patches**, each
1,024² cells at native 0.5 m spacing: 512 × 512 m. One additional rejected tile
remains preserved and excluded. The accepted patches span six geographic areas
in the same Puerto Rico Post-Maria 2018 survey collection.

| Broad pilot family | Areas | Accepted patches |
| --- | --- | ---: |
| Karst | Río Abajo, Camuy | 5 |
| Gorge | Central gorge, Guajataca | 4 |
| River/floodplain | Southern floodplain, Arecibo | 3 |

The exact inventory and geographic roles are retained in
`data/review/numeric-evaluation-v1/selection.json`. Its evaluation-only release is
`6b7e2049f45d94bd87216ac07d96c38ece2885e8f22a00fb9d5e0686d6f07411`.
Different crop files are not necessarily independent landscapes or surveys.

## What this collection establishes

It provides real examples for acquisition, masks, provenance and numeric
loading/precision checks. The bounded codec experiments measure those particular
representations and runs; they do not establish broader terrain-model feasibility
or adequate geographic generalization.

The library is too small and geographically narrow for a representative
world-quality pilot. Its combined source footprint is 3.145728 km²; the 4,096 m
square pilot covers 16.777216 km². At the native patch scale, direct patch assembly
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
