"""Bounded CPU scalar representation evaluation; no model or terrain promotion."""
from __future__ import annotations

import argparse
import importlib.metadata
import json
import os
from pathlib import Path
import platform
import resource
import sys
import tempfile
import time

import numpy as np

from .payload import digest, fields, identity, load_region, read_document
from .metrics import compare_terrain


def measure(height: np.ndarray, valid: np.ndarray, spacing_m: list[float]) -> dict:
    """Measure height and adjacent slope errors, excluding every masked sample/pair."""
    if height.dtype != np.dtype("float32") or height.ndim != 2 or valid.shape != height.shape or valid.dtype != np.dtype("bool"):
        raise ValueError("expected Float32 [row,column] heights and aligned Boolean validity")
    if not np.any(valid):
        raise ValueError("all-masked numeric input")
    if not np.all(np.isfinite(height[valid])) or np.any(np.abs(height[valid]) > 2048):
        raise ValueError("valid heights outside finite normalization range")
    result = {}
    for name, dtype in [("float32", np.float32), ("float16", np.float16)]:
        # Power-of-two scaling without an added zero preserves valid signed zero.
        encoded = np.zeros(height.shape, dtype=dtype)
        encoded[valid] = (height[valid] / np.float32(2048)).astype(dtype)
        decoded = encoded.astype(np.float32) * np.float32(2048)
        result[name] = compare_terrain(height, decoded, valid, spacing_m)
    return result


def synthetic_cases() -> dict:
    ramp = np.tile(np.linspace(-2047.75, 2047.75, 64, dtype=np.float32), (64, 1))
    flat = np.full((64, 64), 1800.125, dtype=np.float32)
    sharp = np.full((64, 64), 1800, dtype=np.float32)
    sharp[:, 31:33] += np.float32(10.125)
    narrow = np.full((64, 64), 1800, dtype=np.float32)
    narrow[:, 32] += np.float32(0.125)
    return {name: measure(a, np.ones(a.shape, dtype=bool), [0.5, 0.5])
            for name, a in [("ramp", ramp), ("flat", flat), ("sharp_ridge", sharp), ("narrow_ridge", narrow)]}


def write_json(path: Path, value: dict) -> bytes:
    data = (json.dumps(value, indent=2, allow_nan=False) + "\n").encode()
    if len(data) > 1024 * 1024:
        raise ValueError("result document exceeds 1 MiB")
    with path.open("xb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    return data


def execute(request_path: Path, output: Path) -> Path:
    """Consume a versioned Rust request, atomically publishing unpromoted metrics."""
    started = time.monotonic()
    if output.exists():
        raise ValueError("worker output already exists")
    request, request_bytes = read_document(request_path)
    fields(request, {"schema_version", "kind", "dataset_sha256", "normalization", "regions"}, "request")
    if type(request["schema_version"]) is not int or request["schema_version"] not in (1, 2) or request["kind"] != "numeric_evaluation_job":
        raise ValueError("unsupported numeric request")
    identity(request["dataset_sha256"])
    fields(request["normalization"], {"offset_m", "scale_m"}, "normalization")
    if request["normalization"] != {"offset_m": 0.0, "scale_m": 2048.0}:
        raise ValueError("unsupported normalization")
    regions = request["regions"]
    if not isinstance(regions, list) or not 1 <= len(regions) <= 32:
        raise ValueError("numeric job requires 1..32 regions")
    seen = set()
    rows = []
    for region in regions:
        height, valid, water = load_region(region, request_path.parent, version=request["schema_version"])
        if region["prepared_sha256"] in seen:
            raise ValueError("duplicate numeric region")
        seen.add(region["prepared_sha256"])
        row = {"prepared_sha256": region["prepared_sha256"], "group": region["group"], "role": region["role"],
                     "valid_cells": int(valid.sum()), "water_cells": int(water.sum()),
                     "invalid_land_cells": int((~valid & ~water).sum()),
                     "representations": measure(height, valid, region["grid"]["spacing_m"])}
        if request["schema_version"] == 2:
            layer = region["layers"]["snow-ice"]
            data = (request_path.parent / layer["path"]).read_bytes()
            if len(data) != layer["bytes"] or digest(data) != layer["sha256"]:
                raise ValueError("snow/ice payload changed")
            ice = np.frombuffer(data, dtype="u1").reshape(height.shape) == 2
            row["snow_ice_cells"] = int(ice.sum())
            row["invalid_land_cells"] = int((~valid & ~water & ~ice).sum())
        rows.append(row)
    metrics = {"schema_version": request["schema_version"], "kind": "numeric_representation_metrics", "regions": rows, "synthetic": synthetic_cases()}
    if read_document(request_path)[1] != request_bytes:
        raise ValueError("request changed during numeric evaluation")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".numeric-worker-", dir=output.parent) as name:
        stage = Path(name)
        metrics_bytes = write_json(stage / "metrics.json", metrics)
        rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
        result = {"schema_version": request["schema_version"], "kind": "numeric_worker_result", "request_sha256": digest(request_bytes),
                  "dataset_sha256": request["dataset_sha256"], "metrics_sha256": digest(metrics_bytes), "gpu_used": False,
                  "environment": {"python": sys.version, "numpy": np.__version__,
                                  "package_version": importlib.metadata.version("topography-training"),
                                  "platform": platform.platform(), "elapsed_seconds": time.monotonic() - started,
                                  "max_self_rss_kib": rss / 1024 if sys.platform == "darwin" else rss,
                                  "rss_scope": "worker process only; excludes Rust and child processes",
                                  "source_sha256": {p.name: digest(p.read_bytes()) for p in [Path(__file__), Path(__file__).with_name("payload.py"), Path(__file__).with_name("metrics.py")]}}}
        write_json(stage / "result.json", result)
        output.mkdir()
        os.replace(stage, output)
        directory_fd = os.open(output.parent, os.O_RDONLY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    return output / "result.json"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("request", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        print(execute(args.request.absolute(), args.output.absolute()))
    except (ValueError, OSError, KeyError, TypeError) as error:
        print(f"Numeric evaluation failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
