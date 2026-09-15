#!/usr/bin/env python3
"""Exercise raw import, native-grid review and resumable HTTPS through the CLI."""

import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import shutil
import ssl
import subprocess
import tempfile
import threading
import tomllib

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests/fixtures/acquisition"
CLI = None
CHUNK = 8 * 1024 * 1024


def run(*args, expected=0, message=None, env=None):
    result = subprocess.run([str(a) for a in args], capture_output=True, text=True, env=env, check=False)
    assert result.returncode == expected, (args, result.returncode, result.stdout, result.stderr)
    if message:
        assert message in result.stderr, result.stderr
    return result.stdout.strip()


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def runtime_file(directory):
    path = directory / "runtime.toml"
    lines = ['schema_version = 1', 'status = "draft"', '[hardware]', 'gpu_vram_budget_gb = 16', '[paths]',
             f'base_dir = {json.dumps(str(directory))}']
    for name in ["catalog_db", "raw", "prepared", "review", "datasets", "models", "worlds", "exports", "cache", "staging"]:
        lines.append(f'{name} = "{name}"')
    path.write_text("\n".join(lines) + "\n")
    return path


def values(path):
    return [float(line.split()[2]) for line in run("gdal_translate", "-q", "-of", "XYZ", path, "/vsistdout/").splitlines()]


def local_checks(directory, runtime, source, asset):
    original_sha = sha(asset)
    receipt_path = Path(run(CLI, "import", source, asset, runtime, 1048576))
    receipt = json.loads(receipt_path.read_text())
    assert receipt["asset_sha256"] == original_sha
    assert receipt["review_state"] == "unreviewed"
    assert receipt["training_permission_recorded"] is False
    assert receipt["synthetic"] is True
    objects = directory / "raw/objects"
    assert (objects / receipt["source_sha256"]).read_bytes() == source.read_bytes()
    assert (objects / receipt["original_metadata_sha256"]).read_bytes() == (directory / "metadata.json").read_bytes()
    assert Path(run(CLI, "import", source, asset, runtime, 1048576)) == receipt_path
    run(CLI, "verify", receipt_path)
    assert len(list((directory / "raw/records").glob("*.json"))) == 1

    review = directory / "review/crop with spaces"
    manifest = Path(run(CLI, "review", receipt_path, review, "1,0,2,3"))
    data = json.loads(manifest.read_text())
    assert data["window_cells"] == [1, 0, 2, 3]
    assert data["review_state"] == "unreviewed"
    assert values(review / "elevation.tif") == [101.25, 102.25, -9999, 112.25, 121.25, 122.25]
    assert values(review / "validity.tif") == [255, 255, 0, 255, 255, 255]
    info = json.loads((review / "inspection.json").read_text())
    assert info["geoTransform"] == [700001, 1, 0, 2000004, 0, -1]
    assert info["size"] == [2, 3]
    before = sha(manifest)
    run(CLI, "review", receipt_path, review, "1,0,2,3", expected=1, message="already exists")
    assert sha(manifest) == before
    run(CLI, "review", receipt_path, directory / "outside", "3,3,2,2", expected=1, message="outside")
    run(CLI, "import", source, asset, runtime, 1, expected=1, message="max-bytes")
    bad_source = directory / "bad-sha.toml"
    bad_source.write_text('asset_sha256 = "' + "f" * 64 + '"\n' + source.read_text())
    run(CLI, "import", bad_source, asset, runtime, 1048576, expected=1, message="SHA-256 mismatch")
    byte_asset = directory / "byte.tif"
    run("gdal_translate", "-q", "-ot", "Byte", asset, byte_asset)
    run(CLI, "import", source, byte_asset, runtime, 1048576, expected=1, message="Float32")
    assert len(list((directory / "raw/records").glob("*.json"))) == 1
    assert sha(asset) == original_sha
    assert not (directory / "prepared").exists()
    assert not (directory / "datasets").exists()

    # A crash after object publication but before receipt publication is recoverable.
    saved_receipt = receipt_path.read_bytes()
    receipt_path.unlink()
    assert Path(run(CLI, "import", source, asset, runtime, 1048576)) == receipt_path
    assert receipt_path.read_bytes() == saved_receipt
    raw_asset = objects / receipt["asset_sha256"]
    original_bytes = raw_asset.read_bytes()
    raw_asset.write_bytes(b"corrupt")
    run(CLI, "verify", receipt_path, expected=1, message="integrity mismatch")
    run(CLI, "import", source, asset, runtime, 1048576, expected=1, message="corrupt")
    assert raw_asset.read_bytes() == b"corrupt"
    raw_asset.write_bytes(original_bytes)
    run(CLI, "verify", receipt_path)

    discovery = directory / "discovery"
    response = FIXTURES / "usgs-products.json"
    result = run(CLI, "discover", "usgs", "-67,18,-66,19", 1, discovery, response)
    assert "truncated" in result
    assert (discovery / "response.json").read_bytes() == response.read_bytes()
    summary = json.loads((discovery / "discovery.json").read_text())
    assert summary["discovery_mode"] == "saved_response"
    candidate = discovery / summary["source_files"][0]
    record = tomllib.loads(candidate.read_text())
    assert record["acquisition_dates"] == []
    assert record["license"]["training_permission"] == "unknown"
    run(CLI, "validate", "source", candidate)
    run(CLI, "discover", "usgs", "-67,18,-66,19", 1, discovery, response, expected=1, message="already exists")


