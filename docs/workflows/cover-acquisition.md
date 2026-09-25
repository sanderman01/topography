# Cover acquisition

`topography cover draft` builds the version-2 `snow_ice` and `vegetation`
descriptors for one raw receipt and review crop from public sources. It saves
the evidence and complete source-grid masks, so a v2 profile no longer needs
hand-built cover masks. The output is a technical draft. The tool never writes
the catalog, records a vote or marks anything accepted. The sources and tolerances in
[`configs/cover-sources-v1.toml`](../../configs/cover-sources-v1.toml) are
draft pilot choices.

```sh
topography cover draft <raw-receipt.json> <review.json> configs/cover-sources-v1.toml <new-dir> [--acquired YYYY-MM-DD[/YYYY-MM-DD]]
topography cover merge <profile.json> <new-dir>/cover.json <new-profile-dir>
```

The DEM acquisition date comes from the source's `acquisition_dates`. Each
entry's leading `YYYY-MM-DD` or `YYYY-MM-DD/YYYY-MM-DD` token is parsed.
`--acquired` supplies or overrides the date. `cover.json` records which was used
(`source_metadata`, `command_flag` or `none`).

## Sources and rules

The tool works on the review window plus a 100 m buffer, in exact source-grid
cells. Cells outside that area are 0 (unknown). Categories are 0 unknown,
1 absent and 2 present. Coarse evidence is placed with nearest-neighbour
warping (`-r near`, exact transforms) and is never interpolated. Cells within a
boundary band of a present/absent change become unknown. The band is a square
of the configured width in metres, converted to cells.

| Source | Use | Reads |
| --- | --- | --- |
| ESA WorldCover v200 2021, 10 m (CC BY 4.0) | Class 70 marks snow/ice present; other valid classes mark it absent (20 m band). Classes 10–40, 90, 95 and 100 mark vegetation present; 50–80 mark it absent (no band). 0 is unknown. | `gdal_translate -projwin` crop of the public COG over HTTPS (the `esa-worldcover` S3 object) |
| GLIMS outlines (cite NSIDC-0272, doi:10.7265/N5V98602) | Outlines dated before 1900 are treated as reconstructions and dropped. For each glacier, the tool takes the `okay` outline nearest the DEM date and burns it with `gdal_rasterize` (30 m band). If that outline is more than 5 years from the DEM date, or there is no date, the outline and its band are unknown. | WFS GeoJSON bbox query |
| Sentinel-2 L2A SCL, Earth Search `sentinel-2-c1-l2a` (Copernicus data) | Scenes within ±15 days of the DEM date, at most 12, nearest first. Class 11 in any scene marks the cell present. A cell is absent only if some scene sees it clear (2, 4–7) and none sees snow. Clouds, shadow and nodata stay unknown (40 m band). | STAC search JSON; per-scene COG window warp |

Snow/ice is present where any source says present. It is absent only where
WorldCover, GLIMS and the seasonal check all say absent. Every other cell is
unknown. Seasonal snow cannot be checked, and snow/ice therefore cannot be
absent, when:

- there is no DEM date;
- the DEM date interval is longer than 31 days;
- the DEM predates Sentinel-2 (2015);
- no scene covers the window;
- the STAC search fails.

In those cases the report says `SNOW/ICE UNRESOLVED`. The resulting profile
blocks automatic preparation until the extent is resolved, so the tile needs
technical attention rather than another review.

If WorldCover cannot be read, vegetation becomes `unavailable` and snow/ice
stays unknown. If GLIMS fails, its layer is unknown. A source failure is never
treated as absence.

## Output

- `snow-ice.tif` and `vegetation.tif`: single-band Byte GeoTIFFs with no nodata,
  on the exact raw source size, geotransform and CRS. The tool checks them with
  the same rules as the automatic stage.
- Captured evidence, each at most 1 MiB with a SHA-256 and HTTPS
  `source_url`:
  - `cover-policy.toml`;
  - `cover-worldcover.tif`, the native crop;
  - `cover-glims.json`;
  - `cover-sentinel2-stac.json`;
  - `cover-sentinel2-scl.tif`, one band per scene. These are nearest-neighbour
    crops on the DEM CRS at 20 m, listed in `cover.json`.
- `cover.json`: evidence entries, both descriptors, source status and details,
  review-window counts per layer and source, limitations and attribution.
  `all_absent` (with the review window) is chosen when the whole review window
  is absent. Otherwise the descriptor is `mask`. Vegetation is `unavailable`
  when WorldCover failed.
- `report.txt`: counts, limitations and citations.

`cover merge` copies the profile's evidence and water mask into a new
directory. It adds the cover evidence, masks and `cover.json` under `cover/`,
and writes a schema-2 `supplier_qa_cover_v1` profile. It keeps the findings and
blockers, fails on mismatched source identity or duplicate evidence IDs, and
enforces the 15-item evidence limit.

## Limits

- Network reads are bounded. JSON responses are capped at 1 MiB and retried by
  curl. WorldCover crops are planned as a VRT and checked against the
  cell limit before any pixels are read. Sentinel-2 reads only the COG blocks
  that cover the work window. There is no hard byte cap on GDAL range reads.
- WorldCover is a 2021 composite at 10 m; its date may differ from the DEM's.
- GLIMS coverage is incomplete, so a missing outline is weak evidence of no glacier.
- Truncated WFS/STAC responses fail and do not return partial results.
- Windows crossing the antimeridian or spanning more than one degree are refused.

Verification: `scripts/check_cover_acquisition.py` (run by `scripts/check.sh`)
runs the workflow against a loopback HTTPS server with synthetic fixtures.
