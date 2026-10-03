"""Paint a world layout in an image editor: export an L0 layout to editable PNGs, and import
them back as an L0 that holds nothing but ground height, terrain classes and roughness.

python -m topography_training.diffusion.paint export <l0-dir> <new-paint-dir>
python -m topography_training.diffusion.paint import <paint-dir> <new-l0-dir>

The paint dir holds one GIMP file, layout.xcf, with the layers height, roughness and
classes; GIMP's batch mode (gimp-console) converts between it and PNG layers losslessly.

The painted layout places no lakes, rivers, troughs or held shapes: the cascade models and
the drainage pass make all of those. A band along the map border is always sea.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np

from .layout import COLORS, NAMES

PX = 1024  # painted image side; averaged down to the 128 m L0 grid on import
H_MIN_M, H_MAX_M = -1000.0, 5000.0  # 16-bit height: 0 is H_MIN_M, 65535 is H_MAX_M
RELIEF_MAX_M = 2000.0  # roughness 255
BORDER_M = 1500.0  # the outer band of the map is sea
BORDER_EDGE_M = -200.0  # depth at the map edge
CELL_M = 128.0
# Judgment defaults by class (from the Fujion regions): local relief (m) and forest share.
RELIEF = {0: 300, 1: 300, 2: 400, 3: 1000, 4: 300, 5: 300, 6: 1200, 7: 300, 8: 150, 9: 200, 10: 300, 11: 250,
          12: 800}
FOREST = {0: 0.3, 1: 0.9, 2: 0.1, 3: 0.9, 4: 0.3, 5: 0.9, 6: 0.7, 7: 0.1, 8: 0.0, 9: 0.5, 10: 0.0, 11: 0.4,
          12: 0.8}


def _resize(a: np.ndarray, size: int, nearest: bool = False) -> np.ndarray:
    from PIL import Image

    img = Image.fromarray(a.astype(np.float32), mode="F")
    return np.asarray(img.resize((size, size), Image.NEAREST if nearest else Image.BILINEAR))


def _blocks(a: np.ndarray, k: int) -> np.ndarray:
    n = a.shape[0] // k
    return a[: n * k, : n * k].reshape(n, k, n, k).mean(axis=(1, 3))


def _majority(c: np.ndarray, k: int) -> np.ndarray:
    n = c.shape[0] // k
    b = c[: n * k, : n * k].reshape(n, k, n, k).transpose(0, 2, 1, 3).reshape(n, n, k * k)
    counts = np.stack([(b == i).sum(-1) for i in range(13)], -1)
    return counts.argmax(-1).astype(np.uint8)


def border_limit(n: int, cell_m: float) -> np.ndarray:
    """Highest ground allowed per cell: the map edge at BORDER_EDGE_M rising to 0 m at
    BORDER_M inland of it, then quickly unconstrained."""
    c = (np.arange(n) + 0.5) * cell_m
    edge = np.minimum(c, n * cell_m - c)
    d = np.minimum(edge[:, None], edge[None, :])
    return np.where(d < BORDER_M, BORDER_EDGE_M * (1 - d / BORDER_M) - 20.0, (d - BORDER_M) * 2.0 - 20.0)


LAYERS = ("classes", "roughness", "height")  # top to bottom in layout.xcf


def _gimp(script: str):
    exe = shutil.which("gimp-console")
    if exe is None:
        raise RuntimeError("gimp-console (GIMP 3) is needed to read and write layout.xcf")
    head = "from gi.repository import Gimp, Gio\nF = lambda p: Gio.File.new_for_path(p)\nNI = Gimp.RunMode.NONINTERACTIVE\n"
    run = subprocess.run([exe, "-i", "--batch-interpreter", "python-fu-eval", "-b", head + script, "--quit"],
                         capture_output=True, text=True, timeout=300)
    if run.returncode or "batch command executed successfully" not in run.stdout + run.stderr:
        raise RuntimeError(f"GIMP batch failed:\n{run.stdout}\n{run.stderr}")


def _to_xcf(pngs: Path, xcf: Path):
    """One 16-bit RGB GIMP image: classes at 60% over the height, roughness hidden."""
    _gimp(f"""
