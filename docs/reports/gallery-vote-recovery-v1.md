# Gallery vote fix and recovery

2026-09-21. The owner reviewed the five new Grand Canyon/Zion tiles and explicitly
said Yes to all. Both Zion choices saved, but the three Grand Canyon choices
failed with `finding requires known evidence IDs`. Gallery v7 now contains
26 Yes, two No and no pending tiles. No repeat review was requested.

The first gallery record copied a source declaration's confirmed training
permission without attaching the evidence required for a curation permission
assessment. The Grand Canyon declaration had that status; Zion's was unknown.
The shared validator therefore rejected the new Grand Canyon visual decisions.

A first visual decision now keeps permission unknown until curation evidence is
attached. Source declarations and descriptive license fields remain intact;
explicit restrictions and prohibitions remain in force. Existing curation
permission findings and captured evidence survive subsequent Yes/No changes.
The evidence validator and preparation/admission gates are unchanged.

The two saved Zion records were preserved. Only the three missing Grand Canyon
Yes choices were transcribed through `curate draft` / `curate record`, with the
owner's exact instruction captured as evidence and Codex identified as recorder.
The recovery does not claim new gallery clicks or technical certification.

| Grand Canyon tile | Recovered record SHA-256 |
| --- | --- |
| Northwest layered cliffs | `9fa3da06b669ff590caa5a68bcf09ec6b3944578dada0737cde9b90c2ca0406e` |
| Central river corridor | `cda6512662cba5c46614686f971ee00730b1094565724d1a010ef5c0a86d58d8` |
| Southeast river bend | `484cfd746b3118f31a4277db2837eb0b00e6aba06df1b597f59e1a415034c969` |

All 57 pre-recovery records, 28 existing heads and 20 prepared entries are
unchanged. The catalog now has 60 records, 31 heads and the same 20 prepared
regions. Extra historical heads are not extra gallery tiles. The five visual
acceptances still await technical qualification; no training admission was made.

The gallery manifest remains
`5f52f7528e2ee89583741df98efc7e4a0caa1b2ad62be0df3c79ad177e37fe7d`.
Recovery evidence is retained in `data/review/gallery-vote-recovery-v1/`:
the consistent SQLite backup, `owner-decision.json`, three curation drafts,
`recover.py` and `recovery.json`. Owner instruction evidence has SHA-256
`f99028a2a526e7b35437773b2fdae488ece807e1b29c0d9e29c6e7263892350d`.

The synthetic HTTP workflow reproduced the exact failure before the fix and
passed afterward. It checks first confirmed/prohibited source votes, unchanged
source bytes, retained prior evidence and continued non-admission. Actual Chromium
button, keyboard, filtering and reload checks also passed. Independent Sol review
found no remaining issues in the fix or regression coverage.
Full `bash scripts/check.sh` passed, including formatting, locked offline build,
Clippy, curation, batch/gallery, preparation, dataset and numeric workflows.
The repository check log is retained alongside the recovery evidence.
`curate check` also passed for all 60 records and 20 prepared regions, with no
unregistered records or unreferenced evidence objects. Its result is retained as
`catalog-integrity.json`.
The restarted gallery's live API reports 26 Yes, two No and no pending tiles;
all three recovered head identities match. A final Chromium inspection passed
without voting, with `gallery-state.json` and `gallery-inspection.png` retained.
