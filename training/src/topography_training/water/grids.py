"""Grid directories for `topography world rain|water|water-finish` (docs/workflows/terrain-cascade.md):
grid.json, little-endian rasters (rows north to south), optional classes and provenance."""

from __future__ import annotations

import json
import os
import shutil
from pathlib import Path

import numpy as np


def write_grid(path: Path, h: np.ndarray | None, spacing_m: float, origin: tuple[float, float],
               classes: np.ndarray | None = None, offset_m: tuple[float, float] | None = None,
               classes_cell_m: float | None = None,
               wind_from_deg: float | None = None, provenance: np.ndarray | None = None,
               provenance_cell_m: float | None = None, elevation_link: Path | None = None,
               size: tuple[int, int] | None = None) -> Path:
    """A grid directory (replaced if present). `origin` is the world's north-west corner in the
    layout plane; `offset_m` [east, south] places the grid inside it. With `elevation_link` the
    ground is a symlink (give `size` as (w, h)) instead of a copy of `h`."""
    if path.exists():
        shutil.rmtree(path)
    path.mkdir(parents=True)
    w, hh = size if h is None else (h.shape[1], h.shape[0])
    grid = {"size": [int(w), int(hh)], "spacing_m": float(spacing_m),
            "origin": [float(origin[0]), float(origin[1])]}
    if offset_m is not None:
        grid["offset_m"] = [float(offset_m[0]), float(offset_m[1])]
    if classes_cell_m is not None:
        grid["classes_cell_m"] = float(classes_cell_m)
    if wind_from_deg is not None:
        grid["wind_from_deg"] = float(wind_from_deg)
    if elevation_link is not None:
        os.symlink(Path(elevation_link).resolve(), path / "elevation.f32")
    else:
        np.ascontiguousarray(h, dtype="<f4").tofile(path / "elevation.f32")
    if classes is not None:
        np.ascontiguousarray(classes, dtype=np.uint8).tofile(path / "classes.u8")
    if provenance is not None:
        np.ascontiguousarray(provenance, dtype=np.uint8).tofile(path / "provenance.u8")
        grid["provenance_cell_m"] = float(provenance_cell_m or spacing_m)
    (path / "grid.json").write_text(json.dumps(grid, indent=1))
    return path


def add_moisture(grid: Path, coarse_out: Path) -> None:
    """The 128 m moisture field and mean land rain of a coarser level's published output."""
    cg = json.loads((coarse_out / "grid.json").read_text())
    m = json.loads((grid / "grid.json").read_text())
    shutil.copy(coarse_out / cg["moisture"]["file"], grid / "moisture.f32")
    m["moisture"] = cg["moisture"] | {"file": "moisture.f32"}
    m["discharge"] = {"mean_land_mm_h": cg["discharge"]["mean_land_mm_h"]}
    (grid / "grid.json").write_text(json.dumps(m, indent=1))


def provenance_window(prov32: np.ndarray, x0_m: float, y0_m: float, n: int, cell_m: float) -> np.ndarray:
    """Nearest 32 m provenance on an n² grid of `cell_m` cells whose north-west corner lies
    (x0_m east, y0_m south) of the map's."""
    c = (np.arange(n) + 0.5) * cell_m
    rows = np.clip(((y0_m + c) // 32.0).astype(int), 0, prov32.shape[0] - 1)
    cols = np.clip(((x0_m + c) // 32.0).astype(int), 0, prov32.shape[1] - 1)
    return prov32[np.ix_(rows, cols)]
