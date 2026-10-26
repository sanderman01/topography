# Fujix: layout summary

The [layout.toml](layout.toml) collage for [fujix v1](../../../docs/plans/fujix-v1.md),
built as `data/worlds/fujix-collage-27`. The map is 65.5 km square with north up. Land
covers 59.6% of it (about 2,560 km²).

The outlines sit 5 to 9 km inside the map edge, with bays between the arms and headlands
and bays on the east and south-east coasts. The collage bends every outline with one
smooth fixed noise warp (about 1 km, wavelengths of 3 to 8 km), so coasts and region
borders are never straight. Many coasts come from the pieces' own sea:

- Canyonlands inlets and Liwa sabkha channels in the north.
- Senja skerries in the north-west.
- Milford's fjords in the west.
- Doubtful Sound's sounds on the SW arm's outer coast.
- Ha Long towers in the southern bay, under a drowned Wanfenglin cone-karst shore.

## Regions

Heights are ground p5 to p95 and the highest point, from `description.md`. Its region
statistics use the unwarped outlines, so they include some neighbouring ground (the
karst's 2,471 m is on the SW arm's flank, at x 23.3, y 50.0 km).

| Region | Class | References | Land km² | Ground m | Highest m |
| --- | ---: | --- | ---: | --- | ---: |
| Fujix volcano | 11 | Kilimanjaro (Kibo at the centre, Shira west, Mawenzi east) | 301 | 840 to 3,670 | 4,988 |
| North-east rift | 17 | Etna (two crops of its flank cone fields) | 99 | 210 to 1,280 | 1,635 |
| Grand Canyon plateau | 13 | Grand Canyon | 270 | 640 to 1,970 | 2,212 |
| North coast mesas | 7 | Canyonlands, Bardenas Reales | 134 | 30 to 1,100 | 1,912 |
| North-west knife ridges | 6 (12 cliffs) | High Tatras, Na Pali | 218 | 290 to 2,050 | 2,693 |
| North-west skerries | 5 | Senja | 21 | 5 to 330 | 550 |
| Western fjords | 3 | Milford Sound | 137 | 100 to 1,780 | 2,287 |
| South-west alpine arm | 6 | Zermatt (Matterhorn to Monte Rosa), Aoraki/Mount Cook | 178 | 690 to 3,700 | 4,310 |
| South-west sounds | 3 | Doubtful Sound (outer coast) | 58 | 90 to 1,330 | 1,693 |
| Southern drowned karst | 15 (14 inland) | Ha Long Bay, Wanfenglin | 127 | 10 to 1,250 | 2,471 |
| South-east limestone range | 6 | Dolomites | 198 | 860 to 2,400 | 2,818 |
| South-east river lowland | 9 | Rakaia (upper valley) | 167 | 90 to 820 | 1,727 |
| Eastern desert basin | 8 | Sossusvlei | 143 | 140 to 1,110 | 1,604 |
| North-east dune field | 8 (16 crest dunes) | Badain Jaran, Liwa | 167 | 10 to 440 | 1,204 |

Changes to the design's reference list:

- Canyonlands replaces Monument Valley. Monument Valley is a flat floor with a few
  buttes and disappeared in the collage. Canyonlands' mesas step down to the north coast,
  and its river canyons flood into inlets.
- Badain Jaran (star megadunes) and Liwa (crest dunes over sabkha flats) replace Great
  Sand Dunes and Taklamakan. Both of those read as plains at 32 m.
- Rakaia replaces Tagliamento. Tagliamento's lowland is a flat plain and its upper part
  reads as mountains. Rakaia gives rough hills with a braided valley that falls south and
  west to the coast.
- Etna's flank cone fields replace Craters of the Moon. Craters of the Moon left about
  30 km² of flat lava plain with a few cones.
- Doubtful Sound is new. It gives the SW arm's outer coast real sounds and headlands
  instead of a smooth ramp.

## Flat ground

The design wants flatter pockets, not plains. The largest areas under 5° are:

