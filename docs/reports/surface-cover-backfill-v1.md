# Surface-cover backfill and technical exceptions

Updated 2026-09-25. Follows the [surface-cover checkpoint](surface-cover-workflow-v2.md).
No vote changed and no raw asset, earlier curation record or earlier product was
modified. Gallery v13 (`data/review/library-gallery-v13/gallery.json`) shows
33 Yes / 2 No: **15 Ready** under the v2 cover contract, 15 surface assessment
pending, 3 needing technical attention. The catalog holds 92 records and 45
prepared products; `curate check` is clean.

## Tooling added

- `topography cover draft` / `cover merge` ([workflow](../workflows/cover-acquisition.md))
  draft snow/ice and vegetation masks from ESA WorldCover 2021, GLIMS outlines and
  Sentinel-2 SCL near the DEM acquisition date. Values in
  `configs/cover-sources-v1.toml` are draft pilot choices.
- The gallery offers read-only water, snow/ice and vegetation overlays for v2
  products; unknown cells are hatched, absent cells are clear.
- Grid transforms now compare with a round-trip tolerance. Exact comparison had
  blocked both dunes tiles through a last-digit float parsing difference.

## Real cases

| Case | Tiles | Outcome |
| --- | --- | --- |
| Snow/ice absent, imagery evidence | Río Abajo ×3 | v2 Ready; vegetation unavailable |
| Snow/ice absent, dated sources | Grand Canyon ×4, Badlands ×2, Banks ×3, gorge-east-ravines | v2 Ready with WorldCover vegetation masks |
| Snow/ice absent, supplier swath dates | Great Sand Dunes ×2 | v2 Ready; striping recorded as a limitation |
| Snow present, small unknown remainder | St. Helens east flank (9% snow), southwest flank | Blocked: 1,417 and 5,337 unknown cells |
| Glacier with unresolved extent | St. Helens crater rim | Blocked: 526,823 unknown cells (debris-covered ice) |
| No usable seasonal evidence | Camuy, Arecibo, Guajataca, Río Abajo context, river-north, gorge-west, central-gorge, Milford ×3, Zion ×2 | Not applied; snow/ice unknown |

Acquisition dates come from point-cloud GPS times or supplier swath reports; see
`data/review/cover-backfill-v1/notes.md`. Dunes analysis:
`data/review/dunes-striping-v1/notes.md`. St. Helens masks:
`data/review/st-helens-surface-cover-v1/notes.md`.

## Dunes striping

The stripes are a supplier gridding artifact, not dune form: a 5.3 m period
nearly parallel to north–south flight lines, visible only on east/west-facing
slopes, and absent from a TIN of the supplier's own ground points. RMS is about
0.9 cm (p99 3.4 cm), below the project's measured RMSEz of 5.25 cm. The tiles
qualify with this limitation; 1 m hillshade texture must not be read as
micro-relief. The earlier profile cited the wrong work unit (Block 5 instead of
Block 4).

## Source record corrections found

- Grand Canyon tiles lie in work unit `AZ_GrandCanyonNP_1_2019`, not `_2_2019`.
- Badlands point cloud is `SD_Southwest_B1_2018`, not the cited Block 5 report.

## Open decisions

- **Sentinel-2 gaps.** Puerto Rico, Milford and Zion fail only the seasonal check.
  Earth Search `sentinel-2-c1-l2a` is sparse there (24 Puerto Rico scenes in 2018
  against 90 in `sentinel-2-l2a`), clouds persist, and Zion predates coverage.
  Options: a fallback collection, another dated source, or a versioned rule for
  climatologically snow-free sites.
- **Small unknown fractions.** St. Helens flanks are blocked by 0.03% and 0.13%
  unknown cells. The v2 rule blocks any unknown snow/ice cell.
- **Shoestring Glacier.** Only a 1980 GLIMS outline covers part of the east flank.
- **Gorge date split.** The applied interval split is a judgement call; earlier
  drafts are retained.
- **Housekeeping.** About 2.5 GB of staged Banks/Milford point clouds remain in
  `data/staging/cover-backfill-v1/lpc/`.
