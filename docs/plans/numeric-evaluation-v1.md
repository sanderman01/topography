# Numeric evaluation implementation plan

> Use superpowers:executing-plans for inline execution. Track progress in TASKS.md.

**Goal:** publish the selected evaluation collection and run a reusable numeric
precision job without asking the owner to review technical implementation choices.

**Spec:** [Numeric evaluation v1](../design/numeric-evaluation-v1.md).
**Architecture:** Rust validates releases and invokes a bounded NumPy worker via
versioned JSON and explicit binary payloads. Existing GDAL and prepared/curation
verification supply the numeric and source boundary. No new Rust dependencies.

## Constraints

Rust 1.85+/edition 2024; Python 3.11+; no worktree; keep raw/accepted data immutable.
No source correspondence. No new human technical findings or training admission.
The initial evaluation is CPU-only; maximum 32 regions, 2,048 cells per side,
one worker, GDAL cache 64 MiB. Inputs and outputs pin exact identities.

## Tasks

- [x] Add reusable workflow checks for the release command and observe unsupported
  arguments before implementation. Create `scripts/check_datasets.py`; reuse the
  preparation fixture builder with a small reusable fixture-return hook.
- [x] Implement strict selection/approval/release contracts and current-parent,
  geometry, group and separation checks in `crates/topography-core/src/datasets/`.
  Add CLI dataset build/verify and prove valid publication, idempotence, malformed
  approvals, corruption, split leakage and source rejection through the CLI.
- [x] Add numeric boundary checks and observe missing evaluator/module. Implement
  Rust GDAL payload staging and bounded worker invocation; implement Python
  loading/metrics/job completion under `training/src/topography_training/`.
  Declare NumPy only, install isolated package and verify outside the checkout.
- [x] Verify mask categories, exclusion of invalid/water fill, signed zero,
  synthetic feature loss, range/all-masked errors, failed worker, altered payload,
  output collision and preservation of parent bytes. Integrate into check.sh.
- [x] Run formatting/build/Clippy/full workflows, publish the actual selected
  twelve-region release with the exact captured assent, and run the CPU job.
- [x] Document implemented contracts/commands/results and remaining codec/GPU
  scope; perform one fresh final review, fix material findings, commit and update
  progress. Preserve acceptance and stop only at a real user decision/readiness.

## Review focus

Approval pins configuration without certifying technical correctness. Rejection
revokes use but preserves prior output. Geographic checks use physical context,
not tile count. Zero can be valid ground; masks control every metric. Publication
must not promote partial worker output or trust an unvalidated successful exit.
