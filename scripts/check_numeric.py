#!/usr/bin/env python3
"""Check the installed numeric worker using small scalar/mask payloads."""

import copy
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

import numpy as np

from topography_training.numeric import measure
from topography_training.payload import load_region


def main():
    with tempfile.TemporaryDirectory(prefix="topography numeric ") as name:
        root = Path(name)
        height = np.array([[-0.0, 100.125, 0, 0], [1800, 1800.125, 1800, 1800],
                           [2047.75, -2047.75, 20.25, 20.5]], dtype="<f4")
        arrays = {"elevation": height, "validity": np.full((3, 4), 255, np.uint8),
                  "provider-validity": np.full((3, 4), 255, np.uint8),
                  "water": np.zeros((3, 4), np.uint8), "quality": np.ones((3, 4), np.uint8),
                  "repairs": np.zeros((3, 4), np.uint8), "features": np.zeros((3, 4), np.uint8)}
        arrays["validity"][0, 2:] = 0
        arrays["water"][0, 2] = 255
        arrays["provider-validity"][0, 3] = 0
        layers = {}
        for key, array in arrays.items():
            data = array.tobytes()
            (root / (key + ".bin")).write_bytes(data)
            layers[key] = {"path": key + ".bin", "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
        region = {"prepared_sha256": "a" * 64, "group": "fixture", "role": "calibration",
                  "grid": {"origin": [1, 2], "size": [4, 3], "spacing_m": [0.5, 1.0]},
                  "masked_fill_m": 0.0, "layers": layers}
        a, valid, water = load_region(region, root)
        assert a.dtype == np.dtype("<f4") and a[0, 0].view(np.uint32) == 0x80000000
        assert valid.sum() == 10 and water.sum() == 1
        metrics = measure(a, valid, [0.5, 1.0])
        assert metrics["float32"]["bit_identical_valid_samples"]
        assert metrics["float32"]["max_height_error_m"] == 0
        assert metrics["float16"]["max_height_error_m"] > 0
        # Hidden water fill cannot enter height or slope metrics.
        changed = a.copy()
        changed[~valid] = 1000000
        assert measure(changed, valid, [0.5, 1.0]) == metrics
        for edit, message in [
            (lambda r: r["layers"]["elevation"].update(sha256="0" * 64), "integrity"),
            (lambda r: r["layers"]["water"].update(path="../water.bin"), "path"),
            (lambda r: r["grid"].update(size=[4000, 3]), "size"),
            (lambda r: r.update(unexpected=True), "fields"),
        ]:
            bad = copy.deepcopy(region)
            edit(bad)
            try:
                load_region(bad, root)
            except ValueError as error:
                assert message in str(error), str(error)
            else:
                raise AssertionError("invalid payload accepted")
        for data, mask, message in [(a, np.zeros_like(valid), "all-masked"),
                                    (np.full_like(a, 3000), valid, "range")]:
            try:
                measure(data, mask, [0.5, 1.0])
            except ValueError as error:
                assert message in str(error)
            else:
                raise AssertionError("invalid numeric case accepted")
        request = root / "request.json"
        job = {"schema_version": 1, "kind": "numeric_evaluation_job", "dataset_sha256": "b" * 64,
               "normalization": {"offset_m": 0.0, "scale_m": 2048.0}, "regions": [region]}
        request.write_text(json.dumps(job))
        result = subprocess.run([sys.executable, "-m", "topography_training.numeric", str(request), str(root / "out")],
                                capture_output=True, text=True)
        assert result.returncode == 0, result.stderr
        assert (root / "out/result.json").is_file()
        # Cover v2 preserves the measured ice surface but excludes it from ground metrics.
        covered = copy.deepcopy(region)
        cover_arrays = {key: value.copy() for key, value in arrays.items()}
        cover_arrays["surface-elevation"] = height.copy()
        cover_arrays["snow-ice"] = np.ones((3, 4), np.uint8)
        cover_arrays["snow-ice"][1, 1] = 2
        cover_arrays["snow-ice"][0, 2] = 2  # Water and ice may overlap.
        cover_arrays["vegetation"] = np.zeros((3, 4), np.uint8)
        cover_arrays["vegetation"][1, :] = 2
        cover_arrays["elevation"][1, 1] = 0
        cover_arrays["validity"][1, 1] = 0

        def cover_layer(key):
            data = cover_arrays[key].tobytes()
            filename = "cover-" + key + ".bin"
            (root / filename).write_bytes(data)
            covered["layers"][key] = {"path": filename, "bytes": len(data),
                                       "sha256": hashlib.sha256(data).hexdigest()}

        for key in cover_arrays:
            cover_layer(key)
        ground, ground_valid, ice_water = load_region(covered, root, version=2)
        assert ground_valid.sum() == 9 and ice_water.sum() == 1
        assert not ground_valid[1, 1] and ground[1, 1] == 0
        for key, index, value, message in [
            ("snow-ice", (0, 2), 0, "unknown snow"),
            ("vegetation", (0, 0), 3, "category"),
            ("validity", (1, 1), 255, "validity"),
            ("surface-elevation", (1, 0), 999, "surface"),
        ]:
            previous = cover_arrays[key][index].copy()
            cover_arrays[key][index] = value
            cover_layer(key)
            try:
                load_region(covered, root, version=2)
            except ValueError as error:
                assert message in str(error), str(error)
            else:
                raise AssertionError("invalid cover payload accepted")
            cover_arrays[key][index] = previous
            cover_layer(key)
        job.update(schema_version=2, regions=[covered])
        request.write_text(json.dumps(job))
        result = subprocess.run([sys.executable, "-m", "topography_training.numeric", str(request), str(root / "cover-out")],
                                capture_output=True, text=True)
        assert result.returncode == 0, result.stderr
        cover_metrics = json.loads((root / "cover-out/metrics.json").read_text())
        assert cover_metrics["schema_version"] == 2
        assert cover_metrics["regions"][0]["snow_ice_cells"] == 2
        assert cover_metrics["regions"][0]["invalid_land_cells"] == 1
        # Failure publishes neither a success result nor a partial output directory.
        job["schema_version"] = 3
        request.write_text(json.dumps(job))
        failed = root / "failed"
        result = subprocess.run([sys.executable, "-m", "topography_training.numeric", str(request), str(failed)],
                                capture_output=True, text=True)
        assert result.returncode != 0 and not failed.exists()
    print("Numeric workflows passed: signed-zero scalar loading, masks, physical slopes, range and failure boundaries.")


if __name__ == "__main__":
    main()
