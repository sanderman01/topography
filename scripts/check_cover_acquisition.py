#!/usr/bin/env python3
"""Exercise game cover (game_cover_v1) offline against loopback WorldCover/GLIMS fixtures."""

import argparse
import json
from pathlib import Path
import shutil
import sqlite3
import tempfile

from check_acquisition import FIXTURES, ROOT, run, runtime_file, sha, values
from check_automatic_preparation import eventually, start_server, tiles, vote
from cover_fixtures import CoverServer, outline

X0, TOP, SIZE = 700000, 2000600, 600
FINDINGS = ["coordinates", "ground_support", "artifacts_and_repairs", "feature_fidelity",
            "water_classification", "unsupported_feature_gaps"]
LEGEND = ROOT / "catalog/labels/terrain-v1.toml"


def fixture(cli, directory):
    directory.mkdir(parents=True)
    runtime = runtime_file(directory)
    source = directory / "source.toml"
    source.write_text((FIXTURES / "source.toml").read_text()
                      .replace("synthetic = true", "synthetic = false")
                      .replace("acquisition_dates = []", 'acquisition_dates = ["2020-06-15"]')
                      .replace('training_permission = "unknown"',
                               'training_permission = "confirmed"\nlicense_name = "Public fixture"\n'
                               'license_url = "https://example.invalid/license"\n'
                               'evidence = "Fixture declaration"\nattribution = "Fixture"\n'
                               'edited_map_reuse = "Permitted"\nredistribution = "Permitted"'))
    shutil.copyfile(FIXTURES / "metadata.json", directory / "metadata.json")
    asc = directory / "surface.asc"
    asc.write_text(f"ncols {SIZE}\nnrows {SIZE}\nxllcorner {X0}\nyllcorner {TOP - SIZE}\ncellsize 1\n"
                   + "\n".join(" ".join(str(100 + (r + c) % 7) for c in range(SIZE)) for r in range(SIZE)) + "\n")
    asset = directory / "surface.tif"
    run("gdal_translate", "-q", "-ot", "Float32", "-a_srs", "EPSG:32619", "-a_nodata", "none", asc, asset)
    raw = Path(run(cli, "import", source, asset, runtime, 64 * 1024 * 1024))
    return runtime, raw


def crop(cli, raw, directory, name, x, y):
    return Path(run(cli, "review", raw, directory / name, f"{x},{y},200,200"))


def cells(path):
    """Complete Byte raster as row-major bytes."""
    with tempfile.TemporaryDirectory() as scratch:
        out = Path(scratch) / "values.bin"
        run("gdal_translate", "-q", "-of", "ENVI", path, out)
        return out.read_bytes()


def at(data, left, top, x, y):
    """Value of a 200x200 crop mask at map coordinates."""
    return data[int(top - y) * 200 + int(x - left)]


def same_grid(review, mask):
    reference = json.loads(run("gdalinfo", "-json", review.parent / "elevation.tif"))
    info = json.loads(run("gdalinfo", "-json", mask))
    for key in ["size", "geoTransform", "coordinateSystem"]:
        assert info[key] == reference[key], key
    assert len(info["bands"]) == 1 and info["bands"][0]["type"] == "Byte"
    assert "noDataValue" not in info["bands"][0]


def worldcover(x, y):
    if x >= 700350 and y >= 2000350:
        return 70
    return 10 if x < 700300 else 60


GLIMS = [outline("G1", 1, "2019-08-01", (700210, 2000210, 700260, 2000260)),
         outline("G1", 2, "1650-01-01", (700200, 2000200, 700300, 2000300)),
         outline("G2", 3, "2000-07-01", (700320, 2000210, 700390, 2000280)),
         outline("G2", 4, "2015-07-01", (700320, 2000210, 700340, 2000230)),
         outline("G3", 5, "2019-08-01", (700270, 2000300, 700290, 2000320), "intrnl_rock")]


