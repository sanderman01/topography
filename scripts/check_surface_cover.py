#!/usr/bin/env python3
"""Exercise versioned snow/ice and vegetation preparation through the public CLI."""

import argparse
import copy
import json
import os
from pathlib import Path
import shutil
import tempfile

from check_acquisition import FIXTURES, ROOT, run, runtime_file, sha, values
from check_automatic_preparation import eventually, request, start_server, tiles, vote


def raster(directory, name, samples, *, kind="Byte", x=700000, nodata=False):
    source = directory / f"{name}.asc"
    source.write_text(
        f"ncols 4\nnrows 4\nxllcorner {x}\nyllcorner 2000000\ncellsize 1\nNODATA_value -9999\n"
        + "\n".join(" ".join(map(str, samples[index:index + 4])) for index in range(0, 16, 4))
        + "\n"
    )
    path = directory / f"{name}.tif"
    run("gdal_translate", "-q", "-ot", kind, "-a_srs", "EPSG:32619",
        *([] if nodata else ["-a_nodata", "none"]), source, path)
    return path


def fixture(cli, directory, *, confirmed=False, runtime=None, west=700000):
    runtime = runtime or runtime_file(directory)
    source = directory / "source.toml"
    source.write_text((FIXTURES / "source.toml").read_text()
                      .replace("synthetic = true", "synthetic = false")
                      .replace("acquisition_dates = []", 'acquisition_dates = ["2020-01-01"]')
                      .replace('training_permission = "unknown"',
                        ('training_permission = "confirmed"\nlicense_name = "Public fixture"\n'
                         'license_url = "https://example.invalid/license"\n'
                         'evidence = "Fixture declaration"\nattribution = "Fixture"\n'
                         'edited_map_reuse = "Permitted"\nredistribution = "Permitted"')
                        if confirmed else 'training_permission = "unknown"'))
    shutil.copyfile(FIXTURES / "metadata.json", directory / "metadata.json")
    surface = [100.25 + index for index in range(16)]
    asset = raster(directory, "surface", surface, kind="Float32", x=west)
    raw = Path(run(cli, "import", source, asset, runtime, 1048576))
    review = Path(run(cli, "review", raw, directory / "review", "0,0,4,4"))
    quality = directory / "quality.json"
    run(cli, "quality", "draft", raw, review, quality)
    return runtime, raw, review, quality, surface


def pinned(path):
    return {"path": path.name, "sha256": sha(path)}


def raster_with_hidden_mask(directory, name, *, x=700000):
    source = raster(directory, f"{name}-source", [1] * 16, x=x)
    output = directory / f"{name}.tif"
    run("gdal_translate", "-q", "-of", "GTiff", "-mask", "1", "-a_nodata", "none", source, output)
    return output


def v2_input(directory, legacy, *, snow, vegetation, vegetation_status="assessed", x=700000):
    v2_input.counter += 1
    prefix = f"cover-{v2_input.counter}"
    result = copy.deepcopy(legacy)
    result["schema_version"] = 2
    evidence = directory / "cover-evidence.txt"
    evidence.write_text("Synthetic scoped cover assessment\n")
    common = {"water": [2, 2] + [1] * 14, "quality": [1] * 16,
              "repairs": [0] * 16, "features": [0] * 16}
    for name, samples in common.items():
        result["masks"][name] = pinned(raster(directory, f"{prefix}-{name}", samples, x=x))
    result["masks"]["snow-ice"] = pinned(raster(directory, f"{prefix}-snow-ice", snow, x=x))
    if vegetation is None:
        result["masks"]["vegetation"] = None
    else:
        result["masks"]["vegetation"] = pinned(raster(directory, f"{prefix}-vegetation", vegetation, x=x))
    result["surface_cover"] = {
        "snow-ice": {"status": "assessed", "note": "Complete scoped snow and ice assessment",
                     "observed_at": "2020-01-01", "evidence": [pinned(evidence)]},
        "vegetation": {"status": vegetation_status, "note": "Optional vegetation assessment",
                       "observed_at": None,
                       "evidence": [] if vegetation_status == "unavailable" else [pinned(evidence)]},
    }
    return result


