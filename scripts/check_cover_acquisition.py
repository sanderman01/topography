#!/usr/bin/env python3
"""Exercise public cover drafting offline against a loopback HTTPS fixture server."""

import argparse
import functools
import http.server
import json
import math
import os
from pathlib import Path
import shutil
import ssl
import subprocess
import tempfile
import threading
import tomllib

from check_acquisition import FIXTURES, ROOT, run, runtime_file, sha, values
from check_automatic_preparation import eventually, start_server, vote
from check_surface_cover import gallery

X0, TOP, SIZE = 700000, 2000600, 600
REVIEW = "200,200,200,200"


class StaticHandler(http.server.SimpleHTTPRequestHandler):
    """Static files with byte ranges for GDAL; query strings are ignored."""

    def log_message(self, *_args):
        pass

    def send_head(self):
        path = Path(self.translate_path(self.path.split("?", 1)[0]))
        if not path.is_file():
            self.send_error(404)
            return None
        data = path.read_bytes()
        start, end = 0, len(data) - 1
        ranged = self.headers.get("Range", "").startswith("bytes=")
        if ranged:
            first, _, last = self.headers["Range"].removeprefix("bytes=").partition("-")
            start = int(first)
            end = min(int(last), len(data) - 1) if last else len(data) - 1
        self.send_response(206 if ranged else 200)
        self.send_header("Content-Length", str(end - start + 1))
        self.send_header("Accept-Ranges", "bytes")
        if ranged:
            self.send_header("Content-Range", f"bytes {start}-{end}/{len(data)}")
        self.end_headers()
        self._body = data[start:end + 1]
        return self

    def do_GET(self):
        if self.send_head():
            self.wfile.write(self._body)

    def do_HEAD(self):
        self.send_head()


