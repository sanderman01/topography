#!/usr/bin/env python3
"""Exercise regional masks and pinned preparation eligibility through the CLI."""

import argparse
import copy
import json
from pathlib import Path
import shutil
import tempfile

from check_acquisition import FIXTURES, ROOT, run, runtime_file, sha


def raster(directory, name, values, *, x=700000, kind="Byte", crs="EPSG:32619", nodata=False):
    ascii_path = directory / f"{name}.asc"
    ascii_path.write_text(
        f"ncols 20\nnrows 10\nxllcorner {x}\nyllcorner 2000000\ncellsize 1\nNODATA_value -9999\n"
        + "\n".join(" ".join(map(str, values[i:i + 20])) for i in range(0, 200, 20)) + "\n")
    path = directory / f"{name}.tif"
    run("gdal_translate", "-q", "-ot", kind, "-a_srs", crs,
        *([] if nodata else ["-a_nodata", "none"]), ascii_path, path)
    return path


def check(cli, directory, synthetic=True, declared_crs="EPSG:32619"):
    runtime = runtime_file(directory)
    source = directory / "source.toml"
    source.write_text((FIXTURES / "source.toml").read_text().replace(
        'acquisition_dates = []', 'acquisition_dates = ["2020-01-01"]').replace(
        'synthetic = true', f'synthetic = {str(synthetic).lower()}').replace(
        'horizontal_crs = "EPSG:32619"', f'horizontal_crs = "{declared_crs}"'))
    shutil.copyfile(FIXTURES / "metadata.json", directory / "metadata.json")
    asset = raster(directory, "elevation", [-9999] + [100.25 + i for i in range(199)],
                   kind="Float32", nodata=True)
    raw = Path(run(cli, "import", source, asset, runtime, 1048576))
    review = Path(run(cli, "review", raw, directory / "native", "0,0,20,10"))
    original_hashes = {p: sha(p) for p in [asset, raw, review]}
    inputs = directory / "quality.json"
    run(cli, "quality", "draft", raw, review, inputs)
    draft = json.loads(inputs.read_text())
    assert all(value is None for value in draft["masks"].values())
    incomplete = directory / "incomplete.json"
    run(cli, "quality", "assess", inputs, incomplete)
    assert len(json.loads(incomplete.read_text())["blockers"]) == 4
    assert json.loads(incomplete.read_text())["counts"] is None

    masks = {
        "water": [1] * 200,
        "quality": [1] * 200,
        "repairs": [0] * 200,
        "features": [0] * 199 + [1],
    }
    for name, samples in masks.items():
        path = raster(directory, name, samples)
        draft["masks"][name] = {"path": path.name, "sha256": sha(path)}
    inputs.write_text(json.dumps(draft))
    report = directory / "assessment.json"
    run(cli, "quality", "assess", inputs, report)
    assessed = json.loads(report.read_text())
    assert assessed["blockers"] == []
    assert assessed["counts"]["terrestrial_cells"] == 200
    assert assessed["counts"]["valid_terrestrial_cells"] == 199
    assert assessed["counts"]["unsupported_feature_cells"] == 0
    run(cli, "quality", "assess", inputs, report, expected=1, message="already exists")

    # Numeric regressions change observable counts/blockers, not implementation details.
    for number, (mask, index, value, expected_blocker) in enumerate([
        ("quality", 1, 2, "99.5%"),
        ("quality", 199, 0, "defining features"),
        ("water", 1, 0, "water classification"),
        ("repairs", 1, 1, "repairs"),
        ("features", 0, 1, "defining features"),
    ]):
        changed = copy.deepcopy(draft)
        samples = masks[mask].copy()
        samples[index] = value
        path = raster(directory, f"case-{number}", samples)
        changed["masks"][mask] = {"path": path.name, "sha256": sha(path)}
        inputs.write_text(json.dumps(changed))
        output = directory / f"case-{number}.json"
        run(cli, "quality", "assess", inputs, output)
        assert any(expected_blocker in b for b in json.loads(output.read_text())["blockers"])

    # Water is excluded by explicit classification, even if provider-invalid.
    water = raster(directory, "one-water", [2] + [1] * 199)
    changed = copy.deepcopy(draft)
    changed["masks"]["water"] = {"path": water.name, "sha256": sha(water)}
    inputs.write_text(json.dumps(changed))
    water_report = directory / "water.json"
    run(cli, "quality", "assess", inputs, water_report)
    water_counts = json.loads(water_report.read_text())["counts"]
    assert water_counts["terrestrial_cells"] == water_counts["valid_terrestrial_cells"] == 199
    assert water_counts["water_cells"] == 1

    for name, path, message in [
        ("shift", raster(directory, "shift", masks["quality"], x=700000.5), "grid mismatch"),
        ("crs", raster(directory, "crs", masks["quality"], crs="EPSG:32618"), "grid mismatch"),
        ("float", raster(directory, "float", masks["quality"], kind="Float32"), "Byte"),
        ("category", raster(directory, "category", [3] * 200), "category"),
        ("nodata", raster(directory, "nodata", masks["quality"], nodata=True), "nodata"),
    ]:
        changed = copy.deepcopy(draft)
        changed["masks"]["quality"] = {"path": path.name, "sha256": sha(path)}
        inputs.write_text(json.dumps(changed))
        output = directory / f"bad-{name}.json"
        run(cli, "quality", "assess", inputs, output, expected=1, message=message)
        assert not output.exists()
    changed = copy.deepcopy(draft)
    changed["masks"]["quality"]["sha256"] = "f" * 64
    inputs.write_text(json.dumps(changed))
    run(cli, "quality", "assess", inputs, directory / "bad-hash.json", expected=1, message="digest mismatch")
    inputs.write_text(json.dumps(draft))

    # Exercise human declarations only in this isolated synthetic catalog.
    decision = directory / "decision.json"
    run(cli, "curate", "draft", runtime, raw, review, ROOT / "catalog/labels/terrain-v1.toml", decision)
    record = json.loads(decision.read_text())
    record["claims"].update(actor={"kind": "human", "name": "synthetic human fixture"},
                            decision="accept", reason="Synthetic preflight verification", primary_class_id=1)
    record["evidence"] = [{"path": report.name, "evidence": {
        "id": "assessment", "description": "Synthetic regional assessment", "source_url": None, "sha256": sha(report)}}]
    for finding in record["claims"]["findings"].values():
        finding.update(state="pass", note="Synthetic fixture only", evidence_ids=["assessment"])
    record["claims"]["coverage"] = {"terrestrial_cells": 200, "valid_terrestrial_cells": 199,
                                    "note": "Measured fixture", "evidence_ids": ["assessment"]}
    license = record["claims"]["license"]
    license.update(training_permission="confirmed", license_name="Synthetic", license_url="https://example.invalid/",
                   evidence="Fixture only", attribution="Fixture", edited_map_reuse="Fixture", redistribution="Fixture")
    record["claims"]["permission_evidence_ids"] = ["assessment"]

    def publish(value):
        decision.write_text(json.dumps(value))
        return run(cli, "curate", "record", decision, runtime)

    head = publish(record)
    if declared_crs != "EPSG:32619":
        result = json.loads(run(cli, "prepare", "check", runtime, inputs, head, expected=1))
        assert any("declared horizontal CRS" in b for b in result["blockers"]), result
        return
    result = json.loads(run(cli, "prepare", "check", runtime, inputs, head, expected=int(synthetic)))
    assert result["eligible"] is (not synthetic) and result["admission"] == "not admitted"
    assert result["blockers"] == (["synthetic source excluded from real-terrain pool"] if synthetic else []), result
    assert result["curation_record_sha256"] == head
    assert result["quality_report_sha256"] == sha(report)
    record["previous_record_sha256"] = head
    record["claims"]["coverage"]["valid_terrestrial_cells"] = 200
    second = publish(record)
    run(cli, "prepare", "check", runtime, inputs, head, expected=1, message="stale")
    result = json.loads(run(cli, "prepare", "check", runtime, inputs, second, expected=1))
    assert any("coverage differs" in b for b in result["blockers"])
    # Changed assessment cannot reuse the accepted evidence.
    changed = copy.deepcopy(draft)
    changed["masks"]["water"] = {"path": water.name, "sha256": sha(water)}
    inputs.write_text(json.dumps(changed))
    result = json.loads(run(cli, "prepare", "check", runtime, inputs, second, expected=1))
    assert any("assessment evidence" in b for b in result["blockers"])
    inputs.write_text(json.dumps(draft))
    record["previous_record_sha256"] = second
    record["claims"]["decision"] = "annotate"
    record["claims"]["actor"]["kind"] = "assistant"
    record["claims"]["license"]["training_permission"] = "unknown"
    third = publish(record)
    result = json.loads(run(cli, "prepare", "check", runtime, inputs, third, expected=1))
    assert any("acceptance" in b for b in result["blockers"])
    assert any("permission" in b for b in result["blockers"])
    run(cli, "curate", "check", runtime)
    assert all(sha(p) == digest for p, digest in original_hashes.items())
    assert not (directory / "prepared").exists() and not (directory / "datasets").exists()

    # The same physical invalidity is irrelevant under explicitly classified water.
    # Build a separate fixture, retaining all original inputs and decisions.
    nan_asset = raster(directory, "nan-water", ["nan"] + [100.25] * 199, kind="Float32")
    nan_raw = Path(run(cli, "import", source, nan_asset, runtime, 1048576))
    nan_review = Path(run(cli, "review", nan_raw, directory / "nan-review", "0,0,20,10"))
    nan_input = copy.deepcopy(draft)
    nan_input["raw_receipt"] = {"path": str(nan_raw), "sha256": sha(nan_raw)}
    nan_input["review"] = {"path": str(nan_review), "sha256": sha(nan_review)}
    nan_input["masks"]["water"] = {"path": water.name, "sha256": sha(water)}
    nan_path = directory / "nan-input.json"
    nan_path.write_text(json.dumps(nan_input))
    nan_report = directory / "nan-report.json"
    run(cli, "quality", "assess", nan_path, nan_report)
    assert json.loads(nan_report.read_text())["blockers"] == []
    nan_decision = directory / "nan-decision.json"
    run(cli, "curate", "draft", runtime, nan_raw, nan_review, ROOT / "catalog/labels/terrain-v1.toml", nan_decision)
    nan_record = json.loads(nan_decision.read_text())
    nan_record["claims"] = copy.deepcopy(record["claims"])
    nan_record["claims"].update(actor={"kind": "human", "name": "synthetic human fixture"}, decision="accept")
    nan_record["claims"]["license"]["training_permission"] = "confirmed"
    nan_record["claims"]["coverage"].update(terrestrial_cells=199, valid_terrestrial_cells=199)
    nan_record["evidence"] = copy.deepcopy(record["evidence"])
    nan_record["evidence"][0]["path"] = nan_report.name
    nan_record["evidence"][0]["evidence"]["sha256"] = sha(nan_report)
    nan_head = publish(nan_record)
    result = json.loads(run(cli, "prepare", "check", runtime, nan_path, nan_head, expected=int(synthetic)))
    assert result["blockers"] == (["synthetic source excluded from real-terrain pool"] if synthetic else []), result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="topography quality ") as temporary:
        directory = Path(temporary)
        mismatch = directory / "mismatched-crs"
        mismatch.mkdir()
        check(args.binary.resolve(), mismatch, synthetic=False, declared_crs="EPSG:32618")
        check(args.binary.resolve(), directory)
        declared = directory / "declared-real-fixture"
        declared.mkdir()
        # Only the metadata declaration changes to exercise eligibility; the data,
        # reviewer, permission and catalog remain explicitly synthetic fixtures.
        check(args.binary.resolve(), declared, synthetic=False)
    print("Regional quality workflows passed: masks, measured coverage, feature gaps, evidence and stale decisions.")


if __name__ == "__main__":
    main()
