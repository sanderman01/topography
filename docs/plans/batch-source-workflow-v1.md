# Batch source workflow implementation plan

Use `superpowers:executing-plans` inline, with TASKS.md as the local ledger.
The owner explicitly requested design/plan changes followed by execution; routine
technical choices are delegated. Keep the current feature branch, no worktree.

**Goal:** One target list drives acquisition; one gallery handles source votes.
**Spec:** [Batch source workflow](../design/batch-source-workflow-v1.md).
**Stack:** Existing Rust/Serde/SQLite/GDAL; browser HTML/CSS/JavaScript, no new dependencies.

## Constraints

Public downloads only; never email. Preserve immutable assets, native Float32
and masks, exact reviewed identities, existing votes and technical findings.
Sequential bounded work, no GPU, no implicit readiness for large crop batches.
Use existing workflow scripts for checks, not a new formal test framework.

## Tasks

1. Add `acquisition/batch.rs` and CLI `batch validate|download|review`.
   Queue v1 contains source paths, optional metadata URLs, per-source byte caps,
   total budget, and named crop windows/classes. Download and review are separate.
   Persist the exact queue/input identities and immutable per-source receipts;
   resume verifies integrity, continues independent item failures, reports counts
   and exits nonzero for incomplete work. Produce gallery inputs from completed crops.
   First write `scripts/check_batch_gallery.py` queue fixtures; run against old CLI
   and observe unsupported-command failure. Verify relative paths with spaces,
   duplicate IDs, changed inputs, corrupt successes and mixed success/failure.

2. Add `review/gallery.rs`, `review/gallery.html` and `review/gallery_server.rs`;
   add narrow curation selection helpers preserving evidence and expected-head
   semantics. CLI `gallery build|serve` consumes pinned gallery inputs and runtime.
   Generate small hillshade thumbnails, copy exact previews, publish atomically.
   Gallery shows persistent decisions and filters; inspector embeds one preview
   at a time. Yes/No saves then advances; keyboard navigation also works from
   preview focus. First extend workflow fixtures for persisted votes, reload,
   stale writes, technical-evidence preservation and damaged preview refusal.
   Verify HTTP allowlist, Host/Origin/token/content-type checks and request bounds.

3. Update master plan, design overview, schemas/examples, CLI/workflow docs and
   scripts/check.sh. Run the actual local HTTPS batch download fixture, gallery
   HTTP/browser checks and full `bash scripts/check.sh`. Build a real gallery
   containing existing source tiles with decisions intact. Record progress and
   continue source expansion through the new tools. One fresh final reviewer
   checks this change; fix material findings, commit without merging or pushing.

## Review focus

- Resume after publication but before an item marker; preserve completed work.
- Source or metadata replacement; never mistake old bytes for the new queue.
- Concurrent review sessions; reject stale votes and retain technical findings.
- HTML metadata, iframe focus and HTTP access; no script injection or accidental vote.
- Hundreds of tiles; load thumbnails and only the active detailed preview.