v2_input.counter = 0


def assess(cli, directory, quality, body, name, *, expected=0, message=None):
    quality.write_text(json.dumps(body))
    output = directory / f"{name}.json"
    run(cli, "quality", "assess", quality, output, expected=expected, message=message)
    return json.loads(output.read_text()) if expected == 0 else None


def quality_checks(cli, directory, *, runtime=None, west=700000):
    runtime, raw, review, quality, surface = fixture(cli, directory, runtime=runtime, west=west)
    legacy = json.loads(quality.read_text())
    for name, samples in {"water": [1] * 16, "quality": [1] * 16,
                          "repairs": [0] * 16, "features": [0] * 16}.items():
        legacy["masks"][name] = pinned(raster(directory, f"legacy-{name}", samples, x=west))
    first = assess(cli, directory, quality, legacy, "legacy-first")
    quality.write_text(json.dumps(legacy))
    run(cli, "quality", "assess", quality, directory / "legacy-second.json")
    assert (directory / "legacy-first.json").read_bytes() == (directory / "legacy-second.json").read_bytes()
    assert first["schema_version"] == 1 and "surface_cover" not in first

    snow = [2, 1, 2] + [1] * 13
    vegetation = [1, 2, 0] + [1] * 13
    valid = v2_input(directory, legacy, snow=snow, vegetation=vegetation, x=west)
    report = assess(cli, directory, quality, valid, "mixed-cover")
    assert report["schema_version"] == 2 and report["policy"] == "native_quality_cover_v1"
    assert report["blockers"] == []
    counts = report["counts"]
    assert counts["water_cells"] == 2 and counts["snow_ice_cells"] == 2
    assert counts["ground_cells"] == counts["valid_terrestrial_cells"] == 13
    assert counts["vegetation_cells"] == counts["unknown_vegetation_cells"] == 1
    assert report["surface_cover"]["snow-ice"]["evidence"][0]["sha256"] == sha(directory / "cover-evidence.txt")
    assert "path" not in report["surface_cover"]["snow-ice"]["evidence"][0]

    unknown_over_water = copy.deepcopy(valid)
    path = raster(directory, "unknown-over-water", [0, 1, 2] + [1] * 13, x=west)
    unknown_over_water["masks"]["snow-ice"] = pinned(path)
    partial = assess(cli, directory, quality, unknown_over_water, "unknown-snow")
    assert partial["counts"]["unknown_snow_ice_cells"] == 1
    assert any("snow/ice classification incomplete" in value for value in partial["blockers"])

    all_ice = copy.deepcopy(valid)
    path = raster(directory, "all-ice", [2] * 16, x=west)
    all_ice["masks"]["snow-ice"] = pinned(path)
    frozen = assess(cli, directory, quality, all_ice, "all-ice-report")
    assert frozen["counts"]["ground_cells"] == 0
    assert any("99.5%" in value for value in frozen["blockers"])

    unavailable = v2_input(directory, legacy, snow=[1] * 16, vegetation=None,
                           vegetation_status="unavailable", x=west)
    no_vegetation = assess(cli, directory, quality, unavailable, "vegetation-unavailable")
    assert no_vegetation["blockers"] == [] and no_vegetation["counts"]["unknown_vegetation_cells"] == 16
    all_vegetation = v2_input(directory, legacy, snow=[1] * 16, vegetation=[2] * 16, x=west)
    vegetated = assess(cli, directory, quality, all_vegetation, "all-vegetation")
    assert vegetated["blockers"] == [] and vegetated["counts"]["vegetation_cells"] == 16

    bad = copy.deepcopy(valid)
    bad["surface_cover"]["snow-ice"].update(status="unavailable", evidence=[])
    assess(cli, directory, quality, bad, "snow-unavailable", expected=1, message="cannot be unavailable")
    bad = copy.deepcopy(valid)
    bad["surface_cover"]["snow-ice"]["evidence"] = []
    assess(cli, directory, quality, bad, "snow-no-evidence", expected=1, message="disagree")
    bad = copy.deepcopy(valid)
    bad["surface_cover"]["snow-ice"]["evidence"][0]["sha256"] = "f" * 64
    assess(cli, directory, quality, bad, "snow-bad-evidence", expected=1, message="digest mismatch")

    for name, path, message in [
        ("category", raster(directory, "bad-cover-category", [3] * 16, x=west), "category"),
        ("alignment", raster(directory, "bad-cover-alignment", [1] * 16, x=west + 1), "grid mismatch"),
        ("nodata", raster(directory, "bad-cover-nodata", [1] * 16, nodata=True, x=west), "nodata"),
        ("hidden-mask", raster_with_hidden_mask(directory, "bad-cover-hidden-mask", x=west),
         "hidden validity mask"),
    ]:
        bad = copy.deepcopy(valid)
        bad["masks"]["snow-ice"] = pinned(path)
        assess(cli, directory, quality, bad, f"bad-snow-{name}", expected=1, message=message)
        bad = copy.deepcopy(valid)
        bad["masks"]["vegetation"] = pinned(path)
        assess(cli, directory, quality, bad, f"bad-vegetation-{name}", expected=1, message=message)
    return runtime, raw, review, quality, valid, report, surface


