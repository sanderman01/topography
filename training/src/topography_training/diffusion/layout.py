"""Authored world layouts (L0 from `topography world layout`): an annotated overview, the
altitude-tier reachability check on any surface, and a reference sheet of the real
regions at world scale.

python -m topography_training.diffusion.layout overview <l0-dir> [--out png]
python -m topography_training.diffusion.layout reach <l0-dir> [--arrays arrays.npz --key l2] [--out png]
python -m topography_training.diffusion.layout refs <l0-dir> --release <context release.json> [--out png]

Positions in the spec are metres from the north-west corner (x east, y south), so a cell
of a north-up grid with spacing c is (y // c, x // c) for L0 and every cascade level.
"""

from __future__ import annotations

import argparse
import json
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path

import numpy as np

# Terrain legend v3 display colours (catalog/labels/terrain-v3.toml), 0 = unknown.
COLORS = {
    0: (128, 128, 128), 1: (128, 0, 128), 2: (255, 165, 0), 3: (0, 255, 255), 4: (139, 0, 0),
    5: (0, 128, 128), 6: (255, 0, 0), 7: (165, 42, 42), 8: (255, 255, 0), 9: (0, 0, 255),
    10: (200, 230, 255), 11: (255, 90, 40), 12: (120, 90, 200), 13: (190, 110, 40), 14: (200, 80, 200),
    15: (90, 200, 170), 16: (240, 220, 140), 17: (70, 40, 40), 18: (150, 190, 110),
}
NAMES = {
    1: "Doline karst", 2: "Gorges", 3: "Fjords", 4: "Calderas and craters", 5: "Archipelago",
    6: "Alpine ridges", 7: "Badlands and mesas", 8: "Star dunes", 9: "Braided rivers",
    10: "Glaciers", 11: "Stratovolcano", 12: "Coastal cliffs", 13: "Layered canyons", 14: "Cone and tower karst",
    15: "Drowned tower karst", 16: "Crest-train dunes", 17: "Cinder cones and lava", 18: "Plain",
}
TIERS = ("starter", "mid", "specialist")
TIER_RGB = {0: (120, 200, 110), 1: (240, 190, 60), 2: (220, 70, 60)}
SITE_RADIUS_M = 250.0
RIVER_WIDTH_M = 48.0  # authored rivers as water at L2: a channel one or two 32 m cells wide  # a site counts as reached if any cell this close is


@dataclass
class L0:
    spec: dict
    cell_m: float
    mid: np.ndarray
    relief: np.ndarray
    coast: np.ndarray
    cls: np.ndarray
    lake: np.ndarray
    lake_level: np.ndarray
    snowline: np.ndarray
    forest: np.ndarray
    hold: np.ndarray  # 0-1 weight with which L1 sampling holds `mid`


def load(d: Path) -> L0:
    m = json.loads((d / "layout.json").read_text())
    if m.get("kind") != "world_layout_l0" or m.get("schema_version") != 1:
        raise ValueError(f"{d} is not an L0 layout")
    w, h = m["size"]
    f = {"f32": "<f4", "u8": "u1"}
    a = {name: np.fromfile(d / name, f[name.rsplit(".", 1)[1]]).reshape(h, w) for name in m["layers"]
         if not name.endswith(".txt")}
    return L0(tomllib.loads(m["config_text"]), float(m["cell_m"]), a["mid.f32"], a["relief.f32"],
              a["coast.f32"], a["class.u8"], a["lake.u8"] == 255, a["lake-level.f32"],
              a["snowline.f32"], a["forest.u8"],
              a["hold.u8"].astype(np.float32) / 255 if "hold.u8" in a else np.zeros((h, w), np.float32))


