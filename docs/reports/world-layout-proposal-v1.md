# First coarse world layout

The next human review is a whole-world **yes or no** on the overall arrangement.
Open `data/worlds/pilot-layout-proposal-v2/review.html`: hill country surrounds a
winding gorge, which opens into a southern floodplain. The page includes an
overhead view, profiles and four preset viewpoints. This follows the master
plan's coarse-layout review before refinement; no individual feature selections
or technical arithmetic decisions are requested.

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
Source repetition is intentional and visible in this small library. The current
question concerns the overall hill/gorge/floodplain arrangement. If accepted,
retain these exact proposal bytes and continue toward the reusable composition
and refinement steps; do not count this as approval of unbuilt fine terrain.