def profile(path, raw, evidence):
    """Source profile without cover descriptors; the worker adds game cover."""
    receipt = json.loads(raw.read_text())
    finding = {"note": "Synthetic supplier assessment", "evidence_ids": ["supplier"]}
    path.write_text(json.dumps({
        "schema_version": 2, "policy": "supplier_qa_cover_v1",
        "source_sha256": receipt["source_sha256"], "asset_sha256": receipt["asset_sha256"],
        "evidence": [{"id": "supplier", "path": evidence.name, "sha256": sha(evidence),
                      "description": "Synthetic supplier assessment",
                      "source_url": "https://example.invalid/source"}],
        "permission_evidence_ids": ["supplier"],
        "findings": {name: dict(finding) for name in FINDINGS},
        "water": {"kind": "all_terrestrial", "windows": [[0, 0, SIZE, SIZE]]}}))
    return path


def legacy_product(cli, directory, runtime, raw, review, left, top):
    """Human Yes plus a v1 product built by the manual CLI chain, as older tiles have."""
    def mask(name, value):
        path = directory / f"legacy-{name}.asc"
        path.write_text(f"ncols 200\nnrows 200\nxllcorner {left}\nyllcorner {top - 200}\ncellsize 1\n"
                        + "\n".join(" ".join(str(value(c)) for c in range(200)) for _ in range(200)) + "\n")
        tif = path.with_suffix(".tif")
        run("gdal_translate", "-q", "-ot", "Byte", "-a_srs", "EPSG:32619", "-a_nodata", "none", path, tif)
        return {"path": tif.name, "sha256": sha(tif)}
    quality = directory / "legacy-quality.json"
    run(cli, "quality", "draft", raw, review, quality)
    body = json.loads(quality.read_text())
    body["masks"] = {"water": mask("water", lambda c: 2 if c < 20 else 1),
                     "quality": mask("quality", lambda _c: 1), "repairs": mask("repairs", lambda _c: 0),
                     "features": mask("features", lambda _c: 0)}
    quality.write_text(json.dumps(body))
    report_path = directory / "legacy-report.json"
    run(cli, "quality", "assess", quality, report_path)
    report = json.loads(report_path.read_text())
    assert report["blockers"] == [], report["blockers"]
    draft = directory / "legacy-decision.json"
    run(cli, "curate", "draft", runtime, raw, review, LEGEND, draft)
    record = json.loads(draft.read_text())
    record["claims"].update(actor={"kind": "human", "name": "Synthetic fixture owner"},
                            decision="accept", reason="Legacy whole-tile Yes", primary_class_id=1)
    record["evidence"] = [{"path": report_path.name, "evidence": {
        "id": "assessment", "description": "Legacy v1 quality assessment", "source_url": None,
        "sha256": sha(report_path)}}]
    for finding in record["claims"]["findings"].values():
        finding.update(state="pass", note="Legacy technical qualification", evidence_ids=["assessment"])
    record["claims"]["coverage"] = {
        "terrestrial_cells": report["counts"]["terrestrial_cells"],
        "valid_terrestrial_cells": report["counts"]["valid_terrestrial_cells"],
        "note": "Legacy measured coverage", "evidence_ids": ["assessment"]}
    record["claims"]["permission_evidence_ids"] = ["assessment"]
    draft.write_text(json.dumps(record))
    head = run(cli, "curate", "record", draft, runtime)
    request = directory / "legacy-request.json"
    request.write_text(json.dumps({"schema_version": 1,
        "inputs": [{"quality_path": quality.name, "curation_record_sha256": head}],
        "grid": {"origin": [left, top], "size": [200, 200], "spacing_m": [1, 1]}, "masked_fill_m": 0.0}))
    return head, Path(run(cli, "prepare", "build", runtime, request))


def gallery(cli, directory, name, entries):
    path = directory / f"{name}-input.json"
    path.write_text(json.dumps({"schema_version": 1, "title": name, "tiles": [
        dict({"id": f"tile-{i}", "title": f"Tile {i}", "primary_class_id": 1}, **entry)
        for i, entry in enumerate(entries)]}))
    return Path(run(cli, "gallery", "build", path, LEGEND, directory / name))


