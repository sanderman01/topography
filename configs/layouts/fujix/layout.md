# Fujix: layout summary

The [layout.toml](layout.toml) collage for [fujix v1](../../../docs/plans/fujix-v1.md),
built as `data/worlds/fujix-collage-49`. The map is 65.5 km square with north up. Land
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
are dry pans below sea level (see Known issues). The north coast has a small bay at x 43
to 47, and the Canyonlands inlets and Liwa channels indent the rest of it.

The collage bends every outline with one smooth fixed noise warp. Its displacement std is
about 620 m and its maximum 2.38 km, with wavelengths of 6 to 18 km. It is kept gentle
enough never to fold (the Jacobian determinant stays above 0.3), so regions stay in one
piece while their coasts and borders curve. No coast touches the 4 km edge clamp; the
report checks this. Region borders blend over 2 to 4.5 km (`border_m`). The design's
300 m borders at canyon rims are not needed: the rims lie inside the Grand Canyon piece.

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
measured over the collage's region map (`region32.u8`), where every land cell belongs to
the region with the largest blend weight, so the land areas add up to the island's.

| Region | Class | References | Land km² | Ground m | Highest m |
| --- | ---: | --- | ---: | --- | ---: |
| Fujix volcano | 11 | Kilimanjaro (Kibo at the centre, Shira west, Mawenzi east) | 360 | 690 to 4,180 | 5,029 |
| North-east rift | 17 | Etna (two crops of its flank cone fields) | 124 | 170 to 1,220 | 1,481 |
| Grand Canyon plateau | 13 | Grand Canyon | 272 | 740 to 2,000 | 2,230 |
| North coast mesas | 7 | Canyonlands, Bardenas Reales | 102 | 30 to 890 | 1,319 |
| North-west knife ridges | 6 (12 cliffs) | High Tatras, Na Pali (west coast) | 228 | 140 to 1,570 | 1,854 |
| North-west skerries | 5 | Broughton Archipelago | 6 | 0 to 90 | 125 |
| Western fjords | 3 | Milford Sound | 150 | 80 to 1,710 | 2,260 |
| Western sound | 3 | Doubtful Sound (main sound, Thompson Sound) | 56 | 50 to 1,640 | 2,284 |
| South-west alpine arm | 6 | Zermatt (Matterhorn to Monte Rosa), Aoraki/Mount Cook (tip) | 145 | 370 to 3,700 | 4,301 |
| South-west sounds | 3 | Doubtful Sound (outer coast) | 32 | 40 to 1,330 | 1,853 |
| Southern drowned karst | 15 (14 inland) | Ha Long Bay, Wanfenglin | 119 | 10 to 930 | 1,548 |
| South-east limestone range | 6 | Dolomites (tilted down to the east) | 228 | 430 to 2,310 | 3,002 |
| South-east river lowland | 9 | Rakaia (upper valley) | 145 | 80 to 880 | 1,455 |
| Eastern desert basin | 8 | Sossusvlei inland, Badain Jaran on the coast | 132 | 40 to 770 | 1,380 |
| North-east dune field | 8 (16 crest dunes) | Badain Jaran, Liwa | 172 | 10 to 660 | 1,380 |

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
- Shasta (volcano flanks), Aletsch (SW arm glaciers), Geirangerfjord (western fjords) and
  Erg Chebbi (eastern desert) are not used. Kilimanjaro, Zermatt, Milford Sound and
  Sossusvlei with Badain Jaran cover those regions on their own.

## Flat ground

The design wants flatter pockets, not plains. The largest areas under 5° are:

- Coastal flats in the dune field, 4 km² near 30 m and 2 km² near 40 m.
- A basin at the knife ridges' south end, 4 km² near 830 m.
- The plateau top on the canyon's south rim, 3.5 km² near 2,100 m.
- Pockets of 1.5 km² in the river lowland and in the desert basin.

There is no large lowland plain. The Sossusvlei pan is only a small flat now (1.5 km²),
because the piece stops short of the coast so the pan does not meet the sea along a
straight line. The caldera's ice floor is flat but is not counted as a settlement site.

## Volcano

The highest point is 5,029 m, on the caldera rim. The highest land outside the volcano
region is 4,301 m, the SW arm's top at x 21.0, y 43.7 km, so the volcano stands 728 m
above it. Above about 1,300 m the cone is roughly 9 km in radius, and the Shira plateau
forms a broad western shoulder.

Kilimanjaro's own caldera is too shallow to show at 32 m, so the collage cuts one into the
summit from the feature's `caldera_diameter_m` and `caldera_floor_m`. It is centred on
Kibo's summit plateau at x 31.5, y 34.5 km (the feature's `at` is the summit, 0.9 km south
of the region's centre):