def preparation_checks(cli, directory, *, runtime=None, west=700000):
    runtime, raw, review, quality, valid, report, surface = quality_checks(
        cli, directory, runtime=runtime, west=west)
    quality.write_text(json.dumps(valid))
    report_path = directory / "accepted-cover-report.json"
    run(cli, "quality", "assess", quality, report_path)
    report = json.loads(report_path.read_text())
    decision = directory / "decision.json"
    run(cli, "curate", "draft", runtime, raw, review,
        ROOT / "catalog/labels/terrain-v1.toml", decision)
    record = json.loads(decision.read_text())
    record["claims"].update(actor={"kind": "human", "name": "Synthetic fixture owner"},
                            decision="accept", reason="Synthetic v2 preparation verification",
                            primary_class_id=1)
    record["evidence"] = [{"path": report_path.name, "evidence": {
        "id": "assessment", "description": "Synthetic v2 quality assessment",
        "source_url": None, "sha256": sha(report_path)}}]
    for finding in record["claims"]["findings"].values():
        finding.update(state="pass", note="Synthetic fixture only", evidence_ids=["assessment"])
    record["claims"]["coverage"] = {
        "terrestrial_cells": report["counts"]["terrestrial_cells"],
        "valid_terrestrial_cells": report["counts"]["valid_terrestrial_cells"],
        "note": "Measured synthetic cover fixture", "evidence_ids": ["assessment"]}
    record["claims"]["license"].update(
        training_permission="confirmed", license_name="Synthetic fixture",
        license_url="https://example.invalid/license", evidence="Fixture only",
        attribution="Fixture", edited_map_reuse="Fixture", redistribution="Fixture")
    record["claims"]["permission_evidence_ids"] = ["assessment"]
    decision.write_text(json.dumps(record))
    head = run(cli, "curate", "record", decision, runtime)
    request = directory / "prepare-v2.json"
    request.write_text(json.dumps({"schema_version": 2,
        "inputs": [{"quality_path": quality.name, "curation_record_sha256": head}],
        "grid": {"origin": [west, 2000004], "size": [4, 4], "spacing_m": [1, 1]},
        "masked_fill_m": -12.5}))
    manifest = Path(run(cli, "prepare", "build", runtime, request))
    product = json.loads(manifest.read_text())
    assert product["schema_version"] == 2 and product["policy"] == "native_ground_cover_v1"
    assert values(manifest.parent / "surface-elevation.tif") == surface
    expected_ground = [-12.5, -12.5, -12.5] + surface[3:]
    assert values(manifest.parent / "elevation.tif") == expected_ground
    assert values(manifest.parent / "validity.tif") == [0, 0, 0] + [255] * 13
    assert values(manifest.parent / "provider-validity.tif") == [255] * 16
    assert values(manifest.parent / "water.tif") == [255, 255] + [0] * 14
    assert values(manifest.parent / "snow-ice.tif") == [2, 1, 2] + [1] * 13
    assert values(manifest.parent / "vegetation.tif") == [1, 2, 0] + [1] * 13
    verified = json.loads(run(cli, "prepare", "verify", runtime, manifest))
    assert verified["registered"] and verified["current"]
    assert Path(run(cli, "prepare", "build", runtime, request)) == manifest
    legacy_request = copy.deepcopy(json.loads(request.read_text()))
    legacy_request["schema_version"] = 1
    (directory / "mixed-request.json").write_text(json.dumps(legacy_request))
    run(cli, "prepare", "build", runtime, directory / "mixed-request.json",
        expected=1, message="same schema version")
    return runtime, manifest


