# First coarse world layout

**Rejected by the owner on 2026-09-21.** The whole-world response identified
repetition, dull rectangular areas, visible stitched patches and a boring river
route. The exact response is retained in
`data/review/world-layout-decisions-v1/rejection.json`, pinned to the manifest and
page below. This rejects the composed layout; accepted source tiles are unchanged.

The preserved page is `data/worlds/pilot-layout-proposal-v2/review.html`: hill
country surrounds a winding gorge and southern floodplain. It includes an
overhead view, profiles and four preset viewpoints. No repeat vote is needed.

This is a small authored proposal, **not an implemented production world
composer or the completed world milestone**. It covers 4,096 × 4,096 m at 16 m
spacing (256² cells). The underlying real patches come from the six accepted,
water-free Río Abajo/central-gorge/south-floodplain regions. Current prepared
source verification passed for all six after authoring. Original source files,
acceptances, evaluation releases and the scalar-model result remain unchanged.

GDAL averages each native 0.5 m source to 16 m only after checking every source
cell is valid land. The fictional layout repeats 512 m patches at a 384 m stride,
with recorded quarter-turn rotations, elevation offsets and normalized overlap
weights. Horizontal/vertical scales remain one. Elevation offsets gradually lower
the uplands toward an irregular floodplain boundary. This is authored fictional
terrain; it does not retain an Earth CRS or claim measured 16 m feature fidelity.

The river and bank transition are authored separately. The river runs north to
south as a single four-connected water component, with strictly descending
surface levels. Ground underneath is positive-zero masked fill, not bathymetry.
Ground, validity, water, water-surface values and proposed class ownership remain
separate arrays. The browser combines ground and water surfaces for display only.
Class IDs 1, 2 and 9 are provisional layout labels; this is not the later, finer
ownership map or a final class-area acceptance claim.

A default 128 m-radius protected hill and an independent local edit are retained
as numeric demonstrations. The edited copy changes 1,965 coarse land cells;
values outside its domain and in all 203 protected cells remain bit-identical.
The 1,353 water cells retain deterministic fill. This demonstrates local array
editing only, not the future dependency-aware regional rerun implementation.
The page displays the original proposal.

## Evidence and limits

The one-off authoring recipe, script, coarse source arrays, Float32 ground,
separate masks, edited copy, verification and review page remain together in
`data/worlds/pilot-layout-proposal-v2/` (about 2.23 MiB). Its manifest is
`087193857ae1f3d6f492e2329488dc761db097c5f57faa2d90a0a36fb4635ad9`;
the review HTML is
`d35253aaf69f84493b5316b363f48c0c70f2e4dc22593e4e280d097b99da858d`.

A first unreviewed draft remains in `pilot-layout-proposal-v1/`. Visual inspection
found an abrupt, straight upland/floodplain transition; v2 softens that transition
and makes its boundary irregular. Neither draft has human acceptance.

Independent checks rehashed all 32 artifacts and checked saved array types,
validity/water separation, monotonic river levels, ownership IDs and exact local
edit/lock preservation. Source checks and the verification record are in
`data/review/world-layout-source-check-v1/`. The overhead image was visually
inspected. JavaScript syntax and all four viewpoints ran in a Node DOM/canvas
fixture; a full browser automation run was not performed.

Fine terrain detail, the reusable Rust world-composition/edit pipeline,
4 m/1 m refinement, measured joins, vehicle fit and export remain future work.
The proposal used only six source patches across 121 placements. Reusing this
small collection does not establish acceptable world variety, and source
expansion alone will not fix layout or blending choices.

## Follow-up and next step

The owner questioned the adequacy of the twelve-patch library. The
[coverage report](pilot-source-coverage-v1.md) records why it supports initial
technical checks but needs expansion before a representative world-quality pilot.
Source expansion is the next priority under master-plan §§6–7.

A later experiment in `data/worlds/pilot-layout-proposal-v3/` replaces the patch
grid with authored ridges, basins and tributaries, using six source patches once
each as localized residuals. Most of its terrain is authored. It remains
**internal only**, with no human acceptance or claim to solve source diversity.
Its manifest is
`41cd22707f36aa8b267d3003b341e7c8909cc7ab5cb415e8a4f9706d925fb4a1`.

Small-array checks verified source identities/transforms, separate water and
ground, connected downstream water paths and bit-exact preservation outside a
local edit and inside locks. Two hillshade images were inspected; no browser
automation was run for v3. These checks do not establish visual quality. The
recipe remains an authoring experiment, not production composition tooling.
