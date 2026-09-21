"""Pinned scalar-to-image VAE experiment and independent output verification."""
from __future__ import annotations

import hashlib
import importlib.metadata
import math
from pathlib import Path
import platform
import resource
import sys
import time

import numpy as np

from .metrics import compare_terrain
from .numeric import write_json
from .payload import digest, fields, identity, load_region, read_document

SIZE = 256
MARGIN = 32
MODEL_REPOSITORY = "stabilityai/sd-vae-ft-mse"
MODEL_REVISION = "31f26fdeee1355a5c34592e401dd41e45d25a493"
MODEL_WEIGHTS_SHA = "a1d993488569e928462932c8c38a0760b874d166399b14414135bd9c42df5815"
MODEL_FILES = {"config.json", "README.md", "diffusion_pytorch_model.safetensors"}


def file_hash(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def pinned_document(reference: dict) -> tuple[Path, dict, bytes]:
    fields(reference, {"path", "sha256"}, "document reference")
    identity(reference["sha256"])
    if not isinstance(reference["path"], str) or not Path(reference["path"]).is_absolute():
        raise ValueError("experiment document paths must be absolute")
    path = Path(reference["path"])
    document, data = read_document(path)
    if digest(data) != reference["sha256"]:
        raise ValueError("experiment document identity mismatch")
    return path, document, data


def check_inputs(job_path: Path) -> tuple[dict, dict, Path, dict]:
    job, job_bytes = read_document(job_path)
    fields(job, {"schema_version", "kind", "dataset_sha256", "numeric_run", "model"}, "codec job")
    if type(job["schema_version"]) is not int or job["schema_version"] != 1 or job["kind"] != "image_codec_evaluation_job":
        raise ValueError("unsupported codec job")
    identity(job["dataset_sha256"])
    run_path, run, _ = pinned_document(job["numeric_run"])
    fields(run, {"schema_version", "kind", "dataset_sha256", "request_sha256", "metrics_sha256",
                 "worker_result_sha256", "current_at_completion", "gpu_used", "training_admission", "topography_version"}, "numeric run")
    if (run["schema_version"] != 1 or run["kind"] != "verified_numeric_evaluation"
            or run["dataset_sha256"] != job["dataset_sha256"]
            or run["training_admission"] != "not admitted" or run["gpu_used"] is not False):
        raise ValueError("codec requires a completed evaluation-only numeric run")
    request, request_bytes = read_document(run_path.parent / "request.json")
    for filename, key in [("request.json", "request_sha256"), ("metrics.json", "metrics_sha256"),
                          ("worker-result.json", "worker_result_sha256")]:
        identity(run[key])
        if file_hash(run_path.parent / filename) != run[key]:
            raise ValueError("numeric run integrity mismatch")
    fields(request, {"schema_version", "kind", "dataset_sha256", "normalization", "regions"}, "numeric request")
    if (request["schema_version"] != 1 or request["kind"] != "numeric_evaluation_job"
            or request["dataset_sha256"] != job["dataset_sha256"]
            or request["normalization"] != {"offset_m": 0.0, "scale_m": 2048.0}
            or not isinstance(request["regions"], list) or not 1 <= len(request["regions"]) <= 32):
        raise ValueError("invalid scalar request for codec")
    seen = set()
    for region in request["regions"]:
        height, valid, _ = load_region(region, run_path.parent)
        if region["prepared_sha256"] in seen:
            raise ValueError("duplicate codec region")
        seen.add(region["prepared_sha256"])
        if min(height.shape) < SIZE or np.any(np.abs(height[valid]) > 2048):
            raise ValueError("codec region size or normalization range invalid")
        window = central_window(height.shape)
        if not np.any(valid[window]) or not np.any(valid[window][MARGIN:-MARGIN, MARGIN:-MARGIN]):
            raise ValueError("all-masked codec window/interior")
    model_path, model, _ = pinned_document(job["model"])
    fields(model, {"schema_version", "kind", "repository", "revision", "artifacts"}, "model")
    if (model["schema_version"] != 1 or model["kind"] != "local_image_codec"
            or model["repository"] != MODEL_REPOSITORY or model["revision"] != MODEL_REVISION):
        raise ValueError("unsupported image codec model")
    fields(model["artifacts"], MODEL_FILES, "model artifacts")
    for filename, artifact in model["artifacts"].items():
        fields(artifact, {"bytes", "sha256"}, "model artifact")
        identity(artifact["sha256"])
        path = model_path.parent / filename
        if (type(artifact["bytes"]) is not int or not 0 < artifact["bytes"] <= 335 * 1024**2
                or path.is_symlink() or path.stat().st_size != artifact["bytes"]
                or file_hash(path) != artifact["sha256"]):
            raise ValueError("model artifact integrity mismatch")
    if model["artifacts"]["diffusion_pytorch_model.safetensors"]["sha256"] != MODEL_WEIGHTS_SHA:
        raise ValueError("model weights differ from pinned public revision")
    return job, request, run_path.parent, {"job_sha256": digest(job_bytes),
            "request_sha256": digest(request_bytes), "model_manifest_sha256": job["model"]["sha256"],
            "input_roots": [str(run_path.parent.resolve()), str(model_path.parent.resolve())]}


def central_window(shape: tuple[int, int]) -> tuple[slice, slice]:
    row, column = [(n - SIZE) // 2 for n in shape]
    return slice(row, row + SIZE), slice(column, column + SIZE)


def cases(request: dict, root: Path):
    """Yield fixed native central windows, followed by four deterministic synthetic cases."""
    for index, region in enumerate(request["regions"]):
        height, valid, water = load_region(region, root)
        window = central_window(height.shape)
        row, col = window[0].start, window[1].start
        sx, sy = region["grid"]["spacing_m"]
        yield {"id": f"region-{index:02}", "prepared_sha256": region["prepared_sha256"],
               "group": region["group"], "role": region["role"], "window_offset_cells": [col, row],
               "origin": [region["grid"]["origin"][0] + col * sx, region["grid"]["origin"][1] - row * sy],
               "spacing_m": [sx, sy]}, height[window].copy(), valid[window].copy(), water[window].copy()
    ramp = np.tile(np.linspace(-2047.75, 2047.75, SIZE, dtype=np.float32), (SIZE, 1))
    flat = np.full((SIZE, SIZE), 1800.125, dtype=np.float32)
    sharp = np.full((SIZE, SIZE), 1800, dtype=np.float32)
    sharp[:, SIZE // 2 - 1:SIZE // 2 + 1] += np.float32(10.125)
    narrow = np.full((SIZE, SIZE), 1800, dtype=np.float32)
    narrow[:, SIZE // 2] += np.float32(0.125)
    for name, height in [("ramp", ramp), ("flat", flat), ("sharp_ridge", sharp), ("narrow_ridge", narrow)]:
        yield {"id": name, "prepared_sha256": None, "group": "synthetic", "role": "synthetic",
               "window_offset_cells": [0, 0], "origin": [0, 0], "spacing_m": [0.5, 0.5]}, height, np.ones(height.shape, bool), np.zeros(height.shape, bool)


def measurements(height: np.ndarray, channels: np.ndarray, valid: np.ndarray, water: np.ndarray,
                 spacing: list[float]) -> tuple[np.ndarray, dict]:
    if channels.shape != (3, SIZE, SIZE) or channels.dtype != np.float32 or not np.all(np.isfinite(channels)):
        raise ValueError("invalid reconstructed channel array")
    decoded = channels.mean(axis=0, dtype=np.float32)
    interior = (slice(MARGIN, -MARGIN), slice(MARGIN, -MARGIN))
    statistics = {"valid_cells": int(valid.sum()), "water_cells": int(water.sum()),
                  "invalid_land_cells": int((~valid & ~water).sum()),
                  "full": compare_terrain(height, decoded, valid, spacing),
                  "interior": compare_terrain(height[interior], decoded[interior], valid[interior], spacing),
                  "mean_bias_m": float(np.mean(decoded[valid].astype(np.float64) - height[valid])),
                  "max_channel_spread_m": float(np.max(np.ptp(channels[:, valid], axis=0))),
                  "channel_max_height_error_m": [float(np.max(np.abs(c[valid].astype(np.float64) - height[valid]))) for c in channels],
                  "decoded_range_m": [float(np.min(decoded[valid])), float(np.max(decoded[valid]))]}
    return decoded, statistics


def save_array(path: Path, array: np.ndarray) -> dict:
    data = array.tobytes()
    with path.open("xb") as stream:
        stream.write(data)
        stream.flush()
        import os
        os.fsync(stream.fileno())
    return {"path": path.name, "bytes": len(data), "sha256": digest(data)}


def reconstruct(job_path: Path, output: Path) -> None:
    """Worker entry; its caller sets memory/thread limits before importing this module."""
    started = time.monotonic()
    job, request, root, identities = check_inputs(job_path)
    import torch
    import diffusers
    from diffusers import AutoencoderKL
    if torch.version.cuda is not None or torch.version.hip is not None:
        raise ValueError("this experiment requires a CPU-only Torch build")
    torch.set_num_threads(2)
    torch.set_num_interop_threads(1)
    torch.manual_seed(0)
    torch.use_deterministic_algorithms(True)
    model_root = Path(job["model"]["path"]).parent
    model = AutoencoderKL.from_pretrained(str(model_root), local_files_only=True,
                use_safetensors=True, torch_dtype=torch.float32, low_cpu_mem_usage=False).to("cpu").eval()
    model.requires_grad_(False)
    rows = []
    for info, height, valid, water in cases(request, root):
        print(f"Reconstructing {info['id']}", flush=True)
        data = np.repeat((height / np.float32(2048))[None, None, :, :], 3, axis=1)
        with torch.inference_mode():
            tensor = torch.from_numpy(data)
            latent = model.encode(tensor).latent_dist.mode()
            channels = model.decode(latent).sample[0].numpy().astype(np.float32) * np.float32(2048)
        decoded, statistics = measurements(height, channels, valid, water, info["spacing_m"])
        arrays = {"original": height.astype("<f4"), "decoded": decoded.astype("<f4"),
                  "channels": channels.astype("<f4"), "valid": valid.astype("u1"), "water": water.astype("u1")}
        artifacts = {name: save_array(output / f"{info['id']}-{name}.bin", array) for name, array in arrays.items()}
        rows.append(dict(info, metrics=statistics, artifacts=artifacts))
    from .codec import ADDRESS_SPACE_BYTES, THREADS, TIMEOUT_SECONDS
    environment = {"python": sys.version, "numpy": np.__version__, "torch": torch.__version__,
                   "diffusers": diffusers.__version__, "safetensors": importlib.metadata.version("safetensors"),
                   "platform": platform.platform(), "elapsed_seconds": time.monotonic() - started,
                   "max_self_rss_kib": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
                   "rss_scope": "worker only; Linux KiB; excludes supervisor and prior Rust checks",
                   "source_sha256": {name: file_hash(Path(__file__).with_name(name)) for name in
                                     ["codec.py", "reconstruction.py", "metrics.py", "numeric.py", "payload.py"]}}
    write_json(output / "worker-result.json", {"schema_version": 1, "kind": "image_codec_worker_result",
               "job_sha256": identities["job_sha256"], "dataset_sha256": job["dataset_sha256"],
               "numeric_run_sha256": job["numeric_run"]["sha256"], "model_manifest_sha256": job["model"]["sha256"],
               "gpu_used": False, "device": "cpu", "precision": "float32", "posterior": "mode",
               "normalization": {"offset_m": 0, "scale_m": 2048}, "window_cells": SIZE,
               "interior_margin_cells": MARGIN, "threads": THREADS, "timeout_seconds": TIMEOUT_SECONDS,
               "address_space_bytes": ADDRESS_SPACE_BYTES, "environment": environment, "cases": rows})


def verify_output(job_path: Path, output: Path) -> dict:
    """Recheck input hashes and recompute reported metrics from every retained output array."""
    job, request, root, identities = check_inputs(job_path)
    result, result_bytes = read_document(output / "worker-result.json")
    fields(result, {"schema_version", "kind", "job_sha256", "dataset_sha256", "numeric_run_sha256",
                   "model_manifest_sha256", "gpu_used", "device", "precision", "posterior", "normalization",
                   "window_cells", "interior_margin_cells", "threads", "timeout_seconds", "address_space_bytes",
                   "environment", "cases"}, "codec result")
    from .codec import ADDRESS_SPACE_BYTES, THREADS, TIMEOUT_SECONDS
    expected = {"schema_version": 1, "kind": "image_codec_worker_result", "job_sha256": identities["job_sha256"],
                "dataset_sha256": job["dataset_sha256"], "numeric_run_sha256": job["numeric_run"]["sha256"],
                "model_manifest_sha256": job["model"]["sha256"], "gpu_used": False, "device": "cpu",
                "precision": "float32", "posterior": "mode", "normalization": {"offset_m": 0, "scale_m": 2048},
                "window_cells": SIZE, "interior_margin_cells": MARGIN, "threads": THREADS,
                "timeout_seconds": TIMEOUT_SECONDS, "address_space_bytes": ADDRESS_SPACE_BYTES}
    if any(result[k] != v for k, v in expected.items()) or type(result["gpu_used"]) is not bool:
        raise ValueError("codec result identity/settings mismatch")
    env = result["environment"]
    fields(env, {"python", "numpy", "torch", "diffusers", "safetensors", "platform", "elapsed_seconds",
                 "max_self_rss_kib", "rss_scope", "source_sha256"}, "codec environment")
    for name in ["python", "numpy", "torch", "diffusers", "safetensors", "platform", "rss_scope"]:
        if not isinstance(env[name], str) or not env[name].strip():
            raise ValueError("missing codec software provenance")
    for name in ["elapsed_seconds", "max_self_rss_kib"]:
        if type(env[name]) not in (int, float) or not math.isfinite(env[name]) or env[name] < 0:
            raise ValueError("invalid codec resource observation")
    names = {"codec.py", "reconstruction.py", "metrics.py", "numeric.py", "payload.py"}
    fields(env["source_sha256"], names, "codec source hashes")
    for name in names:
        if env["source_sha256"][name] != file_hash(Path(__file__).with_name(name)):
            raise ValueError("codec source changed during experiment")
    if not isinstance(result["cases"], list) or len(result["cases"]) != len(request["regions"]) + 4:
        raise ValueError("codec worker omitted cases")
    for row, (info, height, valid, water) in zip(result["cases"], cases(request, root)):
        fields(row, set(info) | {"metrics", "artifacts"}, "codec case")
        if any(row[k] != v for k, v in info.items()):
            raise ValueError("codec case identity mismatch")
        fields(row["artifacts"], {"original", "decoded", "channels", "valid", "water"}, "codec arrays")
        arrays = {}
        for name, artifact in row["artifacts"].items():
            fields(artifact, {"path", "bytes", "sha256"}, "codec artifact")
            expected_name = f"{info['id']}-{name}.bin"
            shape = (3, SIZE, SIZE) if name == "channels" else (SIZE, SIZE)
            dtype = "u1" if name in {"valid", "water"} else "<f4"
            count = math.prod(shape) * np.dtype(dtype).itemsize
            path = output / expected_name
            if (artifact["path"] != expected_name or artifact["bytes"] != count or path.is_symlink()
                    or path.stat().st_size != count or file_hash(path) != artifact["sha256"]):
                raise ValueError("codec output artifact integrity mismatch")
            arrays[name] = np.frombuffer(path.read_bytes(), dtype=dtype).reshape(shape)
        if (arrays["original"].tobytes() != height.astype("<f4").tobytes()
                or not np.array_equal(arrays["valid"], valid.astype("u1"))
                or not np.array_equal(arrays["water"], water.astype("u1"))):
            raise ValueError("codec input arrays changed")
        decoded, statistics = measurements(height, arrays["channels"], valid, water, info["spacing_m"])
        if decoded.tobytes() != arrays["decoded"].tobytes() or row["metrics"] != statistics:
            raise ValueError("codec output measurements disagree with retained arrays")
    return {"schema_version": 1, "kind": "image_codec_evaluation", "job_sha256": identities["job_sha256"],
            "worker_result_sha256": digest(result_bytes), "dataset_sha256": job["dataset_sha256"],
            "numeric_run_sha256": job["numeric_run"]["sha256"], "model_manifest_sha256": job["model"]["sha256"],
            "gpu_used": False, "training_admission": "not admitted", "human_acceptance": None,
            "source_currentness": "verify the release with Rust before and after this experiment"}