- The rim crest is an irregular oval 2.4 to 2.8 km across, at 4,830 to 5,029 m. Where the
  cone was lower than that, the rim is raised and its outer flank falls at about 37° until
  it meets the cone. Of the 56 km² raised by more than 50 m, two thirds lie within 4 km of
  the centre; the rest are deep gullies filled up to 6.7 km out. Ground detail finer than
  400 m is kept on the rim and flanks.
- The inner walls drop 200 to 400 m over about 450 m, at up to about 50°.
- The floor is flat ice (glacier class, 3.3 km²) at 4,600 m, rising 40 m toward the walls.
  No cone sits inside.
- The hollow is closed: the lowest rim notch is 4,814 m, 214 m above the floor, and the
  hollow below it is 3.6 km² (2.1 km across).

The description measures the same: `caldera_floor_m` 4,600, `caldera_rim_m` 4,814.

## Grand Canyon

The canyon runs east to west for about 24 km at y 15.5 to 18 km and leaves west into the
north coast mesas. Rims are 2,000 to 2,200 m. The crop reaches past the outline on every
side, so the side canyons fade out instead of ending on a crop edge. Along the feature
path the river reads 515 to 763 m. Its floor is a closed basin (see Known issues).

## Passes

| Pass | Band | At (x, y m) | Lowest ground m | Where |
| --- | --- | --- | ---: | --- |
| Tip pass | low | 10,500, 56,500 | 12 | Valley across the arm 3 km from its tip, from a sound to the south coast |
| Skerry gap | low | 9,000, 17,500 | 137 | Behind the Na Pali cliffs |
| Limestone gap | low | 57,500, 44,500 | 6 | Drowned-valley heads at the range's east end |
| Fjord saddle | middle | 22,000, 32,000 | 1,881 | Edge of the Shira shoulder above the fjords |
| North-west saddle | middle | 24,500, 27,500 | 1,480 | Between the volcano and the knife ridges |
| Plateau crossing | middle | 34,000, 20,000 | 1,527 | South rim plateau |
| Alpine col | high | 14,750, 49,500 | 2,554 | Col below the Zermatt crest |
| Volcano shoulder | high | 35,000, 31,500 | 2,575 | Between Kibo and Mawenzi |
| Horn col | top | 16,250, 49,000 | 3,520 | Col on the Monte Rosa crest |

Every pass is first reached by exactly its band, without dips. The tip pass is a real col
on land: its valley floor is 20 to 150 m, between a 650 m knob at the tip and the arm,
which rises past 750 m. No sea lies within 500 m of it. It is a low col because the blend
lowers the narrow far arm. Low ships reach 67% of the land, middle 90%, high 98% and top
99.3%.

No band reaches the caldera: its floor at 4,600 m needs a flight height of 4,800 m, above
the top band's 4,500 m, and `reach` marks it `above`. The top band reaches the cone up to
about 4,300 m, so the 16 km² around the summit, rim and caldera are above every band. Only
a ship that flies above the top band, such as the design's light liner or racer, reaches
the caldera floor.

## Remaining warnings

- **Volcano: largest patch under 5° is 0.30 km².** The cone uses Kilimanjaro at 0.667 of
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
- Lava flats, some volcano flanks and the skerry islands keep their source's plain class
  (18), so the class map shows plain patches there.
- The far SW arm is lower and smoother than its pieces. The arm is narrow, so the
  blend's coarse levels mix in a lot of sea. Aoraki's Main Divide at the tip ends near
  700 m.
- The north coast from x 21 to 42 km still runs nearly straight at y 6 to 7 km. Only the
  Canyonlands inlets break it, because a bay there would cut into the canyon rims.
- Inland basins below sea level, not connected to the sea: 89 of them, 5.3 km² in all,
  mostly sabkha flats behind the north-east coast and dune hollows on the desert coast.
  The largest are 1.09 km² at x 58.9, y 18.4 km (down to -82 m), 0.91 km² at x 49.4,
  y 9.9 km (-11 m), 0.83 km² at x 55.9, y 30.6 km (-35 m) and 0.41 km² at x 58.2,
  y 37.2 km (-60 m). They are dry salt pans for now; hydrology v2 will decide them.
- Closed depressions: `report.txt` lists 24 on land of at least 1 km² and 50 m deep, the
  caldera among them. Hydrology v2 must breach these. The largest:
  - The Grand Canyon floor, 8.6 km² and 229 m deep, lowest at x 24.9, y 15.7 km, spilling
    at 722 m. The feature path's west end (x 22.0, y 15.6 km) is at about 700 m. The
    canyon's west end needs a cut of about 200 m, or the river is routed out its east end.
  - 5.4 km², 306 m deep at x 19.6, y 37.5 km (western fjords), spilling at 341 m.
  - 4.9 km², 210 m deep at x 19.0, y 31.2 km (western fjords, west of the fjord saddle),
    spilling at 787 m.
