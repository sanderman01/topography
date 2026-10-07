"""Terrain legend v3: subclasses of existing sources and per-cell training labels.

See docs/plans/biome-forms-v1.md. Class ids follow catalog/labels/terrain-v3.toml (0 is
unknown, also the class-dropped condition). Sources take their subclass from
catalog/labels/subclasses-v1.toml; training fields then label ice cells as glaciers and
quiet ground as plain, so a region's background no longer counts as its landform.
Models trained before legend v3 have 13 class channels and see subclasses as parents.
"""

from __future__ import annotations

import tomllib
from functools import lru_cache
from pathlib import Path

import numpy as np

LABELS = Path(__file__).resolve().parents[4] / "catalog" / "labels"
NUM_CLASSES = 19  # legend v3: 0 unknown, 1..18
LEGACY_CLASSES = 13  # legend v2 runs
GLACIER, ALPINE, BRAIDED, PLAIN = 10, 6, 9, 18
ICE_SHARE = 0.5
ICE_SMOOTH_M = 100.0
LABEL_CELL_M = 32.0  # per-cell labels are worked out on a grid of about this size
PLAIN_RADIUS_M = 1000.0  # local relief: range of heights within this distance
PLAIN_SHARE = 1 / 3  # plain below this share of the region's 90th-percentile local relief
NO_PLAIN = (BRAIDED,)  # flat floors are this class's look


@lru_cache(maxsize=1)
def _spec() -> dict:
    return tomllib.loads((LABELS / "subclasses-v1.toml").read_text())


@lru_cache(maxsize=1)
def legend() -> dict[int, dict]:
    """Legend v3 classes by id: name and colour."""
    t = tomllib.loads((LABELS / "terrain-v3.toml").read_text())
    return {c["id"]: c for c in t["classes"]}


def parents() -> np.ndarray:
    """Parent id per class (classes without a parent are their own)."""
    out = np.arange(NUM_CLASSES)
    for c in _spec()["classes"]:
        if "parent" in c:
            out[c["id"]] = c["parent"]
    out[PLAIN] = 0
    return out


def fallbacks(c: int) -> list[int]:
    for e in _spec()["classes"]:
        if e["id"] == c:
            return list(e.get("fallback", []))
    return []


def source_class(kind: str, name: str, class_id: int) -> int:
    """Class of a source: its listed subclass, else its release class. `kind` is
    "context" (GLO-30 regions), "lidar" (reduced lidar regions) or "tiles" (1 m library,
    matched by title prefix)."""
    for e in _spec()["classes"]:
        for entry in e.get(kind, []):
            if (name.startswith(entry) if kind == "tiles" else name == entry):
                return int(e["id"])
    return int(class_id)


def source_kind(spacing_m: int) -> str:
    return {32: "context", 4: "lidar", 1: "tiles"}[int(spacing_m)]


def class_map(trained: set[int]) -> list[int]:
    """Model class for every legend class: itself if trained, else its first trained
    fallback (searched breadth-first), else 0."""
    out = []
    for c in range(NUM_CLASSES):
        queue, seen, got = [c], set(), 0
        while queue:
            k = queue.pop(0)
            if k in seen:
                continue
            seen.add(k)
            if k in trained:
                got = k
                break
            queue += fallbacks(k)
        out.append(got)
    return out


def trained_classes(counts: dict[int, int]) -> set[int]:
    """Classes with enough training sources (`counts`: class -> sources showing it)."""
    return {c for c, n in counts.items() if c > 0 and n >= int(_spec()["min_sources"])}


def model_lut(config: dict) -> tuple[int, np.ndarray]:
    """(class channels, lookup from legend v3 id to model class) of a run config.
    Pre-v3 runs have 13 channels and see each subclass as its parent."""
    n = int(config.get("arch", {}).get("num_classes", LEGACY_CLASSES))
    if n == LEGACY_CLASSES:
        return n, parents().astype(np.uint8)
    return n, np.array(config["class_map"], np.uint8)


def _blocks(a: np.ndarray, k: int) -> np.ndarray:
    r, c = a.shape[0] // k * k, a.shape[1] // k * k
    with np.errstate(invalid="ignore"):
        return np.nanmean(a[:r, :c].reshape(r // k, k, c // k, k), axis=(1, 3))


def cell_classes(base: int, height: np.ndarray, snow: np.ndarray, water: np.ndarray, cell_m: float,
                 plain: bool = True) -> np.ndarray:
    """Per-cell training labels of a source whose class is `base`: ice is glacier (and
    ice-free ground of glacier sources alpine), quiet land far from relief is plain
    (`plain`), everything else keeps `base`. Sea and water keep `base`. `snow` is a bool
    mask or uint8 soft coverage (0-255)."""
    import warnings

    from scipy import ndimage

    from .data import smooth

    k = max(1, int(round(LABEL_CELL_M / cell_m)))
    g = cell_m * k
    ice_frac = snow.astype(np.float32) / (255.0 if snow.dtype == np.uint8 else 1.0)
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", RuntimeWarning)
        h = _blocks(height.astype(np.float32), k)
        ice = smooth(np.nan_to_num(_blocks(ice_frac, k)), ICE_SMOOTH_M / g) > ICE_SHARE
        wet = _blocks(water.astype(np.float32), k) > 0.5
    land = np.isfinite(h) & (h >= 0) & ~wet
    out = np.full(h.shape, base, np.uint8)
    if base == GLACIER:
        out[land] = ALPINE
    out[land & ice] = GLACIER
    if plain and base not in NO_PLAIN:
        r = max(1, int(round(PLAIN_RADIUS_M / g)))
        hf = np.where(np.isfinite(h), h, np.nanmean(h) if np.isfinite(h).any() else 0.0)
        relief = (ndimage.maximum_filter1d(ndimage.maximum_filter1d(hf, 2 * r + 1, 0), 2 * r + 1, 1)
                  - ndimage.minimum_filter1d(ndimage.minimum_filter1d(hf, 2 * r + 1, 0), 2 * r + 1, 1))
        if land.any():
            p90 = float(np.percentile(relief[land], 90))
            out[land & ~ice & (relief < PLAIN_SHARE * p90)] = PLAIN
    big = np.repeat(np.repeat(out, k, 0), k, 1)
    full = np.full(height.shape, base, np.uint8)
    full[: big.shape[0], : big.shape[1]] = big
    if big.shape != height.shape:  # leftover edge rows/columns copy their neighbours
        full[big.shape[0] :, : big.shape[1]] = big[-1:]
        full[:, big.shape[1] :] = full[:, big.shape[1] - 1 : big.shape[1]]
    return full


def majority(cls: np.ndarray, k: int) -> np.ndarray:
    """Most common class of each k x k block."""
    r, c = cls.shape[0] // k * k, cls.shape[1] // k * k
    b = cls[:r, :c].reshape(r // k, k, c // k, k).transpose(0, 2, 1, 3).reshape(r // k, c // k, k * k)
    counts = np.stack([(b == i).sum(-1) for i in range(NUM_CLASSES)], -1)
    return counts.argmax(-1).astype(np.uint8)
