# Fujix: layout summary

The [layout.toml](layout.toml) collage for [fujix v1](../../../docs/plans/fujix-v1.md),
built as `data/worlds/fujix-collage-45`. The map is 65.5 km square with north up. Land
covers 52.9% of it, about 2,270 km². The main island spans 54.4 km east to west (x 6.3
to 60.7 km) and 55.4 km north to south (y 5.1 to 60.5 km), and land stays at least
4.8 km from the map edge.

The windward west and south:

- The north-west arm, with Na Pali's cliffs and valleys along its west coast. On its north
  side, a sound at least 5 km wide (in warped outlines) separates it from the skerries.
  The skerries are a separate cluster of five rocky islands (about 5 km², up to 125 m,
  from Broughton Archipelago) at x 6 to 11.5 km, y 6 to 9 km. The sound opens east into
  a bay at the canyon river's mouth.
- Milford's northern fjord between the north-west arm and the western fjords.
- The western fjords, with Milford's walls reaching the west coast.
- A branching sound south of the fjords: Doubtful Sound's main sound, with its islands
  and Thompson Sound branching north. It is its own region, so its water and shores come
  from the piece.
- The south-west alpine arm, with Doubtful's outer sounds on its west coast and a small
  bay on its south-east side.
- The southern tower-karst bay, then the south-east lowland as a peninsula that points
  south.

The lee side is plateau and desert. The east coast has bays at y 12 and y 24 in the dune
field and a bay below the limestone range (y 47 to 56). Between them, the desert basin
meets the sea through Badain Jaran megadunes (y 28 to 38). Their hollows near the coast
are dry pans below sea level (see Known issues). The north coast has a small bay at x 43 to 47, and the Canyonlands
inlets and Liwa channels indent the rest of it.

The collage bends every outline with one smooth fixed noise warp. Its displacement std is
about 620 m and its maximum 2.2 km, with wavelengths of 6 to 18 km. It is kept gentle
enough never to fold (the Jacobian determinant stays above 0.3), so regions stay in one
piece while their coasts and borders curve. No coast touches the 4 km edge clamp; the
report checks this.

Ground that the blend carries past the island outline sinks to the seabed within 1 km of
it, so the coast stays within about 0.8 km of the warped outline. Inside the outline, the
pieces' own sea and low ground shape the coast:

- Canyonlands inlets and Liwa sabkha channels in the north.
- Na Pali's cliffs and valleys on the north-west arm. Na Pali is tilted down to the west
  by 45 m/km, so its coastal plain drowns.
- Broughton Archipelago's islands in the north-west.
- Milford's fjords in the west.
- Doubtful Sound's main sound south of the fjords, and its outer sounds on the SW arm.
- Badain Jaran's dune hollows on the desert coast (dry pans below sea level).
- The Dolomites' valleys, which reach the east cape. The range is tilted down to the east
  by 50 m/km.
- Ha Long towers in the southern bay, under a drowned Wanfenglin cone-karst shore.

## Regions

Heights are ground p5 to p95 and the highest point, from `description.md`. They are
measured over the same warped region cells as `report.txt`.