def profile(path, receipt, evidence, *, version, snow_mask=None):
    finding = {"note": "Synthetic supplier assessment", "evidence_ids": ["supplier"]}
    body = {
        "schema_version": version,
        "policy": "supplier_qa_pilot_v1" if version == 1 else "supplier_qa_cover_v1",
        "source_sha256": receipt["source_sha256"], "asset_sha256": receipt["asset_sha256"],
        "evidence": [{"id": "supplier", "path": evidence.name, "sha256": sha(evidence),
                      "description": "Exact synthetic supplier cover assessment",
                      "source_url": "https://example.invalid/source"}],
        "permission_evidence_ids": ["supplier"],
        "findings": {name: dict(finding) for name in ["coordinates", "ground_support",
            "artifacts_and_repairs", "feature_fidelity", "water_classification",
            "unsupported_feature_gaps"]},
        "water": {"kind": "all_terrestrial", "windows": [[0, 0, 4, 4]]},
    }
    if version == 2:
        body.update(
            snow_ice={"kind": "mask", "path": snow_mask.name, "sha256": sha(snow_mask),
                      "note": "Complete synthetic snow and ice classification",
                      "observed_at": "2020-01-01", "evidence_ids": ["supplier"]},
            vegetation={"kind": "unavailable", "note": "Vegetation companion unavailable",
                        "observed_at": None, "evidence_ids": []})
    path.write_text(json.dumps(body))
    return path


def gallery(cli, directory, raw, review, profile_path, name):
    input_path = directory / f"{name}-input.json"
    input_path.write_text(json.dumps({"schema_version": 1, "title": name, "tiles": [{
        "id": "cover", "title": "Cover fixture", "raw": str(raw), "review": str(review),
        "primary_class_id": 1, "preparation_profile": str(profile_path)}]}))
    return Path(run(cli, "gallery", "build", input_path,
                    ROOT / "catalog/labels/terrain-v1.toml", directory / name))


