# Banks Peninsula qualification

2026-09-21. The gallery saved three Yes decisions for the inland Banks Peninsula
crops and one No for the coastal crop. The three selected crops are now technically
qualified and prepared under `supplier_qa_pilot_v1`. The rejected crop and its
original source bytes remain preserved and excluded.

The [source report](banks-peninsula-review-v1.md) identifies both public LINZ
assets, their verified published checksums and the native 1 m grid. The exact
collection assigns CC BY 4.0; the linked
[survey record](https://portal.opentopography.org/datasetMetadata?otCollectionID=OT.022024.2193.2)
records the same license and credits. Codex applies the copying, adaptation,
technical-modification and database grants in
[CC BY 4.0 sections 2–4](https://creativecommons.org/licenses/by/4.0/legalcode.en)
to intended terrain-model training, with attribution, license and change notices.
This is a recorded implementer assessment, not a new human or institutional
legal decision. Source metadata and the survey page are archived; the official
legal code was read through the browser after direct downloads returned HTTP 403.

The published
[Landpro report](https://object.cloud.sdsc.edu/v1/AUTH_opentopography/www/metadata/NZ23_Banks_metadata.pdf)
documents ground classification, manual correction, flight-strip calibration,
regional accuracy checks and NZTM/NZVD2016 coordinates. Native hillshades show
supplier triangulation in forested ravines, retained as routine canopy
interpolation under the owner's instruction. Supplier QA and nominal spacing do
not establish measured local fine-feature accuracy.

Full-footprint imagery and native hillshades showed inland slopes, ravines and
tracks, with no identified open water. All cells are classified terrestrial for
this pilot; canopy, shadow and 3 m context-image sampling can conceal small
streams. Context imagery is inspection evidence only. Each crop has 2,359,296
finite provider-valid cells; no repairs, feature annotations, resampling or
elevation changes were applied.

| Selected crop | Prepared manifest SHA-256 |
| --- | --- |
| Western source, north | `b66c3b8d800586aefecd65e87dabb33d7e987c3f3cf3c8260b74cb5d5794ca4b` |
| Western source, south | `b53a967d326d3ef30d9e2b9918400628916fa93f528a6af905f98b1767e96ea2` |
| Eastern source, north | `ad8ece3361f71a6c3305263b42efb83ec26f104a809b5957fa99bd994e6f01bc` |

Evidence, exact original gallery decisions, masks, curation exports and requests
are under `data/review/banks-peninsula-qualification-v1/`. CLI quality assessments,
eligibility and prepared verification pass. Independent decoded comparisons
verify all **7,077,888 Float32 samples** and every prepared mask. All 43 earlier
curation records and the other 16 current heads remain unchanged. The catalog now
has 46 records, 19 heads (17 accepted, two rejected) and 17 prepared regions.

The three non-overlapping selections add 7.077888 km² in a second survey
collection. They are separate native regions, without a cross-survey join or
extension of the original twelve-region numeric release. Preparation does not
admit a training release or approve a composed world.