| Region | Class | References | Land km² | Ground m | Highest m |
| --- | ---: | --- | ---: | --- | ---: |
| Fujix volcano | 11 | Kilimanjaro (Kibo at the centre, Shira west, Mawenzi east) | 310 | 770 to 3,650 | 4,988 |
| North-east rift | 17 | Etna (two crops of its flank cone fields) | 114 | 170 to 1,190 | 1,481 |
| Grand Canyon plateau | 13 | Grand Canyon | 251 | 730 to 2,010 | 2,230 |
| North coast mesas | 7 | Canyonlands, Bardenas Reales | 95 | 40 to 900 | 1,319 |
| North-west knife ridges | 6 (12 cliffs) | High Tatras, Na Pali (west coast) | 213 | 170 to 1,580 | 1,854 |
| North-west skerries | 5 | Broughton Archipelago | 5 | 0 to 90 | 125 |
| Western fjords | 3 | Milford Sound | 140 | 100 to 1,700 | 2,260 |
| Western sound | 3 | Doubtful Sound (main sound, Thompson Sound) | 52 | 70 to 1,700 | 2,297 |
| South-west alpine arm | 6 | Zermatt (Matterhorn to Monte Rosa), Aoraki/Mount Cook (tip) | 127 | 550 to 3,730 | 4,301 |
| South-west sounds | 3 | Doubtful Sound (outer coast) | 28 | 80 to 1,360 | 1,853 |
| Southern drowned karst | 15 (14 inland) | Ha Long Bay, Wanfenglin | 108 | 10 to 870 | 1,557 |
| South-east limestone range | 6 | Dolomites (tilted down to the east) | 194 | 600 to 2,350 | 3,002 |
| South-east river lowland | 9 | Rakaia (upper valley) | 122 | 190 to 860 | 1,219 |
| Eastern desert basin | 8 | Sossusvlei inland, Badain Jaran on the coast | 126 | 40 to 660 | 994 |
| North-east dune field | 8 (16 crest dunes) | Badain Jaran, Liwa | 150 | 10 to 560 | 1,380 |

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
- Doubtful Sound is new, in two regions: its outer coast along the SW arm (tilted down to
  the west so its sounds keep their sea), and its main sound between the fjords and the
  arm.
- Broughton Archipelago replaces Senja for the skerries. Senja's islets are too small and
  low to survive the blend as a separate archipelago.

## Flat ground

The design wants flatter pockets, not plains. The largest areas under 5° are:

- Coastal flats in the dune field, 4 km² near 30 m and 2 km² near 40 m.
- A basin at the knife ridges' south end, 4 km² near 830 m.
- The plateau top on the canyon's south rim, 3.5 km² near 2,100 m.
- Pockets of 1.5 km² in the river lowland and in the desert basin.

There is no large lowland plain. The Sossusvlei pan is only a small flat now (1.5 km²),
because the piece stops short of the coast so the pan does not meet the sea along a
straight line.

## Volcano

The summit rim is at 4,988 m. That is 757 m above the highest ground more than 15 km away
(4,231 m) and 687 m above the SW arm's top (4,301 m). Kibo's caldera sits at the crop
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
path the river reads 515 to 763 m.

## Passes

| Pass | Band | At (x, y m) | Lowest ground m | Where |
| --- | --- | --- | ---: | --- |
| Tip pass | low | 10,500, 56,500 | 11 | Valley across the arm 3 km from its tip, from a sound to the south coast |
| Skerry gap | low | 9,000, 17,500 | 137 | Behind the Na Pali cliffs |
| Limestone gap | low | 57,500, 44,500 | 6 | Drowned-valley heads at the range's east end |
| Fjord saddle | middle | 22,000, 32,000 | 1,881 | Edge of the Shira shoulder above the fjords |
| North-west saddle | middle | 24,500, 27,500 | 1,480 | Between the volcano and the knife ridges |
| Plateau crossing | middle | 34,000, 20,000 | 1,527 | South rim plateau |
| Alpine col | high | 14,750, 49,500 | 2,554 | Col below the Zermatt crest |
| Volcano shoulder | high | 35,000, 31,500 | 2,572 | Between Kibo and Mawenzi |
| Horn col | top | 16,250, 49,000 | 3,520 | Col on the Monte Rosa crest |

All passes sit in their bands without dips. The tip pass is a real col on land: its
valley floor is 20 to 150 m, between a 650 m knob at the tip and the arm, which rises
past 750 m. No sea lies within 500 m of it. It is a low col because the blend lowers the
narrow far arm. Low ships reach 67% of the land, middle 90%, high 98% and top 99.7%.

## Remaining warnings

- **Volcano: largest patch under 5° is 0.32 km².** The cone uses Kilimanjaro at 0.667 of
  its width and 1.15 of its height, which makes slopes about 1.7 times as steep, so even
  the Shira plateau tilts past 5°. Settlements go on the plateau top north of the volcano.
  A gentler cone would push its footprint far past the design's 10 km radius.
