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
        # Failure publishes neither a success result nor a partial output directory.
        job["schema_version"] = 2
        request.write_text(json.dumps(job))
        failed = root / "failed"
        result = subprocess.run([sys.executable, "-m", "topography_training.numeric", str(request), str(failed)],
                                capture_output=True, text=True)
        assert result.returncode != 0 and not failed.exists()
    print("Numeric workflows passed: signed-zero scalar loading, masks, physical slopes, range and failure boundaries.")


if __name__ == "__main__":
    main()