def serve(root, directory):
    cert, key = directory / "cert.pem", directory / "key.pem"
    run("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1", "-keyout", key,
        "-out", cert, "-subj", "/CN=localhost", "-addext", "subjectAltName=DNS:localhost")
    server = http.server.ThreadingHTTPServer(
        ("127.0.0.1", 0), functools.partial(StaticHandler, directory=str(root)))
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(cert, key)
    server.socket = context.wrap_socket(server.socket, server_side=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    env = dict(os.environ, CURL_CA_BUNDLE=str(cert), NO_PROXY="localhost,127.0.0.1",
               no_proxy="localhost,127.0.0.1")
    return server, f"https://localhost:{server.server_port}", env


def grid(path, width, height, left, top, res, value, *, srs="EPSG:32619"):
    """Byte raster whose cell value is value(x_centre, y_centre)."""
    asc = path.with_suffix(".asc")
    rows = [" ".join(str(value(left + (c + 0.5) * res, top - (r + 0.5) * res)) for c in range(width))
            for r in range(height)]
    asc.write_text(f"ncols {width}\nnrows {height}\nxllcorner {left}\nyllcorner {top - height * res}\n"
                   f"cellsize {res}\n" + "\n".join(rows) + "\n")
    run("gdal_translate", "-q", "-ot", "Byte", "-a_srs", srs, "-a_nodata", "none", asc, path)
    asc.unlink()
    return path


def lonlat(points):
    result = subprocess.run(["gdaltransform", "-s_srs", "EPSG:32619", "-t_srs", "EPSG:4326",
                             "-output_xy"], input="\n".join(f"{x} {y}" for x, y in points),
                            capture_output=True, text=True, check=True)
    return [[float(v) for v in line.split()[:2]] for line in result.stdout.splitlines()]


def outline(glac_id, anlys_id, date, box, line_type="glac_bound"):
    x0, y0, x1, y1 = box
    ring = lonlat([(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)])
    return {"type": "Feature", "id": f"outline.{anlys_id}",
            "geometry": {"type": "Polygon", "coordinates": [ring]},
            "properties": {"line_type": line_type, "anlys_id": anlys_id, "glac_id": glac_id,
                           "rec_status": "okay", "src_date": f"{date}T00:00:00Z"}}


def tile_name():
    (lon, lat), = lonlat([(X0 + SIZE / 2, TOP - SIZE / 2)])
    la, lo = math.floor(lat / 3) * 3, math.floor(lon / 3) * 3
    return f"{'S' if la < 0 else 'N'}{abs(la):02}{'W' if lo < 0 else 'E'}{abs(lo):03}"


def scenario(root, base, name, *, worldcover=None, glims=None, scenes=()):
    """Write fixtures for one scenario and return its policy file."""
    folder = root / name
    folder.mkdir(parents=True)
    if worldcover:
        grid(folder / f"wc-{tile_name()}.tif", 80, 80, X0 - 100, TOP + 100, 10, worldcover)
    if glims is not None:
        (folder / "ows").write_text(json.dumps({"type": "FeatureCollection", "features": glims,
                                                "numberMatched": len(glims)}))
    features = []
    for index, (date, value) in enumerate(scenes, 1):
        grid(folder / f"scl-{index}.tif", 40, 40, X0 - 100, TOP + 100, 20, value)
        features.append({"type": "Feature", "id": f"S2X_{name}_{index}",
                         "properties": {"datetime": f"{date}T10:00:00Z"},
                         "assets": {"scl": {"href": f"{base}/{name}/scl-{index}.tif"}}})
    (folder / "search").write_text(json.dumps({"type": "FeatureCollection", "features": features,
                                                "numberMatched": len(features)}))
    policy = (ROOT / "configs/cover-sources-v1.toml").read_text()
    config = tomllib.loads(policy)
    for old, new in [(config["worldcover"]["url_template"], f"{base}/{name}/wc-{{tile}}.tif"),
                     (config["glims"]["endpoint"], f"{base}/{name}/ows"),
                     (config["sentinel2"]["stac_endpoint"], f"{base}/{name}")]:
        policy = policy.replace(f'"{old}"', f'"{new}"')
    path = root.parent / f"{name}-policy.toml"
    path.write_text(policy)
    return path


def fixture(cli, directory, dates):
    directory.mkdir(parents=True)
    runtime = runtime_file(directory)
    source = directory / "source.toml"
    source.write_text((FIXTURES / "source.toml").read_text()
                      .replace("synthetic = true", "synthetic = false")
                      .replace("acquisition_dates = []", f"acquisition_dates = {json.dumps(dates)}")
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
    review = Path(run(cli, "review", raw, directory / "review", REVIEW))
    return runtime, raw, review


def cells(path):
    """Complete Byte raster as row-major bytes."""
    with tempfile.TemporaryDirectory() as scratch:
        out = Path(scratch) / "values.bin"
        run("gdal_translate", "-q", "-of", "ENVI", path, out)
        return out.read_bytes()


def draft(cli, env, raw, review, policy, output, *extra):
    report = run(cli, "cover", "draft", raw, review, policy, output, *extra, env=env)
    cover = json.loads((output / "cover.json").read_text())
    for item in cover["evidence"]:
        assert sha(output / item["path"]) == item["sha256"], item
        assert item["source_url"] is None or item["source_url"].startswith("https://")
    return report, cover


def same_grid(raw, mask):
    receipt = json.loads(raw.read_text())
    source = json.loads((raw.parent.parent / "objects" / receipt["inspection_sha256"]).read_text())
    info = json.loads(run("gdalinfo", "-json", mask))
    for key in ["size", "geoTransform", "coordinateSystem"]:
        assert info[key] == source[key], key
    assert len(info["bands"]) == 1 and info["bands"][0]["type"] == "Byte"
    assert "noDataValue" not in info["bands"][0]


def at(data, col, row):
    return data[row * SIZE + col]


def window(data):
    return [at(data, c, r) for r in range(200, 400) for c in range(200, 400)]


def x(col):
    return X0 + col + 0.5


def y(row):
    return TOP - row - 0.5


def profile_v1(path, raw, evidence):
    receipt = json.loads(raw.read_text())
    finding = {"note": "Synthetic supplier assessment", "evidence_ids": ["supplier"]}
    path.write_text(json.dumps({
        "schema_version": 1, "policy": "supplier_qa_pilot_v1",
        "source_sha256": receipt["source_sha256"], "asset_sha256": receipt["asset_sha256"],
        "evidence": [{"id": "supplier", "path": evidence.name, "sha256": sha(evidence),
                      "description": "Synthetic supplier assessment",
                      "source_url": "https://example.invalid/source"}],
        "permission_evidence_ids": ["supplier"],
        "findings": {name: dict(finding) for name in ["coordinates", "ground_support",
                     "artifacts_and_repairs", "feature_fidelity", "water_classification",
                     "unsupported_feature_gaps"]},
        "water": {"kind": "all_terrestrial", "windows": [[0, 0, SIZE, SIZE]]}}))
    return path


def check(cli, directory):
    fixtures = directory / "server" / "root"
    fixtures.mkdir(parents=True)
    server, base, env = serve(fixtures, directory / "server")
    try:
        clear = [("2020-06-20", lambda _x, _y: 4)]
        mixed = lambda px, _py: 10 if px < 700300 else 60
        absent = scenario(fixtures, base, "absent", worldcover=mixed, glims=[], scenes=clear)
        runtime, raw, review = fixture(cli, directory / "dated", ["2020-06-15"])

        # All-absent snow/ice, mixed vegetation, exact source grid, then v2 preparation.
        out = directory / "dated" / "cover-absent"
        report, cover = draft(cli, env, raw, review, absent, out)
        assert cover["snow_ice"]["kind"] == "all_absent", cover["sources"]
        assert cover["snow_ice"]["windows"] == [[200, 200, 200, 200]]
        assert cover["snow_ice"]["observed_at"] == "2020-06-15"
        assert cover["acquisition"]["origin"] == "source_metadata"
        assert cover["vegetation"]["kind"] == "mask"
        assert "Snow/ice: all_absent" in report and "UNRESOLVED" not in report
        for name in ["snow-ice.tif", "vegetation.tif"]:
            same_grid(raw, out / name)
        snow, veg = cells(out / "snow-ice.tif"), cells(out / "vegetation.tif")
        assert set(window(snow)) == {1}
        assert at(snow, 0, 0) == 0 and at(snow, 599, 599) == 0 and at(snow, 100, 100) == 1
        assert at(veg, 299, 250) == 2 and at(veg, 300, 250) == 1 and at(veg, 50, 50) == 0
        assert {e["id"] for e in cover["evidence"]} == {
            "cover-policy", "cover-worldcover", "cover-glims", "cover-sentinel2-stac", "cover-sentinel2-scl"}
        prepared = stage_v2(cli, directory / "dated", runtime, raw, review, out)
        assert values(prepared / "snow-ice.tif") == [1] * 40000
        assert values(prepared / "vegetation.tif") == window([float(v) for v in veg])

        # Present classes with boundary bands, current and stale GLIMS outlines.
        glims = [outline("G1", 1, "2019-08-01", (700100, 2000150, 700250, 2000300)),
                 outline("G1", 2, "1650-01-01", (700150, 2000150, 700290, 2000420)),
                 outline("G2", 3, "2000-07-01", (700300, 2000150, 700450, 2000280)),
                 outline("G3", 4, "2019-08-01", (700000, 2000000, 700050, 2000050), "intrnl_rock")]
        present = scenario(fixtures, base, "present",
                           worldcover=lambda px, py: 70 if px >= 700300 and py >= 2000350 else 30,
                           glims=glims, scenes=clear)
        out = directory / "dated" / "cover-present"
        report, cover = draft(cli, env, raw, review, present, out)
        assert cover["snow_ice"]["kind"] == "mask" and "UNRESOLVED" in report
        snow = cells(out / "snow-ice.tif")
        assert sha(out / "snow-ice.tif") == cover["snow_ice"]["sha256"]
        assert at(snow, 350, 210) == 2   # WorldCover 70 interior
        assert at(snow, 305, 220) == 0   # within 20 m of the WorldCover edge
        assert at(snow, 200, 375) == 2   # current GLIMS interior
        assert at(snow, 240, 375) == 0   # within 30 m of the outline
        assert at(snow, 375, 385) == 0   # stale outline
        assert at(snow, 375, 300) == 0   # stale outline band
        assert at(snow, 220, 220) == 1   # reconstruction ignored
        details = cover["sources"]["glims"]["details"]
        assert details["reconstructions_excluded"] == 1 and details["other_line_types_ignored"] == 1
        assert {g["glac_id"]: g["use"] for g in details["glaciers"]}["G1"] == "present"

        # Cloud cover stays unknown unless another scene observes the cell.
        def first(px, py):
            return 9 if py >= 2000300 and px < 700300 else 4

        def second(px, py):
            if py >= 2000300 and px < 700260:
                return 9
            return 11 if px >= 700300 and py <= 2000300 else 4
        cloud = scenario(fixtures, base, "cloud", worldcover=lambda *_: 30, glims=[],
                         scenes=[("2020-06-10", first), ("2020-06-25", second)])
        out = directory / "dated" / "cover-cloud"
        _report, cover = draft(cli, env, raw, review, cloud, out)
        snow = cells(out / "snow-ice.tif")
        assert at(snow, 230, 250) == 0   # cloud in both scenes
        assert at(snow, 280, 220) == 1   # cloud in one, clear in the other
        assert at(snow, 360, 360) == 2   # seasonal snow interior
        assert at(snow, 320, 320) == 0   # seasonal snow band
        assert len(cover["sources"]["sentinel2"]["details"]["scenes"]) == 2
        bands = json.loads(run("gdalinfo", "-json", out / "cover-sentinel2-scl.tif"))["bands"]
        assert len(bands) == 2

        # No DEM date: seasonal snow cannot be assessed, so snow/ice stays unknown.
        runtime_n, raw_n, review_n = fixture(cli, directory / "undated", [])
        out = directory / "undated" / "cover"
        report, cover = draft(cli, env, raw_n, review_n, absent, out)
        assert cover["snow_ice"]["kind"] == "mask" and cover["snow_ice"]["observed_at"] is None
        assert cover["acquisition"]["origin"] == "none"
        assert cover["sources"]["sentinel2"]["status"] == "skipped"
        assert "cannot be absent" in report and "UNRESOLVED" in report
        assert set(window(cells(out / "snow-ice.tif"))) == {0}
        out = directory / "undated" / "cover-flag"
        report, cover = draft(cli, env, raw_n, review_n, absent, out, "--acquired", "2020-06-15")
        assert cover["acquisition"]["origin"] == "command_flag"
        assert cover["snow_ice"]["kind"] == "all_absent"

        # Source failure: no WorldCover or GLIMS response.
        failed = scenario(fixtures, base, "failed", scenes=clear)
        out = directory / "dated" / "cover-failed"
        report, cover = draft(cli, env, raw, review, failed, out)
        assert cover["vegetation"]["kind"] == "unavailable", cover["vegetation"]
        assert cover["snow_ice"]["kind"] == "mask"
        assert cover["sources"]["worldcover"]["status"] == "failed"
        assert cover["sources"]["glims"]["status"] == "failed"
        assert set(window(cells(out / "snow-ice.tif"))) == {0}
        assert "WorldCover unavailable" in report

        # Refusals.
        run(cli, "cover", "draft", raw, review, absent, out, env=env, expected=1, message="already exists")
        run(cli, "cover", "draft", raw, review, absent, directory / "bad-date",
            "--acquired", "2020-02-30", env=env, expected=1, message="--acquired")
        bad = directory / "bad-policy.toml"
        bad.write_text(absent.read_text().replace('status = "draft"', 'status = "accepted"'))
        run(cli, "cover", "draft", raw, review, bad, directory / "bad-policy",
            env=env, expected=1, message="status draft")
    finally:
        server.shutdown()
        server.server_close()


def stage_v2(cli, directory, runtime, raw, review, cover_dir):
    evidence = directory / "supplier.txt"
    evidence.write_text("Synthetic supplier assessment\n")
    legacy = profile_v1(directory / "profile-v1.json", raw, evidence)
    merged = Path(run(cli, "cover", "merge", legacy, cover_dir / "cover.json", directory / "profile-v2"))
    profile = json.loads(merged.read_text())
    assert profile["schema_version"] == 2 and profile["snow_ice"]["kind"] == "all_absent"
    assert profile["vegetation"]["path"] == "cover/vegetation.tif"
    run(cli, "cover", "merge", legacy, cover_dir / "cover.json", directory / "profile-v2",
        expected=1, message="already exists")
    manifest = gallery(cli, directory, raw, review, merged, "cover-gallery")
    process, authority, headers = start_server(cli, runtime, manifest)
    try:
        pending = eventually(authority, 0, {"awaiting_acceptance"}, timeout=60)
        vote(authority, headers, sha(manifest), 0, pending, "accept")
        ready = eventually(authority, 0, {"ready"}, timeout=60)
        product = Path(ready["preparation"]["manifest"])
        assert json.loads(product.read_text())["schema_version"] == 2
        return product.parent
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
    with tempfile.TemporaryDirectory(prefix="topography cover acquisition ") as temporary:
        check(args.binary.resolve(), Path(temporary))
    print("Cover acquisition workflows passed.")


if __name__ == "__main__":
    main()
