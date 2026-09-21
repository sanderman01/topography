#!/usr/bin/env python3
"""Small reconstruction metric and bounded-process checks; no model download."""
import argparse
import json
import math
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time

import numpy as np

from topography_training.metrics import compare_terrain
from topography_training.codec import supervise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--job", type=Path, help="also check completion against a locally pinned pilot job, without loading the model")
    args = parser.parse_args()
    height = np.array([[-0.0, 1], [0, 1]], dtype=np.float32)
    valid = np.ones_like(height, dtype=bool)
    exact = compare_terrain(height, height.copy(), valid, [0.5, 1])
    assert exact["bit_identical_valid_samples"] and exact["max_height_error_m"] == 0
    decoded = np.array([[0, 2], [0, 2]], dtype=np.float32)
    altered = compare_terrain(height, decoded, valid, [0.5, 1])
    assert altered["max_height_error_m"] == 1
    assert math.isclose(altered["height_rmse_m"], math.sqrt(0.5))
    assert altered["max_slope_component_error_m_per_m"] == 2
    assert math.isclose(altered["slope_component_rmse_m_per_m"], math.sqrt(2))
    assert altered["valid_neighbor_pairs"] == 4 and altered["nonflat_pairs"] == 2
    valid[:, 1] = False
    decoded[:, 1] = 1e8
    assert compare_terrain(height, decoded, valid, [0.5, 1])["max_height_error_m"] == 0
    for a, b, mask in [(height, height, np.zeros_like(valid)),
                        (height, np.full_like(height, np.nan), valid),
                        (height.astype(np.float64), decoded, valid)]:
        try:
            compare_terrain(a, b, mask, [0.5, 1])
        except ValueError:
            pass
        else:
            raise AssertionError("invalid terrain comparison accepted")
    with tempfile.TemporaryDirectory(prefix="topography codec ") as name:
        root = Path(name)
        supervise([sys.executable, "-c", "print('completed')"], root / "good.log", 5, 1024)
        for label, code, timeout, diagnostic in [
            ("exit", "raise SystemExit(4)", 5, "failed"),
            ("timeout", "import time; time.sleep(10)", 0.1, "time"),
            ("log", "import os; os.write(1,b'x'*8192); os._exit(0)", 5, "log"),
        ]:
            try:
                supervise([sys.executable, "-c", code], root / f"{label}.log", timeout, 1024)
            except (ValueError, RuntimeError) as error:
                assert diagnostic in str(error), str(error)
            else:
                raise AssertionError(f"invalid child accepted: {label}")
        check_termination(root)
        check_publication(root)
        if args.job:
            check_completion(args.job, root)
    print("Codec checks passed: physical reconstruction metrics, masks, invalid arrays, child failure, timeout and log bounds.")