- The Sossusvlei pan by the east coast, 9 km² near 60 m.
- The Podhale basin below the knife ridges, 3.5 km².
- A coastal flat in the dune field, 3 km².
- The plateau top on the canyon's south rim, 3 km² near 2,100 m.
- A Rakaia terrace, 2.5 km².

There is no large lowland plain.

## Volcano

The summit rim is at 4,988 m. That is 816 m above the highest ground more than 15 km away
(4,172 m) and 680 m above the SW arm's top (4,310 m). Kibo's caldera sits at the crop
centre, about 2.5 km across. Its floor is near 4,880 m, a shallow ice bowl 100 to 150 m
below the rim, because Kibo's real caldera is that shallow. The 4,600 m target would need
a deeper caldera than the reference has. The description's `caldera_floor_m` reads about
3,050 m, because it searches 1.25 km around the highest rim cell and finds the outer
slope. Above about 1,300 m the cone is roughly 9 km in radius, and the Shira plateau forms
a broad western shoulder.

## Grand Canyon

The canyon runs east to west for about 24 km at y 15.5 to 18 km and leaves west into the
north coast mesas. Rims are 2,000 to 2,200 m. The crop now reaches past the outline on
every side, so the side canyons fade out instead of ending on the crop edge. The feature
path follows the river from build 22. Its east end now crosses the warped escarpment,
which is why the path reads up to 1,046 m.

## Passes

| Pass | Band | At (x, y m) | Lowest ground m | Where |
| --- | --- | --- | ---: | --- |
| Tip pass | low | 8,000, 59,000 | 62 | Low neck at the end of the SW arm |
| Skerry gap | low | 9,000, 17,500 | 752 | Behind the Na Pali cliffs |
| Limestone gap | low | 57,500, 44,500 | 604 | East end of the limestone range |
| Fjord saddle | middle | 22,000, 32,000 | 1,970 | Edge of the Shira shoulder above the fjords |
| North-west saddle | middle | 24,500, 27,500 | 1,666 | Between the volcano and the knife ridges |
| Plateau crossing | middle | 34,000, 20,000 | 1,526 | South rim plateau |
| Alpine col | high | 14,750, 49,500 | 2,352 | Col below the Zermatt crest |
| Volcano shoulder | high | 35,000, 31,500 | 2,573 | Between Kibo and Mawenzi |
| Horn col | top | 16,250, 49,000 | 3,553 | Col on the Monte Rosa crest |

All passes sit in their bands without dips. Low ships reach 67% of the land, middle
91%, high 98% and top 99.7%.

## Remaining warnings

- **Volcano: largest patch under 5° is 0.32 km².** The cone uses Kilimanjaro at 0.667 of
  its width and 1.15 of its height, which makes slopes about 1.7 times as steep, so even
  the Shira plateau tilts past 5°. Settlements go on the plateau top north of the volcano.
  A gentler cone would push its footprint far past the design's 10 km radius.
- **Western fjords: largest patch under 5° is 0.34 km².** The fjord walls are steep and the
  fjord heads have only small delta flats. Settlements go on the Shira shoulder or in the
  knife ridges' basin, both close by.
- **SW arm: largest patch under 5° is 0.21 km².** This is an alpine arm of ridges, cirques
  and glaciers with no wide valley floor at 32 m. Settlements go on the low neck at the
  arm's tip (the tip pass) or the karst strand next to it.
- **SW sounds: largest patch under 5° is 0.02 km².** Fiordland's outer coast is steep
  sounds with no flats. Its settlement shares the arm's tip.

## Known issues

- Where a coast is high ground with no sea in its pieces, the ocean blend still makes a
  smooth ramp about 1 to 1.5 km wide. This shows on the SW sounds' north-west side (x 3.5
  to 5 km, y 42 to 52) and along parts of the east coast.
- The SE lowland's south coast (x 38 to 47 km, y about 58.5) is the 0 m contour of
  Rakaia's south tilt and runs fairly straight.
- Several sabkha flats behind the north-east coast lie 10 to 40 m below sea level, with no
  connection to the sea. They are salt pans for now.
- Lava flats and some volcano flanks keep their source's plain class (18), so the class
  map shows plain patches there.
