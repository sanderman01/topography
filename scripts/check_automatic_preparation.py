#!/usr/bin/env python3
"""Exercise automatic source checks and preparation through public workflows."""

import argparse
import http.client
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import tempfile
import time

from check_acquisition import FIXTURES, ROOT, run, runtime_file, sha, values

FINDINGS = ["coordinates", "ground_support", "artifacts_and_repairs", "feature_fidelity",
            "water_classification", "unsupported_feature_gaps"]


def request(authority, method, target, body=None, headers=None):
    connection = http.client.HTTPConnection(authority, timeout=10)
    connection.request(method, target, body=body, headers=headers or {})
    response = connection.getresponse()
    result = response.status, response.read()
    connection.close()
    return result


def start_server(cli, runtime, manifest, env=None):
    process = subprocess.Popen([str(cli), "gallery", "serve", str(runtime), str(manifest), "0"],
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
    url = process.stdout.readline().strip()
    assert url.startswith("http://127.0.0.1:"), (url, process.stderr.read())
    authority = url.removeprefix("http://").rstrip("/")
    code, html = request(authority, "GET", "/")
    assert code == 200
    token = re.search(r'"token":"([a-f0-9]+)"', html.decode())[1]
    headers = {"Origin": url.rstrip("/"), "Content-Type": "application/json", "X-Topography-Token": token}
    return process, authority, headers


def tiles(authority):
    code, body = request(authority, "GET", "/api/tiles")
    assert code == 200, body
    return json.loads(body)["tiles"]


def eventually(authority, index, expected, timeout=20):
    deadline = time.monotonic() + timeout
    observed = []
    while time.monotonic() < deadline:
        tile = tiles(authority)[index]
        state = tile["preparation"]["state"]
        if not observed or observed[-1] != state:
            observed.append(state)
        if state in expected:
            return tile
        time.sleep(0.1)
    raise AssertionError((index, expected, observed, tiles(authority)[index]))


def vote(authority, headers, gallery_sha256, index, tile, decision, expected=200, previous=None):
    previous = tile["head"] if previous is None else previous
    body = json.dumps({"index": index, "gallery_sha256": gallery_sha256,
                       "review_sha256": tile["review_sha256"],
                       "previous_record_sha256": previous, "decision": decision})
    code, data = request(authority, "POST", "/api/decision", body, headers)
    assert code == expected, (code, data)
    return json.loads(data) if code == 200 else data


def write_source(directory):
    source = directory / "source.toml"
    source.write_text((FIXTURES / "source.toml").read_text()
        .replace("synthetic = true", "synthetic = false")
        .replace("acquisition_dates = []", 'acquisition_dates = ["2020-01-01"]')
        .replace('training_permission = "unknown"',
            'training_permission = "confirmed"\nlicense_name = "Public fixture"\n'
            'license_url = "https://example.invalid/license"\n'
            'evidence = "Declared public fixture terms"\nattribution = "Fixture supplier"\n'
            'edited_map_reuse = "Permitted"\nredistribution = "Permitted"'))
    return source


def write_raster(directory):
    samples = [100.25 + index for index in range(48)]
    ascii_path = directory / "source.asc"
    rows = [samples[index:index + 12] for index in range(0, len(samples), 12)]
    ascii_path.write_text("ncols 12\nnrows 4\nxllcorner 700000\nyllcorner 2000000\n"
                          "cellsize 1\nNODATA_value -9999\n"
                          + "\n".join(" ".join(map(str, row)) for row in rows) + "\n")
    asset = directory / "source.tif"
    run("gdal_translate", "-q", "-of", "GTiff", "-ot", "Float32", "-a_srs", "EPSG:32619",
        "-a_nodata", "none", ascii_path, asset)
    return asset, samples


def write_profile(path, receipt, evidence, *, source_sha256=None, windows=None, evidence_sha256=None):
    finding = {"note": "Captured fixture source assessment", "evidence_ids": ["supplier"]}
    path.write_text(json.dumps({
        "schema_version": 1, "policy": "supplier_qa_pilot_v1",
        "source_sha256": source_sha256 or receipt["source_sha256"],
        "asset_sha256": receipt["asset_sha256"],
        "evidence": [{"id": "supplier", "path": evidence.name,
                      "sha256": evidence_sha256 or sha(evidence),
                      "description": "Exact source supplier QA and permission assessment",
                      "source_url": "https://example.invalid/source"}],
        "permission_evidence_ids": ["supplier"],
        "findings": {name: dict(finding) for name in FINDINGS},
        "water": {"kind": "all_terrestrial", "windows": windows or [[0, 0, 12, 4]]},
    }))
    return path


def fixture(cli, directory):
    runtime = runtime_file(directory)
    source = write_source(directory)
    shutil.copyfile(FIXTURES / "metadata.json", directory / "metadata.json")
    asset, samples = write_raster(directory)
    raw = Path(run(cli, "import", source, asset, runtime, 1048576))
    receipt = json.loads(raw.read_text())
    evidence = directory / "supplier.txt"
    evidence.write_text("Fixture supplier QA and permission assessment\n")
    profile = write_profile(directory / "profile.json", receipt, evidence)
    queue = directory / "queue.json"
    queue.write_text(json.dumps({"schema_version": 1, "max_total_bytes": 1048576, "sources": [{
        "id": "source", "source": source.name, "existing_receipt": str(raw), "max_bytes": 1048576,
        "preparation_profile": profile.name,
        "crops": [{"id": f"crop-{index}", "title": f"Crop {index}",
                   "window_cells": [index * 2, 0, 2, 4], "primary_class_id": 1}
                  for index in range(6)]}]}))
    run(cli, "batch", "validate", queue)
    output = directory / "batch"
    report = json.loads(run(cli, "batch", "download", queue, runtime, output))
    assert report["failed"] == 0 and report["completed"] == 1
    report = json.loads(run(cli, "batch", "review", queue, runtime, output))
    assert report["failed"] == 0 and report["completed"] == 6
    gallery_input_path = output / "gallery-input.json"
    gallery_input = json.loads(gallery_input_path.read_text())
    pinned = gallery_input["tiles"][0]["preparation_profile"]
    assert pinned == {"path": str(profile.resolve()), "sha256": sha(profile)}
    return runtime, raw, receipt, profile, evidence, gallery_input_path, gallery_input, samples


def check(cli, directory):
    runtime, raw, receipt, profile, evidence, gallery_input_path, gallery_input, samples = fixture(cli, directory)
    profile_bytes = profile.read_bytes()
    profile.write_bytes(profile_bytes + b"\n")
    run(cli, "gallery", "build", gallery_input_path, ROOT / "catalog/labels/terrain-v1.toml",
        directory / "changed-profile-gallery", expected=1, message="differs")
    profile.write_bytes(profile_bytes)
    wrong = write_profile(directory / "wrong-source.json", receipt, evidence, source_sha256="f" * 64)
    uncovered = write_profile(directory / "unclassified-water.json", receipt, evidence, windows=[[0, 0, 1, 1]])
    corrupt = write_profile(directory / "corrupt-evidence.json", receipt, evidence, evidence_sha256="e" * 64)

    def additional_tile(identifier, permission, *, failed_finding=False, curated_permission=None):
        declaration = directory / f"{identifier}-source.toml"
        text = (FIXTURES / "source.toml").read_text().replace("synthetic = true", "synthetic = false")
        text = text.replace('record_id = "synthetic-ramp"', f'record_id = "{identifier}"')
        text = text.replace("acquisition_dates = []", 'acquisition_dates = ["2020-01-01"]')
        if permission == "confirmed":
            text = text.replace('training_permission = "unknown"',
                'training_permission = "confirmed"\nlicense_name = "Public fixture"\n'
                'license_url = "https://example.invalid/license"\n'
                'evidence = "Declared public fixture terms"\nattribution = "Fixture supplier"\n'
                'edited_map_reuse = "Permitted"\nredistribution = "Permitted"')
        else:
            text = text.replace('training_permission = "unknown"', f'training_permission = "{permission}"')
        declaration.write_text(text)
        extra_raw = Path(run(cli, "import", declaration, directory / "source.tif", runtime, 1048576))
        extra_receipt = json.loads(extra_raw.read_text())
        extra_review = Path(run(cli, "review", extra_raw, directory / f"{identifier}-review", "0,0,2,4"))
        extra_profile = write_profile(directory / f"{identifier}-profile.json", extra_receipt, evidence)
        if failed_finding or curated_permission:
            draft = directory / f"{identifier}-decision.json"
            run(cli, "curate", "draft", runtime, extra_raw, extra_review,
                ROOT / "catalog/labels/terrain-v1.toml", draft)
            decision = json.loads(draft.read_text())
            decision["claims"].update(actor={"kind": "human", "name": "Fixture owner"},
                                      decision="accept", reason="Retain known technical failure",
                                      primary_class_id=1)
            decision["evidence"] = [{"path": evidence.name, "evidence": {
                "id": "supplier", "sha256": sha(evidence),
                "description": "Exact source supplier QA and permission assessment",
                "source_url": "https://example.invalid/source"}}]
            decision["claims"]["permission_evidence_ids"] = ["supplier"]
            for finding in decision["claims"]["findings"].values():
                finding.update(state="unknown", note="", evidence_ids=[])
            if failed_finding:
                decision["claims"]["findings"]["artifacts_and_repairs"] = {
                    "state": "fail", "note": "Known blocking fixture defect",
                    "evidence_ids": ["supplier"]}
            if curated_permission:
                decision["claims"]["license"]["training_permission"] = curated_permission
            draft.write_text(json.dumps(decision))
            run(cli, "curate", "record", draft, runtime)
        return {"id": identifier, "title": identifier.replace("-", " ").title(),
                "raw": str(extra_raw), "review": str(extra_review), "primary_class_id": 1,
                "preparation_profile": str(extra_profile)}

    def configure(index, identifier, profile_path="keep"):
        tile = dict(gallery_input["tiles"][index], id=identifier,
                    title=identifier.replace("-", " ").title())
        if profile_path is None:
            tile.pop("preparation_profile", None)
        elif profile_path != "keep":
            tile["preparation_profile"] = str(profile_path)
        return tile

    gallery_input["tiles"] = [configure(0, "valid"), configure(1, "reject"),
        configure(2, "missing-profile", None), configure(3, "wrong-source", wrong),
        configure(4, "unclassified-water", uncovered), configure(5, "corrupt-evidence", corrupt),
        additional_tile("unknown-permission", "unknown"),
        additional_tile("prohibited-permission", "prohibited"),
        additional_tile("failed-finding", "confirmed", failed_finding=True),
        additional_tile("curated-restricted", "confirmed", curated_permission="restricted"),
        additional_tile("curated-prohibited", "confirmed", curated_permission="prohibited")]
    gallery_input_path.write_text(json.dumps(gallery_input))
    manifest = Path(run(cli, "gallery", "build", gallery_input_path,
                        ROOT / "catalog/labels/terrain-v1.toml", directory / "gallery"))
    gallery_sha256 = sha(manifest)
    published_profile = json.loads(manifest.read_text())["tiles"][0]["preparation_profile"]
    assert published_profile == {"path": str(profile), "sha256": sha(profile)}

    process, authority, headers = start_server(cli, runtime, manifest)
    ready_manifest = None
    human_head = None
    try:
        valid = eventually(authority, 0, {"awaiting_acceptance"})
        assert valid["decision"] == "pending" and valid["head"] is None
        assert valid["preparation"]["manifest"] is None
        for index in range(2, 11):
            attention = eventually(authority, index, {"needs_attention"})
            expected_decision = "accept" if index >= 8 else "pending"
            assert attention["decision"] == expected_decision
            assert attention["preparation"]["manifest"] is None
        failed = json.loads(run(cli, "curate", "status", runtime, tiles(authority)[8]["review_sha256"]))
        assert failed["claims"]["findings"]["artifacts_and_repairs"]["state"] == "fail"
        assert failed["automation"]["kind"] == "supplier_qa_qualification_v1"
        for index, permission in [(9, "restricted"), (10, "prohibited")]:
            status = json.loads(run(cli, "curate", "status", runtime, tiles(authority)[index]["review_sha256"]))
            assert status["claims"]["license"]["training_permission"] == permission
            assert status["automation"]["kind"] == "supplier_qa_qualification_v1"

        saved = vote(authority, headers, gallery_sha256, 0, valid, "accept")
        assert saved["decision"] == "accept"
        human_head = saved["head"]
        ready = eventually(authority, 0, {"assessment_pending"})
        assert ready["decision"] == "accept" and ready["preparation"]["manifest"]
        ready_manifest = Path(ready["preparation"]["manifest"])
        product = json.loads(ready_manifest.read_text())
        assert product["review_state"] == "unreviewed"
        first_crop = [samples[index] for index in [0, 1, 12, 13, 24, 25, 36, 37]]
        assert values(ready_manifest.parent / "elevation.tif") == first_crop
        assert values(ready_manifest.parent / "validity.tif") == [255] * 8
        assert values(ready_manifest.parent / "provider-validity.tif") == [255] * 8
        assert values(ready_manifest.parent / "water.tif") == [0] * 8
        status = json.loads(run(cli, "curate", "status", runtime, ready["review_sha256"]))
        assert status["claims"]["actor"]["kind"] == "human"
        assert status["claims"]["license"]["training_permission"] == "confirmed"
        assert status["claims"]["permission_evidence_ids"] == ["supplier"]
        assert set(status["claims"]["findings"]) == set(FINDINGS)

        reject = eventually(authority, 1, {"awaiting_acceptance"})
        rejected = vote(authority, headers, gallery_sha256, 1, reject, "reject")
        assert rejected["decision"] == "reject"
        assert eventually(authority, 1, {"not_selected"})["preparation"]["manifest"] is None

        missing = tiles(authority)[2]
        accepted_attention = vote(authority, headers, gallery_sha256, 2, missing, "accept")
        assert accepted_attention["decision"] == "accept"
        retained = eventually(authority, 2, {"needs_attention"})
        assert retained["decision"] == "accept" and retained["preparation"]["manifest"] is None
        corrupt_tile = tiles(authority)[5]
        assert vote(authority, headers, gallery_sha256, 5, corrupt_tile, "accept")["decision"] == "accept"
        corrupt_retained = eventually(authority, 5, {"needs_attention"})
        assert corrupt_retained["decision"] == "accept"

    finally:
        process.terminate()
        process.wait(timeout=10)

    assert ready_manifest and ready_manifest.exists()
    with sqlite3.connect(directory / "catalog_db") as database:
        before_records = database.execute("SELECT COUNT(*) FROM records").fetchone()[0]
    process, authority, headers = start_server(cli, runtime, manifest)
    try:
        restarted = eventually(authority, 0, {"assessment_pending"})
        assert restarted["decision"] == "accept"
        assert Path(restarted["preparation"]["manifest"]) == ready_manifest
        time.sleep(0.3)
        with sqlite3.connect(directory / "catalog_db") as database:
            after_records = database.execute("SELECT COUNT(*) FROM records").fetchone()[0]
        assert after_records == before_records, "restart created extra curation records"

        automatic_head = restarted["head"]
        assert automatic_head != human_head
        replaced = vote(authority, headers, gallery_sha256, 0, restarted, "reject", previous=human_head)
        assert replaced["decision"] == "reject"
        eventually(authority, 0, {"not_selected"})
        vote(authority, headers, gallery_sha256, 0, restarted, "accept", expected=409,
             previous=automatic_head)
        assert ready_manifest.exists(), "later rejection must preserve immutable prepared output"
    finally:
        process.terminate()
        process.wait(timeout=10)


def interrupted_no_check(cli, directory):
    runtime, _raw, _receipt, _profile, _evidence, gallery_input_path, gallery_input, _samples = fixture(cli, directory)
    gallery_input["tiles"] = gallery_input["tiles"][:1]
    gallery_input_path.write_text(json.dumps(gallery_input))
    manifest = Path(run(cli, "gallery", "build", gallery_input_path,
                        ROOT / "catalog/labels/terrain-v1.toml", directory / "gallery"))
    gallery_sha256 = sha(manifest)

    wrappers = directory / "bin"
    wrappers.mkdir()
    reached = directory / "regional-preparation-reached"
    release = directory / "regional-preparation-release"
    real_translate = shutil.which("gdal_translate")
    wrapper = wrappers / "gdal_translate"
    wrapper.write_text(f'''#!{Path(os.sys.executable)}
import os, pathlib, sys, time
args = sys.argv[1:]
if "TILED=YES" in args:
    pathlib.Path({str(reached)!r}).write_text("qualified")
    deadline = time.monotonic() + 20
    while not pathlib.Path({str(release)!r}).exists():
        if time.monotonic() >= deadline:
            sys.stderr.write("automatic preparation barrier timed out")
            sys.exit(1)
        time.sleep(0.05)
os.execv({real_translate!r}, [{real_translate!r}] + args)
''')
    wrapper.chmod(0o755)
    env = dict(os.environ, PATH=str(wrappers) + os.pathsep + os.environ["PATH"])
    process, authority, headers = start_server(cli, runtime, manifest, env=env)
    try:
        pending = eventually(authority, 0, {"awaiting_acceptance"})
        saved = vote(authority, headers, gallery_sha256, 0, pending, "accept")
        deadline = time.monotonic() + 20
        while not reached.exists() and time.monotonic() < deadline:
            time.sleep(0.05)
        assert reached.read_text() == "qualified", "regional preparation did not reach the interruption barrier"
        in_progress = tiles(authority)[0]
        assert in_progress["preparation"]["state"] == "preparing"
        rejected = vote(authority, headers, gallery_sha256, 0, in_progress, "reject",
                        previous=saved["head"])
        assert rejected["decision"] == "reject"
        release.write_text("continue")
        stopped = eventually(authority, 0, {"not_selected"})
        assert stopped["preparation"]["manifest"] is None
        deadline = time.monotonic() + 20
        worker_finished = False
        while time.monotonic() < deadline:
            for status_path in (directory / "review" / "automatic-preparation").rglob("status.json"):
                status = json.loads(status_path.read_text())
                if status["head"] == rejected["head"] and status["preparation"]["state"] == "not_selected":
                    worker_finished = True
                    break
            if worker_finished:
                break
            time.sleep(0.05)
        assert worker_finished, "worker did not finish the rejected interrupted job"
    finally:
        release.touch()
        process.terminate()
        process.wait(timeout=10)
    with sqlite3.connect(directory / "catalog_db") as database:
        table = database.execute(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='prepared_regions'"
        ).fetchone()[0]
        registered = database.execute("SELECT COUNT(*) FROM prepared_regions").fetchone()[0] if table else 0
        assert registered == 0

    process, authority, _headers = start_server(cli, runtime, manifest)
    try:
        restarted = eventually(authority, 0, {"not_selected"})
        assert restarted["decision"] == "reject" and restarted["preparation"]["manifest"] is None
    finally:
        process.terminate()
        process.wait(timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="topography automatic preparation ") as temporary:
        check(args.binary.resolve(), Path(temporary))
    with tempfile.TemporaryDirectory(prefix="topography interrupted preparation ") as temporary:
        interrupted_no_check(args.binary.resolve(), Path(temporary))
    print("Automatic preparation passed: profiles, exact rasters, attention, restart reuse and interrupted No.")


if __name__ == "__main__":
    main()
