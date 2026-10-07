"""Real exemplars for small hero landforms: craters, calderas and cinder cones held at L1.

python -m topography_training.diffusion.exemplar find --release <context release.json> --out <toml>

`find` locates examples in accepted GLO-30 context regions and writes the catalog
(catalog/exemplars/exemplars-v1.toml): summit craters near a volcano's top, calderas as
the biggest closed depression near a region's centre, single cones and cone fields as
windows with many small prominent hills. The chain (`compose`) then fills each painted
caldera or crater (class 4) and cinder-cone patch (class 17) with the 128 m relief of a
real example, scaled to the painted patch for craters and calderas and at its own size
for cones, and holds it at 128 m while L1 samples. Large forms come from real terrain;
the painter paints only the class and height.
"""

from __future__ import annotations

import argparse
import json
import sys
import tomllib
from pathlib import Path

import numpy as np

CATALOG = Path(__file__).resolve().parents[4] / "catalog" / "exemplars" / "exemplars-v1.toml"
CELL_M = 128.0
CALDERA, CRATER, CONE, CONE_FIELD = "caldera", "crater", "cone", "cone-field"
DROWNED = "drowned-karst"  # towers and islands rising from a shallow sea
# Where each kind is searched for (accepted context-v5 regions).
SEARCH = {
    CRATER: ["fuji", "kilimanjaro", "mayon", "taranaki-glo", "villarrica", "etna", "teide", "shasta"],
    CALDERA: ["crater-lake", "aniakchak", "okmok", "mashu", "valles-caldera", "ngorongoro", "cerro-galan",
              "bromo-tengger", "rinjani", "tambora", "aso"],
    # Checked by eye (2026-10-07): these give round cones and cone clusters at 128 m.
    CONE: ["craters-of-the-moon", "etna", "teide", "kilimanjaro", "mayon"],
    CONE_FIELD: ["kilimanjaro", "etna"],
    DROWNED: ["ha-long-bay", "phang-nga-bay"],
}
CLASS_KINDS = {4: (CRATER, CALDERA), 17: (CONE, CONE_FIELD), 15: (DROWNED,)}
CRATER_MAX_M = 2500.0  # painted class 4 patches narrower than this are summit craters
CONE_MAX_M = 1500.0  # painted class 17 patches narrower than this are single cones
FIELD_M = 6000.0  # side of a cone-field exemplar window
HOLD_WEIGHT = 0.95
MIN_CELLS = 4  # painted patches smaller than this are ignored
DROWNED_SHORE_M = 2000.0  # drowned-karst exemplars reach this far onto neighbouring karst
KARST_CLASSES = (1, 14)
ANY_SHORE_M = 800.0  # and this far onto any other shore of the bay
MAX_ELONGATION = 4.0  # bounding-box side squared over area


def _height128(release: Path, region: str) -> np.ndarray:
    from . import cascade

    r = cascade.load_release(release, {region})[0]
    return np.nan_to_num(cascade.block_mean(r.height, 4), nan=0.0)


def _depressions(h: np.ndarray):
    from scipy import ndimage

    from . import hydrology

    filled, _, _ = hydrology.flood(h, np.zeros(h.shape, bool))
    depth = filled - h
    labels, count = ndimage.label(depth > 5.0)
    return depth, labels, count


def _peaks(h: np.ndarray, lo_m: float = 40.0, hi_m: float = 350.0) -> np.ndarray:
    """Cone score per cell: hills standing 40-350 m above the ground within 1 km, weighted
    by how round they are (rings of equal height around the top), 0 elsewhere."""
    from scipy import ndimage

    top = h == ndimage.maximum_filter(h, size=5)
    prom = h - ndimage.minimum_filter(h, size=17)
    ok = top & (prom >= lo_m) & (prom <= hi_m)
    score = np.zeros(h.shape, np.float32)
    yy, xx = np.mgrid[-6:7, -6:7]
    ring = np.rint(np.hypot(yy, xx)).astype(int)
    for y, x in zip(*np.nonzero(ok)):
        if not (6 <= y < h.shape[0] - 6 and 6 <= x < h.shape[1] - 6):
            continue
        w = h[y - 6 : y + 7, x - 6 : x + 7]
        spread = np.mean([w[ring == r].std() for r in range(1, 7)])
        drop = h[y, x] - np.mean(w[ring == 5])
        score[y, x] = max(drop, 0.0) / (1.0 + spread / 10.0)
    return score