img = Gimp.file_load(NI, F({str(pngs / 'height.png')!r}))
img.convert_rgb()
img.get_layers()[0].set_name('height')
for name in ('roughness', 'classes'):
    layer = Gimp.file_load_layer(NI, img, F({str(pngs)!r} + '/' + name + '.png'))
    layer.set_name(name)
    img.insert_layer(layer, None, 0)
    layer.set_visible(name == 'classes')
    layer.set_opacity(60.0 if name == 'classes' else 100.0)
Gimp.file_save(NI, img, F({str(xcf)!r}), None)
""")


def _from_xcf(xcf: Path, pngs: Path):
    """Each named layer of layout.xcf as its own PNG (height 16-bit gray, roughness 8-bit
    gray, classes 8-bit RGB); other layers are ignored."""
    _gimp(f"""
img = Gimp.file_load(NI, F({str(xcf)!r}))
for layer in img.get_layers():
    name = layer.get_name()
    if name not in {LAYERS!r}:
        continue
    out = Gimp.Image.new_with_precision(img.get_width(), img.get_height(), Gimp.ImageBaseType.RGB,
                                        img.get_precision())
    copy = Gimp.Layer.new_from_drawable(layer, out)
    out.insert_layer(copy, None, 0)
    copy.set_visible(True)
    copy.set_opacity(100.0)
    copy.set_offsets(0, 0)
    out.flatten()
    if name != 'classes':
        out.convert_grayscale()
    out.convert_precision(Gimp.Precision.U16_NON_LINEAR if name == 'height' else Gimp.Precision.U8_NON_LINEAR)
    Gimp.file_save(NI, out, F({str(pngs)!r} + '/' + name + '.png'), None)
    out.delete()
""")
    missing = [n for n in ("height", "classes") if not (pngs / f"{n}.png").exists()]
    if missing:
        raise ValueError(f"layout.xcf has no layer named {', '.join(missing)}")


def export(l0_dir: Path, out: Path) -> Path:
    from PIL import Image

    from .layout import load

    l0 = load(l0_dir)
    out.mkdir(parents=True)
    pngs = Path(tempfile.mkdtemp(dir=out))
    h = np.clip(_resize(l0.mid, PX), H_MIN_M, H_MAX_M)
    gray = np.rint((h - H_MIN_M) / (H_MAX_M - H_MIN_M) * 65535).astype(np.uint16)
    Image.fromarray(gray).save(pngs / "height.png")  # uint16 -> 16-bit grayscale PNG
    k = PX // l0.cls.shape[0]
    cls = np.repeat(np.repeat(l0.cls, k, 0), k, 1)
    Image.fromarray(np.array([COLORS[i] for i in range(13)], np.uint8)[cls]).save(pngs / "classes.png")
    rough = np.clip(_resize(l0.relief, PX) / RELIEF_MAX_M * 255, 0, 255)
    Image.fromarray(np.rint(rough).astype(np.uint8)).save(pngs / "roughness.png")
    _to_xcf(pngs, out / "layout.xcf")
    shutil.rmtree(pngs)
    (out / "terrain-v2.gpl").write_text(
        "GIMP Palette\nName: Terrain legend v2\nColumns: 4\n#\n"
        + "".join(f"{r:3d} {g:3d} {b:3d}\t{i} {NAMES.get(i, 'Unknown / sea')}\n" for i, (r, g, b) in COLORS.items()))
    spec = l0.spec
    settings = {k: spec[k] for k in ("name", "wind_from_deg", "windward_snowline_m", "lee_snowline_m", "treeline_m")}
    settings["tiers"] = spec.get("tiers", {"starter_ceiling_m": 1500, "mid_ceiling_m": 3000, "clearance_m": 100})
    (out / "paint.json").write_text(json.dumps({
        "kind": "world_layout_paint", "schema_version": 1, "extent_m": l0.cell_m * l0.mid.shape[0], "px": PX,
        "height": {"black_m": H_MIN_M, "white_m": H_MAX_M}, "roughness_255_m": RELIEF_MAX_M,
        "settings": settings, "from": str(l0_dir.resolve())}, indent=1))
    sea = (0 - H_MIN_M) / (H_MAX_M - H_MIN_M)
    (out / "README.txt").write_text(f"""Paint a layout for {settings['name']}