class RangeHandler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_HEAD(self):
        payload = self.server.control["payload"]
        self.send_response(200)
        self.send_header("Content-Length", str(len(payload)))
        if self.path != "/no-etag.tif":
            self.send_header("ETag", '"' + hashlib.sha256(payload).hexdigest() + '"')
        self.end_headers()

    def do_GET(self):
        control = self.server.control
        payload = control["payload"]
        etag = '"' + hashlib.sha256(payload).hexdigest() + '"'
        start, end = map(int, self.headers["Range"].removeprefix("bytes=").split("-"))
        control["requests"].append((self.path, start))
        if self.path == "/changed.tif" or self.headers.get("If-Match") != etag:
            self.send_error(412)
            return
        part = payload[start:end + 1]
        self.send_response(206)
        self.send_header("Content-Length", str(len(part)))
        self.send_header("ETag", etag)
        claimed_start = start + 1 if self.path == "/bad-range.tif" else start
        self.send_header("Content-Range", f"bytes {claimed_start}-{end}/{len(payload)}")
        self.end_headers()
        if self.path == "/flaky.tif" and start > 0 and control["interrupt"]:
            self.wfile.write(part[:len(part) // 2])
            self.wfile.flush()
            self.close_connection = True
        else:
            self.wfile.write(part)


def https_checks(directory, runtime, source, asset):
    cert, key = directory / "cert.pem", directory / "key.pem"
    run("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1", "-keyout", key,
        "-out", cert, "-subj", "/CN=localhost", "-addext", "subjectAltName=DNS:localhost")
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), RangeHandler)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(cert, key)
    server.socket = context.wrap_socket(server.socket, server_side=True)
    base = f"https://localhost:{server.server_port}"
    original = asset.read_bytes()
    payload = original + bytes(CHUNK + 64 - len(original))
    server.control = {"payload": payload, "requests": [], "interrupt": True}
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    env = dict(os.environ, CURL_CA_BUNDLE=str(cert), NO_PROXY="localhost,127.0.0.1", no_proxy="localhost,127.0.0.1")

    def candidate(route):
        path = directory / (route + ".toml")
        path.write_text(source.read_text().replace("https://example.invalid/ramp.tif", base + "/" + route + ".tif"))
        return path

    def acquire(route, **kwargs):
        return run(CLI, "acquire", candidate(route), runtime, 2 * CHUNK, env=env, **kwargs)

    try:
        before = len(list((directory / "raw/records").glob("*.json")))
        acquire("flaky", expected=1, message="curl failed")
        assert len(list((directory / "raw/records").glob("*.json"))) == before
        first = next((directory / "staging/http").glob("*/0.chunk"))
        completed = first.read_bytes()
        first.write_bytes(b"x" + completed[1:])
        acquire("flaky", expected=1, message="corrupt staged chunk")
        first.write_bytes(completed)
        server.control["interrupt"] = False
        receipt_path = Path(acquire("flaky"))
        receipt = json.loads(receipt_path.read_text())
        assert receipt["asset_sha256"] == hashlib.sha256(payload).hexdigest()
        assert server.control["requests"].count(("/flaky.tif", 0)) == 1
        assert Path(acquire("flaky")) == receipt_path
        run(CLI, "verify", receipt_path)

        # A changed HEAD identity starts a separate staging set and never mixes chunks.
        server.control["payload"] = payload[:-1] + b"x"
        changed = Path(acquire("flaky"))
        assert changed != receipt_path
        assert json.loads(changed.read_text())["asset_sha256"] == hashlib.sha256(server.control["payload"]).hexdigest()
        assert server.control["requests"].count(("/flaky.tif", 0)) == 2
        assert first.read_bytes() == completed
        acquire("changed", expected=1, message="curl failed")
        assert server.control["requests"].count(("/changed.tif", 0)) == 1
        acquire("bad-range", expected=1, message="did not exactly match")
        acquire("no-etag", expected=1, message="strong HTTP ETag")
        run(CLI, "acquire", candidate("too-large"), runtime, 1, expected=1, message="exceeds limit", env=env)
        assert not any(route == "/too-large.tif" for route, _ in server.control["requests"])
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def main():
    global CLI
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--local-only", action="store_true")
    args = parser.parse_args()
    CLI = args.binary.resolve()
    executables = ["gdal_translate", "gdalinfo", "gdaldem"]
    if not args.local_only:
        executables += ["curl", "openssl"]
    for executable in executables:
        if not shutil.which(executable):
            raise SystemExit(f"Required for acquisition checks: {executable}")
    with tempfile.TemporaryDirectory(prefix="topography acquisition ") as temporary:
        directory = Path(temporary)
        runtime = runtime_file(directory)
        source = directory / "source.toml"
        shutil.copyfile(FIXTURES / "source.toml", source)
        shutil.copyfile(FIXTURES / "metadata.json", directory / "metadata.json")
        asset = directory / "ramp.tif"
        run("gdal_translate", "-q", "-of", "GTiff", "-ot", "Float32", "-a_srs", "EPSG:32619", FIXTURES / "ramp.asc", asset)
        local_checks(directory, runtime, source, asset)
        if not args.local_only:
            https_checks(directory, runtime, source, asset)
    print("Acquisition workflows passed: local import, native crop/mask, integrity and recovery"
          + ("." if args.local_only else ", HTTPS resume and changed-source rejection."))


if __name__ == "__main__":
    main()