def draft_checks(cli, directory, server, raw):
    review = crop(cli, raw, directory, "review-draft", 200, 200)
    env = server.policy("mixed", worldcover=worldcover, glims=GLIMS)
    out = directory / "cover-mixed"
    report = run(cli, "cover", "draft", raw, review, out, env=env)
    assert "game_cover_v1" in report, report
    cover = json.loads((out / "cover.json").read_text())
    assert cover["policy"] == "game_cover_v1" and cover["review_sha256"] == sha(review)
    assert len(cover["sources"]["worldcover"]["urls"]) == 1
    glaciers = {g["glac_id"]: g["anlys_id"] for g in cover["sources"]["glims"]["glaciers"]}
    assert glaciers == {"G1": 1, "G2": 4}, glaciers
    assert cover["sources"]["glims"]["ignored_outlines"] == 1
    for name in ["snow-ice.tif", "vegetation.tif"]:
        same_grid(review, out / name)
    snow, veg = cells(out / "snow-ice.tif"), cells(out / "vegetation.tif")
    assert set(snow) == {1, 2} and set(veg) == {1, 2}
    point = lambda data, x, y: at(data, 700200, 2000400, x, y)
    assert point(snow, 700380, 2000380) == 2   # WorldCover 70
    assert point(snow, 700235, 2000235) == 2   # latest G1 outline
    assert point(snow, 700280, 2000280) == 1   # 1650 reconstruction ignored
    assert point(snow, 700330, 2000220) == 2   # latest G2 outline
    assert point(snow, 700370, 2000260) == 1   # older G2 outline ignored
    assert point(snow, 700280, 2000310) == 1   # other line types ignored
    assert point(veg, 700250, 2000350) == 2 and point(veg, 700320, 2000300) == 1
    assert point(veg, 700380, 2000380) == 1
    assert cover["counts"]["snow_ice_present"] == snow.count(2)

    # Outside WorldCover coverage: a recorded judgment, all absent.
    out = directory / "cover-outside"
    run(cli, "cover", "draft", raw, review, out, env=server.policy("outside", worldcover=None))
    cover = json.loads((out / "cover.json").read_text())
    assert "outside WorldCover coverage" in cover["note"]
    assert set(cells(out / "snow-ice.tif")) == {1} and set(cells(out / "vegetation.tif")) == {1}

    # Unreachable GLIMS is a technical failure, not absence.
    failed = server.policy("glims-missing", glims=None)
    run(cli, "cover", "draft", raw, review, directory / "cover-failed", env=failed,
        expected=1, message="GLIMS unavailable")
    run(cli, "cover", "draft", raw, review, directory / "cover-mixed", env=env,
        expected=1, message="already exists")
    return failed


