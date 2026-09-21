#!/usr/bin/env python3
"""Measure classified ground returns in a 64 m sample or a whole review crop.

Developer probe, not source admission. Requires a reviewed matching COPC survey,
explicit projected metre CRSs and readiness for point decompression. No Z comparison.
"""

import argparse
from array import array
from collections import Counter
import csv
import hashlib
import io
import json
import math
import os
from pathlib import Path
import resource
import selectors
import signal
import subprocess
import sys
import tempfile
import time


MIB = 1024*1024


def workload_limits(full_review):
    return {"timeout_seconds": 1800 if full_review else 900,
            "max_points": 20_000_000 if full_review else 1_000_000,
            "max_csv_bytes": (2048 if full_review else 128)*MIB,
            "child_address_space_bytes": 2048*MIB,
            "parent_address_space_bytes": 512*MIB,
            "child_file_bytes": 64*MIB, "log_bytes": MIB,
            "copc_workers": 1, "copc_live_chunks": 2}


def footprint_plan(info, full_review, limits):
    x, dx, rx, y, ry, dy = info["geoTransform"]
    assert all(math.isfinite(n) for n in info["geoTransform"]), "non-finite transform"
    assert dx == 0.5 and dy == -0.5 and rx == ry == 0, "requires native north-up 0.5 m cells"
    width, height = info["size"]
    assert all(type(n) is int for n in [width, height]), "invalid grid dimensions"
    if full_review:
        assert 1 <= width <= 2048 and 1 <= height <= 2048, "full review requires 1..2048 cells per side"
        west, north = x, y
    else:
        assert width >= 128 and height >= 128, "sample requires at least 128 cells per side"
        west, north = x+((width-128)//2)*dx, y+((height-128)//2)*dy
        width = height = 128
    east, south = west+width/2, north-height/2
    return {"schema_version": 1, "kind": "ground_support_workload_plan",
            "mode": "full_review" if full_review else "centre_sample",
            "grid_cells": [width, height], "cells": width*height,
            "bounds": [west, south, east, north], "buffer_m": 2,
            "area_m2": width*height/4,
            "buffered_bounds": [west-2, south-2, east+2, north+2],
            "buffered_area_m2": (width/2+4)*(height/2+4),
            "limits": limits, "process_limits_on_execution": full_review,
            "unknown": ["total crop points", "transferred bytes", "wall-clock duration", "peak RSS"],
            "no_z_comparison": True, "admission": "not evaluated"}


def child_limits(limits):
    def apply():
        for kind, cap in [(resource.RLIMIT_AS, limits["child_address_space_bytes"]),
                          (resource.RLIMIT_FSIZE, limits["child_file_bytes"]),
                          (resource.RLIMIT_CPU, math.ceil(limits["timeout_seconds"]))]:
            _, hard = resource.getrlimit(kind)
            cap = cap if hard == resource.RLIM_INFINITY else min(cap, hard)
            resource.setrlimit(kind, (cap, cap))
        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    return apply


def bounded_command(args, sink, limits, *, cwd=None, max_bytes=MIB,
                    max_lines=None, log=None):
    """Drain both pipes in bounded chunks; kill the process group on any failure."""
    started = time.monotonic()
    sizes = {"stdout": 0, "stderr": 0}
    lines = 0
    errors = io.BytesIO() if log is None else log
    with subprocess.Popen([str(a) for a in args], cwd=cwd, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, start_new_session=True,
                          preexec_fn=child_limits(limits)) as process:
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(process.stdout, selectors.EVENT_READ, "stdout")
                selector.register(process.stderr, selectors.EVENT_READ, "stderr")
                while selector.get_map():
                    remaining = limits["timeout_seconds"]-(time.monotonic()-started)
                    if remaining <= 0:
                        raise TimeoutError("subprocess exceeded workload timeout")
                    for key, _ in selector.select(min(remaining, 0.25)):
                        block = os.read(key.fileobj.fileno(), 65536)
                        if not block:
                            selector.unregister(key.fileobj)
                            continue
                        channel = key.data
                        sizes[channel] += len(block)
                        cap = max_bytes if channel == "stdout" else limits["log_bytes"]
                        assert sizes[channel] <= cap, f"{channel} exceeded byte limit; incomplete evidence"
                        if channel == "stdout":
                            lines += block.count(b"\n")
                            assert max_lines is None or lines <= max_lines, "point limit exceeded; incomplete evidence"
                            sink(block)
                        else:
                            errors.write(block)
            remaining = limits["timeout_seconds"]-(time.monotonic()-started)
            process.wait(timeout=max(0.001, remaining))
            if process.returncode:
                detail = errors.getvalue().decode(errors="replace") if log is None else "see retained log"
                raise RuntimeError(f"{args[0]} exited {process.returncode}: {detail}")
        finally:
            # Also stop descendants which might retain a pipe after the direct child exits.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
    return time.monotonic()-started


def run(*args, limits=None, **kwargs):
    if limits is not None:
        output = io.BytesIO()
        bounded_command(args, output.write, limits, **kwargs)
        return output.getvalue().decode()
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


def pipeline(directory, name, stages, reuse=False, limits=None, stream_points=False):
    path = directory / f"{name}-pipeline.json"
    if reuse:
        assert json.loads(path.read_text()) == stages, "retained pipeline differs from requested sample"
        assert (directory / f"{name}-metadata.json").is_file()
        return None
    save(path, stages)
    command = ["pdal", "pipeline", str(path), "--stream", "--metadata",
               str(directory / f"{name}-metadata.json")]
    if limits is not None:
        with (directory / f"{name}.log").open("xb") as log:
            if stream_points:
                with (directory / "points.csv").open("xb") as points:
                    return bounded_command(command, points.write, limits, cwd=directory,
                                           max_bytes=limits["max_csv_bytes"],
                                           max_lines=limits["max_points"]+1, log=log)
            return bounded_command(command, log.write, limits, cwd=directory, log=log)
    started = time.monotonic()
    result = subprocess.run(command,
                            cwd=directory, capture_output=True, text=True, timeout=900, check=False)
    (directory / f"{name}.log").write_text(result.stdout + result.stderr)
    result.check_returncode()
    return time.monotonic() - started


def inspect_counts(directory, points, west, south, width=128, height=128, limits=None):
    """Compare every PDAL 0.5 m bin with independently counted retained XY points."""
    limits = workload_limits(False) if limits is None else limits
    assert points.stat().st_size <= limits["max_csv_bytes"], "CSV exceeds byte limit"
    expected = array("I", [0]) * (width*height)
    classes = Counter()
    retained = 0
    total = 0
    with points.open() as stream:
        for row in csv.DictReader(stream):
            total += 1
            assert total <= limits["max_points"], "sample exceeded point limit; incomplete evidence"
            cls = int(float(row["Classification"]))
            classes[cls] += 1
            x, y = float(row["X"]), float(row["Y"])
            if cls == 2 and int(float(row["Withheld"])) == 0 and int(float(row["Synthetic"])) == 0:
                col, bottom_row = math.floor((x-west)*2), math.floor((y-south)*2)
                if 0 <= col < width and 0 <= bottom_row < height:
                    expected[(height-1-bottom_row)*width+col] += 1
                    retained += 1
    # PDAL's count band uses nodata for empty bins; these mean zero direct returns.
    pending = b""
    decoded = 0

    def compare(block):
        nonlocal pending, decoded
        lines = (pending+block).split(b"\n")
        pending = lines.pop()
        assert len(pending) < 512, "invalid oversized XYZ row"
        for line in lines:
            assert decoded < len(expected), "too many count bins"
            fields = line.split()
            assert len(fields) == 3, "invalid XYZ row"
            n = float(fields[2])
            assert math.isfinite(n) and n.is_integer() and n >= -1, "invalid count band"
            assert max(0, int(n)) == expected[decoded], "PDAL count bins differ from retained XY samples"
            decoded += 1

    bounded_command(["gdal_translate", "-q", "-of", "XYZ", directory / "ground-counts.tif", "/vsistdout/"],
                    compare, limits, max_bytes=width*height*160)
    assert not pending and decoded == len(expected), "incomplete count bins"
    info = json.loads(run("gdalinfo", "-json", directory / "ground-counts.tif", limits=limits))
    assert info["size"] == [width, height]
    assert all(abs(a-b) <= 1e-8 for a, b in zip(info["geoTransform"], [west, 0.5, 0, south+height/2, 0, -0.5]))
    assert total > 0, "empty point sample; check source coverage and coordinate metadata"
    support = []
    for factor in [1, 2, 4, 8]:
        occupied = cells = maximum = 0
        for r in range(0, height, factor):
            for c in range(0, width, factor):
                n = sum(expected[rr*width+cc] for rr in range(r, min(r+factor, height))
                        for cc in range(c, min(c+factor, width)))
                cells += 1
                occupied += n > 0
                maximum = max(maximum, n)
        support.append({"cell_spacing_m": factor/2, "cells": cells,
                        "cells_with_direct_ground_returns": occupied,
                        "empty_cells": cells-occupied, "maximum_returns": maximum})
    return {"buffered_points": total, "buffered_classification_counts": dict(sorted(classes.items())),
            "ground_returns_in_sample": retained, "ground_returns_per_m2": retained/(width*height/4),
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
        # A rectangular footprint also exercises partial 4 m aggregation bins.
        rectangle = directory / "rectangle"
        rectangle.mkdir()
        rectangular_points = rectangle / "points.csv"
        rectangular_points.write_text(points.read_text().replace("63.9,63.9", "4.9,2.9"))
        stages[0]["filename"] = "points.csv"
        stages[-1].update(width=10, height=6)
        pipeline(rectangle, "counts", stages, limits=workload_limits(True))
        metrics = inspect_counts(rectangle, rectangular_points, 0, 0, 10, 6,
                                 workload_limits(True))
        assert metrics["ground_returns_in_sample"] == 3
        assert metrics["ground_returns_per_m2"] == 0.2
        assert [s["cells"] for s in metrics["direct_support"]] == [60, 15, 6, 2]
        assert [s["cells_with_direct_ground_returns"] for s in metrics["direct_support"]] == [2]*4
        info = {"size": [1024, 1024], "geoTransform": [169400, 0.5, 0, 258100, 0, -0.5]}
        plan = footprint_plan(info, True, workload_limits(True))
        assert plan["bounds"] == [169400, 257588, 169912, 258100]
        assert plan["buffered_area_m2"] == 266256
        assert plan["cells"] == 1048576 and plan["area_m2"] == 262144
        assert footprint_plan(info, False, workload_limits(False))["area_m2"] == 4096
        for size in [[2049, 1], [1, 2049], [0, 8]]:
            try:
                footprint_plan(dict(info, size=size), True, workload_limits(True))
            except AssertionError:
                pass
            else:
                raise AssertionError("out-of-bounds grid accepted")
        assert footprint_plan(dict(info, size=[2048, 2048]), True,
                              workload_limits(True))["cells"] == 4194304
        # Exercise the real PDAL stdout path with six local fixture points.
        text_stages = [{"type": "readers.text", "filename": str(points)},
                       {"type": "writers.text", "filename": "STDOUT",
                        "order": "X,Y,Classification,Withheld,Overlap,Synthetic",
                        "keep_unspecified": False, "precision": 8}]
        streamed = directory / "streamed"
        streamed.mkdir()
        exact_limits = dict(workload_limits(True), max_points=6)
        pipeline(streamed, "sample", text_stages, limits=exact_limits, stream_points=True)
        with (streamed / "points.csv").open() as stream:
            assert len(list(csv.DictReader(stream))) == 6
        # Retained pipeline equality continues to support the existing reuse path.
        assert pipeline(streamed, "sample", text_stages, reuse=True) is None
        for label, failed_limits in [("points", dict(exact_limits, max_points=5)),
                                     ("bytes", dict(exact_limits, max_csv_bytes=32))]:
            failed = directory / label
            failed.mkdir()
            try:
                pipeline(failed, "sample", text_stages, limits=failed_limits, stream_points=True)
            except AssertionError:
                pass
            else:
                raise AssertionError(f"{label} cap failed to stop local fixture")
            assert not (failed / "report.json").exists()
        try:
            bounded_command([sys.executable, "-c", "import time; time.sleep(1)"],
                            lambda _: None, dict(exact_limits, timeout_seconds=0.05))
        except TimeoutError:
            pass
        else:
            raise AssertionError("timeout failed to stop local subprocess")
        # Exercise --plan through the public CLI, with a source URL that must not be read.
        wkt = run("gdalsrsinfo", "-o", "wkt2", "EPSG:6566").strip()
        inspection = directory / "inspection.json"
        save(inspection, dict(info, coordinateSystem={"wkt": wkt}))
        review = directory / "review.json"
        save(review, {"schema_version": 1, "kind": "native_grid_review",
                      "raw_receipt_sha256": "0"*64,
                      "artifacts": [{"path": inspection.name, "sha256": sha(inspection)}]})
        metadata = directory / "source-metadata.json"
        save(metadata, {"stages": {"readers.copc": {"srs": {
            "horizontal": wkt, "units": {"horizontal": "metre"}, "isgeographic": False}}}})
        planned_output = directory / "must not exist"
        command = [sys.executable, str(Path(__file__).resolve()), "https://example.invalid/source.copc.laz",
                   str(review), str(planned_output), "--etag", '"fixture"',
                   "--source-metadata", str(metadata), "--target-crs", "EPSG:6566", "--full-review", "--plan"]
        observed = json.loads(run(*command))
        assert observed["bounds"] == plan["bounds"]
        assert observed["limits"]["max_points"] == 20_000_000
        assert not planned_output.exists()
        for flags in [["--max-points", "20000001"], ["--max-csv-mib", "2049"],
                      ["--timeout-seconds", "1801"], ["--max-points", "0"]]:
            failed = subprocess.run(command+flags, capture_output=True, text=True, timeout=30)
            assert failed.returncode != 0 and "must be positive" in failed.stderr
        assert not planned_output.exists()
    print("Synthetic sample/rectangular counts, streamed point/byte limits, timeout, reuse, and dry plan verified.")


def main():
    if not __debug__:
        raise RuntimeError("This developer probe requires Python assertions; do not use -O")
    os.environ.update(GDAL_NUM_THREADS="1", GDAL_CACHEMAX="64", GDAL_PAM_ENABLED="NO", PROJ_NETWORK="OFF",
                      OMP_NUM_THREADS="1", OPENBLAS_NUM_THREADS="1")
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
    parser.add_argument("--full-review", action="store_true", help="Measure the entire native 0.5 m review grid (up to 2048 cells per side); requires resource readiness")
    parser.add_argument("--plan", action="store_true", help="Validate local inputs and print workload bounds; no network, PDAL decoding, or output directory")
    parser.add_argument("--max-points", type=int, help="Lower the buffered point cap (full-review default/maximum: 20000000)")
    parser.add_argument("--max-csv-mib", type=int, help="Lower the CSV byte cap (full-review default/maximum: 2048 MiB)")
    parser.add_argument("--timeout-seconds", type=int, help="Lower the timeout (full-review default/maximum: 1800 seconds for the whole run)")
    args = parser.parse_args()
    limits = workload_limits(args.full_review)
    for key, value in [("max_points", args.max_points),
                       ("max_csv_bytes", None if args.max_csv_mib is None else args.max_csv_mib*MIB),
                       ("timeout_seconds", args.timeout_seconds)]:
        if value is not None:
            assert args.full_review, "custom limits require --full-review"
            assert 0 < value <= limits[key], f"{key} must be positive and no greater than {limits[key]}"
            limits[key] = value
    assert args.copc_url.startswith("https://") and "?" not in args.copc_url
    assert args.etag.startswith('"') and args.etag.endswith('"') and not any(c in args.etag for c in "\r\n")
    review = args.review.resolve()
    manifest = json.loads(review.read_text())
    assert manifest["schema_version"] == 1 and manifest["kind"] == "native_grid_review"
    for a in manifest["artifacts"]:
        assert Path(a["path"]).name == a["path"]
        assert sha(review.parent / a["path"]) == a["sha256"]
    info = json.loads((review.parent / "inspection.json").read_text())
    plan = footprint_plan(info, args.full_review, limits)
    assert run("gdalsrsinfo", "-o", "wkt2", args.target_crs).strip() == run("gdalsrsinfo", "-o", "wkt2", info["coordinateSystem"]["wkt"]).strip(), "target CRS differs from review grid"
    target = json.loads(run("gdalsrsinfo", "-o", "PROJJSON", args.target_crs))
    assert target["type"] == "ProjectedCRS", "target CRS must be projected"
    assert all(axis["unit"] == "metre" for axis in target["coordinate_system"]["axis"]), "target CRS must use metres"
    width, height = plan["grid_cells"]
    west, south, east, north = plan["bounds"]
    source_metadata = json.loads(args.source_metadata.read_text())
    header = source_metadata["stages"]["readers.copc"]
    source_crs = header["srs"]["horizontal"]
    assert header["srs"]["units"]["horizontal"] == "metre" and not header["srs"]["isgeographic"]
    output = args.output.resolve()
    plan.update(review_sha256=sha(review), source_metadata_sha256=sha(args.source_metadata),
                copc_url=args.copc_url, http_etag=args.etag, source_horizontal_crs=source_crs,
                target_horizontal_crs=args.target_crs, output=str(output),
                readiness_required=args.full_review and not args.reuse_sample)
    if args.plan:
        print(json.dumps(plan, indent=2))
        return
    if args.full_review:
        assert sys.platform == "linux", "full-review resource limits require Linux"
        _, hard = resource.getrlimit(resource.RLIMIT_AS)
        cap = limits["parent_address_space_bytes"]
        resource.setrlimit(resource.RLIMIT_AS, (cap if hard == resource.RLIM_INFINITY else min(cap, hard), hard))

        def deadline(_signum, _frame):
            raise TimeoutError("whole-review workload exceeded total timeout; incomplete evidence")

        signal.signal(signal.SIGALRM, deadline)
        signal.setitimer(signal.ITIMER_REAL, limits["timeout_seconds"])
    assert not (output / "report.json").exists(), "report already exists"
    if args.reuse_sample:
        assert json.loads((output / "source-metadata.json").read_text()) == source_metadata
    else:
        output.mkdir(parents=True, exist_ok=False)
        save(output / "source-metadata.json", source_metadata)
    active_limits = limits if args.full_review else None
    # Read a 2 m buffer and retain all classes. Full detail: no resolution thinning.
    bounds = {"minx": west-2, "miny": south-2, "maxx": east+2, "maxy": north+2, "crs": args.target_crs}
    stages = [{"type": "readers.copc", "filename": {"path": args.copc_url, "headers": {"If-Match": args.etag}},
               "bounds": bounds, "requests": 1, "keep_alive": 2},
              {"type": "filters.reprojection", "in_srs": source_crs, "out_srs": args.target_crs, "error_on_failure": True},
              {"type": "filters.crop", "bounds": f"([{west-2},{east+2}],[{south-2},{north+2}])"},
              {"type": "writers.text", "filename": "STDOUT" if args.full_review else "points.csv", "order": "X,Y,Classification,Withheld,Overlap,Synthetic",
               "keep_unspecified": False, "precision": 8}]
    acquisition_seconds = pipeline(output, "sample", stages, args.reuse_sample,
                                   active_limits, stream_points=args.full_review)
    actual_header = json.loads((output / "sample-metadata.json").read_text())["stages"]["readers.copc"]
    assert actual_header["srs"]["horizontal"] == source_crs, "source CRS changed or captured header belongs to another survey"
    points = output / "points.csv"
    assert points.stat().st_size <= limits["max_csv_bytes"], "sample CSV exceeds byte cap"
    stages = [{"type": "readers.text", "filename": "points.csv", "override_srs": args.target_crs},
              {"type": "filters.expression", "expression": "Classification == 2 && Withheld == 0 && Synthetic == 0"},
              {"type": "writers.gdal", "filename": "ground-counts.tif", "dimension": "X", "output_type": "count",
               "resolution": 0.5, "origin_x": west, "origin_y": south, "width": width, "height": height,
               "binmode": True, "window_size": 0, "nodata": -1, "data_type": "double", "allow_empty": True}]
    counting_seconds = pipeline(output, "counts", stages, args.reuse_sample, active_limits)
    metrics = inspect_counts(output, points, west, south, width, height, limits)
    report = {"schema_version": 1, "kind": "ground_support_full_review" if args.full_review else "ground_support_sample", "review_sha256": sha(review),
              "raw_receipt_sha256": manifest["raw_receipt_sha256"], "copc_url": args.copc_url, "http_etag": args.etag,
              "source_horizontal_crs": source_crs, "target_horizontal_crs": args.target_crs,
              "sample_bounds": [west, south, east, north], "buffer_m": 2, "sample_area_m2": plan["area_m2"],
              "pdal_version": run("pdal", "--version", limits=active_limits).strip(), "gdal_version": run("gdalinfo", "--version", limits=active_limits).strip(),
              "acquisition_seconds": acquisition_seconds, "counting_seconds": counting_seconds,
              "max_child_rss_kib_linux": None if args.reuse_sample else resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss,
              "metrics": metrics, "admission": "not evaluated", "terrestrial_coverage": None,
              "limitations": ["Whole-crop return support only; not terrestrial quality coverage" if args.full_review else "64 m centre sample only; not whole-crop quality coverage",
                              "Return counts include overlap; they are not independent pulses or measured feature resolution",
                              "No interpolation, terrain reconstruction, water classification or Z comparison",
                              "Horizontal CRSs require prior review; vertical geoid differences remain unresolved"],
              "artifacts": [{"path": p.name, "sha256": sha(p), "bytes": p.stat().st_size}
                            for p in sorted(output.iterdir()) if p.is_file()]}
    if args.full_review:
        report.update(workload_plan=plan, grid_cells=[width, height],
                      aggregation="North-west anchored; partial east/south edge bins retained at coarser spacings")
    # Publish only a complete report, including when a deadline interrupts JSON writing.
    pending_report = output / "report-pending.json"
    save(pending_report, report)
    os.link(pending_report, output / "report.json")
    pending_report.unlink()
    if args.full_review:
        signal.setitimer(signal.ITIMER_REAL, 0)
    print(output / "report.json")


if __name__ == "__main__":
    main()
