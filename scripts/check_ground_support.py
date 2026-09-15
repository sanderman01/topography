#!/usr/bin/env python3
"""Measure classified ground returns in a 64 m crop-centre sample using PDAL.

Developer probe, not source admission. Requires a reviewed matching COPC survey,
explicit projected metre CRSs and readiness for point decompression. No Z comparison.
"""

import argparse
from collections import Counter
import csv
import hashlib
import json
import math
import os
from pathlib import Path
import resource
import subprocess
import sys
import tempfile
import time


def run(*args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, capture_output=True,
                          text=True, timeout=900, **kwargs).stdout


def sha(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(65536), b""):
            h.update(block)
    return h.hexdigest()


def save(path, value):
    with path.open("x") as stream:
        json.dump(value, stream, indent=2)


def pipeline(directory, name, stages, reuse=False):
    path = directory / f"{name}-pipeline.json"
    if reuse:
        assert json.loads(path.read_text()) == stages, "retained pipeline differs from requested sample"
        assert (directory / f"{name}-metadata.json").is_file()
        return None
    save(path, stages)
    started = time.monotonic()
    result = subprocess.run(["pdal", "pipeline", str(path), "--stream", "--metadata",
                             str(directory / f"{name}-metadata.json")],
                            cwd=directory, capture_output=True, text=True, timeout=900, check=False)
    (directory / f"{name}.log").write_text(result.stdout + result.stderr)
    result.check_returncode()
    return time.monotonic() - started


def inspect_counts(directory, points, west, south):
    """Compare every PDAL 0.5 m bin with independently counted retained XY points."""
    expected = [0] * (128 * 128)
    classes = Counter()
    retained = 0
    total = 0
    with points.open() as stream:
        for row in csv.DictReader(stream):
            total += 1
            assert total <= 1_000_000, "sample exceeded point limit; incomplete evidence"
            cls = int(float(row["Classification"]))
            classes[cls] += 1
            x, y = float(row["X"]), float(row["Y"])
            if cls == 2 and int(float(row["Withheld"])) == 0 and int(float(row["Synthetic"])) == 0:
                col, bottom_row = math.floor((x-west)*2), math.floor((y-south)*2)
                if 0 <= col < 128 and 0 <= bottom_row < 128:
                    expected[(127-bottom_row)*128+col] += 1
                    retained += 1
    # PDAL's count band uses nodata for empty bins; these mean zero direct returns.
    decoded = [float(line.split()[2]) for line in run(
        "gdal_translate", "-q", "-of", "XYZ", directory / "ground-counts.tif", "/vsistdout/").splitlines()]
    assert all(math.isfinite(n) and n.is_integer() and n >= -1 for n in decoded), "invalid count band"
    actual = [max(0, int(n)) for n in decoded]
    assert actual == expected, "PDAL count bins differ from retained XY samples"
    info = json.loads(run("gdalinfo", "-json", directory / "ground-counts.tif"))
    assert info["size"] == [128, 128]
    assert all(abs(a-b) <= 1e-8 for a, b in zip(info["geoTransform"], [west, 0.5, 0, south+64, 0, -0.5]))
    assert total > 0, "empty point sample; check source coverage and coordinate metadata"
    support = []
    for factor in [1, 2, 4, 8]:
        bins = [sum(expected[(r+dr)*128+c+dc] for dr in range(factor) for dc in range(factor))
                for r in range(0, 128, factor) for c in range(0, 128, factor)]
        support.append({"cell_spacing_m": factor/2, "cells": len(bins),
                        "cells_with_direct_ground_returns": sum(n > 0 for n in bins),
                        "empty_cells": bins.count(0), "maximum_returns": max(bins)})
    return {"buffered_points": total, "buffered_classification_counts": dict(sorted(classes.items())),
            "ground_returns_in_sample": retained, "ground_returns_per_m2": retained/4096,
            "direct_support": support, "count_bins_verified": True}


def synthetic_check():
    with tempfile.TemporaryDirectory(prefix="topography ground support ") as temporary:
        directory = Path(temporary)
        points = directory / "points.csv"
        points.write_text("X,Y,Classification,Withheld,Overlap,Synthetic\n"
                          "0.1,0.1,2,0,0,0\n0.2,0.2,2,0,1,0\n0.6,0.2,2,1,0,0\n"
                          "0.7,0.3,2,0,0,1\n1.2,1.2,1,0,0,0\n63.9,63.9,2,0,0,0\n")
        stages = [{"type": "readers.text", "filename": "points.csv", "override_srs": "EPSG:6566"},
                  {"type": "filters.expression", "expression": "Classification == 2 && Withheld == 0 && Synthetic == 0"},
                  {"type": "writers.gdal", "filename": "ground-counts.tif", "dimension": "X", "output_type": "count",
                   "resolution": 0.5, "origin_x": 0, "origin_y": 0, "width": 128, "height": 128,
                   "binmode": True, "window_size": 0, "nodata": -1, "data_type": "double", "allow_empty": True}]
        pipeline(directory, "counts", stages)
        metrics = inspect_counts(directory, points, 0, 0)
        assert metrics["ground_returns_in_sample"] == 3
        assert metrics["direct_support"][0]["cells_with_direct_ground_returns"] == 2
    print("Synthetic PDAL count bins and classification/flag filtering verified.")


