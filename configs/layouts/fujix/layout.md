# Fujix: layout summary

The [layout.toml](layout.toml) collage for [fujix v1](../../../docs/plans/fujix-v1.md),
built as `data/worlds/fujix-collage-22`. The map is 65.5 km square with north up. Land
covers 58.3% of it (about 2,500 km², the low end of the design's range).

The outlines sit 5 to 9 km inside the map edge, with bays between the arms. Most coasts
come from the pieces' own sea:

- Canyonlands inlets and Liwa sabkha channels in the north.
- Senja skerries in the north-west.
- Milford's fjord in the west.
- Ha Long towers in the southern bay.

The west side of the SW arm and the east coast of the desert and dune field have no sea in
their pieces, so their outlines make those coasts, and they read smoother.

## Regions

Heights are ground p5 to p95 and the highest point, from `description.md`.

| Region | Class | References | Land km² | Ground m | Highest m |
| --- | ---: | --- | ---: | --- | ---: |
| Fujix volcano | 11 | Kilimanjaro (Kibo at the centre, Shira west, Mawenzi east) | 301 | 880 to 3,680 | 4,988 |
| North-east rift | 17 | Etna (two crops of its flank cone fields) | 99 | 210 to 1,230 | 1,484 |
| Grand Canyon plateau | 13 | Grand Canyon | 270 | 760 to 2,050 | 2,242 |
| North coast mesas | 7 | Canyonlands, Bardenas Reales | 148 | 20 to 870 | 1,401 |
| North-west knife ridges | 6 (12 cliffs) | High Tatras, Na Pali | 217 | 330 to 2,130 | 2,793 |
| North-west skerries | 5 | Senja | 26 | 5 to 430 | 658 |
| Western fjords | 3 | Milford Sound | 135 | 110 to 1,800 | 2,288 |
| South-west alpine arm | 6 | Zermatt (Matterhorn to Monte Rosa), Aoraki/Mount Cook | 200 | 1,060 to 3,830 | 4,379 |
| Southern drowned karst | 15 (14 inland) | Ha Long Bay, Wanfenglin | 126 | 5 to 760 | 1,641 |
| South-east limestone range | 6 | Dolomites | 192 | 970 to 2,430 | 2,807 |
| South-east river lowland | 9 | Rakaia (upper valley) | 171 | 90 to 850 | 1,290 |
| Eastern desert basin | 8 | Sossusvlei | 140 | 100 to 750 | 1,118 |
| North-east dune field | 8 (16 crest dunes) | Badain Jaran, Liwa | 183 | 10 to 520 | 1,641 |

Changes to the design's reference list:

- Canyonlands replaces Monument Valley. Monument Valley is a flat floor with a few
  buttes and disappeared in the collage. Canyonlands' mesas step down to the north coast,
  and its river canyons flood into inlets.
- Badain Jaran (star megadunes) and Liwa (crest dunes over sabkha flats) replace Great
  Sand Dunes and Taklamakan. Both of those read as plains at 32 m.
- Rakaia replaces Tagliamento. Tagliamento's lowland is a flat plain and its upper part
  reads as mountains. Rakaia gives rough hills with a braided valley that bends south to
  a bay on the east coast.
- Etna's flank cone fields replace Craters of the Moon. Craters of the Moon left about
  30 km² of flat lava plain with a few cones. Etna packs cones and lava flows onto a
  slope that falls from the volcano to the north-east.

## Flat ground

The design wants flatter pockets, not plains. Each of the largest areas under 5° is about
4 to 17 km²:

- The Sossusvlei pan in the eastern desert, 17 km² near 120 m.
- Strand flats behind the karst bay, 4 to 9 km².
- The plateau top on the canyon's south rim, 8 km² near 2,100 m.
- The Podhale basin below the knife ridges, 3.5 km².

There is no large lowland plain.

## Volcano

The summit rim is at 4,988 m. That is 654 m above the highest ground more than 15 km away
(4,334 m in the SW arm) and 610 m above the arm's top (4,379 m). Kibo's caldera sits at
the crop centre, about 2.5 km across. Its floor is near 4,880 m, a shallow ice bowl 100 to
150 m below the rim, because Kibo's real caldera is that shallow. The 4,600 m target would
need a deeper caldera than the reference has. The description's `caldera_floor_m` reads
about 3,050 m, because it searches 1.25 km around the highest rim cell and finds the
outer slope. Above about 1,300 m the cone is roughly 9 km in radius, and the Shira plateau
forms a broad western shoulder.

## Grand Canyon

The canyon runs east to west for about 24 km at y 15.5 to 18 km. The river falls from
820 m at the east end to 510 m at the west end, where it leaves into the north coast
mesas. Rims are 2,000 to 2,240 m and both lie inside the outline. The plateau's east end
is a jagged escarpment down to the rift and dunes. The feature path follows the measured
river.

## Passes

| Pass | Band | At (x, y m) | Lowest ground m | Where |
| --- | --- | --- | ---: | --- |
| Tip pass | low | 8,000, 59,000 | 32 | Low neck at the end of the SW arm |
| Skerry gap | low | 9,000, 17,500 | 764 | Behind the Na Pali cliffs |
| Limestone gap | low | 57,500, 44,500 | 299 | East end of the limestone range |
| Fjord saddle | middle | 22,000, 32,000 | 1,910 | Edge of the Shira shoulder above the fjords |
| North-west saddle | middle | 24,500, 27,500 | 1,595 | Between the volcano and the knife ridges |
| Plateau crossing | middle | 34,000, 20,000 | 1,528 | South rim plateau |
| Alpine col | high | 14,750, 49,500 | 2,372 | Col below the Zermatt crest |
| Volcano shoulder | high | 35,000, 31,500 | 2,573 | Between Kibo and Mawenzi |
| Horn col | top | 16,250, 49,000 | 3,562 | Col on the Monte Rosa crest |

All passes sit in their bands without dips. Low ships reach 64% of the land, middle
89%, high 98% and top 99.7%.

## Remaining warnings

- **Volcano: largest patch under 5° is 0.38 km².** The cone uses Kilimanjaro at 0.667 of
  its width and 1.15 of its height, which makes slopes about 1.7 times as steep, so even
  the Shira plateau tilts past 5°. Settlements go on the plateau top north of the volcano.
  A gentler cone would push its footprint far past the design's 10 km radius.
- **Western fjords: largest patch under 5° is 0.18 km².** The fjord walls are steep, now
  raised toward the design's 2,500 m, and the fjord heads have only small delta flats.
  Settlements go on the Shira shoulder or in the knife ridges' basin, both close by.
- **SW arm: largest patch under 5° is 0.35 km².** This is an alpine arm of ridges, cirques
  and glaciers with no wide valley floor at 32 m. Settlements go on the low neck at the
  arm's tip (the tip pass) or the karst strand next to it.

## Known issues

- The west flank of the SW arm (x 4 to 8 km) and the east coast (x about 58 km) are
  smooth ramps from the ocean blend. Their pieces bring no sea, so the outline makes the
  coast. The pull-push fill also leaves faint banding in those ramps.
- Several sabkha flats behind the north-east coast lie 10 to 40 m below sea level, with no
  connection to the sea. They are salt pans for now.
- Lava flats and some volcano flanks keep their source's plain class (18), so the class
  map shows plain patches there.
