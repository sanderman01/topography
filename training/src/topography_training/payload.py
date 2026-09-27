"""Strict scalar Float32/Byte payload loading for versioned CPU evaluation."""
from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
import re

import numpy as np

LAYERS = {"elevation", "validity", "provider-validity", "water", "quality", "repairs", "features"}
COVER_LAYERS = {"snow-ice", "vegetation", "surface-elevation"}


def fields(value: dict, expected: set[str], name: str) -> None:
    if not isinstance(value, dict) or set(value) != expected:
        raise ValueError(f"{name} fields differ from the contract")


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def identity(value: str) -> None:
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value):
        raise ValueError("invalid SHA-256 identity")


def read_document(path: Path) -> tuple[dict, bytes]:
    with path.open("rb") as stream:
        data = stream.read(1024 * 1024 + 1)
    if len(data) > 1024 * 1024:
        raise ValueError("JSON document exceeds 1 MiB")
    def reject_constant(value):
        raise ValueError(f"nonfinite JSON value: {value}")
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON field: {key}")
            result[key] = value
        return result
    return json.loads(data, parse_constant=reject_constant, object_pairs_hook=unique), data


def load_region(region: dict, directory: Path, *, version: int = 1) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """Load [row, column] Float32 metres and independent Boolean valid/water masks."""
    fields(region, {"prepared_sha256", "group", "role", "grid", "masked_fill_m", "layers"}, "region")
    identity(region["prepared_sha256"])
    if region["role"] not in {"calibration", "validation"} or not isinstance(region["group"], str) or not re.fullmatch(r"[A-Za-z0-9_-]{1,80}", region["group"]):
        raise ValueError("invalid geographic role/group")
    grid = region["grid"]
    fields(grid, {"origin", "size", "spacing_m"}, "grid")
    size = grid["size"]
    if not isinstance(size, list) or len(size) != 2 or any(type(n) is not int or not 1 <= n <= 2048 for n in size):
        raise ValueError("grid size must be 1..2048 cells per side")
    for key in ["origin", "spacing_m"]:
        value = grid[key]
        if not isinstance(value, list) or len(value) != 2 or any(type(n) not in (int, float) or not math.isfinite(n) for n in value):
            raise ValueError(f"invalid grid {key}")
    if any(not 0.5 <= n <= 1 for n in grid["spacing_m"]):
        raise ValueError("invalid native spacing")
    fill = region["masked_fill_m"]
    if type(fill) not in (int, float) or not math.isfinite(fill) or abs(fill) > np.finfo(np.float32).max:
        raise ValueError("invalid masked fill")
    if type(version) is not int or version not in (1, 2):
        raise ValueError("unsupported payload version")
    fields(region["layers"], LAYERS | (COVER_LAYERS if version == 2 else set()), "layers")
    arrays = {}
    for name, layer in region["layers"].items():
        fields(layer, {"path", "bytes", "sha256"}, "layer")
        identity(layer["sha256"])
        filename = layer["path"]
        if not isinstance(filename, str) or not re.fullmatch(r"[A-Za-z0-9_-]+\.bin", filename):
            raise ValueError("payload path must be a flat binary filename")
        path = directory / filename
        count = size[0] * size[1]
        floating = name in {"elevation", "surface-elevation"}
        expected = count * (4 if floating else 1)
        if type(layer["bytes"]) is not int or layer["bytes"] != expected or path.is_symlink() or path.stat().st_size != expected:
            raise ValueError("payload size/integrity mismatch")
        data = path.read_bytes()
        if len(data) != expected or digest(data) != layer["sha256"]:
            raise ValueError("payload integrity mismatch")
        arrays[name] = np.frombuffer(data, dtype="<f4" if floating else "u1").reshape(size[1], size[0])
    for name in ["validity", "provider-validity", "water"]:
        if not np.all(np.isin(arrays[name], [0, 255])):
            raise ValueError(f"invalid {name} mask category")
    for name, allowed in [("quality", [0, 1, 2]), ("repairs", [0]), ("features", [0, 1])]:
        if not np.all(np.isin(arrays[name], allowed)):
            raise ValueError(f"invalid {name} mask category")
    a = arrays["elevation"]
    valid = arrays["validity"] == 255
    water = arrays["water"] == 255
    expected = (arrays["provider-validity"] == 255) & (arrays["quality"] == 1) & ~water
    if version == 2:
        for name in ["snow-ice", "vegetation"]:
            if not np.all(np.isin(arrays[name], [0, 1, 2])):
                raise ValueError(f"invalid {name} mask category")
        if np.any(arrays["snow-ice"] == 0):
            raise ValueError("unknown snow/ice classification")
        expected &= np.isfinite(arrays["surface-elevation"])
    # Products before game_cover_v5 also excluded snow/ice from ground.
    legacy = expected & (arrays["snow-ice"] == 1) if version == 2 else expected
    if not (np.array_equal(valid, expected) or np.array_equal(valid, legacy)) or np.any((arrays["features"] == 1) & ~valid):
        raise ValueError("inconsistent terrain validity/masks")
    if version == 2 and not np.array_equal(a[valid].view(np.uint32), arrays["surface-elevation"][valid].view(np.uint32)):
        raise ValueError("ground elevation differs from measured surface")
    if not np.all(np.isfinite(a[valid])):
        raise ValueError("nonfinite valid ground")
    if not np.all(a[~valid].view(np.uint32) == np.float32(fill).view(np.uint32)):
        raise ValueError("masked elevation differs from deterministic fill")
    if not np.any(valid):
        raise ValueError("all-masked region")
    return a, valid, water
