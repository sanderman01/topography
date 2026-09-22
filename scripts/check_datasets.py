#!/usr/bin/env python3
"""Exercise evaluation releases and numeric jobs with tiny local prepared sources."""

import argparse
import copy
import json
from pathlib import Path
import shlex
import tempfile

from check_acquisition import run, sha
from check_preparation import check as prepare_fixture
from check_surface_cover import preparation_checks as prepare_cover_fixture


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def check_cover(cli, directory, python=None):
    (directory / "left").mkdir()
    (directory / "right").mkdir()
    runtime, left = prepare_cover_fixture(cli, directory / "left")
    _, right = prepare_cover_fixture(cli, directory / "right", runtime=runtime, west=702000)
    selection = {"schema_version": 2, "purpose": "numeric_evaluation_only",
                 "maximum_context_m": 40.0, "separation_buffer_m": 40.0,
                 "normalization": {"offset_m": 0.0, "scale_m": 2048.0},
                 "groups": [{"id": "left", "role": "calibration", "prepared": [sha(left)]},
                            {"id": "right", "role": "validation", "prepared": [sha(right)]}]}
    request, approval_path = directory / "selection.json", directory / "approval.json"
    write(request, selection)
    approval = {"schema_version": 1, "kind": "numeric_evaluation_approval",
                "selection_sha256": sha(request), "actor_kind": "human",
                "actor_name": "Synthetic fixture only", "recorder": "workflow check",
                "decision": "approve", "exact_response": "Synthetic cover evaluation only",
                "scope": "numeric_evaluation_only", "reviewed_proposal_sha256": "0" * 64,
                "reviewed_page_sha256": "0" * 64}
    write(approval_path, approval)
    release = Path(run(cli, "dataset", "build", runtime, request, approval_path))
    assert json.loads(release.read_text())["schema_version"] == 2
    status = json.loads(run(cli, "dataset", "verify", runtime, release))
    assert status["current"] and status["surface_cover"] == "assessed"
    if python:
        completed = Path(run(cli, "evaluate", "numeric", runtime, release, python, directory / "numeric"))
        assert json.loads(completed.read_text())["schema_version"] == 2
        metrics = json.loads((completed.parent / "metrics.json").read_text())
        for region in metrics["regions"]:
            assert region["snow_ice_cells"] == 2 and region["valid_cells"] == 13
            assert region["invalid_land_cells"] == 0
        payload = json.loads((completed.parent / "request.json").read_text())
        assert all({"snow-ice", "vegetation", "surface-elevation"} <= set(r["layers"])
                   for r in payload["regions"])
    damaged = right.parent / "vegetation.tif"
    original = damaged.read_bytes()
    damaged.write_bytes(b"corrupt")
    run(cli, "dataset", "verify", runtime, release, expected=1, message="integrity")
    damaged.write_bytes(original)


