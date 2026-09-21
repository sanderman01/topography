# Río Abajo dataset permission evidence

Reviewed 2026-09-21. This is an evidence review for the original USGS OPR DEM
`PR_PRVI_D_2018`, tile `19QGA68505700`, and NOAA's distributed copy used in the
[karst trial](rio-abajo-karst-trial.md). **The project owner approved this
dataset-specific permission basis on 2026-09-21. Terrain acceptance and admission
remain unresolved.** The original reviewed version is retained with its hash;
the approval does not change original source metadata or the agency's wording.

The published evidence supports broad reuse, but the inspected dataset metadata
and agency policies do not explicitly discuss ML training for this tile. That
absence is not a published prohibition. Applying the broader permission to
training is the interpretation approved by the project owner under this project's
[licensing gate](../plans/terrain-design-master-plan.md#2-source-acquisition-strategy).

## Dataset-specific evidence

The [original tile XML](https://prd-tnm.s3.amazonaws.com/StagedProducts/Elevation/OPR/Projects/PR_PRVI_D_2018/metadata/USGS_OPR_PR_PRVI_D_2018_19QGA68505700.xml)
names USGS as originator, identifies this exact OPR tile, and connects it to 3DEP.
Its publication date is 2020-05-04. Its access constraint reports none, while
requiring comprehension of the metadata and compliance with the USGS disclaimer.
The use constraint:

- disclaims accuracy warranties and cautions about age and suitability;
- requests acknowledgement of originating agencies;
- obliges users to describe modifications;
- forbids misrepresentation and implying that USGS approved or endorsed changes.

These are recorded conditions for this tile, not an ML-specific license. Its
legacy disclaimer URL now leads to
[Privacy Policies](https://www.usgs.gov/office-of-the-director/privacy-policies),
which does not recover the historical disclaimer; the tile's own wording remains
available in the retained XML.

The [NOAA collection record](https://www.fisheries.noaa.gov/inport/item/60105/full-list)
identifies the Puerto Rico post-Maria DEM delivery, credits USGS and Dewberry,
and traces acquisition to USGS OPR project directories including
`PR_PRVI_D_2018`. NOAA records conversion to cloud-optimized GeoTIFF. It gives no
access restriction and cautions about temporal changes and suitability for
critical uses. NOAA and its partners do not accept responsibility for conclusions
from analysis. The record was updated 2026-03-04.

The [NOAA collection XML](https://noaa-nos-coastal-lidar-pds.s3.amazonaws.com/dem/USGS_PostMaria_PuertoRico_DEM_2018_9101/USGS_PostMaria_PuertoRico_DEM_m9101_met.xml)
also supplies a citation instruction in its legal-constraints section and a
suitability/liability disclaimer. Neither retained XML states CC0, a
dataset-specific public-domain declaration, or explicit ML-training permission.
The NOAA hosting location alone is not evidence of a different license.

## Applicable policy evidence and its limits

The current [3DEP program page](https://www.usgs.gov/3d-elevation-program)
states: “All 3DEP products are available free of charge and without use restrictions.”
The [product policy](https://www.usgs.gov/3d-elevation-program/about-3dep-products-services)
includes original-product-resolution DEMs among 3DEP source products. Together
with the tile XML, this provides a specific provenance chain to the program's
broad use statement. It is stronger evidence than an assumption about everything
on a USGS or NOAA portal. Interpreting that statement as covering training is
still an inference; no separate ML allowance was found in the material reviewed.

USGS's [copyright policy](https://www.usgs.gov/information-policies-and-instructions/copyrights-and-credits)
describes USGS-authored or produced data as U.S. public-domain material while
recognizing third-party exceptions and requesting credit. Its
[data-licensing guidance](https://www.usgs.gov/data-management/data-licensing)
distinguishes contractor/partner data and jurisdictional limits of U.S.
public-domain status; CC0 can waive rights more broadly. This guidance does not
itself assign CC0 to this dataset. Do not relabel the tile CC0 based on that page.

USGS's [credit guidance](https://www.usgs.gov/information-policies-and-instructions/acknowledging-or-crediting-usgs)
also provides examples for resale and adding value to USGS information. That
supports general reuse context, without answering this tile's ML question.

## Attribution, edited maps and redistribution

Carry forward the USGS/Dewberry credit, NOAA collection citation and exact tile
identity. Include the original metadata in any redistributed source package and
retain the transformation history with derived terrain. Keeping this provenance
is also a project requirement; it is not a newly inferred share-alike license.

For edited game maps, explicitly identify cropping, reprojection, repair,
blending, sculpting or other modifications actually performed. Preserve the
distinction between the measured source and authored output, and make no claim
of agency endorsement. Attribution in the USGS tile text is appreciated rather
than framed as a mandatory copyright license; describing modifications is
expressly obligatory. No royalty, noncommercial-only or share-alike condition
was found in the inspected tile/collection material. This does not resolve rights
in model weights or outputs by itself.

## Recorded human decision

The master plan requires a dataset-specific recorded clause or confirmation
that permits training; it excludes ambiguous permission from prepared and
training pools. It does **not** require contacting an agency in every case, nor
does it require the literal words “machine learning” in an otherwise applicable
permission. Requiring correspondence solely because those words are absent would
add a rule the plan does not contain.

The project owner was asked whether this permission evidence satisfies the
training-permission gate or whether permission should remain unknown pending
clarification. The reply was **“2. Looks good.”** The first numbered answer,
“1. Ready.”, separately authorized the bounded east-ridges point-cloud workload.

This confirms the documented connection of this exact dataset to the 3DEP
unrestricted-use statement as the project's ML-training permission basis, while
retaining the source conditions above. It is not a new agency letter, a CC0
designation, or human acceptance of terrain quality. The evidence record pins
the exact reviewed report, raw receipt, source asset and all three existing crop
identities. Assistant-authored catalog annotations carry this captured human
decision without impersonating a human terrain reviewer.

If clarification is chosen, a focused request can identify both the original
tile and NOAA copy, reference the 3DEP statement, and ask whether it covers
training terrain-generation models and distributing edited terrain and trained
weights, including any contractor or geographic limitations. The published
contacts are `3DEP@usgs.gov` on the program page and
`coastal.info@noaa.gov` on the collection record. No message was sent.

Permission confirmation still leaves whole-crop quality, terrain acceptance
and the other independent admission gates open; see the
[ground-support report](rio-abajo-ground-support.md).

## Retained evidence and verification

Only two small metadata documents were downloaded, into
`data/review/rio-abajo-permission-2026-09-21/`. The directory also contains
`sources.json` with source URLs, retrieval notes, byte counts and hashes. These
ignored local files need separate backup.

The same directory now retains `reviewed-report.md`, the exact report before
approval, and `human-decision.json`, which captures the question, response,
interpretation and immutable source/report identities. `sources.json` remains
the original retrieval record and accurately describes the pre-approval state.
Continuation drafts and catalog IDs are kept separately under
`data/review/rio-abajo-curation-2026-09-21/`. Original records remain preserved.

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `original-usgs-tile.xml` | 11,312 | `9c16846d2b516d04bd213a8cc235998437ddcea775cb72867477c6003249c7d9` |
| `noaa-dem-collection.xml` | 70,806 | `6313f0cb304ffc26e5be51c660cbc3c032faf7adbc546bab79f41a62133f0c81` |

Both hashes match the earlier retained metadata. XML parsing and inspection
covered the identification, lineage and constraints fields. The current NOAA
record, USGS program page, copyright/credit guidance and legacy URL redirect were
read through the browser tool. The product and licensing policy text was also
available through the official-domain search results, but subsequent full-page
fetches timed out; direct policy-page downloads returned HTTP 403, including a
retry outside the sandbox. No HTML snapshot is claimed. No new raster processing,
model work or executable changes were needed for this review.
