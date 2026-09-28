#!/usr/bin/env python3
"""Exercise game cover (game_cover_v6) offline against loopback WorldCover/GLIMS/Overpass fixtures."""

import argparse
import json
from pathlib import Path
import shutil
import sqlite3
import tempfile

from check_acquisition import FIXTURES, ROOT, run, runtime_file, sha, values
from check_automatic_preparation import eventually, start_server, tiles, vote
from cover_fixtures import CoverServer, osm_relation, osm_way, outline

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
    if 700200 <= x < 700230 and y >= 2000370:
        return 80
    return 10 if x < 700300 else 60


def box(x0, y0, x1, y1):
    return [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]


# A lake on WorldCover vegetation, a multipolygon (outer split in two ways) with an
# island, a 6 m river across glacier G1 and a default-width stream.
OSM = [osm_way(1, box(700240, 2000370, 700270, 2000390), natural="water"),
       osm_relation(2, [("outer", [(700320, 2000300), (700390, 2000300), (700390, 2000340)]),
                        ("outer", [(700320, 2000300), (700320, 2000340), (700390, 2000340)]),
                        ("inner", box(700345, 2000310, 700365, 2000330))], natural="water"),
       osm_way(3, [(700200, 2000250), (700290, 2000250)], waterway="river", width="6"),
       osm_way(4, [(700290, 2000300), (700290, 2000360)], waterway="stream")]


GLIMS = [outline("G1", 1, "2019-08-01", (700210, 2000210, 700260, 2000260)),
         outline("G1", 2, "1650-01-01", (700200, 2000200, 700300, 2000300)),
         outline("G2", 3, "2000-07-01", (700320, 2000210, 700390, 2000280)),
         outline("G2", 4, "2015-07-01", (700320, 2000210, 700340, 2000230)),
         outline("G3", 5, "2019-08-01", (700270, 2000300, 700290, 2000320), "intrnl_rock")]


