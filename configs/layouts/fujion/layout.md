# Fujion: layout summary

Draft 1 of the [layout.toml](layout.toml) spec for the [brief](brief.md). North is
up; each character is about 680 m wide and 1,370 m tall.

```text
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~.~~~~~~~~~~~~~
~~~~~~~~~~~NNPPPPPPPPPPPPPPPPP=PPPDDDDDDDD.~~~~~
~~~~~~NNNNNNNNPPPPPPPPPPPPP==P=PDDDDDDDDDDDD~~~~
~~~~NNNNNNNNNNNPPPPPPPPPP=PPPPPPDDDDDDDDDDDD.~~~
~~~~NNNNNNNNNNNNPPPPPPPPP==PPPPPPDDDDDDDDDDD.~~~
~~~~NNNNNxxNNNNNPPPPPPPP=PPPPPPPPPRxDDDDDDD.~~~~
~~~~NNNNNNNNNNNN.VVVVVVoooVoo.PRRRR...BBBB..~~~~
~~~~.NNNNNNNNNNNVVVVVVVVVVVVRRRR..BBBBBBBBBB~~~~
~~~~FFFFFNNNNVVVVVVVVVVVVVVVcVVV..BBBBBBBBBB~~~~
~~~FFFFFFFFFFVVVVVVVVVVVVVVVVVVVV.BBBBBBBBB.~~~~
~~~FFFF~~~FFFVVVVVVXXVVVVVVVVVVoooBBBBBBBBB~~~~~
~~~~FFFFFFFFFVVVVVVVV^^VVVVVVVVoooBBBBBBBBB~~~~~
~~~~FFFFFFFFFVVVVVVVVVVVVVVVVVVVVBBBBBBBBBB~~~~~
~~~~FFFFFFFFFVVVVVVVVVVVVVVVVVVVVBBBBBBBBBB~~~~~
~~~~~~~~~~~FV~AVVVVVVVVVVVVVVVVV.BBBBBBBBBB~~~~~
~~~~~~FFFFFAAAAAAVVVVVVVVVVVVVV.....BKKKKK~~~~~~
~~~~~~~..AAAAAAAAAVVVVVVVVVVV...KKKKKKKKKK~~~~~~
~~~~~~.AAAAAAAAAAAA.VVVVKKKKKKKKKKKxKKKKKKk~~~~~
~~~~~AAAAAAoAAAAAAAAKKkkkkkkkkkKKKKKKKKKKKk~~~~~
~~~~AAxAAAAAAAAAAAAkkkkkkkkkkkkkkkkKKKKKKkk~~~~~
~~~~~~AAAAAAAAA~~kkkkkkkkkkkkkkkkkkkkkKKKkk~~~~~
~~~~~~~~~~~~~~~~~~kkkkkkkkkkkkkkkkkkkkkkkkk~~~~~
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
```

`^` summit, `V` volcano, `c` flank cone, `F` fjords, `N` Na Pali arm,
`P` canyon plateau, `=` main gorge, `D` dunes, `R` rift cones, `B` badlands and
mesas, `K` karst, `k` towers in the sea, `A` alpine arm, `o` lakes,
`.` lava apron, `x` starter pass, `X` mid-tier pass, `~` sea.

## Areas

Island about 660 km² of land at 256 m cells, fjords cut out.

| Region | Class | Land km² | Target |
| --- | --- | --- | --- |
| Volcano cone | 11, 4 crater, 10 ice | 171 | 200; neighbours take its outer flanks |
| Canyon plateau | 2 | 91 | 90 |
| Badlands and mesas | 7 | 80 | 60 |
| Na Pali arm | 12 | 66 | 70 |
| Alpine arm | 6 | 59 | 45 |
| Fjords | 3 | 53 | plus the bay and fjord water |
| Drowned karst and SE arm | 1 | 51 | plus the towers in the bay |
| Dunes | 8 | 47 | 55 |
| Lava apron (default) | 11 | 32 | none |
| Rift cones | 11 | 9 | 20, mostly covered by dunes |

## Heights and tiers

| Feature | Height |
| --- | --- |
| Summit crater rim / floor | 3,776 / 3,550 m, ice-filled, 800 m across |
| Horn peak, SW arm | 2,600 m |
| Cirque basin (mid tier) | floor 1,800 m, rim 2,300 m, lip 1,750 m, tarn 1,820 m |
| Flank cone | 1,600 m |
| Na Pali ridges | up to 1,200 m |
| Canyon plateau rim | 1,100 m at the volcano's foot to 700 m at the coast |
| Main gorge floor | 740 m to sea level, 400-700 m deep, about 12.3 km |
| Lakes | head 780 m, rift 820 m, east 640 m |
| Karst towers | 100-400 m, great tower 550 m |
| Star dunes | 150-250 m above their base |
| Islet | 400 m |

Starter passes: Na Pali gap 1,000 m, rift gap 1,000 m, karst arm 1,200 m, alpine
arm tip 1,250 m. Mid-tier passes: alpine saddle 2,200 m, volcano shoulder 2,700 m.

## Known issues

- Fjord land is 53 km²: the bay is narrow and the fjords cut into the
  arms and the cone's lower west flank.
- Tower karst at about 45° latitude is a deliberate cheat (brief).
- Slots 25-50 m wide exist only from the 4 m level down.
- The cascade takes one terrain class per field today (`cascade_chain`
  derives its layout from one real region). This world needs an authored class
  map, envelope and feature lines, which is step 1 of the
  [cascade plan](../../../docs/plans/terrain-cascade-v1.md).
