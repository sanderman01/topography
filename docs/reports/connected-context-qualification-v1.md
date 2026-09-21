# Wider connected-context qualification

Completed 2026-09-21. The owner selected both pending tiles in gallery v3 and
confirmed the gallery worked. The catalog contained the actual saved Yes records;
no duplicate votes were transcribed from conversation. Technical qualification
preserves those selections and identifies Codex as the technical assessor.

| Crop | Original gallery record | Prepared manifest |
| --- | --- | --- |
| Central gorge | `2d4f91ab6ebfaf2ec750af957751a4ff138267c0f1646f070406e80b7dff766c` | `5825e5aea634a94716141cd2931d392e29341bf8be50b159416424a5ae93de88` |
| Río Abajo | `459fd985514827a39f6977e20da6744e841db2ed7ab64d74d3f23cc2ae9cc657` | `3c44de23f1ed38205b75d40c2607b29d47dfc9edab21233e9c6d27986255e056` |

Each crop is 2,048² cells at native 0.5 m spacing: 1,024 × 1,024 m.
Both reuse already qualified raw sources and their exact-source permission and
supplier QA evidence under [supplier_qa_pilot_v1](../design/source-quality-pilot-v1.md).
No new LiDAR measurement, confidence threshold or feature annotation was required.

Codex inspected the full wider footprints in native hillshade and public USGS
context imagery at 2 m image sampling. No open water surface or concrete defect
requiring exclusion was identified. Both masks classify all cells as terrestrial
and supplier-supported, with zero repairs and zero optional feature annotations.
Canopy can conceal small streams; this is a documented classification judgment,
not proof of absent water or independently measured fine-feature accuracy.
The gorge's native roads and clearings remain present.

The CLI reports 4,194,304 usable terrestrial cells per crop, no assessment blockers,
and eligible current source decisions. Prepared verification reports intact files,
registration and current parents. Independent decoded comparison checked all
8,388,608 Float32 samples against their native reviews bit for bit and checked
every output mask. Original thirteen decision heads and all historical records
remain unchanged. The catalog now has 39 historical records, 15 current heads
(14 accepted, one rejected), and 14 prepared regions.

Retained local evidence is under `data/review/connected-context-qualification-v1/`:
catalog backup, actual gallery records, context download list and receipts,
classification evidence, masks, assessments, curation exports, preparation requests,
`records.json`, `native-comparison.json` and `coverage.json`. Prepared products are
under `data/prepared/<manifest-sha256>/manifest.json`.

These wider contexts overlap older crops. There are still six independent areas
and one source collection; unique selected footprint is 4.406211 km².
They are outside the existing twelve-crop numeric releases and their 512 m split
contract. Broader public geographic coverage remains the next priority before
another world-quality review. No new model training or world acceptance follows.
