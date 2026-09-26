# Cover acquisition

Water, snow/ice, vegetation and land cover are game layers under policy
`game_cover_v4` ([`configs/cover-game-v4.toml`](../../configs/cover-game-v4.toml), built into
the binary). The gallery worker generates them for every accepted tile; no
profile descriptors, dates or scripts are needed. `topography cover draft`
writes the same layers for one crop so you can inspect them:

```sh
topography cover draft <raw-receipt.json> <review.json> <new-dir> [cover-policy.toml]
```

The environment variable `TOPOGRAPHY_COVER_POLICY` can point the worker and
`cover draft` at another copy of the policy, for example the loopback fixtures
used by the checks. Nothing here writes curation or records a vote.

## Rules

Masks lie on the review crop grid (1 absent, 2 present, no unknown cells) and
are placed by nearest neighbour. `land-cover.tif` keeps the raw WorldCover
class of each cell (0 outside coverage) so renderers can choose materials.

- Water present: the tile's existing water mask (provider or assessed), or
  WorldCover class 80, or OpenStreetMap water. OSM comes from the smallest
  Geofabrik regional extract containing the crop, downloaded once and reduced to
  `<runtime cache>/osm-extracts/<region>.water.gpkg` (refreshed after 180 days;
  `source = "overpass"` uses one cached Overpass bbox query per crop instead).
  Features: `natural=water`, `water=*`, `waterway=riverbank` and tidal flats
  (`natural=mud`, `wetland=tidalflat`) as areas and multipolygons;
  `waterway=river|stream|canal` lines buffered to a numeric `width` tag, else
  12 / 3 / 8 m. Lines burn all touched cells.
- HTTP requests are spaced per host (Overpass 10 s, S3 0.1 s, others 1 s) across
  all topography processes, and rate limits are retried with backoff.
- Water wins: water cells are never snow/ice or vegetation. Under water the
  ground uses the deterministic masked fill.
- Snow/ice present: ESA WorldCover 2021 class 70, or inside the most recent
  GLIMS `glac_bound` outline of each glacier. Outlines dated before 1900 are
  ignored. Survey dates, seasons and boundary bands are not considered.
- Vegetation present: WorldCover classes 10, 20, 30, 40, 90, 95 and 100.
- Everything else is absent, including WorldCover nodata.
- A crop with no WorldCover tile (outside its coverage) is all absent. The
  note in `cover.json` records this judgment.
- If WorldCover, GLIMS or Overpass cannot be reached after retries, or
  Overpass reports an incomplete result, the job fails as a technical error.
  The tile shows *needs attention* and the worker retries it at the next service start.

## Output

- `water.tif`, `snow-ice.tif`, `vegetation.tif`: single-band Byte GeoTIFFs with
  no nodata and the review crop's size, geotransform and CRS.
- `cover.json`: the policy text and digest, WorldCover URLs and missing tiles,
  the GLIMS query URL and chosen outlines, the Overpass query and response
  digest, licences, citations and attribution ("© OpenStreetMap contributors,
  ODbL"), counts per water source and a note. It is the only cover evidence file.

Sources: ESA WorldCover v200 (CC BY 4.0, cite Zanaga et al. 2022,
doi:10.5281/zenodo.7254221), read as HTTPS range requests of the public COG,
GLIMS glacier outlines via WFS (cite NSIDC-0272, doi:10.7265/N5V98602) and
OpenStreetMap via the Overpass API (ODbL; sent with a User-Agent, curl retries
four times, responses up to 64 MiB).
GLIMS responses are capped at 1 MiB; truncated responses fail. Crops spanning
more than one degree or crossing the antimeridian are refused.

Products built earlier (v1 terrain, strict-evidence v2 or `game_cover_v1`)
stay on disk, but the worker replaces them for accepted tiles with a
`game_cover_v4` successor; see the
[automatic-preparation design](../design/automatic-preparation-v1.md).

Verification: `scripts/check_cover_acquisition.py` (run by `scripts/check.sh`)
exercises the rules, the worker paths and a source failure against a loopback
HTTPS server with synthetic fixtures (`scripts/cover_fixtures.py`).
