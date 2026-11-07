"""The hydrology v3 water table (configs/hydrology/water-v3.toml) for Python: r per class, the
hot-dry classes, the game settings and per-level thresholds, as the Rust model reads them."""

from __future__ import annotations

import copy
import tomllib
from functools import lru_cache
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
TABLE = ROOT / "configs" / "hydrology" / "water-v3.toml"


@lru_cache(maxsize=None)
def load(path: Path = TABLE) -> dict:
    t = tomllib.loads(path.read_text())
    if t.get("schema_version") != 3 or t.get("kind") != "water_table":
        raise ValueError(f"{path} is not a schema 3 water table")
    return t


def _class(cls: int) -> dict:
    classes = load()["classes"]
    return classes.get(str(cls), classes["0"])


def r(cls: int) -> float:
    return float(_class(cls)["r"])


def hot_dry_classes() -> list[int]:
    return [int(k) for k, v in load()["classes"].items() if v.get("hot_dry", False)]


def lake_min_m2() -> float:
    return float(load()["lake_min_m2"])


def below_sea_coast_km() -> float:
    return float(load()["below_sea_coast_km"])


def level(spacing_m: float) -> dict:
    """The largest level not coarser than `spacing_m` (the finest level below every level)."""
    levels = sorted((float(k), v) for k, v in load()["levels"].items())
    fit = [v for s, v in levels if s <= spacing_m + 1e-9]
    return copy.deepcopy(fit[-1] if fit else levels[0][1])