def automatic_checks(cli, directory):
    runtime, raw, review, _quality, surface = fixture(cli, directory, confirmed=True)
    receipt = json.loads(raw.read_text())
    evidence = directory / "supplier-cover.txt"
    evidence.write_text("Synthetic supplier cover assessment\n")
    legacy_profile = profile(directory / "profile-v1.json", receipt, evidence, version=1)
    legacy_gallery = gallery(cli, directory, raw, review, legacy_profile, "legacy-gallery")
    legacy_gallery_sha = sha(legacy_gallery)
    process, authority, headers = start_server(cli, runtime, legacy_gallery)
    try:
        pending = eventually(authority, 0, {"awaiting_acceptance"})
        saved = vote(authority, headers, legacy_gallery_sha, 0, pending, "accept")
        assert saved["decision"] == "accept"
        ready = eventually(authority, 0, {"assessment_pending"})
        legacy_manifest = Path(ready["preparation"]["manifest"])
        assert json.loads(legacy_manifest.read_text())["schema_version"] == 1
        legacy_status = json.loads(run(cli, "curate", "status", runtime, ready["review_sha256"]))
        human = {key: legacy_status["claims"][key] for key in ["actor", "decision", "reason", "primary_class_id"]}
    finally:
        process.terminate()
        process.wait(timeout=10)

    process, authority, _headers = start_server(cli, runtime, legacy_gallery)
    try:
        pending_cover = eventually(authority, 0, {"assessment_pending"})
        assert pending_cover["decision"] == "accept"
        assert Path(pending_cover["preparation"]["manifest"]) == legacy_manifest
    finally:
        process.terminate()
        process.wait(timeout=10)

    snow_mask = raster(directory, "automatic-snow-ice", [2, 1, 2] + [1] * 13)
    cover_profile = profile(directory / "profile-v2.json", receipt, evidence, version=2,
                            snow_mask=snow_mask)
    cover_gallery = gallery(cli, directory, raw, review, cover_profile, "cover-gallery")
    process, authority, _headers = start_server(cli, runtime, cover_gallery)
    try:
        upgraded = eventually(authority, 0, {"ready"})
        assert upgraded["decision"] == "accept"
        cover_manifest = Path(upgraded["preparation"]["manifest"])
        product = json.loads(cover_manifest.read_text())
        assert product["schema_version"] == 2 and cover_manifest != legacy_manifest
        assert values(cover_manifest.parent / "surface-elevation.tif") == surface
        assert values(cover_manifest.parent / "elevation.tif") == [0.0, surface[1], 0.0] + surface[3:]
        upgraded_status = json.loads(run(cli, "curate", "status", runtime, upgraded["review_sha256"]))
        assert {key: upgraded_status["claims"][key] for key in human} == human
        assert upgraded_status["automation"]["kind"] == "supplier_qa_qualification_v2"
    finally:
        process.terminate()
        process.wait(timeout=10)

    process, authority, _headers = start_server(cli, runtime, cover_gallery)
    try:
        restarted = eventually(authority, 0, {"ready"})
        assert Path(restarted["preparation"]["manifest"]) == cover_manifest
    finally:
        process.terminate()
        process.wait(timeout=10)


def source_mask_toctou_check(cli, directory):
    runtime, raw, review, _quality, _surface = fixture(cli, directory, confirmed=True)
    receipt = json.loads(raw.read_text())
    evidence = directory / "supplier-cover.txt"
    evidence.write_text("Synthetic supplier cover assessment\n")
    snow_mask = raster(directory, "race-snow-ice", [1] * 16)
    replacement = raster(directory, "race-replacement", [1] * 15 + [2])
    cover_profile = profile(directory / "race-profile-v2.json", receipt, evidence, version=2,
                            snow_mask=snow_mask)
    manifest = gallery(cli, directory, raw, review, cover_profile, "race-gallery")
    wrappers = directory / "bin"
    wrappers.mkdir()
    marker = directory / "source-mask-replaced"
    real_translate = shutil.which("gdal_translate")
    wrapper = wrappers / "gdal_translate"
    wrapper.write_text(f'''#!{Path(os.sys.executable)}
import os, pathlib, shutil, sys
args = sys.argv[1:]
target = pathlib.Path({str(snow_mask)!r})
marker = pathlib.Path({str(marker)!r})
if "-srcwin" in args and str(target) in args and not marker.exists():
    shutil.copyfile({str(replacement)!r}, target)
    marker.write_text("replaced before GDAL read")
os.execv({real_translate!r}, [{real_translate!r}] + args)
''')
    wrapper.chmod(0o755)
    env = dict(os.environ, PATH=str(wrappers) + os.pathsep + os.environ["PATH"])
    process, authority, _headers = start_server(cli, runtime, manifest, env=env)
    try:
        attention = eventually(authority, 0, {"needs_attention"})
        assert marker.exists()
        assert attention["decision"] == "pending" and attention["preparation"]["manifest"] is None
        assert "changed" in attention["preparation"]["message"] or "digest" in attention["preparation"]["message"]
    finally:
        process.terminate()
        process.wait(timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="topography surface cover ") as temporary:
        preparation_checks(args.binary.resolve(), Path(temporary))
    with tempfile.TemporaryDirectory(prefix="topography automatic surface cover ") as temporary:
        automatic_checks(args.binary.resolve(), Path(temporary))
    with tempfile.TemporaryDirectory(prefix="topography cover mask race ") as temporary:
        source_mask_toctou_check(args.binary.resolve(), Path(temporary))
    print("Surface-cover workflows passed.")


if __name__ == "__main__":
    main()
