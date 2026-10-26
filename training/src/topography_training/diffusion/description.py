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


RIM_RAYS = 72


def rim_crests(h: np.ndarray, cell_m: float, at, diameter_m: float):
    """The highest ground on rays from `at` out to 0.8 of diameter_m: azimuths (degrees
    clockwise from north), crest heights and crest distances (m)."""
    from scipy import ndimage

    az = np.arange(RIM_RAYS) * 360.0 / RIM_RAYS
    radii = np.arange(0.0, 0.8 * diameter_m, cell_m / 2)
    t = np.radians(az)
    px, py = at[0] + np.outer(np.sin(t), radii), at[1] - np.outer(np.cos(t), radii)
    prof = ndimage.map_coordinates(h.astype(np.float64), [py / cell_m - 0.5, px / cell_m - 0.5], order=1)
    k = np.argmax(prof, 1)
    return az, prof[np.arange(RIM_RAYS), k], radii[k]


def caldera(h: np.ndarray, cell_m: float, f: dict) -> dict:
    """The largest closed hollow reaching within the caldera's radius of the volcano's `at`:
    its floor and lowest point, its breach (the lowest rim, where it would spill) and centre;
    and on rays from that centre, the median rim crest and the crest's equivalent diameter.
    Empty if there is no hollow."""
    from scipy import ndimage

    from .hydrology import flood

    d = float(f.get("caldera_diameter_m", 1000.0))
    r = max(2, int(1.5 * d / cell_m))  # the window's edges are the outlets
    i, j = int(f["at"][0] / cell_m), int(f["at"][1] / cell_m)
    j0, i0 = max(0, j - r), max(0, i - r)
    win = h[j0 : j + r + 1, i0 : i + r + 1].astype(np.float64)
    filled, _, _ = flood(win, np.zeros(win.shape, bool))
    labels, count = ndimage.label(filled - win > 0.05)
    yy, xx = np.mgrid[0 : win.shape[0], 0 : win.shape[1]]
    near = np.hypot((xx + i0 + 0.5) * cell_m - f["at"][0], (yy + j0 + 0.5) * cell_m - f["at"][1]) <= d / 2
    ids = np.unique(labels[near & (labels > 0)])
    if not ids.size:
        return {}
    k = ids[np.argmax(ndimage.sum(np.ones(win.shape), labels, ids))]
    on = labels == k
    at = [float((xx[on].mean() + i0 + 0.5) * cell_m), float((yy[on].mean() + j0 + 0.5) * cell_m)]
    low = np.argmin(np.where(on, win, np.inf))
    lj, li = np.unravel_index(int(low), win.shape)
    _, crest, dist = rim_crests(h, cell_m, at, d)
    # The crest ring's area, from its distance on each ray.
    area = float(np.mean(dist**2)) * np.pi
    return {"floor_m": float(win[on].min()), "breach_m": float(filled[on].min()), "rim_m": float(np.median(crest)),
            "diameter_m": 2 * float(np.sqrt(area / np.pi)), "at_m": at,
            "low_at_m": [float((li + i0 + 0.5) * cell_m), float((lj + j0 + 0.5) * cell_m)]}


def _feature(f: dict, h: np.ndarray, cell_m: float) -> dict:
    target = {k: v for k, v in f.items() if k not in ("id", "kind", "name", "at", "path", "note")}
    out = {"id": f["id"], "kind": f["kind"], "name": f.get("name", f["id"]), "target": target}
    if "at" in f:
        out["at_m"] = f["at"]
    if "path" in f:
        out["path_m"] = f["path"]
    if f["kind"] == "volcano":
        out["measured"] = {"summit_m": round(float(_window(h, cell_m, f["at"], f["radius_m"]).max()), 1)}
        if "caldera_diameter_m" in f:
            c = caldera(h, cell_m, f)
            out["measured"] |= {"caldera_floor_m": round(c["floor_m"], 1), "caldera_rim_m": round(c["rim_m"], 1),
                                "caldera_breach_m": round(c["breach_m"], 1),
                                "caldera_diameter_m": round(c["diameter_m"]),
                                "caldera_at_m": [round(v) for v in c["at_m"]]} if c else {"caldera_floor_m": None}
    elif "path" in f:
        cells = [_window(h, cell_m, p, cell_m) for p in f["path"]]
        out["measured"] = {"min_m": round(float(min(c.min() for c in cells)), 1),
                           "max_m": round(float(max(c.max() for c in cells)), 1)}
    return out


def measure(spec: dict, h: np.ndarray, cell_m: float, level: str, region_map: np.ndarray | None = None) -> dict:
    """Targets from the layout and values measured on surface h (north-up, cell_m metres).
    `region_map` (the collage's `region32.u8`, resampled to h's grid: 1 to N in spec order,
    0 none) replaces the authored outlines, so every land cell counts in one region."""
    from . import layout

    names, tops, clearance = layout.flight(spec)
    bands = layout.band_map(h, cell_m, spec)
    n, land = h.shape[0], h >= 0
    regions = []
    for i, r in enumerate(spec["regions"]):
        inside = region_map == i + 1 if region_map is not None else polygon_sd(r["outline"], n, cell_m) > 0
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
