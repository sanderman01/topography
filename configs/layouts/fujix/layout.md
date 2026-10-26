# Fujix: layout summary

The [layout.toml](layout.toml) collage for [fujix v1](../../../docs/plans/fujix-v1.md),
built as `data/worlds/fujix-collage-31`. The map is 65.5 km square with north up. Land
covers 58.7% of it (about 2,520 km², near the low end of the design's 2,500 to 3,000 km²).

The outlines sit 5.5 to 9 km inside the map edge. There are bays between the arms, and
headlands and bays on the east and south-east coasts.

The collage bends every outline with one smooth fixed noise warp. Its displacement std is
about 620 m and its maximum 2.2 km, with wavelengths of 6 to 18 km. It is kept gentle
enough never to fold (the Jacobian determinant stays above 0.3), so regions stay in one
piece while their coasts and borders curve. No coast touches the 4 km edge clamp; the
report now checks this.

Many coasts come from the pieces' own sea:

- Canyonlands inlets and Liwa sabkha channels in the north.
- Senja skerries in the north-west.
- Milford's fjords in the west.
- Doubtful Sound's sounds on the SW arm's outer coast.
- Ha Long towers in the southern bay, under a drowned Wanfenglin cone-karst shore.

## Regions

Heights are ground p5 to p95 and the highest point, from `description.md`. They are
measured over the same warped region cells as `report.txt`.

| Region | Class | References | Land km² | Ground m | Highest m |
| --- | ---: | --- | ---: | --- | ---: |
| Fujix volcano | 11 | Kilimanjaro (Kibo at the centre, Shira west, Mawenzi east) | 310 | 770 to 3,650 | 4,988 |
| North-east rift | 17 | Etna (two crops of its flank cone fields) | 114 | 170 to 1,200 | 1,487 |
| Grand Canyon plateau | 13 | Grand Canyon | 251 | 730 to 2,020 | 2,230 |
| North coast mesas | 7 | Canyonlands, Bardenas Reales | 130 | 30 to 920 | 1,323 |
| North-west knife ridges | 6 (12 cliffs) | High Tatras, Na Pali | 232 | 380 to 1,980 | 2,789 |
| North-west skerries | 5 | Senja | 22 | 5 to 400 | 599 |
| Western fjords | 3 | Milford Sound | 149 | 120 to 1,840 | 2,359 |
| South-west alpine arm | 6 | Zermatt (Matterhorn to Monte Rosa), Aoraki/Mount Cook | 143 | 1,050 to 3,750 | 4,309 |
| South-west sounds | 3 | Doubtful Sound (outer coast) | 34 | 60 to 1,690 | 2,051 |
| Southern drowned karst | 15 (14 inland) | Ha Long Bay, Wanfenglin | 110 | 10 to 840 | 1,557 |
| South-east limestone range | 6 | Dolomites | 203 | 940 to 2,430 | 2,826 |
| South-east river lowland | 9 | Rakaia (upper valley) | 166 | 180 to 880 | 1,444 |
| Eastern desert basin | 8 | Sossusvlei | 131 | 90 to 780 | 1,109 |
| North-east dune field | 8 (16 crest dunes) | Badain Jaran, Liwa | 173 | 10 to 550 | 1,380 |

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
- Doubtful Sound is new, as its own region along the SW arm's outer coast. It is tilted
  down to the west so its sounds keep their sea.

## Flat ground

The design wants flatter pockets, not plains. The largest areas under 5° are:

- The Sossusvlei pan by the east coast, 14 km² near 60 m.
- Coastal flats in the dune field, 3 to 4 km².
- The Podhale basin below the knife ridges, 4 km².
- The plateau top on the canyon's south rim, 3.5 km² near 2,100 m.

There is no large lowland plain.

## Volcano

The summit rim is at 4,988 m. That is 726 m above the highest ground more than 15 km away
(4,262 m) and 680 m above the SW arm's top (4,309 m). Kibo's caldera sits at the crop
centre, about 2.5 km across. Its floor is near 4,880 m, a shallow ice bowl 100 to 150 m
below the rim, because Kibo's real caldera is that shallow. The 4,600 m target would need
a deeper caldera than the reference has. The description's `caldera_floor_m` reads about
3,050 m, because it searches 1.25 km around the highest rim cell and finds the outer
slope. Above about 1,300 m the cone is roughly 9 km in radius, and the Shira plateau forms
a broad western shoulder.

## Grand Canyon

The canyon runs east to west for about 24 km at y 15.5 to 18 km and leaves west into the
north coast mesas. Rims are 2,000 to 2,200 m. The crop reaches past the outline on every
side, so the side canyons fade out instead of ending on a crop edge. Along the feature
path the river reads 515 to 784 m.

## Passes

| Pass | Band | At (x, y m) | Lowest ground m | Where |
| --- | --- | --- | ---: | --- |
| Tip pass | low | 10,500, 56,500 | -17 | Isthmus between a sound and the south coast, behind the arm's tip |
| Skerry gap | low | 9,000, 17,500 | 1,012 | Behind the Na Pali cliffs |
| Limestone gap | low | 57,500, 44,500 | 362 | East end of the limestone range |
| Fjord saddle | middle | 22,000, 32,000 | 1,885 | Edge of the Shira shoulder above the fjords |
| North-west saddle | middle | 24,500, 27,500 | 1,480 | Between the volcano and the knife ridges |
| Plateau crossing | middle | 34,000, 20,000 | 1,527 | South rim plateau |
| Alpine col | high | 14,750, 49,500 | 2,368 | Col below the Zermatt crest |
| Volcano shoulder | high | 35,000, 31,500 | 2,573 | Between Kibo and Mawenzi |
| Horn col | top | 16,250, 49,000 | 3,561 | Col on the Monte Rosa crest |

All passes sit in their bands without dips. The tip pass's lowest ground is the sound's
water within 500 m; the isthmus itself is 0 to 60 m. Low ships reach 66% of the land,
middle 91%, high 98% and top 99.7%.

## Remaining warnings

- **Volcano: largest patch under 5° is 0.32 km².** The cone uses Kilimanjaro at 0.667 of
  its width and 1.15 of its height, which makes slopes about 1.7 times as steep, so even
  the Shira plateau tilts past 5°. Settlements go on the plateau top north of the volcano.
  A gentler cone would push its footprint far past the design's 10 km radius.
- **Western fjords: largest patch under 5° is 0.33 km².** The fjord walls are steep and the
  fjord heads have only small delta flats. Settlements go on the Shira shoulder or in the
  knife ridges' basin, both close by.
- **SW arm: largest patch under 5° is 0.30 km².** This is an alpine arm of ridges, cirques
  and glaciers with no wide valley floor at 32 m. Settlements go on the isthmus at the tip
  pass or the karst strand next to it.
- **SW sounds: largest patch under 5° is 0.02 km².** Fiordland's outer coast is steep
  sounds with no flats. Its settlement shares the tip pass isthmus.

## Known issues

- Where a high coast has no sea in its pieces, the pyramid blend's coarse bands carry
  land past the outline and leave a smooth apron up to about 1.5 km wide. Short stretches
  remain on the SW sounds' west side (x 6 to 8 km, y 44 to 51) and on the east coast
  (x 56 to 60 km).
- The SE lowland's south coast is smooth: Rakaia's gentle hills meet the sea along their
  0 m contour.
- With the gentler warp, region borders curve over long wavelengths but still show the
  outlines' polygon shape at map scale.
- Several sabkha flats behind the north-east coast lie 10 to 40 m below sea level, with no
  connection to the sea. They are salt pans for now.
- Lava flats and some volcano flanks keep their source's plain class (18), so the class
  map shows plain patches there.
