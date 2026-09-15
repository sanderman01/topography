#!/usr/bin/env python3
"""Exercise crop decisions, captured evidence and catalog recovery through the CLI."""

import argparse
import copy
import json
from pathlib import Path
import shutil
import sqlite3
import tempfile

from check_acquisition import FIXTURES, ROOT, run, runtime_file, sha


def check(cli, directory):
    runtime = runtime_file(directory)
    source = directory / "source.toml"
    shutil.copyfile(FIXTURES / "source.toml", source)
    shutil.copyfile(FIXTURES / "metadata.json", directory / "metadata.json")
    asset = directory / "ramp.tif"
    run("gdal_translate", "-q", "-of", "GTiff", "-ot", "Float32", "-a_srs", "EPSG:32619", FIXTURES / "ramp.asc", asset)
    receipt = Path(run(cli, "import", source, asset, runtime, 1048576))
    review = Path(run(cli, "review", receipt, directory / "review", "1,0,2,3"))
    subject = sha(review)
    draft = directory / "draft.json"
    run(cli, "curate", "check", runtime, expected=1)
    assert not (directory / "catalog_db").exists()
    run(cli, "curate", "draft", runtime, receipt, review, ROOT / "catalog/labels/terrain-v1.toml", draft)
    assert not (directory / "catalog_db").exists()
    original = json.loads(draft.read_text())
    assert original["claims"]["actor"]["kind"] == "unassigned"
    assert original["claims"]["coverage"] is None
    assert original["claims"]["primary_class_id"] is None
    run(cli, "curate", "record", draft, runtime, expected=1, message="assign an actor")

    evidence = directory / "evidence.json"
    evidence.write_text('{"synthetic":true,"observation":"fixture only; no real terrain acceptance"}\n')
    annotation = copy.deepcopy(original)
    annotation["claims"]["actor"] = {"kind": "assistant", "name": "synthetic workflow"}
    annotation["claims"]["reason"] = "Exercise annotation without human acceptance."
    annotation["claims"]["primary_class_id"] = 1
    annotation["evidence"] = [{"path": "evidence.json", "evidence": {
        "id": "fixture", "sha256": sha(evidence), "description": "Synthetic fixture", "source_url": None}}]

    def publish(value, expected=0, message=None):
        draft.write_text(json.dumps(value))
        return run(cli, "curate", "record", draft, runtime, expected=expected, message=message)

    for mutate, message in [
        (lambda d: d["claims"].update(decision="accept"), "human actor"),
        (lambda d: d["claims"].update(primary_class_id=999), "not active"),
        (lambda d: d["target"].update(review_sha256="f"*64), "pinned target"),
        (lambda d: d.update(misspelled=True), "unknown field"),
        (lambda d: d["claims"]["findings"]["ground_support"].update(state="pass", note="Claim", evidence_ids=["missing"]), "known evidence"),
        (lambda d: d["claims"].update(coverage={"terrestrial_cells": 7, "valid_terrestrial_cells": 7, "note": "Invalid", "evidence_ids": ["fixture"]}), "coverage counts"),
    ]:
        value = copy.deepcopy(annotation)
        mutate(value)
        publish(value, expected=1, message=message)

    first = publish(annotation)
    assert publish(annotation) == first
    status = json.loads(run(cli, "curate", "status", runtime, subject))
    assert status["claims"]["decision"] == "annotate"
    assert status["admission"] == "not admitted"
    assert any("synthetic" in b for b in status["preparation_blockers"])
    assert json.loads(run(cli, "curate", "check", runtime))["indexed_records"] == 1
    captured = directory / "catalog_db.curation/objects" / sha(evidence)
    assert captured.read_bytes() == evidence.read_bytes()
    evidence.write_text("changed original evidence")
    run(cli, "curate", "check", runtime)
    publish(annotation, expected=1, message="evidence digest mismatch")
    evidence.write_bytes(captured.read_bytes())

    # A human declaration is exercised only on this synthetic fixture.
    acceptance = copy.deepcopy(annotation)
    acceptance["previous_record_sha256"] = first
    acceptance["claims"]["actor"] = {"kind": "human", "name": "synthetic human fixture"}
    acceptance["claims"]["decision"] = "accept"
    acceptance["claims"]["reason"] = "Synthetic acceptance exercise; data remains excluded."
    second = publish(acceptance)
    status = json.loads(run(cli, "curate", "status", runtime, subject))
    assert status["claims"]["decision"] == "accept" and status["admission"] == "not admitted"
    stale = copy.deepcopy(annotation)
    stale["claims"]["reason"] = "Stale new annotation"
    publish(stale, expected=1, message="stale decision")
    assert publish(annotation) == first  # Older identical replay preserves current head.
    assert json.loads(run(cli, "curate", "status", runtime, subject))["previous_record_sha256"] == first

    rejection = copy.deepcopy(acceptance)
    rejection["previous_record_sha256"] = second
    rejection["claims"].update(decision="reject", reason="Synthetic rejection", primary_class_id=2)
    third = publish(rejection)
    record_path = directory / "catalog_db.curation/records" / f"{third}.json"
    before = record_path.read_bytes()
    raw_before = sha(asset)
    # Simulate a crash after file publication and before catalog registration.
    with sqlite3.connect(directory / "catalog_db") as db:
        db.execute("UPDATE heads SET record_id=?", (second,))
        db.execute("DELETE FROM records WHERE id=?", (third,))
    recovery = json.loads(run(cli, "curate", "check", runtime))
    assert recovery["unregistered_records"] == [third]
    assert publish(rejection) == third
    assert record_path.read_bytes() == before
    assert json.loads(run(cli, "curate", "check", runtime))["indexed_records"] == 3
    assert sha(asset) == raw_before
    assert not (directory / "prepared").exists() and not (directory / "datasets").exists()

    export = directory / "portable"
    manifest = Path(run(cli, "curate", "export", runtime, subject, export))
    assert json.loads(manifest.read_text())["records_oldest_first"] == [first, second, third]
    for path in (export / "records").iterdir():
        assert sha(path) == path.stem and str(directory) not in path.read_text()
    for path in (export / "objects").iterdir():
        assert sha(path) == path.name
    run(cli, "curate", "export", runtime, subject, export, expected=1, message="already exists")

    record_path.write_bytes(b"corrupt")
    run(cli, "curate", "check", runtime, expected=1, message="record integrity mismatch")
    record_path.write_bytes(before)
    captured_before = captured.read_bytes()
    captured.write_bytes(b"corrupt")
    run(cli, "curate", "check", runtime, expected=1, message="object integrity mismatch")
    captured.unlink()
    run(cli, "curate", "check", runtime, expected=1, message="open")
    captured.write_bytes(captured_before)
    run(cli, "curate", "check", runtime)

    # An unrelated SQLite file must never be initialized as a Topography catalog.
    other = directory / "unrelated"
    other.mkdir()
    other_runtime = runtime_file(other)
    with sqlite3.connect(other / "catalog_db") as db:
        db.execute("CREATE TABLE unrelated (value TEXT)")
    before = sha(other / "catalog_db")
    draft.write_text(json.dumps(annotation))
    run(cli, "curate", "record", draft, other_runtime, expected=1, message="unrelated catalog")
    assert sha(other / "catalog_db") == before


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="topography curation ") as temporary:
        check(args.binary.resolve(), Path(temporary))
    print("Curation workflows passed: explicit decisions, evidence, history, recovery, integrity and portable export.")


if __name__ == "__main__":
    main()
