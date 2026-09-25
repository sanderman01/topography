# Validation and review

Applies to implementation verification and the terrain acceptance workflow.
Processing completion, numerical validity, and human acceptance are distinct
outcomes; record each against the exact artifact or revision inspected.

## Verify what exists

Run the existing development check from the repository root:

```sh
bash scripts/check.sh
```

It runs rustfmt, a locked offline workspace build, Clippy with warnings denied,
and the CLI's help/version and contract-validation workflows. It does not run
formal tests, Python model jobs, training or world validation. When Python 3.11+
and GDAL tools are installed, it also runs small synthetic raw import/review
checks. The separate HTTPS workflow check is documented in
[scripts](../../scripts/README.md). No formal test runner is configured.

- For a bug, first reproduce the failure through the user's actual workflow as
  closely as practical. Verify the same path after the fix, including relevant
  exit status, diagnostics, and output artifacts.
- Exercise new behavior through its public command or batch boundary with a
  small representative input and a meaningful failure case. Prefer deterministic
  synthetic fixtures for exact numeric checks; use real survey data for fidelity
  and human review when the functionality exists.
- Add formal tests when requested. Rust integration tests belong in each
  crate's `tests/`, Python tests in `training/tests/`, and small shared synthetic
  inputs in `tests/fixtures/`. Keep tests about observable contracts rather than
  private implementation structure. See [repository test layout](../../tests/README.md)
  and [Rust test organization](https://doc.rust-lang.org/book/ch11-03-test-organization.html).
- Use existing tools and workflows; avoid disposable one-off test scripts.
  Document reusable verification commands when adding a consumer. For docs-only
  work, check links, stated commands, and consistency with current behavior.
- Report commands run, observed results, and material gaps. A mocked provider or
  tiny CPU run does not establish live acquisition or full GPU feasibility.

## Separate exact invariants from measured tolerances

| Area | Evidence to require when implemented |
| --- | --- |
| Physical grid | Correct CRS, units, origin, axis/row orientation, extent, dimensions, and spacing |
| Numeric preparation | Ramps, flats, ridges, and mask boundaries preserve declared values and validity without shifts or invalid-cell leakage |
| Source admission | Traceable survey, supported terrain features, recorded quality decision, and dataset-specific training-permission evidence |
| Dataset release | No geographic split leakage, paired layers aligned, fixed normalization, compatible versioned labels |
| Model feasibility | Loader/codec error in metres, preserved features, measured peak VRAM within 16 GB, runtime, and comparison with real-patch blending |
| World composition | Declared class areas, height/slope joins, connected water surfaces, and landmark placement |
| Local edits | Values outside the edit domain and inside locks unchanged exactly; transition bands and dependent regions checked |
| Export | Duplicated border texels bit-identical, texels equal encoded master samples, coarse LOD texels equal level-0 texels at nested positions, compatible masks, correct partial tiles, and no-clipping quantization error within half a step plus declared arithmetic tolerance |
| Recovery | Interrupted work remains identifiable; restart validates completed artifacts and preserves accepted outputs and lineage |

Exact border equality and protected-cell equality are not visual judgments.
Measured numeric tolerances must be justified by the relevant representation,
encoding, or experiment and stored with the versioned contract that uses them.
They do not define whether terrain is suitable for play. Apply the plan's initial
terrestrial coverage policy (at least 99.5% valid cells for the initial training
pool, with no unsupported gaps through defining features) without presenting
every proposed gate as an implemented or permanently fixed rule.

## Make human review repeatable

- Source selection is a whole-tile yes/no decision presented in batches. The
  implementer handles masks, technical evidence and checks. Individual feature
  marking and detailed justifications are optional; do not ask for them as a
  condition of selection. Preserve the decision for unchanged terrain while
  technical work continues, without turning visual approval into technical proof.
- Provide checkpoints after discovery, source review, crop/label selection,
  dataset release, model evaluation, coarse composition, regional generation,
  and export. Batch processing should remain usable between checkpoints.
- Offer accept/reject decisions, reasons, annotations, variant comparisons, and
  locks tied to immutable artifact identities. Rejection does not remove raw data.
- When useful, supplement numeric metrics with hillshades from multiple
  directions, contours, cross-sections, oblique views, or repeatable camera
  routes. Optional camera-height controls are viewing aids. The owner decides
  whole-tile suitability with a yes/no judgment; these views do not define
  acceptance thresholds.
- Record representation limits during review. A heightfield cannot represent
  caves, arches, or overhangs; flag features the chosen representation or survey
  cannot support.
- Preserve the reviewed outputs, configuration, camera settings, and decision
  records. See the [master plan](../plans/terrain-design-master-plan.md), sections
  9–11, for the acceptance gates and milestone sequence.