def _rim_radius(h: np.ndarray, y: float, x: float, hollow_m: float) -> float:
    """Distance of the highest ring around a hollow (its rim): breached calderas have a
    small closed depression but a wide rim."""
    yy, xx = np.indices(h.shape)
    r = np.hypot(yy - y, xx - x) * CELL_M
    edges = np.arange(0.0, 3 * hollow_m + 3000.0, CELL_M)
    ring = np.array([h[(r >= a) & (r < a + CELL_M)].mean() if ((r >= a) & (r < a + CELL_M)).any() else -np.inf
                     for a in edges])
    ok = edges >= 0.5 * hollow_m
    return float(edges[ok][int(np.argmax(ring[ok]))] + CELL_M / 2)


def find(release: Path) -> list[dict]:
    from scipy import ndimage

    found = []
    regions = {r["id"] for r in json.loads(release.read_text())["regions"]}
    for kind, names in SEARCH.items():
        for name in names:
            if name not in regions:
                continue
            h = _height128(release, name)
            n = h.shape[0]
            if kind in (CRATER, CALDERA):
                depth, labels, count = _depressions(h)
                if not count:
                    continue
                idx = np.arange(1, count + 1)
                cy, cx = (np.unravel_index(int(h.argmax()), h.shape) if kind == CRATER else (n / 2, n / 2))
                com = np.array(ndimage.center_of_mass(np.ones_like(h), labels, idx))
                dist = np.hypot(com[:, 0] - cy, com[:, 1] - cx) * CELL_M
                area = ndimage.sum(np.ones_like(h), labels, idx) * CELL_M**2
                vol = ndimage.sum(depth, labels, idx)
                near = dist < (2000.0 if kind == CRATER else 8000.0)
                if kind == CALDERA:
                    near &= area > 3e6
                if not near.any():
                    continue
                k = int(np.argmax(np.where(near, vol, -1)))
                y, x = com[k]
                radius = _rim_radius(h, y, x, float(np.sqrt(area[k] / np.pi)))
                found.append({"kind": kind, "region": name, "row": round(float(y), 1), "col": round(float(x), 1),
                              "radius_m": round(radius), "depth_m": round(float(depth[labels == k + 1].max()))})
            elif kind == DROWNED:  # the window with the most islands
                labels, count = ndimage.label(h > 0)
                size = ndimage.sum(np.ones_like(h), labels, np.arange(1, count + 1)) * CELL_M**2
                isl = np.isin(labels, np.flatnonzero(size < 2e6) + 1)  # towers and islets, not big islands
                m = int(FIELD_M / CELL_M)
                dens = ndimage.uniform_filter(isl.astype(np.float32), m) * m * m
                dens[: m // 2], dens[-m // 2 :], dens[:, : m // 2], dens[:, -m // 2 :] = 0, 0, 0, 0
                y, x = np.unravel_index(int(dens.argmax()), dens.shape)
                found.append({"kind": kind, "region": name, "row": float(y), "col": float(x),
                              "radius_m": round(FIELD_M / 2), "island_cells": int(round(dens.max()))})
            else:
                score = _peaks(h)
                m = int(FIELD_M / CELL_M)
                score[:m // 2], score[-m // 2 :], score[:, : m // 2], score[:, -m // 2 :] = 0, 0, 0, 0
                if kind == CONE:  # the roundest, most prominent cone
                    y, x = np.unravel_index(int(score.argmax()), score.shape)
                    found.append({"kind": kind, "region": name, "row": float(y), "col": float(x), "radius_m": 700,
                                  "score": round(float(score.max()))})
                    continue
                cones = (score > 20).astype(np.float32)  # windows with the most good cones
                dens = ndimage.uniform_filter(cones, m) * m * m
                y, x = np.unravel_index(int(dens.argmax()), dens.shape)
                found.append({"kind": kind, "region": name, "row": float(y), "col": float(x),
                              "radius_m": round(FIELD_M / 2), "cones": int(round(dens.max()))})
    return found


def write_catalog(entries: list[dict], release: Path, out: Path) -> None:
    lines = ["# Real exemplars for held hero landforms (docs/plans/biome-forms-v1.md), found by",
             "# `python -m topography_training.diffusion.exemplar find`; positions are 128 m cells",
             "# of the region (row 0 north). Review by eye before relying on an entry.",
             "schema_version = 1", f'release = "{release}"', ""]
    for e in entries:
        lines.append("[[exemplars]]")
        for k, v in e.items():
            lines.append(f'{k} = "{v}"' if isinstance(v, str) else f"{k} = {v}")
        lines.append("")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("\n".join(lines))


def load(path: Path = CATALOG) -> tuple[Path, list[dict]]:
    t = tomllib.loads(path.read_text())
    release = Path(t["release"])
    if not release.is_absolute():
        release = path.resolve().parents[2] / release
    return release, t["exemplars"]


def residual(release: Path, e: dict, half_m: float) -> np.ndarray:
    """The exemplar's 128 m relief above its smoothed surroundings, a square of
    2 * half_m around it (edges repeat the region's)."""
    from .data import smooth

    h = _height128(release, e["region"])
    r = int(np.ceil(half_m / CELL_M))
    rows = np.clip(np.arange(int(e["row"]) - r, int(e["row"]) + r + 1), 0, h.shape[0] - 1)
    cols = np.clip(np.arange(int(e["col"]) - r, int(e["col"]) + r + 1), 0, h.shape[1] - 1)
    win = h[np.ix_(rows, cols)]
    res = win - smooth(win, _base_sigma(e["radius_m"]))
    # Fade to zero toward the window edge, so no square shows.
    yy, xx = np.indices(res.shape) - r
    fade = np.clip((r - np.hypot(yy, xx)) / (0.35 * r), 0, 1)
    return (res * fade * fade * (3 - 2 * fade)).astype(np.float32)


def _base_sigma(radius_m: float) -> float:
    """Smoothing (cells) that separates a landform of this radius from its surroundings."""
    return max(2.0, 1.2 * radius_m / CELL_M)


def _hollow(h: np.ndarray) -> float:
    """Depth of the deepest closed depression (m)."""
    from . import hydrology

    filled, _, _ = hydrology.flood(h.astype(np.float64), np.zeros(h.shape, bool))
    return float((filled - h).max())


def compose(l0, target: np.ndarray, seed: int = 0, path: Path = CATALOG):
    """(fine target, weight, report lines): painted calderas, craters and cinder cones of an
    authored layout filled with real exemplars in `target` (the 128 m hold target)."""
    from scipy import ndimage

    from .data import smooth

    out, weight, used = target.astype(np.float32).copy(), np.zeros(target.shape, np.float32), []
    if not path.exists():
        return out, weight, ["no exemplar catalog"]
    release, entries = load(path)
    rng = np.random.default_rng(seed)
    for cls, kinds in CLASS_KINDS.items():
        labels, count = ndimage.label(l0.cls == cls)
        for k in range(1, count + 1):
            patch = labels == k
            area = float(patch.sum()) * CELL_M**2
            ys, xs = np.nonzero(patch)
            span = max(np.ptp(ys), np.ptp(xs)) + 1
            if patch.sum() < MIN_CELLS or (cls != 15 and span * span / patch.sum() > MAX_ELONGATION):
                continue  # slivers and painted outlines are not landforms
            d = 2 * np.sqrt(area / np.pi)
            if cls == 4:
                kind = CRATER if d < CRATER_MAX_M else CALDERA
            elif cls == 17:
                kind = CONE if d < CONE_MAX_M else CONE_FIELD
            else:
                kind = kinds[0]
            pool = [e for e in entries if e["kind"] == kind]
            if not pool:
                continue
            if kind in (CRATER, CALDERA):  # the two closest in size, one picked by seed
                pool = sorted(pool, key=lambda e: abs(np.log(2 * e["radius_m"] / d)))[:2]
            e = pool[int(rng.integers(len(pool)))]
            cy, cx = ndimage.center_of_mass(patch)
            if kind not in (CONE_FIELD, DROWNED):  # scaled to the painted patch, height with it
                s = d / (2 * e["radius_m"])
                res = residual(release, e, 2.0 * e["radius_m"])
                res = ndimage.zoom(res, s, order=1) * float(np.clip(s, 0.5, 2.0))
                sigma = _base_sigma(d / 2)
                reach = patch | (ndimage.distance_transform_edt(~patch) * CELL_M < 0.5 * d)
            else:  # real size; a field covers the painted patch, mirrored if needed
                ys, xs = np.nonzero(patch)
                half = max(e["radius_m"], CELL_M * (max(np.ptp(ys), np.ptp(xs)) / 2 + 4))
                res = residual(release, e, half)
                sigma = _base_sigma(e["radius_m"])
                near = ndimage.distance_transform_edt(~patch) * CELL_M
                reach = patch | (near < 300.0)
                if kind == DROWNED:  # the karst shore too, so the coast breaks into towers and inlets
                    reach |= ((near < DROWNED_SHORE_M) & np.isin(l0.cls, KARST_CLASSES)) | (near < ANY_SHORE_M)
            # Place the exemplar centred on the patch.
            n = res.shape[0]
            y0, x0 = int(round(cy)) - n // 2, int(round(cx)) - n // 2
            ex = np.zeros(target.shape, np.float32)
            have = np.zeros(target.shape, bool)
            ty, tx = slice(max(y0, 0), min(y0 + n, target.shape[0])), slice(max(x0, 0), min(x0 + n, target.shape[1]))
            ex[ty, tx] = res[ty.start - y0 : ty.stop - y0, tx.start - x0 : tx.stop - x0]
            have[ty, tx] = True
            m = np.clip(smooth((reach & have).astype(np.float32), 1.5), 0, 1)
            if kind in (CRATER, CALDERA):  # at least as deep as painted (128 m blurs real craters)
                box = np.s_[max(int(cy) - n, 0) : int(cy) + n, max(int(cx) - n, 0) : int(cx) + n]
                base0 = smooth(target.astype(np.float32), sigma)
                painted = _hollow(target[box])
                if _hollow((base0 + ex)[box]) < painted:  # bisect the factor that matches it
                    lo, hi = 1.0, 3.0
                    for _ in range(8):
                        f = (lo + hi) / 2
                        lo, hi = (f, hi) if _hollow((base0 + f * ex)[box]) < painted else (lo, f)
                    ex *= (lo + hi) / 2
            # Cones, cone fields and drowned karst keep the painted height and gain the real
            # relief on top (a wide smoothing would pull in neighbouring mountains); craters
            # and calderas replace the painted local shape.
            keep = kind in (CONE, CONE_FIELD, DROWNED)
            base = target.astype(np.float32) if keep else smooth(target.astype(np.float32), sigma)
            out = (out + m * (base + ex - out)).astype(np.float32)
            weight = np.maximum(weight, m * HOLD_WEIGHT)
            used.append(f"class {cls} patch at ({cy:.0f}, {cx:.0f}), {d:.0f} m: {kind} from {e['region']}")
    return out, weight, used


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = p.add_subparsers(dest="command", required=True)
    f = sub.add_parser("find")
    f.add_argument("--release", type=Path, required=True)
    f.add_argument("--out", type=Path, default=CATALOG)
    args = p.parse_args(argv)
    entries = find(args.release)
    root = CATALOG.parents[2]
    rel = args.release.resolve()
    write_catalog(entries, rel.relative_to(root) if rel.is_relative_to(root) else rel, args.out)
    for e in entries:
        print(e)
    print(f"wrote {args.out} ({len(entries)} exemplars)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
