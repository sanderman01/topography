# Fujion: brief

Owner's brief for a 32 km island world, agreed in an interview on 2026-10-03.
Step 1 of the [layout spec process](../../../docs/plans/world-layout-spec-v1.md);
the placed layout is [layout.toml](layout.toml), summarised in [layout.md](layout.md).

## Purpose

A compact island holding every terrain class in the library, for low-altitude
dirigible travel between small settlements (placed later). Biomes are compressed
on purpose: about 12 km of good canyon, not the Grand Canyon. Challenges the
terrain must support: navigation by eye and simple tools, climbing through passes
or over ridges, and flying canyons to shelter from sandstorms.

## Shape and geology

- **Hybrid island.** The windward half is star-shaped: rift and range arms run out
  as peninsulas with fjords and bays between them. The lee half is a broad,
  blocky plateau. Wet sides erode into fingers; dry sides stay solid.
- **An X of two crossing structures.** A volcanic rift runs SW-NE through the
  central volcano; an older uplifted belt runs NW-SE across the wind and brings
  the limestone (karst) and layered rock (canyons, mesas). Fuji, at a collision
  zone where an arc meets the mainland, is the precedent.
- **Climate:** a compressed New Zealand South Island at about 45° latitude.

## Central volcano

- Isolated stratovolcano, 3,776 m (Fuji's height), 16 km base. Steeper than
  Fuji, with a concave Mayon-like profile: gentle lower flanks, over 35° near the top.
- Summit crater about 800 m across, filled with ice and snow. No cone-in-caldera.
- Ice cap on the upper cone, an outlet glacier down the windward flank.
- The ever-present landmark: it must stay the highest point by a clear margin.

## Wind and climate

Prevailing wind from the south-west. Wettest in the south-west (fjords, glaciers,
low snowline); wet along the south coast (karst); driest in the north-east lee
beyond the volcano (canyon plateau, badlands, dunes). Snowline about 1,600 m
windward and 2,100 m lee; treeline about 1,000 m.

## Regions

| Area | Terrain |
| --- | --- |
| W bay, windward | Fjords, with an outlet glacier and a small braided outwash fan at one fjord head |
| SW arm | Alpine ridges and cirques, glaciers, a horn peak |
| NW arm | Na Pali-style knife ridges and sea cliffs, sea stacks |
| S bay | Ha Long-style drowned karst: towers on the shore and standing in the sea |
| NE lee, north | Canyon plateau: a main gorge with side canyons |
| NE lee, east | Badlands and mesas |
| NE lee, coast | Dune field, tall against the end of the rift, low toward the coast |
| NE arm | Rift with cinder cones and lava fields running into the desert |

## Altitude tiers

Vehicle progression sets the terrain's height rules:

| Tier | Ceiling | Terrain rule |
| --- | --- | --- |
| Starter | 1,500 m | Coast always followable; each of the four arms has a low pass at 1,100-1,300 m, placed far out on the arm (a detour) |
| Mid | 2,500-3,000 m | High saddles at 2,000-2,400 m near the volcano and a route over its shoulder at about 2,700 m: slow, costly shortcuts |
| Specialist | above 3,776 m | Summit crater |

Mid-tier-only destinations sit in basins above 1,500 m ringed by higher ridges.

## Hero features

- **Canyon system:** main gorge about 12-15 km, 400-700 m deep, floor 150-400 m
  wide, cut by a snowmelt river from the volcano to the north coast. Two or three
  side canyons of 3-5 km, mostly 60-150 m wide; some dead ends, some climbing out
  onto the plateau; one or two risky slots 25-50 m wide.
- **Water:** two or three lava-dammed lakes at the volcano's N and E foot (like
  Fuji's Five Lakes), a meltwater tarn in the SW cirque basin (mid tier), short
  steep rivers in the wet SW and S, dry washes in the NE, no summit lake.
- **Landmarks, one per quadrant:** NE rift cinder cones and a 1,600 m flank cone
  with an open-sided crater (like Hōei); a 2,600 m horn peak on the SW arm; one
  500-600 m karst tower; a 300-500 m volcanic islet off the Na Pali coast.
- **Dunes:** about 50-60 km²; star dunes 150-250 m high banked against the
  rift's end cones, like Great Sand Dunes, fading to 10-40 m crescent and ridge
  dunes toward the coast.

## Defaults accepted

- Coasts: fjord mouths with skerries, a cliffed headland at the SW arm tip, Na
  Pali walls, drowned towers, a delta and beaches at the dune coast, low bluffs
  elsewhere.
- Seabed: short shelf, then down to about -200 m at the map edge (coarse only).
- Each region reserves at least one 0.5-1 km² patch under 5° for a settlement.

## Not wanted

- A cone inside a larger caldera.
- Large floodplains or flat plains (the braided river stays a small fan).
- A summit crater lake.
- A volcano visibility check: being clearly the highest point is enough.