def gallery_checks(cli, directory, server, runtime, raw, failed_env):
    evidence = directory / "supplier.txt"
    evidence.write_text("Synthetic supplier assessment\n")
    source_profile = profile(directory / "profile.json", raw, evidence)
    profiled = crop(cli, raw, directory, "review-profiled", 200, 200)
    legacy = crop(cli, raw, directory, "review-legacy", 300, 300)
    human_head, legacy_manifest = legacy_product(cli, directory, runtime, raw, legacy, 700300, 2000300)
    human = json.loads(run(cli, "curate", "status", runtime, sha(legacy)))
    manifest = gallery(cli, directory, "gallery", [
        {"raw": str(raw), "review": str(profiled), "preparation_profile": str(source_profile)},
        {"raw": str(raw), "review": str(legacy)}])
    env = server.policy("gallery", worldcover=worldcover, glims=GLIMS)
    process, authority, headers = start_server(cli, runtime, manifest, env=env)
    try:
        # Profile without cover descriptors: game cover, then v2 after the Yes.
        pending = eventually(authority, 0, {"awaiting_acceptance"}, timeout=60)
        vote(authority, headers, sha(manifest), 0, pending, "accept")
        ready = eventually(authority, 0, {"ready"}, timeout=60)
        product = Path(ready["preparation"]["manifest"])
        assert json.loads(product.read_text())["schema_version"] == 2
        snow = [int(v) for v in values(product.parent / "snow-ice.tif")]
        assert snow[int(2000400 - 2000235) * 200 + 35] == 2 and set(snow) == {1, 2}
        status = json.loads(run(cli, "curate", "status", runtime, ready["review_sha256"]))
        assert status["automation"]["kind"] == "supplier_qa_qualification_v2"

        # Legacy v1 product, no profile: automatic v2 successor keeping the human Yes.
        upgraded = eventually(authority, 1, {"ready"}, timeout=60)
        product = Path(upgraded["preparation"]["manifest"])
        body = json.loads(product.read_text())
        assert body["schema_version"] == 2 and product != legacy_manifest and legacy_manifest.exists()
        assert values(product.parent / "water.tif") == values(legacy_manifest.parent / "water.tif")
        assert set(values(product.parent / "snow-ice.tif")) == {1.0, 2.0}
        status = json.loads(run(cli, "curate", "status", runtime, upgraded["review_sha256"]))
        assert status["automation"]["kind"] == "game_cover_upgrade_v1"
        assert status["automation"]["inherited_human_record_sha256"] == human_head
        assert status["previous_record_sha256"] == human_head
        for key in ["actor", "decision", "reason", "primary_class_id", "license", "permission_evidence_ids"]:
            assert status["claims"][key] == human["claims"][key], key
        assert tiles(authority)[1]["decision"] == "accept"
    finally:
        process.terminate()
        process.wait(timeout=10)
    check = json.loads(run(cli, "curate", "check", runtime))
    assert not check["unreferenced_objects"] and not check["unregistered_records"], check

    # Restart: nothing is rebuilt and no record is added.
    with sqlite3.connect(directory / "catalog_db") as database:
        before = database.execute("SELECT COUNT(*) FROM records").fetchone()[0]
    process, authority, _headers = start_server(cli, runtime, manifest, env=env)
    try:
        assert Path(eventually(authority, 1, {"ready"})["preparation"]["manifest"]) == product
    finally:
        process.terminate()
        process.wait(timeout=10)
    with sqlite3.connect(directory / "catalog_db") as database:
        assert database.execute("SELECT COUNT(*) FROM records").fetchone()[0] == before

    # Unreachable cover sources leave the tile needing attention; the vote is kept.
    attention = crop(cli, raw, directory, "review-attention", 0, 0)
    failing = gallery(cli, directory, "failing-gallery", [
        {"raw": str(raw), "review": str(attention), "preparation_profile": str(source_profile)}])
    process, authority, headers = start_server(cli, runtime, failing, env=failed_env)
    try:
        tile = eventually(authority, 0, {"needs_attention"}, timeout=60)
        assert "GLIMS unavailable" in tile["preparation"]["message"], tile
        vote(authority, headers, sha(failing), 0, tile, "accept")
        tile = eventually(authority, 0, {"needs_attention"}, timeout=60)
        assert tile["decision"] == "accept" and tile["preparation"]["manifest"] is None
    finally:
        process.terminate()
        process.wait(timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    for executable in ["gdal_translate", "gdalinfo", "gdalwarp", "gdal_rasterize", "gdal_create",
                       "gdalbuildvrt", "gdaltransform", "curl", "openssl"]:
        if not shutil.which(executable):
            print(f"Cover acquisition checks skipped: {executable} is not installed.")
            return
    cli = args.binary.resolve()
    with tempfile.TemporaryDirectory(prefix="topography game cover ") as temporary:
        directory = Path(temporary)
        server = CoverServer(directory / "server")
        try:
            runtime, raw = fixture(cli, directory / "work")
            failed = draft_checks(cli, directory / "work", server, raw)
            gallery_checks(cli, directory / "work", server, runtime, raw, failed)
        finally:
            server.close()
    print("Cover acquisition workflows passed.")


if __name__ == "__main__":
    main()