def check_termination(root):
    log = root / "terminated.log"
    sleeper = "import os,time; print(os.getpid(),flush=True); time.sleep(10)"
    code = ("from pathlib import Path; from topography_training.codec import supervise; "
            f"supervise({[sys.executable, '-c', sleeper]!r},Path({str(log)!r}),1,1024)")
    with (root / "supervisor.log").open("wb") as stream:
        supervisor = subprocess.Popen([sys.executable, "-c", code], stdout=stream, stderr=stream)
        child = None
        try:
            deadline = time.monotonic() + 5
            while not log.exists() or not log.read_text().strip():
                assert time.monotonic() < deadline, "child did not start"
                time.sleep(0.01)
            child = int(log.read_text().strip())
            supervisor.send_signal(signal.SIGTERM)
            supervisor.wait(timeout=5)
            try:
                os.kill(child, 0)
            except ProcessLookupError:
                child = None
            assert child is None, "SIGTERM left codec worker running without its supervisor"
        finally:
            if child:
                try:
                    os.kill(child, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            if supervisor.poll() is None:
                supervisor.kill()
                supervisor.wait()


def check_publication(root):
    from topography_training.codec import publish_result
    original_fsync = os.fsync
    def fail_sync(fd):
        raise OSError("synthetic sync failure")
    failed = root / "failed-publication"
    failed.mkdir()
    try:
        os.fsync = fail_sync
        try:
            publish_result(failed, {"complete": True})
        except OSError:
            pass
        else:
            raise AssertionError("sync failure ignored")
    finally:
        os.fsync = original_fsync
    assert not (failed / "result.json").exists(), "failed write published completion"
    good = root / "good-publication"
    good.mkdir()
    publish_result(good, {"complete": True})
    assert json.loads((good / "result.json").read_text()) == {"complete": True}
    before = (good / "result.json").read_bytes()
    try:
        publish_result(good, {"complete": False})
    except OSError:
        pass
    else:
        raise AssertionError("completion overwritten")
    assert (good / "result.json").read_bytes() == before


def check_completion(job_path, root):
    from topography_training import reconstruction
    from topography_training.codec import ADDRESS_SPACE_BYTES, THREADS, TIMEOUT_SECONDS
    from topography_training.numeric import write_json
    from topography_training.reconstruction import check_inputs, cases, file_hash, measurements, save_array, verify_output
    job, request, source, identities = check_inputs(job_path)
    rows = []
    for info, height, valid, water in cases(request, source):
        channels = np.repeat(height[None, :, :], 3, axis=0)
        decoded, statistics = measurements(height, channels, valid, water, info["spacing_m"])
        arrays = {"original": height.astype("<f4"), "decoded": decoded.astype("<f4"),
                  "channels": channels.astype("<f4"), "valid": valid.astype("u1"), "water": water.astype("u1")}
        artifacts = {name: save_array(root / f"{info['id']}-{name}.bin", array) for name, array in arrays.items()}
        rows.append(dict(info, metrics=statistics, artifacts=artifacts))
    names = {"codec.py", "reconstruction.py", "numeric.py", "metrics.py", "payload.py"}
    environment = {name: "synthetic completion fixture; no model run" for name in
                   ["python", "numpy", "torch", "diffusers", "safetensors", "platform", "rss_scope"]}
    environment.update(elapsed_seconds=0, max_self_rss_kib=1,
                       source_sha256={name: file_hash(Path(reconstruction.__file__).with_name(name)) for name in names})
    result = {"schema_version": 1, "kind": "image_codec_worker_result", "job_sha256": identities["job_sha256"],
              "dataset_sha256": job["dataset_sha256"], "numeric_run_sha256": job["numeric_run"]["sha256"],
              "model_manifest_sha256": job["model"]["sha256"], "gpu_used": False, "device": "cpu",
              "precision": "float32", "posterior": "mode", "normalization": {"offset_m": 0, "scale_m": 2048},
              "window_cells": 256, "interior_margin_cells": 32, "threads": THREADS,
              "timeout_seconds": TIMEOUT_SECONDS, "address_space_bytes": ADDRESS_SPACE_BYTES,
              "environment": environment, "cases": rows}
    path = root / "worker-result.json"
    write_json(path, result)
    assert verify_output(job_path, root)["human_acceptance"] is None
    intact = path.read_bytes()
    for edit in [lambda r: r["cases"][0]["metrics"].update(mean_bias_m=100),
                 lambda r: r.update(cases=r["cases"][:-1]),
                 lambda r: r.update(environment={}),
                 lambda r: r["cases"][0]["artifacts"]["decoded"].update(path="../outside.bin")]:
        altered = json.loads(intact)
        edit(altered)
        path.write_text(json.dumps(altered))
        try:
            verify_output(job_path, root)
        except ValueError:
            pass
        else:
            raise AssertionError("invalid completion accepted")
    path.write_bytes(intact)
    artifact = root / result["cases"][0]["artifacts"]["decoded"]["path"]
    artifact.write_bytes(b"corrupt")
    try:
        verify_output(job_path, root)
    except ValueError:
        pass
    else:
        raise AssertionError("corrupt completion accepted")
    print("Pinned pilot completion boundary passed; all fixture outputs remain temporary, with no model evaluation.")


if __name__ == "__main__":
    main()