def profile(path, raw, evidence, strict=False):
    """Source profile; without cover descriptors the worker adds game cover directly."""
    receipt = json.loads(raw.read_text())
    finding = {"note": "Synthetic supplier assessment", "evidence_ids": ["supplier"]}
    absent = {"kind": "all_absent", "windows": [[0, 0, SIZE, SIZE]], "note": "Strict assessment",
              "observed_at": None, "evidence_ids": ["supplier"]}
    covers = {"snow_ice": absent, "vegetation": absent} if strict else {}
    path.write_text(json.dumps({**covers,
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
    env = server.policy("mixed", worldcover=worldcover, glims=GLIMS, osm=OSM)
    out = directory / "cover-mixed"
    report = run(cli, "cover", "draft", raw, review, out, env=env)
    assert "game_cover_v6" in report, report
    cover = json.loads((out / "cover.json").read_text())
    assert cover["policy"] == "game_cover_v6" and cover["review_sha256"] == sha(review)
    assert "© OpenStreetMap contributors, ODbL" in cover["attribution"]
    assert cover["sources"]["osm"]["areas"] == 2 and cover["sources"]["osm"]["lines"] == 2
    assert len(cover["sources"]["worldcover"]["urls"]) == 1
    glaciers = {g["glac_id"]: g["anlys_id"] for g in cover["sources"]["glims"]["glaciers"]}
    assert glaciers == {"G1": 1, "G2": 4}, glaciers
    assert cover["sources"]["glims"]["ignored_outlines"] == 1
    for name in ["water.tif", "snow-ice.tif", "vegetation.tif", "land-cover.tif"]:
        same_grid(review, out / name)
    snow, veg = cells(out / "snow-ice.tif"), cells(out / "vegetation.tif")
    water = cells(out / "water.tif")
    assert set(snow) == {1, 2} and set(veg) == {1, 2} and set(water) == {1, 2}
    point = lambda data, x, y: at(data, 700200, 2000400, x, y)
    assert point(water, 700215, 2000385) == 2 and point(veg, 700215, 2000385) == 1  # WorldCover 80
    assert point(water, 700255, 2000380) == 2 and point(veg, 700255, 2000380) == 1  # lake wins
    assert point(water, 700330, 2000320) == 2 and point(water, 700355, 2000320) == 1  # island dry
    assert point(water, 700235, 2000251) == 2 and point(snow, 700235, 2000251) == 1  # river on ice
    assert point(water, 700250, 2000258) == 1                                         # 6 m wide
    assert point(water, 700290, 2000330) == 2 and point(water, 700296, 2000330) == 1  # 3 m stream
    assert not any(w == 2 and (v == 2 or i == 2) for w, v, i in zip(water, veg, snow))
    assert cover["counts"]["water_present"] == water.count(2)
    assert point(snow, 700380, 2000380) == 2   # WorldCover 70
    assert point(snow, 700235, 2000235) == 2   # latest G1 outline
    assert point(snow, 700280, 2000280) == 1   # 1650 reconstruction ignored
    assert point(snow, 700330, 2000220) == 2   # latest G2 outline
    assert point(snow, 700370, 2000260) == 1   # older G2 outline ignored
    assert point(snow, 700280, 2000310) == 1   # other line types ignored
    assert point(veg, 700250, 2000350) == 2 and point(veg, 700320, 2000300) == 1
    assert point(veg, 700380, 2000380) == 1
    assert cover["counts"]["snow_ice_present"] == snow.count(2)
    land = cells(out / "land-cover.tif")
    assert point(land, 700215, 2000385) == 80 and point(land, 700380, 2000380) == 70
    assert point(land, 700255, 2000380) == 10  # raw class under OSM lake water
    assert point(land, 700320, 2000300) == 60
    assert cover["counts"]["land_cover_classes"] == {str(c): land.count(c) for c in set(land)}

    # Outside WorldCover coverage: a recorded judgment, all absent.
    out = directory / "cover-outside"
    run(cli, "cover", "draft", raw, review, out, env=server.policy("outside", worldcover=None))
    cover = json.loads((out / "cover.json").read_text())
    assert "outside WorldCover coverage" in cover["note"]
    assert set(cells(out / "snow-ice.tif")) == {1} and set(cells(out / "vegetation.tif")) == {1}
    assert set(cells(out / "land-cover.tif")) == {0}
    run(cli, "cover", "draft", raw, review, directory / "cover-osm-failed",
        env=server.policy("osm-missing", osm=None), expected=1, message="OpenStreetMap unavailable")

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
    strict_profile = profile(directory / "strict-profile.json", raw, evidence, strict=True)
    profiled = crop(cli, raw, directory, "review-profiled", 200, 200)
    legacy = crop(cli, raw, directory, "review-legacy", 300, 300)
    strict = crop(cli, raw, directory, "review-strict", 0, 300)
    human_head, legacy_manifest = legacy_product(cli, directory, runtime, raw, legacy, 700300, 2000300)
    human = json.loads(run(cli, "curate", "status", runtime, sha(legacy)))
    manifest = gallery(cli, directory, "gallery", [
        {"raw": str(raw), "review": str(profiled), "preparation_profile": str(source_profile)},
        {"raw": str(raw), "review": str(legacy)},
        {"raw": str(raw), "review": str(strict), "preparation_profile": str(strict_profile)}])
    env = server.policy("gallery", worldcover=worldcover, glims=GLIMS)
    process, authority, headers = start_server(cli, runtime, manifest, env=env)
    try:
        # Profile without cover descriptors: game cover, then v3 after the Yes.
        pending = eventually(authority, 0, {"awaiting_acceptance"}, timeout=60)
        vote(authority, headers, sha(manifest), 0, pending, "accept")
        ready = eventually(authority, 0, {"ready"}, timeout=60)
        product = Path(ready["preparation"]["manifest"])
        assert json.loads(product.read_text())["schema_version"] == 3
        assert set(values(product.parent / "land-cover.tif")) <= {10.0, 60.0, 70.0, 80.0}
        snow = [int(v) for v in values(product.parent / "snow-ice.tif")]
        assert snow[int(2000400 - 2000235) * 200 + 35] == 2 and set(snow) == {1, 2}
        status = json.loads(run(cli, "curate", "status", runtime, ready["review_sha256"]))
        assert status["automation"]["kind"] == "supplier_qa_qualification_v2"
        assert current_cover(product)
        location = tiles(authority)[0]["location"]
        assert set(location["corners"]) == {"nw", "ne", "se", "sw"}
        lat, lon = location["centre"]
        assert 17 < lat < 19 and -68 < lon < -66, location
        assert location["google_earth"].startswith(f"https://earth.google.com/web/@{lat:.5f},{lon:.5f},")

        # Strict-evidence v2 product: replaced right away by a game cover successor.
        vote(authority, headers, sha(manifest), 2, eventually(authority, 2, {"awaiting_acceptance"},
                                                            timeout=60), "accept")
        strict_ready = eventually(authority, 2, {"ready"}, timeout=90)
        assert current_cover(Path(strict_ready["preparation"]["manifest"]))
        status = json.loads(run(cli, "curate", "status", runtime, strict_ready["review_sha256"]))
        assert status["automation"]["kind"] == "game_cover_upgrade_v2"

        # Legacy v1 product, no profile: automatic v3 successor keeping the human Yes.
        upgraded = eventually(authority, 1, {"ready"}, timeout=60)
        product = Path(upgraded["preparation"]["manifest"])
        body = json.loads(product.read_text())
        assert body["schema_version"] == 3 and product != legacy_manifest and legacy_manifest.exists()
        assert values(product.parent / "water.tif") == values(legacy_manifest.parent / "water.tif")
        assert set(values(product.parent / "snow-ice.tif")) == {1.0, 2.0}
        assert current_cover(product)
        status = json.loads(run(cli, "curate", "status", runtime, upgraded["review_sha256"]))
        assert status["automation"]["kind"] == "game_cover_upgrade_v2"
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


def current_cover(product):
    report = json.loads((product.parent / "quality-0.json").read_text())
    return report["surface_cover"]["snow-ice"]["note"].startswith("game_cover_v6:")


def successor_checks(cli, directory, server, runtime, raw):
    """A vote on an incomplete receipt carries to the complete receipt of the same asset."""
    source = directory / "incomplete-source.toml"
    source.write_text((directory / "source.toml").read_text()
                      .replace('acquisition_dates = ["2020-06-15"]', "acquisition_dates = []")
                      .replace('record_id = "synthetic-ramp"', 'record_id = "synthetic-ramp-incomplete"'))
    incomplete = Path(run(cli, "import", source, directory / "surface.tif", runtime, 64 * 1024 * 1024))
    assert json.loads(incomplete.read_text())["unresolved_metadata"]
    review = crop(cli, incomplete, directory, "review-carried", 0, 400)
    evidence = directory / "carried-evidence.txt"
    evidence.write_text("Strict glacier assessment\n")
    draft = directory / "carried-decision.json"
    run(cli, "curate", "draft", runtime, incomplete, review, LEGEND, draft)
    record = json.loads(draft.read_text())
    record["claims"].update(actor={"kind": "human", "name": "Synthetic fixture owner"},
                            decision="accept", reason="Crater Yes", primary_class_id=1)
    record["evidence"] = [{"path": evidence.name, "evidence": {
        "id": "strict", "description": "Strict assessment", "source_url": None, "sha256": sha(evidence)}}]
    for finding in record["claims"]["findings"].values():
        finding.update(state="pass", note="Technical qualification", evidence_ids=["strict"])
    record["claims"]["findings"]["ground_support"].update(
        state="fail", note="The crop includes a glacier; no measured ground beneath ice.")
    record["claims"]["coverage"] = {"terrestrial_cells": 40000, "valid_terrestrial_cells": 0,
                                    "note": "Strict exclusion", "evidence_ids": ["strict"]}
    record["claims"]["permission_evidence_ids"] = ["strict"]
    draft.write_text(json.dumps(record))
    human_head = run(cli, "curate", "record", draft, runtime)
    manifest = gallery(cli, directory, "successor-gallery", [{"raw": str(incomplete), "review": str(review)}])
    tile = json.loads(manifest.read_text())["tiles"][0]
    assert tile["raw"] == str(raw.resolve()) and tile["predecessor"]["review_sha256"] == sha(review)
    process, authority, _headers = start_server(
        cli, runtime, manifest, env=server.policy("successor", worldcover=worldcover, glims=GLIMS))
    try:
        ready = eventually(authority, 0, {"ready"}, timeout=90)
        assert ready["decision"] == "accept" and current_cover(Path(ready["preparation"]["manifest"]))
    finally:
        process.terminate()
        process.wait(timeout=10)
    status = json.loads(run(cli, "curate", "status", runtime, tile["review_sha256"]))
    assert status["automation"]["kind"] == "game_cover_upgrade_v2"
    records = directory / "catalog_db.curation/records"
    carried = json.loads((records / f"{status['previous_record_sha256']}.json").read_text())
    assert carried["carried_from"]["kind"] == "metadata_successor_v1"
    assert carried["carried_from"]["record_sha256"] == human_head
    assert carried["claims"]["reason"] == "Crater Yes" and carried["claims"]["decision"] == "accept"
    ground = carried["claims"]["findings"]["ground_support"]
    assert ground["state"] == "pass" and "strict snow/ice" in ground["note"]
    assert carried["claims"]["license"]["training_permission"] == "confirmed"


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
            successor_checks(cli, directory / "work", server, runtime, raw)
        finally:
            server.close()
    print("Cover acquisition workflows passed.")


if __name__ == "__main__":
    main()
