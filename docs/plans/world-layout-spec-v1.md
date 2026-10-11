# World layout spec v1

Status: planned 2026-09-29; steps 1–6 implemented 2026-10-03 for Fujion
(`configs/layouts/fujion/`, `topography world layout`, `layout.py`), with the L0 then
painted in GIMP (`paint.py`, see the [cascade workflow](../workflows/terrain-cascade.md#authoring-a-layout)).
The spec is not yet validated by `topography validate`. This is the process for agreeing on
a world before any generation. It feeds level L0 of the
[terrain cascade](terrain-cascade-v1.md).

## Why

The caldera-island world started from a one-paragraph request: an island, an
ice-capped caldera, twelve biomes placed by climate. I turned that straight into
primitives and generated. The owner then found the canyon country missing, the
karst flat and the overall look childlike. Most of that was decided before the
first render and could have been caught on paper. A written spec with real
reference places and sizes makes those choices explicit and cheap to change.

## Steps

1. **Brief (owner).** A few paragraphs in `configs/layouts/<name>/brief.md`: what the
   world is for, its mood, the features it must have, and anything to avoid.
   Name real places for the look you want ("a Canyonlands-like section",
   "Milford Sound walls") where you can.
2. **Draft spec (agent).** `configs/layouts/<name>/layout.toml` plus a readable
   `layout.md` summary, written from the brief. Contents below.
3. **Fit check (agent, automatic).** Area per region, real-world size of each
   hero feature against the map, relief against the height budget, rain-shadow
   and snowline consistency, rivers that reach the sea or a lake. Anything that
   does not fit is listed with a proposed change, not silently shrunk.
4. **Reference sheet (agent).** For each region and hero feature, a hillshade of
   its real reference at the same scale as it will appear in the world, from the
   GLO-30 context regions. The owner sees what "Canyonlands-like" will mean
   before any model runs.
5. **Owner review.** Yes, or notes on the spec and reference sheet. Repeat 2–5
   until Yes. This is a document review, not a terrain vote.
6. **L0 layout.** `world sketch` renders the spec to L0 rasters and an annotated
   overview map (region names, feature lines, envelope contours). Owner Yes/No.
7. **Generation.** L1–L2 in minutes, then L3, then L4, with owner review of the
   whole-world overview after L2 and a flyover after L3 (cascade plan).

The brief, spec, reference sheet and review decisions are small text and image
files. Commit the brief and spec with the recipe; keep images under `data/`.

## Spec contents

**World.** Extent (32,768 m for now), sea level, prevailing wind, windward and
lee snowline, treeline, height budget (lowest valley floor, highest summit).

**Regions.** One entry per region:

- terrain class and a short name ("Red Mesa canyonlands"); painted layouts use the
  legend v3 palette, whose subclasses name the look directly (layered canyon, cone
  karst, drowned karst, crest-train dunes, cinder cones, plain; see
  [biome forms](biome-forms-v1.md));
- one to three real reference places, with GLO-30 coordinates;
- position and outline as a polygon on the 256 m grid, or a centre and radii;
- target area in km², with the fit check reporting the actual area;
- height envelope: floor and summit, and the typical local relief (for canyons,
  the rim-to-floor depth);
- coast type where the region meets the sea (cliffs, skerries, beaches, fjord
  mouths);
- cover: forest share, bare rock, snow/ice, farmland or none;
- transition to each neighbour: sharp (a range front, a canyon rim) or gradual,
  with a width.

**Hero features.** Named landmarks the player navigates by, each with real
dimensions and a reference: the caldera (rim diameter, rim height, crater floor,
ice or lake), the main canyon (length, depth, width at the rim), each fjord
(length, wall height), glaciers, major peaks. Lines and points on the map, with
the heights the cascade must hold.

**Water network.** Rivers from source to mouth with rough profiles, lakes with a
surface level, waterfalls where wanted. Lakes are flat, and rivers descend.

**Routes.** Optional preferred flight corridors and passes that must stay open.

## Scale budget

A 32 km map has about 1,000 km² of area, and an island uses half to two thirds
of it. Real landforms are large: the Grand Canyon is 446 km long, Crater Lake's
caldera is 8–10 km across, Geirangerfjord is 15 km long. The fit check applies
these rules:

- A region needs about 60 km² or more to show its landform with structure around
  it. Smaller regions become single features inside a neighbour.
- A hero feature is scaled no more than 1.5× smaller than its reference, or it
  stops reading as that landform. If it does not fit, the spec picks a smaller
  real reference ("a Verdon-sized gorge", not "the Grand Canyon").
- Twelve full regions do not fit in 32 km at these sizes. The spec picks the
  five to eight that matter most and gives the rest a small appearance as
  features inside larger regions, or leaves them for another world.

## Changes to tools (planned)

- `world sketch` keeps its recipe format for feature placement but only emits L0
  rasters and an overview map. Its detailed height primitives stay as a test
  input, not a world base.
- A fit-check report in text and a reference-sheet page with hillshades at world
  scale.
- The spec format is versioned TOML, validated by `topography validate`.
