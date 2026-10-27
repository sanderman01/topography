"""Make a hydrology grid dir from a world collage, a cascade chain world or a 1 m band.

Usage (training/.venv/bin/python -I scripts/hydrology_grid.py ...):
  <collage-dir> <new-grid-dir>
      32 m from a collage (arrays.npz `l2` plus class32.u8).
  --chain-l2 <chain-world-dir> <new-grid-dir>
      32 m from a cascade chain world's arrays.npz `l2`; classes are the most common class of
      the world's classes.u8 in each 32 m cell.
  --world <world-dir> <new-grid-dir>
      The world's own elevation.f32 and classes.u8 (linked, not copied).
  --band <l4-world-dir> <first-row> <rows> <halo> <new-grid-dir>
      Rows first-row .. first-row + rows of a 1 m world, the first and last `halo` rows a
      halo (`offset_m`, `core_rows`); classes from its source world (`classes_cell_m`).
Then: topography world hydrology --grid <new-grid-dir> --out <new-output-dir> [--coarse ...]
The layout's wind_from_deg goes into grid.json where the source names a layout.
"""

import json
import os
import shutil
import sys
import tomllib
from pathlib import Path

import numpy as np


def layout_wind(src: Path) -> float | None:
    """`wind_from_deg` of a layout copy (layout.json `config_text`) in `src`, if any."""
    path = src / "layout.json"
    if not path.exists():
        return None
    text = json.loads(path.read_text()).get("config_text")
    wind = tomllib.loads(text).get("wind_from_deg") if text else None
    return float(wind) if wind is not None else None


def world_manifest(src: Path) -> dict:
    return json.loads((src / "manifest.json").read_text())


def world_wind(manifest: dict) -> float | None:
    layout = manifest.get("layout")
    return layout_wind(Path(layout)) if layout else None


def block_mode(classes: np.ndarray, k: int) -> np.ndarray:
    """Most common value in each k x k block (ties to the lower value)."""
    h, w = classes.shape[0] // k, classes.shape[1] // k
    blocks = classes[: h * k, : w * k].reshape(h, k, w, k).transpose(0, 2, 1, 3).reshape(-1, k * k)
    best = np.zeros(blocks.shape[0], np.uint8)
    count = np.zeros(blocks.shape[0], np.int32)
    for v in np.unique(blocks):
        n = (blocks == v).sum(1)
        more = n > count
        best[more], count[more] = v, n[more]
    return best.reshape(h, w)


def write(out: Path, grid: dict, files: dict) -> None:
    """Stage `files` (name: array or path to link) and grid.json, then rename into place."""
    stage = out.with_name(out.name + ".partial")
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    for name, data in files.items():
        if isinstance(data, Path):
            os.symlink(data.resolve(), stage / name)
        else:
            data.tofile(stage / name)
    (stage / "grid.json").write_text(json.dumps(grid, indent=1))
    stage.rename(out)
    print(out)


def collage(src: Path, out: Path) -> None:
    meta = json.loads((src / "collage.json").read_text())
    h = np.load(src / "arrays.npz")["l2"].astype("<f4")
    cell_m = float(meta["cell_m"])
    classes = meta["class32"]
    if classes["size"] != [h.shape[1], h.shape[0]] or classes["cell_m"] != cell_m:
        sys.exit("class32 does not match the l2 grid")
    grid = {
        "size": [h.shape[1], h.shape[0]],
        "spacing_m": cell_m,
        # Layout plane: x east, y north, origin at the north-west corner.
        "origin": [0.0, cell_m * h.shape[0]],
        "source": str(src.resolve()),
        "source_array": "arrays.npz:l2",
    }
    wind = layout_wind(src)
    if wind is not None:
        grid["wind_from_deg"] = wind
    classes_u8 = np.fromfile(src / classes["file"], np.uint8)
    write(out, grid, {"elevation.f32": h, "classes.u8": classes_u8})


def chain_l2(src: Path, out: Path) -> None:
    m = world_manifest(src)
    h = np.load(src / "arrays.npz")["l2"].astype("<f4")
    w4, h4 = m["size"]
    k = h4 // h.shape[0]
    cell_m = float(m["spacing_m"]) * k
    classes = np.fromfile(src / "classes.u8", np.uint8).reshape(h4, w4)
    grid = {
        "size": [h.shape[1], h.shape[0]],
        "spacing_m": cell_m,
        "origin": [0.0, cell_m * h.shape[0]],
        "source": str(src.resolve()),
        "source_array": "arrays.npz:l2",
        "classes": f"classes.u8, most common per {k} x {k} block",
    }
    wind = world_wind(m)
    if wind is not None:
        grid["wind_from_deg"] = wind
    write(out, grid, {"elevation.f32": h, "classes.u8": block_mode(classes, k)})


def world(src: Path, out: Path) -> None:
    m = world_manifest(src)
    w, h = m["size"]
    cell_m = float(m["spacing_m"])
    grid = {
        "size": [w, h],
        "spacing_m": cell_m,
        "origin": [0.0, cell_m * h],
        "source": str(src.resolve()),
    }
    wind = world_wind(m)
    if wind is not None:
        grid["wind_from_deg"] = wind
    write(out, grid, {"elevation.f32": src / "elevation.f32", "classes.u8": src / "classes.u8"})


def band(src: Path, first: int, rows: int, halo: int, out: Path) -> None:
    m = world_manifest(src)
    w, h = m["size"]
    cell_m = float(m["spacing_m"])
    base = Path(m["source_world"])
    bm = world_manifest(base)
    k = round(float(bm["spacing_m"]) / cell_m)
    if first % k or rows % k or first + rows > h or 2 * halo >= rows:
        sys.exit(f"rows must be whole {k}-row blocks inside the world, with a smaller halo")
    elevation = np.memmap(src / "elevation.f32", "<f4", mode="r", shape=(h, w))
    bw, bh = bm["size"]
    classes = np.memmap(base / "classes.u8", np.uint8, mode="r", shape=(bh, bw))
    grid = {
        "size": [w, rows],
        "spacing_m": cell_m,
        "origin": [0.0, cell_m * h],
        "offset_m": [0.0, cell_m * first],
        "core_rows": [halo, rows - halo],
        "classes_cell_m": cell_m * k,
        "source": f"{src.resolve()} rows {first}-{first + rows}",
    }
    files = {
        "elevation.f32": np.ascontiguousarray(elevation[first : first + rows]),
        "classes.u8": np.ascontiguousarray(classes[first // k : (first + rows) // k]),
    }
    write(out, grid, files)


def main() -> None:
    args = sys.argv[1:]
    if args and args[0] in ("--chain-l2", "--world", "--band") and Path(args[-1]).exists():
        sys.exit(f"{args[-1]} already exists")
    match args:
        case ["--chain-l2", src, out]:
            chain_l2(Path(src), Path(out))
        case ["--world", src, out]:
            world(Path(src), Path(out))
        case ["--band", src, first, rows, halo, out]:
            band(Path(src), int(first), int(rows), int(halo), Path(out))
        case [src, out] if not src.startswith("--"):
            if Path(out).exists():
                sys.exit(f"{out} already exists")
            collage(Path(src), Path(out))
        case _:
            sys.exit(__doc__)


if __name__ == "__main__":
    main()
