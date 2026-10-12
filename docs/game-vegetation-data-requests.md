# Vegetation and class data requests from the airship game

The airship game now places trees, shades ground and draws distant forests from the export's
biome, land-cover and vegetation layers (`airshipgame` branch `feat/landscape-visuals`,
`docs/design/landscape.md`, `docs/art-style.md`). This note lists what it would use from the next
export, most useful first. It complements the water contract in
[export-data-flaws.md](export-data-flaws.md); where both touch the manifest, one schema should
cover both.

The game works without any of these. It falls back to rules on land cover and a legend file it
carries itself, so each request can land on its own.

## 1. Legends in the manifest

The manifest says that `biome` holds ids from `catalog/labels/terrain-v3.toml` and `land-cover`
holds ESA WorldCover codes, but not what the ids mean. The game copied both legends into its map
package (`assets/basegame/map/terrain-v3.legend.json`) and keys its palette and vegetation rules by
stable class keys, so new or renumbered classes need no code change once the legend says what they
are. Please write the legends into `manifest.json`, in this shape:

```json
"legends": {
  "biome": {
    "legend": "topography terrain-v3",
    "classes": [{ "id": 6, "key": "alpine_ridges", "name": "Alpine ridges and cirques" }]
  },
  "land-cover": {
    "legend": "ESA WorldCover v200",
    "classes": [{ "id": 10, "key": "tree_cover", "name": "Tree cover" }]
  }
}
```

- `key` is stable across exports and legend versions: a class keeps its key when its id or name
  changes. The game uses the keys in the legend file above; please adopt them (or tell us the ones
  you prefer before the next export).
- List only classes that can occur, or add `"present": true/false` per class.
- Unlabelled cells (biome id 0 exists in a few texels) should be named, for example
  `{ "id": 0, "key": "unknown" }`.

## 2. Forest potential, not a binary vegetation mask

The vegetation layer is binary and, in `fujion-v3-chain-14-l4`, equal to land cover 10 except for
a rim at water. It carries no information the game does not already have. Forests come out as
solid blobs with no density gradient, so the game invents clearings and edge fringes with noise.

`cover.py` (`Sampler.fields`) already computes the continuous field before thresholding it:

```python
share = forest * _ramp(treeline - h, 0.0, TREE_FADE_M) * (1 - _ramp(slope, *TREE_STEEP_DEG))
veg = land & ~snow & (share + 0.25 * patchy > 0.5) & (share > 0.05)
```

Please export `share` (with or without `patchy` folded in, documented which) as a new layer:

- Name `forest-potential`, prefix `forest`, 8-bit grayscale PNG, `value / 255` = 0 to 1.
- Document it as a procedural control, not measured canopy cover. The game uses it as tree
  density.
- Keep the binary `vegetation` layer for now; the game will switch when this lands.

## 3. Other continuous fields, if cheap

All 8-bit PNG on the existing tile grid, one file per layer, each with an entry in the manifest
(see 5). In order of use to the game:

| Layer | Encoding | Source in `cover.py` | Game use |
| --- | --- | --- | --- |
| `hollow` | 128 = flat, 0 = most convex, 255 = most concave | `terrain_features` `hollow` | Lush gullies and dry spurs in ground shading; denser trees in hollows. |
| `treeline-margin` | 4 m per code, 128 = at the tree line | `treeline - h` | Smaller, sparser trees toward the tree line. |
| `snow-cover` | 0 to 1 | `ramp(h - line)` before `shed` and thresholding | Soft snow edges instead of a binary mask. |
| `needleleaf` | 0 to 1 share of conifers | a rule from tree-line margin, aspect and biome | Species mix; today the game guesses it per biome and altitude. |
| `moisture` | 0 to 1 | the sketch's wind-side moisture | Lush vs dry grass, shrubs. |

Canopy height and flow accumulation would also help but need new rules or hydrology; they can
wait for the water work.

## 4. Coarse levels

Masks at levels 2 to 4 are decimated, so a binary mask aliases there and per-tile tree density
jumps between levels. The game places its far trees (2 to 7 km) from level 2. Please build coarse
levels by area mean for continuous layers (including the new ones above) and by mode for class
layers, from a global pyramid before cutting tiles. Heights stay as they are unless the water
contract changes them.

## 5. Per-layer metadata

For every layer in `masks.layers`: prefix, format, encoding (scale, offset, unit), how coarse
levels were filtered (mean, mode, max, decimated), whether values are observed or procedural, and
what a missing file means. The game treats a missing mask as empty today, which is wrong for
exclusions such as water and snow.
