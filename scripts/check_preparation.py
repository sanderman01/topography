#!/usr/bin/env python3
"""Exercise admitted native-grid preparation, immutable publication and recovery."""

import argparse
import copy
import json
from pathlib import Path
import shutil
import sqlite3
import struct
import tempfile

from check_acquisition import FIXTURES, ROOT, run, runtime_file, sha, values


def raster(directory, name, samples, width, height, west=700000, kind="Byte", nodata=False, spacing=1.0):
    ascii_path = directory / f"{name}.asc"
    ascii_path.write_text(f"ncols {width}\nnrows {height}\nxllcorner {west}\nyllcorner 2000000\ncellsize {spacing}\nNODATA_value -9999\n"
                          + "\n".join(" ".join(map(str, samples[i:i + width])) for i in range(0, len(samples), width)) + "\n")
    path = directory / f"{name}.tif"
    run("gdal_translate", "-q", "-ot", kind, "-a_srs", "EPSG:32619",
        *([] if nodata else ["-a_nodata", "none"]), ascii_path, path)
    return path


def check(cli, directory, spacing=1.0):
    runtime = runtime_file(directory)
    source = directory / "source.toml"
    # Exercise admission using artificial declarations in isolated storage only.
    source.write_text((FIXTURES / "source.toml").read_text().replace("synthetic = true", "synthetic = false")
                      .replace("acquisition_dates = []", 'acquisition_dates = ["2020-01-01"]')
                      .replace("native_spacing_m = [1.0, 1.0]", f"native_spacing_m = [{spacing}, {spacing}]"))
    shutil.copyfile(FIXTURES / "metadata.json", directory / "metadata.json")
    elevations = [100.25 + i for i in range(400)]
    elevations[0] = -9999
    elevations[1] = -0.0
    elevations[20] = "nan"
    asset = raster(directory, "source", elevations, 40, 10, kind="Float32", nodata=True, spacing=spacing)
    raw = Path(run(cli, "import", source, asset, runtime, 1048576))
    inputs, heads, drafts = [], [], []
    for i in range(2):
        folder = directory / f"crop-{i}"
        review = Path(run(cli, "review", raw, folder, f"{i * 20},0,20,10"))
        quality = folder / "quality.json"
        run(cli, "quality", "draft", raw, review, quality)
        q = json.loads(quality.read_text())
        for name, samples in {
            "water": ([2] + [1] * 199) if i == 1 else [1] * 200,
            "quality": [1] * 200, "repairs": [0] * 200, "features": [0] * 199 + [1],
        }.items():
            path = raster(folder, name, samples, 20, 10, west=700000 + i * 20 * spacing, spacing=spacing)
            q["masks"][name] = {"path": path.name, "sha256": sha(path)}
        quality.write_text(json.dumps(q))
        report = folder / "quality-report.json"
        run(cli, "quality", "assess", quality, report)
        draft = folder / "decision.json"
        run(cli, "curate", "draft", runtime, raw, review, ROOT / "catalog/labels/terrain-v1.toml", draft)
        d = json.loads(draft.read_text())
        d["claims"].update(actor={"kind": "human", "name": "synthetic fixture"}, decision="accept",
                           primary_class_id=i + 1, reason="Synthetic preparation workflow, not real terrain")
        d["evidence"] = [{"path": report.name, "evidence": {"id": "assessment", "sha256": sha(report),
                            "description": "Synthetic fixture only", "source_url": None}}]
        for finding in d["claims"]["findings"].values():
            finding.update(state="pass", note="Fixture", evidence_ids=["assessment"])
        counts = json.loads(report.read_text())["counts"]
        d["claims"]["coverage"] = {"terrestrial_cells": counts["terrestrial_cells"],
            "valid_terrestrial_cells": counts["valid_terrestrial_cells"], "note": "Fixture", "evidence_ids": ["assessment"]}
        d["claims"]["license"].update(training_permission="confirmed", license_name="Fixture", license_url="https://example.invalid/",
            evidence="Fixture", attribution="Fixture", edited_map_reuse="Fixture", redistribution="Fixture")
        d["claims"]["permission_evidence_ids"] = ["assessment"]
        draft.write_text(json.dumps(d))
        head = run(cli, "curate", "record", draft, runtime)
        heads.append(head)
        drafts.append((draft, d))
        inputs.append({"quality_path": str(quality), "curation_record_sha256": head})
    request = {"schema_version": 1, "inputs": inputs,
               "grid": {"origin": [700000, 2000000 + 10 * spacing], "size": [40, 10], "spacing_m": [spacing, spacing]}, "masked_fill_m": 0.0}
    path = directory / "region.json"
    path.write_text(json.dumps(request))
    before = {p: sha(p) for p in [asset, raw]}
    manifest = Path(run(cli, "prepare", "build", runtime, path))
    product = json.loads(manifest.read_text())
    assert product["kind"] == "prepared_native_region"
    assert product["resampling"] == "none"
    assert product["review_state"] == "unreviewed" and product["training_admission"] == "not admitted"
    assert [p["curation_record_sha256"] for p in product["parents"]] == heads
    assert manifest.parent.name == sha(manifest)
    expected = [0.0 if i in [0, 20] else value for i, value in enumerate(elevations)]
    assert values(manifest.parent / "elevation.tif") == expected
    assert values(manifest.parent / "validity.tif") == [0 if i in [0, 20] else 255 for i in range(400)]
    assert values(manifest.parent / "water.tif") == [255 if i == 20 else 0 for i in range(400)]
    assert values(manifest.parent / "provider-validity.tif") == [0 if i == 0 else 255 for i in range(400)]
    binary = directory / "prepared.bin"
    run("gdal_translate", "-q", "-of", "ENVI", "-co", "SUFFIX=ADD", manifest.parent / "elevation.tif", binary)
    header = binary.with_suffix(".bin.hdr").read_text()
    order = "<" if "byte order = 0" in header else ">"
    assert binary.read_bytes() == struct.pack(order + "400f", *expected)
    info = json.loads(run("gdalinfo", "-json", manifest.parent / "elevation.tif"))
    assert info["geoTransform"] == [700000, spacing, 0, 2000000 + 10 * spacing, 0, -spacing]
    assert info["size"] == [40, 10]
    assert info["coordinateSystem"] == json.loads((directory / "crop-0/inspection.json").read_text())["coordinateSystem"]
    assert info["bands"][0]["unit"] == "m" and "noDataValue" not in info["bands"][0]
    status = json.loads(run(cli, "prepare", "verify", runtime, manifest))
    assert status["registered"] and status["current"]
    if spacing != 1.0:
        return
    assert Path(run(cli, "prepare", "build", runtime, path)) == manifest

    # Geometry errors never publish more products; preserved elevations are immutable.
    for edit, message in [
        (lambda r: r["grid"].update(origin=[700000.5, 2000010]), "aligned"),
        (lambda r: r["grid"].update(size=[41, 10]), "uncovered"),
        (lambda r: r["inputs"].append(r["inputs"][0]), "overlap"),
        (lambda r: r["grid"].update(spacing_m=[0.5, 0.5]), "spacing"),
        (lambda r: r["grid"].update(size=[1000000000, 1000000000]), "4096"),
    ]:
        bad = copy.deepcopy(request)
        edit(bad)
        bad_path = directory / "bad.json"
        bad_path.write_text(json.dumps(bad))
        run(cli, "prepare", "build", runtime, bad_path, expected=1, message=message)
    assert len(list((directory / "prepared").iterdir())) == 1
    assert all(sha(p) == digest for p, digest in before.items())

    # Files are published first; identical retry re-registers an intact orphan.
    with sqlite3.connect(directory / "catalog_db") as db:
        db.execute("DELETE FROM prepared_regions")
    orphan = json.loads(run(cli, "prepare", "verify", runtime, manifest, expected=1))
    assert not orphan["registered"]
    assert Path(run(cli, "prepare", "build", runtime, path)) == manifest
    elevation = manifest.parent / "elevation.tif"
    intact = elevation.read_bytes()
    elevation.write_bytes(b"corrupt")
    run(cli, "prepare", "verify", runtime, manifest, expected=1, message="integrity")
    run(cli, "prepare", "build", runtime, path, expected=1, message="integrity")
    assert elevation.read_bytes() == b"corrupt"
    elevation.write_bytes(intact)
    run(cli, "prepare", "verify", runtime, manifest)
    # Later rejection revokes current use while preserving output and acceptance history.
    draft, d = drafts[0]
    d["previous_record_sha256"] = heads[0]
    d["claims"].update(decision="reject", reason="Synthetic superseding rejection")
    draft.write_text(json.dumps(d))
    run(cli, "curate", "record", draft, runtime)
    stale = json.loads(run(cli, "prepare", "verify", runtime, manifest, expected=1))
    assert stale["registered"] and not stale["current"]
    run(cli, "prepare", "build", runtime, path, expected=1, message="stale")
    assert elevation.read_bytes() == intact
    run(cli, "curate", "check", runtime)
    assert not (directory / "datasets").exists()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="topography preparation ") as temporary:
        directory = Path(temporary)
        fractional = directory / "fractional-spacing"
        fractional.mkdir()
        check(args.binary.resolve(), fractional, spacing=0.51)
        check(args.binary.resolve(), directory)
    print("Preparation workflows passed: exact land values, masks, alignment, registration, recovery and revocation.")


if __name__ == "__main__":
    main()