- **Western fjords: largest patch under 5° is 0.33 km².** The fjord walls are steep and the
  fjord heads have only small delta flats. Settlements go on the Shira shoulder or in the
  knife ridges' basin, both close by.
- **SW arm: largest patch under 5° is 0.19 km².** This is an alpine arm of ridges, cirques
  and glaciers with no wide valley floor at 32 m. Settlements go in the tip pass valley or
  on the karst strand next to the arm.
- **SW sounds: largest patch under 5° is 0.01 km².** Fiordland's outer coast is steep
  sounds with no flats. Its settlement shares the tip pass valley.
- **Western sound: largest patch under 5° is 0.03 km².** Doubtful's main sound has steep
  walls and no flats. Its settlement shares the tip pass valley or the fjord heads.
- **North coast mesas: largest patch under 5° is 0.42 km².** The bay at the canyon river's
  mouth took the region's flattest coastal ground. The canyon's south rim plateau
  (3.5 km²) is the nearest settlement site.
- **North-west skerries: largest patch under 5° is 0.07 km².** The skerries are a few
  small rocky islands. Settlements go in the knife ridges' basin, across the sound.

## Known issues

- Coasts that outlines make, rather than a piece's own sea, are smooth arcs. Gap closing
  fills any cove narrower than 4 km, and the blend's ocean weight (3 km border) smooths the
  last kilometre of land into a steep, even slope. Such coasts remain on the NW arm's
  north side (facing the skerry sound), the SW arm's south-east side, the dune-field bays
  at y 12 and y 24, and the lowland bay and south coast. About 75 km² of land lies outside
  the outline, all of it within 0.8 km (182 km², reaching 2.2 km out, before the sink).
- The south-east range's diagonal coast (x 58 to 51 km, y 44 to 51 km) is a smooth
  straight bevel about 12 km long. Two attempts failed and were reverted:
  - A cape halfway along pushed the range/lowland seam to 1.31.
  - A south tilt, or a Kenai fjord-coast region, pushed it to 1.39 and 1.46, and
    Kenai's water did not reach the coast.
- The west coast is only partly piece-made: Na Pali (y 16 to 27), Milford's fjord mouth
  (y 31 to 33) and Doubtful's sound (y 42 to 46). For y 28 to 31, 34 to 41 and 45 to 51 the
  coast is the blend apron, 0.4 to 0.7 km outside the warped outline. Together this is a
  roughly 24 km near-straight north to south coast at x 6.3 to 7.7 km, with a smooth
  straight band at x 7.0 to 7.1 km, y 34 to 39.5 km. A smooth headland sits at x 7 to
  10.7 km, y 27 to 28.3 km.
- The NW arm's north shore (x 7 to 16 km, y 11 to 15 km) is a smooth outline bevel.
- The dune-field bays, the karst bay's east shore and the lowland bay are outline-made
  bevels.
- Skerries and lava flats carry plain class 18.
- The far SW arm is lower and smoother than its pieces. The arm is narrow, so the
  blend's coarse levels mix in a lot of sea. Aoraki's Main Divide at the tip ends near
  700 m.
- The north coast from x 21 to 42 km still runs nearly straight at y 6 to 7 km. Only the
  Canyonlands inlets break it, because a bay there would cut into the canyon rims.
- Several sabkha flats behind the north-east coast lie 10 to 40 m below sea level, with no
  connection to the sea. They are salt pans for now.
- Lava flats, some volcano flanks and the skerry islands keep their source's plain class
  (18), so the class map shows plain patches there.
- Dry basins below sea level on the desert coast, not connected to the sea: 0.83 km² at
  x 55.2 to 56.3 km, y 30.3 to 31.3 km (-35 m), 0.41 km² at x 57.9 to 58.6 km, y 37.0 to
  37.6 km (-60 m), and 0.13 km² near y 33 km (-35 m). They are dry pans, not lagoons.
  Hydrology v2 will decide them.
