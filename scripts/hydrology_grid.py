"""Turn a world collage (arrays.npz `l2` at 32 m plus class32.u8) into a hydrology grid dir.

Usage: training/.venv/bin/python -I scripts/hydrology_grid.py <collage-dir> <new-grid-dir>
Then: topography world hydrology --grid <new-grid-dir> --out <new-output-dir>
The layout's wind_from_deg (from the collage's layout.json) goes into grid.json.
"""

import json
import shutil
import sys
import tomllib
from pathlib import Path

import numpy as np


def layout_wind(src: Path) -> float | None:
    """`wind_from_deg` of the layout copy the collage keeps in layout.json, if any."""
    path = src / "layout.json"
    if not path.exists():
        return None
    text = json.loads(path.read_text()).get("config_text")
    wind = tomllib.loads(text).get("wind_from_deg") if text else None
    return float(wind) if wind is not None else None


def main() -> None:
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    src, out = Path(sys.argv[1]), Path(sys.argv[2])
    if out.exists():
        sys.exit(f"{out} already exists")
    collage = json.loads((src / "collage.json").read_text())
    h = np.load(src / "arrays.npz")["l2"].astype("<f4")
    cell_m = float(collage["cell_m"])
    classes = collage["class32"]
    if classes["size"] != [h.shape[1], h.shape[0]] or classes["cell_m"] != cell_m:
        sys.exit("class32 does not match the l2 grid")
    stage = out.with_name(out.name + ".partial")
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    h.tofile(stage / "elevation.f32")
    shutil.copyfile(src / classes["file"], stage / "classes.u8")
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
    (stage / "grid.json").write_text(json.dumps(grid, indent=1))
    stage.rename(out)
    print(out)


if __name__ == "__main__":
    main()