def line_cells(path, width_m: float, cell_m: float, n: int) -> np.ndarray:
    """Cells of an n x n grid within width_m / 2 of a polyline (at least the cells it crosses)."""
    c = (np.arange(n) + 0.5) * cell_m
    x, y = np.meshgrid(c, c)
    d = np.full(x.shape, np.inf)
    for (ax, ay), (bx, by) in zip(path, path[1:]):
        dx, dy = bx - ax, by - ay
        t = np.clip(((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy), 0, 1)
        d = np.minimum(d, np.hypot(x - ax - t * dx, y - ay - t * dy))
    return d < max(width_m, cell_m) / 2


def lake_cells(lk: dict, cell_m: float, n: int) -> np.ndarray:
    """Cells of an authored lake on an n x n grid (round or along a valley path)."""
    if "at" in lk:
        c = (np.arange(n) + 0.5) * cell_m
        x, y = np.meshgrid(c, c)
        return np.hypot(x - lk["at"][0], y - lk["at"][1]) < lk["radius_m"]
    return line_cells(lk["path"], lk["width_m"], cell_m, n)


def rivers(spec: dict, cell_m: float, n: int) -> np.ndarray:
    """Authored rivers (gorges with `river = true`) as water cells on an n x n grid."""
    out = np.zeros((n, n), bool)
    for f in spec.get("features", []):
        if f["kind"] == "gorge" and f.get("river"):
            out |= line_cells(f["path"], RIVER_WIDTH_M, cell_m, n)
    return out


def _font(size: int):
    from PIL import ImageFont

    for name in ("/usr/share/fonts/TTF/DejaVuSans.ttf", "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf", "DejaVuSans.ttf"):
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            continue
    return ImageFont.load_default(size=size)


def _label(draw, xy, text, font, fill=(255, 255, 255)):
    draw.text(xy, text, font=font, fill=fill, stroke_width=2, stroke_fill=(0, 0, 0))


def _shade(h: np.ndarray, cell_m: float) -> np.ndarray:
    gy, gx = np.gradient(h, cell_m)
    az, alt = np.radians(315), np.radians(45)
    slope = np.arctan(np.hypot(gx, gy))
    aspect = np.arctan2(-gx, gy)
    return np.clip(np.sin(alt) * np.cos(slope) + np.cos(alt) * np.sin(slope) * np.cos(az - aspect), 0, 1)


def _base(h: np.ndarray, cell_m: float, colors: np.ndarray, sea: np.ndarray, px: int):
    """Colours shaded by a hillshade of h, resized to px; sea darkened by depth."""
    from PIL import Image

    s = 0.35 + 0.65 * _shade(h, cell_m)
    rgb = colors.astype(np.float32) * s[..., None]
    depth = np.clip(-h / 200.0, 0, 1)[..., None]
    water = np.array([70, 120, 190], np.float32) * (1 - 0.5 * depth) * (0.7 + 0.3 * s[..., None])
    rgb = np.where(sea[..., None], water, rgb)
    return Image.fromarray(np.clip(rgb, 0, 255).astype(np.uint8)).resize((px, px), Image.LANCZOS)


def _contours(img, h: np.ndarray, px: int, step: float = 250.0):
    """Darken contour lines of h (land only) on img in place; every fourth line is bolder."""
    from PIL import Image

    big = np.asarray(Image.fromarray(h.astype(np.float32), mode="F").resize((px, px), Image.BILINEAR))
    band = np.floor(big / step)
    edge = np.zeros(big.shape, bool)
    edge[:-1] |= band[:-1] != band[1:]
    edge[:, :-1] |= band[:, :-1] != band[:, 1:]
    edge &= big > 0
    major = edge & (np.mod(np.maximum(band, 0), 4) == 0)
    a = np.asarray(img).astype(np.float32)
    a[edge] *= 0.7
    a[major] *= 0.6
    return Image.fromarray(a.astype(np.uint8))


def _region_anchor(r: dict) -> tuple[float, float]:
    if "outline" in r:
        p = np.array(r["outline"], float)
        return float(p[:, 0].mean()), float(p[:, 1].mean())
    if "center" in r:
        return tuple(r["center"])
    p = r["path"]
    return tuple(p[len(p) // 2])


def overview(l0: L0, out: Path, px: int = 2048) -> Path:
    from PIL import ImageDraw

    spec, n = l0.spec, l0.mid.shape[0]
    k = px / (n * l0.cell_m)  # pixels per metre
    colors = np.array([COLORS[c] for c in range(len(COLORS))], np.uint8)[l0.cls]
    colors = (0.55 * colors + 0.45 * 200).astype(np.uint8)  # pastel, so the shading reads
    colors[l0.lake] = (90, 150, 220)
    img = _contours(_base(l0.mid, l0.cell_m, colors, (l0.mid < 0) & ~l0.lake, px), l0.mid, px)
    d = ImageDraw.Draw(img)
    big, small = _font(26), _font(17)
    P = lambda p: (p[0] * k, p[1] * k)  # noqa: E731

    coast = spec.get("island", {}).get("coast")
    if coast:
        d.line([P(p) for p in coast + coast[:1]], fill=(20, 20, 20), width=2)
    for r in spec.get("ridges", []):
        d.line([P(p) for p in r["path"]], fill=(255, 255, 255), width=1)
    for f in spec.get("features", []):
        kind = f["kind"]
        if kind in ("gorge", "fjord", "braided_river"):
            color = {"gorge": (150, 40, 0), "fjord": (0, 60, 120), "braided_river": (40, 90, 200)}[kind]
            d.line([P(p) for p in f["path"]], fill=color, width=4 if kind == "gorge" and f.get("river") else 2)
            _label(d, P(f["path"][-1]), f["id"], small)
        elif kind in ("volcano", "cone", "peak"):
            x, y = P(f["at"])
            d.polygon([(x, y - 9), (x - 8, y + 6), (x + 8, y + 6)], fill=(0, 0, 0))
            _label(d, (x + 10, y - 10), f"{f['id']} {f['summit_m']:.0f} m", small)
        elif kind == "cones":
            for p in f["points"]:
                x, y = P(p)
                d.polygon([(x, y - 6), (x - 5, y + 4), (x + 5, y + 4)], fill=(60, 20, 0))
        elif kind == "basin":
            x, y = P(f["at"])
            rr = f["radius_m"] * k
            d.ellipse([x - rr, y - rr, x + rr, y + rr], outline=(0, 0, 0), width=2)
            _label(d, (x + rr, y - rr), f"{f['id']} floor {f['floor_m']:.0f} m", small)
    for lk in spec.get("lakes", []):
        x, y = P(lk["at"] if "at" in lk else lk["path"][len(lk["path"]) // 2])
        _label(d, (x - 40, y - 8), f"{lk['id']} {lk['surface_m']:.0f} m", small, (190, 225, 255))
    for p in spec.get("passes", []):
        x, y = P(p["at"])
        c = TIER_RGB[TIERS.index(p["tier"])]
        d.polygon([(x, y - 8), (x + 8, y), (x, y + 8), (x - 8, y)], fill=c, outline=(0, 0, 0))
        _label(d, (x + 10, y - 2), f"{p['id']} {p['height_m']:.0f}", small, c)
    for s in spec.get("sites", []):
        x, y = P(s["at"])
        d.rectangle([x - 5, y - 5, x + 5, y + 5], fill=(255, 255, 255), outline=(0, 0, 0))
    for r in spec.get("regions", []):
        x, y = P(_region_anchor(r))
        _label(d, (x - 60, y - 14), r["name"], big)

    # Legend, scale bar and north arrow.
    present = sorted(set(np.unique(l0.cls).tolist()) - {0})
    y0 = 12
    for c in present:
        d.rectangle([px - 300, y0, px - 278, y0 + 18], fill=COLORS[c], outline=(0, 0, 0))
        _label(d, (px - 270, y0 - 1), f"{c} {NAMES[c]}", small)
        y0 += 24
    for i, t in enumerate(TIERS[:2]):
        d.polygon([(px - 289, y0 + 9 - 8), (px - 281, y0 + 9), (px - 289, y0 + 17), (px - 297, y0 + 9)],
                  fill=TIER_RGB[i], outline=(0, 0, 0))
        _label(d, (px - 270, y0), f"{t} pass", small)
        y0 += 24
    bar = 5000 * k
    d.rectangle([20, px - 40, 20 + bar, px - 32], fill=(255, 255, 255), outline=(0, 0, 0))
    _label(d, (20, px - 66), "5 km", small)
    d.polygon([(px - 40, 30), (px - 50, 60), (px - 30, 60)], fill=(255, 255, 255), outline=(0, 0, 0))
    _label(d, (px - 46, 64), "N", small)
    _label(d, (20, 12), f"{spec['name']} L0 layout: mid surface (contours 250 m), classes, features", big)
    img.save(out)
    return out


# ---- reachability -------------------------------------------------------------------------


def open_sea(h: np.ndarray) -> np.ndarray:
    from scipy import ndimage

    labels, _ = ndimage.label(h < 0)
    edge = np.unique(np.concatenate([labels[0], labels[-1], labels[:, 0], labels[:, -1]]))
    return np.isin(labels, edge[edge > 0])


def tier_map(h: np.ndarray, spec: dict) -> np.ndarray:
    """Lowest tier (0 starter, 1 mid, 2 specialist) that reaches each cell from the open
    sea, flying no lower than `clearance_m` above the ground and no higher than the tier's
    ceiling."""
    from scipy import ndimage

    t = spec["tiers"]
    sea = open_sea(h)
    out = np.full(h.shape, 2, np.int8)
    for i, ceiling in reversed(list(enumerate((t["starter_ceiling_m"], t["mid_ceiling_m"])))):
        flyable = sea | (h + t["clearance_m"] <= ceiling)
        labels, _ = ndimage.label(flyable)
        ids = np.unique(labels[sea])
        out[np.isin(labels, ids[ids > 0])] = i
    return out


def _at(tiers: np.ndarray, cell_m: float, p, radius_m: float = SITE_RADIUS_M) -> int:
    r = max(1, int(radius_m / cell_m))
    i, j = int(p[0] / cell_m), int(p[1] / cell_m)
    win = tiers[max(0, j - r) : j + r + 1, max(0, i - r) : i + r + 1]
    return int(win.min())


def reach(h: np.ndarray, cell_m: float, spec: dict) -> tuple[str, np.ndarray, int]:
    """Report of tier reachability against the spec; returns (text, tier map, warnings)."""
    tiers = tier_map(h, spec)
    land = h >= 0
    lines, warnings = [], 0

    def check(ok: bool, text: str):
        nonlocal warnings
        warnings += not ok
        lines.append(("  ok   " if ok else "  WARN ") + text)

    lines.append(f"Tier reachability at {cell_m:.0f} m (ceilings {spec['tiers']['starter_ceiling_m']:.0f} / "
                 f"{spec['tiers']['mid_ceiling_m']:.0f} m, clearance {spec['tiers']['clearance_m']:.0f} m)")
    for i, t in enumerate(TIERS):
        lines.append(f"  info land reached by {t}: {(land & (tiers <= i)).sum() / max(land.sum(), 1):.1%}")
    gated = {tuple(d["at"]): d for d in spec.get("destinations", [])}
    for d in spec.get("destinations", []):
        got = _at(tiers, cell_m, d["at"])
        check(TIERS[got] == d["tier"], f"destination {d['id']:<18} first reached by {TIERS[got]} (spec {d['tier']})")
    for s in spec.get("sites", []):
        want = gated.get(tuple(s["at"]), {"tier": "starter"})["tier"]
        got = _at(tiers, cell_m, s["at"])
        check(TIERS[got] == want, f"site {s['region']:<24} first reached by {TIERS[got]} (spec {want})")
    for p in spec.get("passes", []):
        r = max(1, int(500 / cell_m))
        i, j = int(p["at"][0] / cell_m), int(p["at"][1] / cell_m)
        low = float(h[max(0, j - r) : j + r + 1, max(0, i - r) : i + r + 1].min())
        got = _at(tiers, cell_m, p["at"], 500)
        check(got <= TIERS.index(p["tier"]),
              f"pass {p['id']:<18} lowest ground {low:5.0f} m within 500 m, first reached by {TIERS[got]} (spec {p['tier']})")
    lines.append(f"{warnings} warning(s)")
    return "\n".join(lines) + "\n", tiers, warnings


def reach_png(h: np.ndarray, cell_m: float, tiers: np.ndarray, spec: dict, out: Path, px: int = 2048):
    from PIL import ImageDraw

    colors = np.array([TIER_RGB[i] for i in range(3)], np.uint8)[tiers]
    img = _base(h, cell_m, colors, open_sea(h), px)
    d = ImageDraw.Draw(img)
    k, f = px / (h.shape[0] * cell_m), _font(17)
    for p in spec.get("passes", []) + spec.get("destinations", []):
        x, y = p["at"][0] * k, p["at"][1] * k
        d.ellipse([x - 6, y - 6, x + 6, y + 6], outline=(0, 0, 0), width=2)
        _label(d, (x + 8, y - 8), p["id"], f)
    for s in spec.get("sites", []):
        x, y = s["at"][0] * k, s["at"][1] * k
        d.rectangle([x - 5, y - 5, x + 5, y + 5], fill=(255, 255, 255), outline=(0, 0, 0))
    _label(d, (20, 12), "green: starter, amber: mid tier, red: specialist only", _font(26))
    img.save(out)


# ---- reference sheet ----------------------------------------------------------------------


def _footprint_m(r: dict) -> tuple[float, float]:
    if "outline" in r:
        p = np.array(r["outline"], float)
        return float(np.ptp(p[:, 0])), float(np.ptp(p[:, 1]))
    if "center" in r:
        return 2 * r["radius_m"], 2 * r["radius_m"]
    p = np.array(r["path"], float)
    return float(np.ptp(p[:, 0]) + r["width_m"]), float(np.ptp(p[:, 1]) + r["width_m"])


def refs(l0: L0, release: Path, out: Path, m_per_px: float = 32.0) -> tuple[Path, list[str]]:
    """Each region's layout crop beside its real references, cropped to the same size and
    shown at the same scale."""
    from PIL import Image, ImageDraw

    from . import cascade
    from .cascade_eval import shade

    regions = [r for r in l0.spec["regions"] if r.get("refs")]
    wanted = {i for r in regions for i in r["refs"]}
    real = {r.name: r for r in cascade.load_release(release, wanted)}
    missing = sorted(wanted - real.keys())
    rows, label_w, pad, font = [], 300, 12, _font(18)
    for r in regions:
        w_m, h_m = _footprint_m(r)
        x0, y0 = _region_anchor(r)
        tiles = []
        # The authored L0 at the same scale.
        c = l0.cell_m
        i0, i1 = int(max(0, (x0 - w_m / 2) / c)), int(min(l0.mid.shape[1], (x0 + w_m / 2) / c))
        j0, j1 = int(max(0, (y0 - h_m / 2) / c)), int(min(l0.mid.shape[0], (y0 + h_m / 2) / c))
        crop = l0.mid[j0:j1, i0:i1]
        rgb = shade(crop, c, crop < 0)
        tiles.append(("layout (L0)", Image.fromarray(rgb).resize(
            (max(1, int(crop.shape[1] * c / m_per_px)), max(1, int(crop.shape[0] * c / m_per_px))), Image.BILINEAR)))
        for ref in r["refs"]:
            if ref not in real:
                continue
            g = real[ref]
            hgt = np.nan_to_num(g.height, nan=0.0)
            step = max(1, round(m_per_px / g.spacing_m))
            hw, hh = int(w_m / g.spacing_m / 2), int(h_m / g.spacing_m / 2)
            cy, cx = hgt.shape[0] // 2, hgt.shape[1] // 2
            sub = hgt[max(0, cy - hh) : cy + hh : step, max(0, cx - hw) : cx + hw : step]
            sea = (g.water | open_sea(hgt))[max(0, cy - hh) : cy + hh : step, max(0, cx - hw) : cx + hw : step]
            tiles.append((ref, Image.fromarray(shade(sub, g.spacing_m * step, sea))))
        rows.append((r, tiles, w_m, h_m))
    width = max(label_w + sum(t.width + pad for _, t in tiles) for _, tiles, _, _ in rows) + pad
    height = sum(max(t.height for _, t in tiles) + 40 for _, tiles, _, _ in rows) + 60
    sheet = Image.new("RGB", (width, height), (30, 30, 30))
    d = ImageDraw.Draw(sheet)
    _label(d, (pad, 12), f"{l0.spec['name']}: regions beside their references, same crop size, {m_per_px:.0f} m per pixel",
           _font(24))
    y = 60
    for r, tiles, w_m, h_m in rows:
        _label(d, (pad, y + 20), r["name"], font)
        _label(d, (pad, y + 46), f"class {r['class_id']}, {w_m / 1000:.0f} x {h_m / 1000:.0f} km", font, (200, 200, 200))
        x = label_w
        for name, t in tiles:
            sheet.paste(t, (x, y + 24))
            _label(d, (x, y), name, font, (230, 230, 160))
            x += t.width + pad
        y += max(t.height for _, t in tiles) + 40
    sheet.save(out)
    return out, missing


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = p.add_subparsers(dest="command", required=True)
    for name in ("overview", "reach", "refs"):
        s = sub.add_parser(name)
        s.add_argument("l0", type=Path, help="output of `topography world layout`")
        s.add_argument("--out", type=Path)
        if name == "reach":
            s.add_argument("--arrays", type=Path, help="cascade_chain arrays.npz (default: the L0 mid surface)")
            s.add_argument("--key", default="l2", help="array in --arrays: l1 (128 m), l2 (32 m) or l3 (4 m)")
        if name == "refs":
            s.add_argument("--release", type=Path, required=True, help="context release.json")
    args = p.parse_args(argv)
    l0 = load(args.l0)
    if args.command == "overview":
        print(overview(l0, args.out or args.l0 / "overview.png"))
    elif args.command == "reach":
        if args.arrays:
            h = np.load(args.arrays)[args.key].astype(np.float32)
            cell = l0.cell_m * l0.mid.shape[0] / h.shape[0]
        else:
            h, cell = l0.mid, l0.cell_m
        text, tiers, warnings = reach(h, cell, l0.spec)
        print(text, end="")
        out = args.out or (args.arrays.parent / f"reach-{args.key}.png" if args.arrays else args.l0 / "reach.png")
        reach_png(h, cell, tiers, l0.spec, out)
        print(out)
    else:
        out, missing = refs(l0, args.release, args.out or args.l0 / "refs.png")
        if missing:
            print(f"warning: not in the release: {', '.join(missing)}", file=sys.stderr)
        print(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