Open layout.xcf in GIMP. It has three layers, read by name; any other layers you add
(sketches, references) are ignored. Keep the image at 16-bit integer precision.

The map is {l0.cell_m * l0.mid.shape[0] / 1000:.1f} km square, {PX} px: {l0.cell_m * l0.mid.shape[0] / PX:.0f} m per pixel, north up.
The models see this through a 2 km envelope: paint broad shapes. Valleys, fjord channels,
lakes, rivers and shorelines finer than about 2 km are drawn by the models.
The outer {BORDER_M / 1000:.1f} km of the map is always sea, whatever is painted there.

height      gray: black {H_MIN_M:.0f} m, white {H_MAX_M:.0f} m. Sea level is {sea:.1%} gray
            ({round(sea * 255)} of 255); 1,000 m is {1000 / (H_MAX_M - H_MIN_M):.1%} of the range
            ({255 * 1000 / (H_MAX_M - H_MIN_M):.1f} of 255). Ground below sea level connected to the
            edge is sea; terrain continues under it.
classes     terrain class per pixel in the colours of terrain-v2.gpl, shown at 60% opacity
            over the height (opacity is ignored on import). Use the pencil tool without
            antialiasing; mixed colours snap to the nearest class.
roughness   hidden; local relief, white = {RELIEF_MAX_M:.0f} m. Delete the layer to use each
            class's default.

Palette: Windows > Dockable Dialogs > Palettes > Import Palette > terrain-v2.gpl.
Save with File > Save (layout.xcf); no export needed.

