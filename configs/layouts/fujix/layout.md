# Fujix: layout summary

The [layout.toml](layout.toml) collage for [fujix v1](../../../docs/plans/fujix-v1.md),
built as `data/worlds/fujix-collage-13`. The map is 65.5 km square with north up. Land
covers 64.9% of it (about 2,790 km²). Coasts come from the pieces' own sea and low
ground, and they stay at least 4 km from the map edge.

## Regions

Heights are ground p5 to p95 and the highest point, from `description.md`.

| Region | Class | References | Land km² | Ground m | Highest m |
| --- | ---: | --- | ---: | --- | ---: |
| Fujix volcano | 11 | Kilimanjaro (Kibo at the centre, Shira west, Mawenzi east) | 301 | 880 to 3,680 | 4,988 |
| North-east rift | 17 | Craters of the Moon (Great Rift cones, heights doubled) | 99 | 620 to 1,050 | 1,358 |
| Grand Canyon plateau | 13 | Grand Canyon | 269 | 790 to 2,210 | 2,533 |
| North coast mesas | 7 | Canyonlands, Bardenas Reales | 157 | 40 to 910 | 1,486 |
| North-west knife ridges | 6 (12 cliffs) | High Tatras, Na Pali | 228 | 270 to 2,130 | 2,793 |
| North-west skerries | 5 | Senja | 25 | 0 to 430 | 665 |
| Western fjords | 3 | Milford Sound | 185 | 50 to 1,550 | 2,054 |
| South-west alpine arm | 6 | Zermatt (Matterhorn to Monte Rosa), Aoraki/Mount Cook | 234 | 1,250 to 3,840 | 4,378 |
| Southern drowned karst | 15 (14 inland) | Ha Long Bay, Wanfenglin | 216 | 20 to 560 | 1,265 |
| South-east limestone range | 6 | Dolomites | 197 | 970 to 2,420 | 2,807 |
| South-east river lowland | 9 | Rakaia (upper valley) | 193 | 90 to 880 | 1,501 |
| Eastern desert basin | 8 | Sossusvlei | 154 | 40 to 740 | 1,117 |
| North-east dune field | 8 (16 crest dunes) | Badain Jaran, Liwa | 204 | 20 to 590 | 1,146 |

Changes to the design's reference list:

- Canyonlands replaces Monument Valley. Monument Valley is a flat floor with a few
  buttes and disappeared in the collage. Canyonlands' mesas step down to the north coast,
  and its river canyons flood into inlets.
- Badain Jaran (star megadunes) and Liwa (crest dunes over sabkha flats) replace Great
  Sand Dunes and Taklamakan. Both of those read as plains at 32 m.
- Rakaia replaces Tagliamento. Tagliamento's lowland is a flat plain and its upper part
  reads as mountains. Rakaia gives rough hills with a braided valley that bends south to
  a bay on the east coast.

## Volcano

The summit rim is at 4,988 m. That is 654 m above the highest ground more than 15 km away
(4,334 m in the SW arm) and 610 m above the arm's top (4,378 m). Kibo's caldera sits at the crop centre, about 2.5 km across. Its floor is
near 4,880 m, a shallow ice bowl 100 to 150 m below the rim, because Kibo's real caldera
is that shallow. The 4,600 m target would need a deeper caldera than the reference
has. The description's `caldera_floor_m` reads about 3,050 m, because it searches 1.25 km
around the highest rim cell and finds the outer slope. Above about 1,300 m the cone is
roughly 9 km in radius, and the Shira plateau forms a broad western shoulder.

## Grand Canyon

The canyon runs east to west for about 24 km at y 15.5 to 18 km. The river falls from
770 m at the east end to 490 m at the west end, where it leaves into the north coast
mesas. Rims are 2,000 to 2,400 m in the middle and lower toward both ends. Both rims lie
inside the outline. The feature path follows the measured river (516 to 817 m).

## Passes

| Pass | Band | At (x, y m) | Lowest ground m | Where |
| --- | --- | --- | ---: | --- |
| Tip pass | low | 8,000, 59,000 | 339 | Low neck at the end of the SW arm |
| Skerry gap | low | 9,000, 17,500 | 786 | Behind the Na Pali cliffs |
| Limestone gap | low | 57,500, 44,500 | 634 | East end of the limestone range |
| Fjord saddle | middle | 22,000, 32,000 | 1,715 | Edge of the Shira shoulder above the fjords |
| North-west saddle | middle | 24,500, 27,500 | 1,595 | Between the volcano and the knife ridges |
| Plateau crossing | middle | 34,000, 20,000 | 1,615 | South rim plateau |
| Alpine col | high | 13,000, 50,500 | 2,762 | South-west end of the Zermatt crest |
| Volcano shoulder | high | 35,000, 31,500 | 2,573 | Between Kibo and Mawenzi |
| Horn col | top | 16,250, 49,000 | 3,561 | Col on the Monte Rosa crest |

All passes sit in their bands without dips. Low ships reach 65% of the land, middle
89%, high 98% and top 99.8%.

## Remaining warnings

- **Volcano: largest patch under 5° is 0.38 km².** The cone uses Kilimanjaro at 0.667 of
  its width and 1.15 of its height, which makes slopes about 1.7 times as steep, so even
  the Shira plateau tilts past 5°. Settlements go on the flatter plateau top north of the
  volcano or the fjord heads. Making the cone gentler would push its footprint far past the
  design's 10 km radius.
- **Western fjords: largest patch under 5° is 0.48 km².** The fjords have steep walls and
  small delta flats at their heads, just under 0.5 km². The flatter ground nearby lies
  on the Shira shoulder and in the knife ridges' basin.

## Known issues

- The east coast of the desert basin and the dune field (x about 60 km) is still fairly
  straight. Those pieces have no sea of their own, so the outline makes that coast.
- Several sabkha flats behind the north-east coast lie 10 to 40 m below sea level, with no
  connection to the sea. They are salt pans for now.
- The canyon's east end opens onto the lower rift and dune ground (about 400 m) and needs
  attention when hydrology adds the river.
- Lava flats in the rift and some volcano flanks keep their source's plain class (18), so
  the class map shows plain patches there.