def main():
    if sys.argv[1:] == ["--self-check"]:
        synthetic_check()
        return
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("copc_url")
    parser.add_argument("review", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--etag", required=True)
    parser.add_argument("--source-metadata", type=Path, required=True, help="Captured PDAL COPC reader metadata for this exact source/ETag")
    parser.add_argument("--target-crs", required=True, help="Review grid's projected metre CRS")
    parser.add_argument("--reuse-sample", action="store_true", help="Recheck retained sample/count outputs with identical pipelines; no download")
    args = parser.parse_args()
    assert args.copc_url.startswith("https://") and "?" not in args.copc_url
    assert args.etag.startswith('"') and args.etag.endswith('"') and "\n" not in args.etag
    review = args.review.resolve()
    manifest = json.loads(review.read_text())
    assert manifest["schema_version"] == 1 and manifest["kind"] == "native_grid_review"
    for a in manifest["artifacts"]:
        assert Path(a["path"]).name == a["path"]
        assert sha(review.parent / a["path"]) == a["sha256"]
    info = json.loads((review.parent / "inspection.json").read_text())
    x, dx, rx, y, ry, dy = info["geoTransform"]
    assert dx == 0.5 and dy == -0.5 and rx == ry == 0
    assert run("gdalsrsinfo", "-o", "wkt2", args.target_crs).strip() == run("gdalsrsinfo", "-o", "wkt2", info["coordinateSystem"]["wkt"]).strip(), "target CRS differs from review grid"
    width, height = info["size"]
    assert width >= 128 and height >= 128
    west = x + ((width-128)//2)*dx
    north = y + ((height-128)//2)*dy
    south, east = north-64, west+64
    source_metadata = json.loads(args.source_metadata.read_text())
    header = source_metadata["stages"]["readers.copc"]
    source_crs = header["srs"]["horizontal"]
    assert header["srs"]["units"]["horizontal"] == "metre" and not header["srs"]["isgeographic"]
    output = args.output.resolve()
    assert not (output / "report.json").exists(), "report already exists"
    if args.reuse_sample:
        assert json.loads((output / "source-metadata.json").read_text()) == source_metadata
    else:
        output.mkdir(parents=True, exist_ok=False)
        save(output / "source-metadata.json", source_metadata)
    os.environ.update(GDAL_NUM_THREADS="1", GDAL_CACHEMAX="64", GDAL_PAM_ENABLED="NO", PROJ_NETWORK="OFF")
    # Read a 2 m buffer and retain all classes. Full detail: no resolution thinning.
    bounds = {"minx": west-2, "miny": south-2, "maxx": east+2, "maxy": north+2, "crs": args.target_crs}
    stages = [{"type": "readers.copc", "filename": {"path": args.copc_url, "headers": {"If-Match": args.etag}},
               "bounds": bounds, "requests": 1, "keep_alive": 2},
              {"type": "filters.reprojection", "in_srs": source_crs, "out_srs": args.target_crs, "error_on_failure": True},
              {"type": "filters.crop", "bounds": f"([{west-2},{east+2}],[{south-2},{north+2}])"},
              {"type": "writers.text", "filename": "points.csv", "order": "X,Y,Classification,Withheld,Overlap,Synthetic",
               "keep_unspecified": False, "precision": 8}]
    acquisition_seconds = pipeline(output, "sample", stages, args.reuse_sample)
    actual_header = json.loads((output / "sample-metadata.json").read_text())["stages"]["readers.copc"]
    assert actual_header["srs"]["horizontal"] == source_crs, "source CRS changed or captured header belongs to another survey"
    points = output / "points.csv"
    assert points.stat().st_size < 128*1024*1024, "sample CSV exceeds 128 MiB"
    stages = [{"type": "readers.text", "filename": "points.csv", "override_srs": args.target_crs},
              {"type": "filters.expression", "expression": "Classification == 2 && Withheld == 0 && Synthetic == 0"},
              {"type": "writers.gdal", "filename": "ground-counts.tif", "dimension": "X", "output_type": "count",
               "resolution": 0.5, "origin_x": west, "origin_y": south, "width": 128, "height": 128,
               "binmode": True, "window_size": 0, "nodata": -1, "data_type": "double", "allow_empty": True}]
    counting_seconds = pipeline(output, "counts", stages, args.reuse_sample)
    metrics = inspect_counts(output, points, west, south)
    report = {"schema_version": 1, "kind": "ground_support_sample", "review_sha256": sha(review),
              "raw_receipt_sha256": manifest["raw_receipt_sha256"], "copc_url": args.copc_url, "http_etag": args.etag,
              "source_horizontal_crs": source_crs, "target_horizontal_crs": args.target_crs,
              "sample_bounds": [west, south, east, north], "buffer_m": 2, "sample_area_m2": 4096,
              "pdal_version": run("pdal", "--version").strip(), "gdal_version": run("gdalinfo", "--version").strip(),
              "acquisition_seconds": acquisition_seconds, "counting_seconds": counting_seconds,
              "max_child_rss_kib_linux": None if args.reuse_sample else resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss,
              "metrics": metrics, "admission": "not evaluated", "terrestrial_coverage": None,
              "limitations": ["64 m centre sample only; not whole-crop quality coverage",
                              "Return counts include overlap; they are not independent pulses or measured feature resolution",
                              "No interpolation, terrain reconstruction, water classification or Z comparison",
                              "Horizontal CRSs require prior review; vertical geoid differences remain unresolved"],
              "artifacts": [{"path": p.name, "sha256": sha(p), "bytes": p.stat().st_size}
                            for p in sorted(output.iterdir()) if p.is_file()]}
    save(output / "report.json", report)
    print(output / "report.json")


if __name__ == "__main__":
    main()