Then: python -m topography_training.diffusion.paint import <this dir> <new L0 dir>
""")
    return out


def _read_height(path: Path) -> np.ndarray:
    from PIL import Image

    img = Image.open(path)
    if img.mode in ("I;16", "I;16B", "I;16L", "I"):
        a = np.asarray(img).astype(np.float64) / 65535
    else:
        print(f"warning: {path.name} is {img.mode}, not 16-bit grayscale; heights lose precision", file=sys.stderr)
        a = np.asarray(img.convert("L")).astype(np.float64) / 255
    return (H_MIN_M + a * (H_MAX_M - H_MIN_M)).astype(np.float32)


def _read_classes(path: Path) -> np.ndarray:
    from PIL import Image

    rgb = np.asarray(Image.open(path).convert("RGB")).astype(np.int32)
    palette = np.array([COLORS[i] for i in range(13)], np.int32)
    d = ((rgb[..., None, :] - palette[None, None]) ** 2).sum(-1)
    return d.argmin(-1).astype(np.uint8)


def _toml(settings: dict) -> str:
    t = settings["tiers"]
    return (f'schema_version = 1\nname = "{settings["name"]}"\npainted = true\n'
            f'wind_from_deg = {settings["wind_from_deg"]}\nwindward_snowline_m = {settings["windward_snowline_m"]}\n'
            f'lee_snowline_m = {settings["lee_snowline_m"]}\ntreeline_m = {settings["treeline_m"]}\n\n[tiers]\n'
            f'starter_ceiling_m = {t["starter_ceiling_m"]}\nmid_ceiling_m = {t["mid_ceiling_m"]}\n'
            f'clearance_m = {t["clearance_m"]}\n')


def import_(paint: Path, out: Path) -> Path:
    meta = json.loads((paint / "paint.json").read_text())
    extent, settings = float(meta["extent_m"]), meta["settings"]
    if (paint / "layout.xcf").exists():
        with tempfile.TemporaryDirectory() as tmp:
            _from_xcf(paint / "layout.xcf", Path(tmp))
            return _import_pngs(Path(tmp), meta, out)
    return _import_pngs(paint, meta, out)


def _import_pngs(paint: Path, meta: dict, out: Path) -> Path:
    from PIL import Image
    from scipy import ndimage

    extent, settings = float(meta["extent_m"]), meta["settings"]
    n = int(extent / CELL_M)
    height = _read_height(paint / "height.png")
    if height.shape[0] % n or height.shape != (height.shape[0],) * 2:
        raise ValueError(f"height.png must be square and a multiple of {n} px")
    mid = np.minimum(_blocks(height, height.shape[0] // n), border_limit(n, CELL_M)).astype(np.float32)
    classes = _read_classes(paint / "classes.png")
    cls = _majority(classes, classes.shape[0] // n)
    rough_path = paint / "roughness.png"
    if rough_path.exists():
        r = np.asarray(Image.open(rough_path).convert("L")).astype(np.float32) / 255 * RELIEF_MAX_M
        relief = _blocks(r, r.shape[0] // n).astype(np.float32)
    else:
        relief = np.array([RELIEF[i] for i in range(13)], np.float32)[cls]
    land = mid >= 0
    # Open sea keeps little relief, or the models raise islands; archipelagos keep theirs.
    coast = np.where(land, ndimage.distance_transform_edt(land), -ndimage.distance_transform_edt(~land)) * CELL_M
    fade = np.clip(1 + coast / 1500.0, 0, 1)
    relief = np.where((coast < 0) & (cls != 5), relief * fade * fade * (3 - 2 * fade), relief).astype(np.float32)
    forest = np.rint(np.array([FOREST[i] for i in range(13)])[cls] * 255).astype(np.uint8)
    # Snowline from wind exposure across the land: windward low, lee high.
    ys, xs = np.nonzero(land)
    cy, cx = ((ys.mean(), xs.mean()) if len(ys) else (n / 2, n / 2))
    b = np.radians(settings["wind_from_deg"])
    tx, ty = -np.sin(b), np.cos(b)  # toward which the wind blows; y is south
    yy, xx = np.mgrid[0:n, 0:n]
    e = np.clip(((xx - cx) * tx + (yy - cy) * ty) / (n / 2), -1, 1)
    snowline = (settings["windward_snowline_m"]
                + (settings["lee_snowline_m"] - settings["windward_snowline_m"]) * (e + 1) / 2).astype(np.float32)
    layers = {
        "mid.f32": mid, "relief.f32": relief, "coast.f32": coast.astype(np.float32), "class.u8": cls,
        "lake.u8": np.zeros((n, n), np.uint8), "lake-level.f32": np.full((n, n), np.nan, np.float32),
        "snowline.f32": snowline, "forest.u8": forest, "hold.u8": np.zeros((n, n), np.uint8),
    }
    out.mkdir(parents=True)
    digests = {}
    for name, a in layers.items():
        data = a.astype("<f4" if name.endswith(".f32") else "u1").tobytes()
        (out / name).write_bytes(data)
        digests[name] = hashlib.sha256(data).hexdigest()
    text = _toml(settings)
    (out / "layout.json").write_text(json.dumps({
        "schema_version": 1, "kind": "world_layout_l0", "name": settings["name"], "cell_m": CELL_M, "size": [n, n],
        "extent_m": extent, "orientation": "row-major, north-up; row 0 is y = 0 (north)",
        "config_path": str(paint.resolve()), "config_sha256": hashlib.sha256(text.encode()).hexdigest(),
        "config_text": text, "treeline_m": settings["treeline_m"], "painted_from": str(paint.resolve()),
        "layers": digests}, indent=1))
    km2 = CELL_M * CELL_M / 1e6
    print(f"land {land.sum() * km2:.0f} km², highest {mid.max():.0f} m; classes on land: "
          + ", ".join(f"{c} {(cls[land] == c).sum() * km2:.0f} km²" for c in np.unique(cls[land])))
    return out


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = p.add_subparsers(dest="command", required=True)
    e = sub.add_parser("export")
    e.add_argument("l0", type=Path)
    e.add_argument("out", type=Path)
    i = sub.add_parser("import")
    i.add_argument("paint", type=Path)
    i.add_argument("out", type=Path)
    args = p.parse_args(argv)
    if args.out.exists():
        print("error: output directory exists", file=sys.stderr)
        return 2
    print(export(args.l0, args.out) if args.command == "export" else import_(args.paint, args.out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