def check(cli, directory, python=None):
    left = directory / "left"
    right = directory / "right"
    left.mkdir()
    right.mkdir()
    runtime, first, drafts, heads = prepare_fixture(cli, left, fixture_only=True)
    _, second, _, _ = prepare_fixture(cli, right, fixture_only=True, west=702000, runtime=runtime)
    selection = {"schema_version": 1, "purpose": "numeric_evaluation_only",
                 "maximum_context_m": 40.0, "separation_buffer_m": 40.0,
                 "normalization": {"offset_m": 0.0, "scale_m": 2048.0},
                 "groups": [{"id": "left", "role": "calibration", "prepared": [sha(first)]},
                            {"id": "right", "role": "validation", "prepared": [sha(second)]}]}
    request = directory / "selection.json"
    approval_path = directory / "approval.json"
    write(request, selection)
    approval = {"schema_version": 1, "kind": "numeric_evaluation_approval",
                "selection_sha256": sha(request), "actor_kind": "human",
                "actor_name": "Synthetic fixture only", "recorder": "workflow check",
                "decision": "approve", "exact_response": "Synthetic approval; no real terrain",
                "scope": "numeric_evaluation_only", "reviewed_proposal_sha256": "0" * 64,
                "reviewed_page_sha256": "0" * 64}
    write(approval_path, approval)
    release = Path(run(cli, "dataset", "build", runtime, request, approval_path))
    assert release.parent.name == sha(release)
    status = json.loads(run(cli, "dataset", "verify", runtime, release))
    assert status["current"] and status["purpose"] == "numeric_evaluation_only"
    assert Path(run(cli, "dataset", "build", runtime, request, approval_path)) == release
    before = {p: sha(p) for p in [first, second, release]}
    fit_approval_path = directory / "fit-approval.json"
    fit_approval = {"schema_version": 1, "kind": "representation_fit_approval",
                    "scope": "representation_fit_only", "dataset_sha256": sha(release),
                    "actor_kind": "human", "actor_name": "Synthetic fixture only",
                    "recorder": "workflow check", "decision": "approve",
                    "exact_response": "Synthetic fitting approval",
                    "plan_sha256": "1" * 64, "proposal_sha256": "2" * 64,
                    "trial": "scalar_codec_fit_v1"}
    write(fit_approval_path, fit_approval)
    fit = Path(run(cli, "dataset", "fit-build", runtime, release, fit_approval_path))
    assert fit != release and fit.parent.name == sha(fit)
    fit_status = json.loads(run(cli, "dataset", "fit-verify", runtime, fit))
    assert fit_status["purpose"] == "representation_fit_only" and fit_status["current"]
    assert fit_status["dataset_sha256"] == sha(release)
    assert Path(run(cli, "dataset", "fit-build", runtime, release, fit_approval_path)) == fit
    run(cli, "dataset", "verify", runtime, fit, expected=1)
    run(cli, "dataset", "fit-verify", runtime, release, expected=1)
    run(cli, "dataset", "fit-build", runtime, release, approval_path, expected=1)
    for change in [{"scope": "numeric_evaluation_only"}, {"dataset_sha256": "0" * 64},
                   {"actor_kind": "assistant"}, {"trial": "unbounded_training"}]:
        write(fit_approval_path, dict(fit_approval, **change))
        run(cli, "dataset", "fit-build", runtime, release, fit_approval_path, expected=1)
    write(fit_approval_path, fit_approval)
    # Two individually prepared pieces of one source may not cross roles.
    pieces = []
    for i, head in enumerate(heads):
        piece_request = directory / f"piece-{i}.json"
        write(piece_request, {"schema_version": 1,
              "inputs": [{"quality_path": str(left / f"crop-{i}/quality.json"),
                          "curation_record_sha256": head}],
              "grid": {"origin": [700000 + i * 20, 2000010], "size": [20, 10], "spacing_m": [1, 1]},
              "masked_fill_m": 0.0})
        pieces.append(Path(run(cli, "prepare", "build", runtime, piece_request)))
    for prepared, message in [([sha(p) for p in pieces], "source asset"),
                              ([sha(first), sha(pieces[0])], "overlap")]:
        bad = copy.deepcopy(selection)
        if message == "source asset":
            for group, member in zip(bad["groups"], prepared):
                group["prepared"] = [member]
        else:
            bad["groups"][0]["prepared"] = prepared
        write(request, bad)
        write(approval_path, dict(approval, selection_sha256=sha(request)))
        run(cli, "dataset", "build", runtime, request, approval_path, expected=1, message=message)
    for edit, message in [
        (lambda s: s.update(schema_version=3), "unsupported"),
        (lambda s: s.update(schema_version=2), "surface-cover contract versions differ"),
        (lambda s: s.update(typo=True), "unknown field"),
        (lambda s: s["groups"][1].update(prepared=[sha(first)]), "duplicate"),
        (lambda s: s["groups"][1].update(role="calibration"), "both"),
        (lambda s: s.update(separation_buffer_m=4000), "separation"),
        (lambda s: s.update(maximum_context_m=4000), "buffer"),
    ]:
        bad = copy.deepcopy(selection)
        edit(bad)
        write(request, bad)
        current_approval = dict(approval, selection_sha256=sha(request))
        write(approval_path, current_approval)
        run(cli, "dataset", "build", runtime, request, approval_path, expected=1, message=message)
    write(request, selection)
    write(approval_path, dict(approval, actor_kind="assistant"))
    run(cli, "dataset", "build", runtime, request, approval_path, expected=1, message="human")
    write(approval_path, dict(approval, selection_sha256="0" * 64))
    run(cli, "dataset", "build", runtime, request, approval_path, expected=1, message="approval")
    write(approval_path, approval)
    intact = release.read_bytes()
    release.write_bytes(b"corrupt")
    run(cli, "dataset", "verify", runtime, release, expected=1, message="integrity")
    release.write_bytes(intact)
    raster = first.parent / "water.tif"
    intact = raster.read_bytes()
    raster.write_bytes(b"corrupt")
    run(cli, "dataset", "fit-verify", runtime, fit, expected=1, message="integrity")
    run(cli, "dataset", "verify", runtime, release, expected=1, message="integrity")
    raster.write_bytes(intact)
    if python:
        output = directory / "numeric result"
        result_path = Path(run(cli, "evaluate", "numeric", runtime, release, python, output))
        result = json.loads(result_path.read_text())
        assert result["dataset_sha256"] == sha(release) and result["gpu_used"] is False
        metrics = json.loads((output / "metrics.json").read_text())
        assert len(metrics["regions"]) == 2
        for item in metrics["regions"]:
            assert item["valid_cells"] == 398 and item["water_cells"] == 1
            assert item["representations"]["float32"]["bit_identical_valid_samples"]
        assert metrics["synthetic"]["narrow_ridge"]["float16"]["newly_flat_pairs"] > 0
        run(cli, "evaluate", "numeric", runtime, release, python, output, expected=1, message="exists")
        failed = directory / "failed-worker"
        run(cli, "evaluate", "numeric", runtime, release, "/bin/false", failed, expected=1)
        assert not failed.exists()
        run(cli, "evaluate", "numeric", runtime, release, "/bin/true", failed, expected=1)
        assert not failed.exists()
        # A successful but incomplete worker must never become a verified run.
        worker = directory / "altered worker.py"
        worker.write_text('''import json, os, sys
from pathlib import Path
from topography_training.numeric import execute
from topography_training.payload import digest
mode = sys.argv[1]
output = Path(sys.argv[-1])
execute(Path(sys.argv[-2]), output)
metrics_path, result_path = output / "metrics.json", output / "result.json"
metrics, result = json.loads(metrics_path.read_text()), json.loads(result_path.read_text())
if mode == "metrics":
    for row in metrics["regions"]:
        for representation in row["representations"].values():
            del representation["max_slope_component_error_m_per_m"]
elif mode == "counts":
    for row in metrics["regions"]:
        for representation in row["representations"].values():
            representation["newly_flat_pairs"] = 999999999
elif mode == "synthetic":
    metrics["synthetic"] = dict.fromkeys(["ramp", "flat", "sharp_ridge", "narrow_ridge"])
elif mode == "environment":
    result["environment"] = {}
elif mode.startswith("log"):
    os.write(1, b"x" * (2 * 1024 * 1024))
    os._exit(1 if mode == "log-failure" else 0)
metrics_path.write_text(json.dumps(metrics))
result["metrics_sha256"] = digest(metrics_path.read_bytes())
result_path.write_text(json.dumps(result))
''')
        failures = []
        for mode in ["metrics", "counts", "synthetic", "environment", "log-success", "log-failure"]:
            wrapper = directory / f"worker-{mode}"
            wrapper.write_text(f'#!/bin/sh\nexec {shlex.quote(str(python))} {shlex.quote(str(worker))} {mode} "$@"\n')
            wrapper.chmod(0o755)
            destination = directory / f"invalid-{mode}"
            try:
                run(cli, "evaluate", "numeric", runtime, release, wrapper, destination,
                    expected=1, message="log exceeded" if mode.startswith("log") else None)
                assert not destination.exists()
            except AssertionError:
                failures.append(mode)
        assert not failures, f"invalid worker outputs were accepted: {failures}"
    # Current source rejection must revoke reuse of this unchanged release.
    draft, decision = drafts[0]
    decision["previous_record_sha256"] = heads[0]
    decision["claims"].update(decision="reject", reason="Synthetic rejection")
    write(draft, decision)
    run(cli, "curate", "record", draft, runtime)
    run(cli, "dataset", "verify", runtime, release, expected=1, message="current")
    run(cli, "dataset", "fit-verify", runtime, fit, expected=1, message="current")
    assert all(sha(p) == digest for p, digest in before.items())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--python", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="topography datasets ") as name:
        check(args.binary.resolve(), Path(name), args.python.absolute() if args.python else None)
    with tempfile.TemporaryDirectory(prefix="topography cover datasets ") as name:
        check_cover(args.binary.resolve(), Path(name), args.python.absolute() if args.python else None)
    print("Dataset workflows passed: scoped approval, geographic separation, immutable release and source revocation.")


if __name__ == "__main__":
    main()
