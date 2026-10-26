"""World description: a layout's regions, passes and features with their targets and the
values measured on a surface (docs/plans/fujix-v1.md, schemas/world-description-v1.md)."""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np

SCHEMA_VERSION = 1
PASS_RADIUS_M = 500.0  # a pass is its lowest ground this close to its point


def polygon_sd(outline, n: int, cell_m: float) -> np.ndarray:
    """Metres from a polygon outline (metres from the north-west corner), positive inside."""
    from PIL import Image, ImageDraw
    from scipy import ndimage

    img = Image.new("1", (n, n), 0)
    ImageDraw.Draw(img).polygon([(x / cell_m, y / cell_m) for x, y in outline], fill=1)
    inside = np.array(img, bool)
    return ((ndimage.distance_transform_edt(inside) - ndimage.distance_transform_edt(~inside)) * cell_m).astype(
        np.float32)


def _window(h: np.ndarray, cell_m: float, at, radius_m: float) -> np.ndarray:
    r = max(1, int(radius_m / cell_m))
    i, j = int(at[0] / cell_m), int(at[1] / cell_m)
    return h[max(0, j - r) : j + r + 1, max(0, i - r) : i + r + 1]


def _feature(f: dict, h: np.ndarray, cell_m: float) -> dict:
    target = {k: v for k, v in f.items() if k not in ("id", "kind", "name", "at", "path", "note")}
    out = {"id": f["id"], "kind": f["kind"], "name": f.get("name", f["id"]), "target": target}
    if "at" in f:
        out["at_m"] = f["at"]
    if "path" in f:
        out["path_m"] = f["path"]
    if f["kind"] == "volcano":
        win = _window(h, cell_m, f["at"], f["radius_m"])
        j, i = np.unravel_index(int(np.argmax(win)), win.shape)
        r0 = max(1, int(f["radius_m"] / cell_m))
        top = (max(0, int(f["at"][0] / cell_m) - r0) + i, max(0, int(f["at"][1] / cell_m) - r0) + j)
        crater = _window(h, cell_m, (top[0] * cell_m, top[1] * cell_m), f.get("caldera_diameter_m", 1000) / 2)
        out["measured"] = {"summit_m": round(float(win.max()), 1), "caldera_floor_m": round(float(crater.min()), 1)}
    elif "path" in f:
        cells = [_window(h, cell_m, p, cell_m) for p in f["path"]]
        out["measured"] = {"min_m": round(float(min(c.min() for c in cells)), 1),
                           "max_m": round(float(max(c.max() for c in cells)), 1)}
    return out


def measure(spec: dict, h: np.ndarray, cell_m: float, level: str, masks: list[np.ndarray] | None = None) -> dict:
    """Targets from the layout and values measured on surface h (north-up, cell_m metres).
    `masks` replaces the region outlines with the builder's own region cells (one per region)."""
    from . import layout

    names, tops, clearance = layout.flight(spec)
    bands = layout.band_map(h, cell_m, spec)
    n, land = h.shape[0], h >= 0
    regions = []
    for i, r in enumerate(spec["regions"]):
        inside = masks[i] if masks is not None else polygon_sd(r["outline"], n, cell_m) > 0
        cells = inside & land
        hl = h[cells]
        regions.append({
            "id": r["id"], "name": r["name"], "class_id": r["class_id"], "terrain": r.get("terrain", ""),
            "references": sorted({p["source"] for p in r.get("pieces", [])}), "outline_m": r["outline"],
            "target": {k: r.get(k) for k in ("area_km2", "floor_m", "summit_m")},
            "measured": {"land_km2": round(float(cells.sum()) * cell_m**2 / 1e6, 1),
                         **({"p5_m": round(float(np.percentile(hl, 5)), 1), "p95_m": round(float(np.percentile(hl, 95)), 1),
                             "max_m": round(float(hl.max()), 1)} if hl.size else {})},
        })
    passes = []
    for p in spec.get("passes", []):
        got = layout.band_at(bands, cell_m, p["at"], PASS_RADIUS_M)
        passes.append({"id": p["id"], "name": p.get("name", p["id"]), "at_m": p["at"],
                       "target": {"band": p.get("band", p.get("tier"))},
                       "measured": {"ground_m": round(float(_window(h, cell_m, p["at"], PASS_RADIUS_M).min()), 1),
                                    "band": names[got]}})
    return {
        "kind": "world_description", "schema_version": SCHEMA_VERSION, "world": spec["name"], "level": level,
        "cell_m": cell_m, "extent_m": n * cell_m,
        "coordinates": "metres from the north-west corner, x east, y south",
        "flight": {"clearance_m": clearance, "bands": [{"name": a, "top_m": t} for a, t in zip(names, tops)]},
        "regions": regions, "passes": passes, "features": [_feature(f, h, cell_m) for f in spec.get("features", [])],
        "rivers": [], "lakes": [], "water_bodies": [],
    }


def markdown(d: dict) -> str:
    lines = [f"# {d['world']}: world description", "",
             f"Level `{d['level']}`, {d['cell_m']:.0f} m cells, {d['extent_m'] / 1000:.1f} km square. "
             f"Positions are {d['coordinates']}.", "", "## Regions", "",
             "| Region | Class | References | Land km² | Ground p5 to p95 m | Highest m |",
             "| --- | ---: | --- | ---: | --- | ---: |"]
    for r in d["regions"]:
        m = r["measured"]
        lines.append(f"| {r['name']} | {r['class_id']} | {', '.join(r['references'])} | {m['land_km2']} | "
                     f"{m.get('p5_m', '')} to {m.get('p95_m', '')} | {m.get('max_m', '')} |")
    lines += ["", "## Passes", "", "| Pass | Band wanted | Band measured | Lowest ground m |", "| --- | --- | --- | ---: |"]
    for p in d["passes"]:
        lines.append(f"| {p['name']} | {p['target']['band']} | {p['measured']['band']} | {p['measured']['ground_m']} |")
    lines += ["", "## Features", "", "| Feature | Kind | Target | Measured |", "| --- | --- | --- | --- |"]
    for f in d["features"]:
        lines.append(f"| {f['name']} | {f['kind']} | {json.dumps(f['target'])} | {json.dumps(f.get('measured', {}))} |")
    return "\n".join(lines) + "\n"


def write(out: Path, desc: dict) -> None:
    (out / "description.json").write_text(json.dumps(desc, indent=1))
    (out / "description.md").write_text(markdown(desc))
