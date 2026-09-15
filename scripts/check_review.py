#!/usr/bin/env python3
"""Compare native review crops and browser samples against their verified raw source."""

import argparse
import base64
import hashlib
import json
import math
import os
from pathlib import Path
import struct
import subprocess
import tempfile


def run(*args):
    env = dict(os.environ, GDAL_CACHEMAX="64", GDAL_NUM_THREADS="1", GDAL_PAM_ENABLED="NO",
               GDAL_DISABLE_READDIR_ON_OPEN="TRUE", GDAL_GEOREF_SOURCES="INTERNAL")
    return subprocess.run([str(a) for a in args], env=env, capture_output=True,
                          text=True, check=True).stdout.strip()


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def samples(path, temporary, name, window=None, mask=False):
    output = temporary / (name + ".bin")
    args = ["gdal_translate", "-q", "-of", "ENVI", "-ot", "Byte" if mask else "Float32",
            "-co", "SUFFIX=ADD"]
    if window:
        args += ["-srcwin", *map(str, window)]
    if mask:
        args += ["-b", "mask"]
    run(*args, path, output)
    header = dict(tuple(part.strip() for part in line.split("=", 1))
                  for line in output.with_suffix(".bin.hdr").read_text().splitlines() if "=" in line)
    # ENVI padding is outside the sample payload; the source/crop byte comparison is exact.
    size = int(header["samples"]) * int(header["lines"].strip()) * (1 if mask else 4)
    data = output.read_bytes()[:size]
    assert len(data) == size and header["bands"].strip() == "1"
    if not mask and header["byte order"].strip() == "1":
        data = b"".join(data[i:i + 4][::-1] for i in range(0, len(data), 4))
    return data


def check(binary, receipt_path, manifest_path):
    run(binary, "verify", receipt_path)
    receipt = json.loads(receipt_path.read_text())
    asset = receipt_path.parent.parent / "objects" / receipt["asset_sha256"]
    manifest = json.loads(manifest_path.read_text())
    assert manifest["schema_version"] == 1 and manifest["kind"] == "native_grid_review"
    assert manifest["raw_receipt_sha256"] == digest(receipt_path)
    assert manifest["asset_sha256"] == receipt["asset_sha256"]
    window = manifest["window_cells"]
    assert len(window) == 4 and all(isinstance(v, int) and v >= 0 for v in window)
    assert 1 <= window[2] <= 2048 and 1 <= window[3] <= 2048
    directory = manifest_path.parent
    for artifact in manifest["artifacts"]:
        path = directory / artifact["path"]
        assert path.parent == directory and digest(path) == artifact["sha256"]
        assert path.stat().st_size == artifact["bytes"]
    with tempfile.TemporaryDirectory(prefix="topography review verification ") as name:
        temporary = Path(name)
        original = samples(asset, temporary, "raw", window)
        crop = samples(directory / "elevation.tif", temporary, "crop")
        original_mask = samples(asset, temporary, "raw-mask", window, mask=True)
        crop_mask = samples(directory / "elevation.tif", temporary, "crop-mask", mask=True)
        assert crop == original, "decoded Float32 bits differ from source window"
        assert crop_mask == original_mask, "provider mask differs from source window"
        # The separately published mask must carry the same validity as the elevation.
        explicit = temporary / "validity.bin"
        run("gdal_translate", "-q", "-of", "ENVI", "-ot", "Byte", directory / "validity.tif", explicit)
        assert explicit.read_bytes()[:len(crop_mask)] == crop_mask
    info = json.loads(run("gdalinfo", "-json", asset))
    crop_info = json.loads(run("gdalinfo", "-json", directory / "elevation.tif"))
    gt = info["geoTransform"]
    expected = [gt[0] + window[0] * gt[1] + window[1] * gt[2], gt[1], gt[2],
                gt[3] + window[0] * gt[4] + window[1] * gt[5], gt[4], gt[5]]
    assert crop_info["size"] == window[2:]
    assert all(math.isclose(a, b, rel_tol=0, abs_tol=1e-9) for a, b in zip(crop_info["geoTransform"], expected))
    assert crop_info["coordinateSystem"] == info["coordinateSystem"]
    html = (directory / "preview.html").read_text()
    payload = json.loads(html.split('<script id="terrain-data" type="application/json">', 1)[1].split('</script>', 1)[0])
    assert base64.b64decode(payload["elevation_le_f32"], validate=True) == crop
    assert base64.b64decode(payload["mask"], validate=True) == crop_mask
    metrics = json.loads((directory / "metrics.json").read_text())
    assert metrics == payload["metrics"]
    finite = [v[0] for v, valid in zip(struct.iter_unpack('<f', crop), crop_mask) if valid and math.isfinite(v[0])]
    assert metrics["finite_valid_cells"] == len(finite)
    assert metrics["minimum_native"] == (min(finite) if finite else None)
    assert metrics["maximum_native"] == (max(finite) if finite else None)
    return {"review": str(manifest_path), "review_sha256": digest(manifest_path), "window_cells": window,
            "decoded_float32_bits_equal": True, "provider_masks_equal": True, "browser_samples_equal": True,
            "coordinate_metadata_equal": True, "metrics": metrics}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("receipt", type=Path)
    parser.add_argument("reviews", type=Path, nargs="+")
    parser.add_argument("--output", type=Path, help="Write a new comparison report; never replace an existing report")
    args = parser.parse_args()
    report = {"schema_version": 1, "raw_receipt_sha256": digest(args.receipt),
              "gdal_version": run("gdalinfo", "--version"),
              "checks": [check(args.binary.resolve(), args.receipt, review) for review in args.reviews],
              "scope": "Native extraction/browser decoding only; no model precision, ground support or acceptance claim."}
    text = json.dumps(report, indent=2) + "\n"
    if args.output:
        with args.output.open("x") as output:
            output.write(text)
        print(args.output)
    else:
        print(text, end="")


if __name__ == "__main__":
    main()
