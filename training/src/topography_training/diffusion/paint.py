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
BORDER_HOLD_M = 3000.0  # sampling is steered toward sea over this distance from the edge
SEA_RELIEF_M = 600.0  # relief fades out over this distance offshore (except island classes)
CELL_M = 128.0
# Judgment defaults by legend v3 class (from the Fujion regions): local relief (m) and forest share.
# Dunes: the real 128 m dune residual is about 40 m.
RELIEF = {0: 300, 1: 300, 2: 400, 3: 1000, 4: 300, 5: 300, 6: 1200, 7: 300, 8: 100, 9: 200, 10: 300, 11: 250,
          12: 800, 13: 500, 14: 300, 15: 250, 16: 60, 17: 150, 18: 80}
FOREST = {0: 0.3, 1: 0.9, 2: 0.1, 3: 0.9, 4: 0.3, 5: 0.9, 6: 0.7, 7: 0.1, 8: 0.0, 9: 0.5, 10: 0.0, 11: 0.4,
          12: 0.8, 13: 0.1, 14: 0.9, 15: 0.9, 16: 0.0, 17: 0.2, 18: 0.4}
ISLAND_CLASSES = (5, 15)  # keep their relief offshore, so islands rise from the sea
DROWNED_KARST = 15
DROWNED_SEABED_M = (-40.0, -10.0)  # drowned karst sits on a shallow seabed, so towers break the surface
DROWNED_FADE_M = 1000.0
PALETTE = "terrain-v3.gpl"


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
    counts = np.stack([(b == i).sum(-1) for i in range(len(COLORS))], -1)
    return counts.argmax(-1).astype(np.uint8)


def border_limit(n: int, cell_m: float) -> np.ndarray:
    """Highest ground allowed per cell: the map edge at BORDER_EDGE_M rising to 0 m at
    BORDER_M inland of it, then quickly unconstrained."""
    c = (np.arange(n) + 0.5) * cell_m
    edge = np.minimum(c, n * cell_m - c)
    d = np.minimum(edge[:, None], edge[None, :])
    return np.where(d < BORDER_M, BORDER_EDGE_M * (1 - d / BORDER_M) - 20.0, (d - BORDER_M) * 2.0 - 20.0)


def border_hold(mid: np.ndarray, cell_m: float) -> tuple[np.ndarray, np.ndarray]:
    """(target, weight) steering sampling toward sea near the map edge, so the coast
    forms naturally before the border band instead of being cut by the final clamp."""
    n = mid.shape[0]
    c = (np.arange(n) + 0.5) * cell_m
    edge = np.minimum(c, n * cell_m - c)
    d = np.minimum(edge[:, None], edge[None, :])
    t = np.clip(d / BORDER_HOLD_M, 0, 1)
    weight = 1 - t * t * (3 - 2 * t)
    return np.minimum(mid, border_limit(n, cell_m)).astype(np.float32), weight.astype(np.float32)


LAYERS = ("classes", "roughness", "hold", "height")  # top to bottom in layout.xcf


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
for name in ('hold', 'roughness', 'classes'):
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


def replace_layers(xcf: Path, pngs: dict[str, Path]):
    """Replace the pixels of named layers in layout.xcf from PNGs, keeping every other
    layer and the layer order; a missing layer is added hidden above `height`."""
    _gimp(f"""
img = Gimp.file_load(NI, F({str(xcf)!r}))
for name, png in {({k: str(v) for k, v in pngs.items()})!r}.items():
    old = [l for l in img.get_layers() if l.get_name() == name]
    new = Gimp.file_load_layer(NI, img, F(png))
    new.set_name(name)
    if old:
        pos = img.get_item_position(old[0])
        visible, opacity = old[0].get_visible(), old[0].get_opacity()
        img.remove_layer(old[0])
    else:
        height = [l for l in img.get_layers() if l.get_name() == 'height']
        pos = img.get_item_position(height[0]) if height else 0
        visible, opacity = False, 100.0
    img.insert_layer(new, None, pos)
    new.set_visible(visible)
    new.set_opacity(opacity)
Gimp.file_save(NI, img, F({str(xcf)!r}), None)
""")


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
    Image.fromarray(np.array([COLORS[i] for i in range(len(COLORS))], np.uint8)[cls]).save(pngs / "classes.png")
    rough = np.clip(_resize(l0.relief, PX) / RELIEF_MAX_M * 255, 0, 255)
    Image.fromarray(np.rint(rough).astype(np.uint8)).save(pngs / "roughness.png")
    held = np.clip(_resize(l0.hold, PX) * 255, 0, 255)
    Image.fromarray(np.rint(held).astype(np.uint8)).save(pngs / "hold.png")
    _to_xcf(pngs, out / "layout.xcf")
    shutil.rmtree(pngs)
    write_palette(out)
    spec = l0.spec
    settings = {k: spec[k] for k in ("name", "wind_from_deg", "windward_snowline_m", "lee_snowline_m", "treeline_m")}
    settings["hemisphere"] = spec.get("hemisphere", "north")
    settings["tiers"] = spec.get("tiers", {"starter_ceiling_m": 1500, "mid_ceiling_m": 3000, "clearance_m": 100})
    (out / "paint.json").write_text(json.dumps({
        "kind": "world_layout_paint", "schema_version": 1, "extent_m": l0.cell_m * l0.mid.shape[0], "px": PX,
        "height": {"black_m": H_MIN_M, "white_m": H_MAX_M}, "roughness_255_m": RELIEF_MAX_M,
        "settings": settings, "from": str(l0_dir.resolve())}, indent=1))
    write_readme(out, settings, l0.cell_m * l0.mid.shape[0])
    return out


def write_readme(out: Path, settings: dict, extent_m: float) -> None:
    sea = (0 - H_MIN_M) / (H_MAX_M - H_MIN_M)
    (out / "README.txt").write_text(f"""Paint a layout for {settings['name']}

Open layout.xcf in GIMP. It has three layers, read by name; any other layers you add
(sketches, references) are ignored. Keep the image at 16-bit integer precision.

The map is {extent_m / 1000:.1f} km square, {PX} px: {extent_m / PX:.0f} m per pixel, north up.
The models see this through a 2 km envelope: paint broad shapes. Valleys, fjord channels,
lakes, rivers and shorelines finer than about 2 km are drawn by the models.
The outer {BORDER_M / 1000:.1f} km of the map is always sea, whatever is painted there.

height      gray: black {H_MIN_M:.0f} m, white {H_MAX_M:.0f} m. Sea level is {sea:.1%} gray
            ({round(sea * 255)} of 255); 1,000 m is {1000 / (H_MAX_M - H_MIN_M):.1%} of the range
            ({255 * 1000 / (H_MAX_M - H_MIN_M):.1f} of 255). Ground below sea level connected to the
            edge is sea; terrain continues under it.
classes     terrain class per pixel in the colours of {PALETTE}, shown at 60% opacity
            over the height (opacity is ignored on import). Use the pencil tool without
            antialiasing; mixed colours snap to the nearest class. Subclasses name a look:
            13 layered canyons (Grand Canyon), 14 cone and tower karst, 15 drowned tower
            karst (paint it in shallow sea; its seabed is set to {DROWNED_SEABED_M[0]:.0f} to
            {DROWNED_SEABED_M[1]:.0f} m), 16 crest-train dunes, 17 cinder cones and lava, 18 plain.
            Calderas (4) and cinder cones (17) are filled with real examples
            (catalog/exemplars).
roughness   hidden; local relief, white = {RELIEF_MAX_M:.0f} m. Delete the layer to use each
            class's default.
hold        hidden; white holds the height layer's large-scale shape (above about 400 m)
            while L1 samples, black leaves the model free. Use it sparingly, for landmarks
            the model would otherwise flatten (the volcano). Optional.

Palette: Windows > Dockable Dialogs > Palettes > Import Palette > {PALETTE}.
Save with File > Save (layout.xcf); no export needed.

Then: python -m topography_training.diffusion.paint import <this dir> <new L0 dir>
""")


def write_palette(out: Path) -> Path:
    """The legend v3 GIMP palette in `out`."""
    path = out / PALETTE
    path.write_text("GIMP Palette\nName: Terrain legend v3\nColumns: 4\n#\n"
                    + "".join(f"{r:3d} {g:3d} {b:3d}\t{i} {NAMES.get(i, 'Unknown / sea')}\n"
                              for i, (r, g, b) in COLORS.items()))
    return path


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
    palette = np.array([COLORS[i] for i in range(len(COLORS))], np.int32)
    d = ((rgb[..., None, :] - palette[None, None]) ** 2).sum(-1)
    return d.argmin(-1).astype(np.uint8)


def _toml(settings: dict) -> str:
    t = settings["tiers"]
    return (f'schema_version = 1\nname = "{settings["name"]}"\npainted = true\n'
            f'hemisphere = "{settings.get("hemisphere", "north")}"\n'
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
        relief = np.array([RELIEF[i] for i in range(len(COLORS))], np.float32)[cls]
    # Drowned karst stands on a shallow seabed, so its towers can break the surface; the
    # shallowing fades out over about 1 km so the painted outline leaves no shelf edge.
    from .data import smooth

    shallow = np.clip(smooth((cls == DROWNED_KARST).astype(np.float32), DROWNED_FADE_M / CELL_M) * 2 - 0.5, 0, 1)
    shallow = np.where(mid < 0, shallow, 0)
    mid = mid + shallow * (np.clip(mid, *DROWNED_SEABED_M) - mid)
    mid = np.minimum(mid, border_limit(n, CELL_M)).astype(np.float32)
    land = mid >= 0
    # Open sea keeps little relief, or the models raise islands; island classes keep theirs.
    coast = np.where(land, ndimage.distance_transform_edt(land), -ndimage.distance_transform_edt(~land)) * CELL_M
    fade = np.clip(1 + coast / SEA_RELIEF_M, 0, 1)
    keep = np.maximum(np.isin(cls, ISLAND_CLASSES).astype(np.float32), shallow)  # island classes keep relief
    faded = relief * fade * fade * (3 - 2 * fade)
    relief = np.where(coast < 0, faded + keep * (relief - faded), relief).astype(np.float32)
    forest = np.rint(np.array([FOREST[i] for i in range(len(COLORS))])[cls] * 255).astype(np.uint8)
    hold_path = paint / "hold.png"
    hold = np.zeros((n, n), np.uint8)
    if hold_path.exists():
        w = np.asarray(Image.open(hold_path).convert("L")).astype(np.float32)
        hold = np.rint(_blocks(w, w.shape[0] // n)).astype(np.uint8)
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
        "snowline.f32": snowline, "forest.u8": forest, "hold.u8": hold,
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


def relabel(paint_dir: Path, out: Path, mapping: dict[int, int], small: list[tuple[int, int, float]],
            hemisphere: str | None = None) -> Path:
    """A copy of a paint dir with its classes layer relabelled: every class in `mapping` to
    its new class, and patches of class a smaller than `km2` to class b (`small`), for
    example the cones dotted around a volcano. Other layers stay as painted."""
    from PIL import Image
    from scipy import ndimage

    shutil.copytree(paint_dir, out)
    meta = json.loads((out / "paint.json").read_text())
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        _from_xcf(out / "layout.xcf", tmp)
        cls = _read_classes(tmp / "classes.png")
        new = cls.copy()
        for a, b in mapping.items():
            new[cls == a] = b
        px_km2 = (float(meta["extent_m"]) / cls.shape[0] / 1000) ** 2
        for a, b, km2 in small:
            labels, count = ndimage.label(cls == a)
            sizes = ndimage.sum(np.ones(cls.shape), labels, np.arange(1, count + 1)) * px_km2
            new[np.isin(labels, np.flatnonzero(sizes < km2) + 1)] = b
        Image.fromarray(np.array([COLORS[i] for i in range(len(COLORS))], np.uint8)[new]).save(tmp / "classes.png")
        replace_layers(out / "layout.xcf", {"classes": tmp / "classes.png"})
    if hemisphere:
        meta["settings"]["hemisphere"] = hemisphere
    meta["relabelled_from"] = {"paint": str(paint_dir.resolve()), "map": {str(k): v for k, v in mapping.items()},
                               "small": [list(s) for s in small]}
    (out / "paint.json").write_text(json.dumps(meta, indent=1))
    for old in out.glob("terrain-v*.gpl"):
        old.unlink()
    write_palette(out)
    write_readme(out, meta["settings"], float(meta["extent_m"]))
    changed = {f"{a}->{b}": int(((cls == a) & (new == b)).sum()) for a, b in
               list(mapping.items()) + [(a, b) for a, b, _ in small]}
    print(f"relabelled pixels: {changed}")
    return out


VOLCANO_CLASSES = (11, 10, 4)  # cone, its glaciers and summit crater
GLACIER = 10
VOLCANO_FADE_M = 1500.0  # the real cone fades out over this distance around the volcano biome


def _write_height(h: np.ndarray, path: Path) -> None:
    from PIL import Image

    gray = np.rint(np.clip((h - H_MIN_M) / (H_MAX_M - H_MIN_M), 0, 1) * 65535).astype(np.uint16)
    Image.fromarray(gray).save(path)


def roughen(paint_dir: Path, out: Path, class_id: int, amp_m: float = 2000.0, wave_m: float = 6000.0,
            band_m: float = 3000.0, seed: int = 0) -> Path:
    """A copy of a paint dir whose layers are warped by smooth noise near the outline of
    one class (for example a drowned-karst bay painted as a half disc), so the painted
    shoreline gets embayments and headlands. Displacement is about `amp_m` at a wavelength
    of about `wave_m`, plus a third of that at a third of the wavelength, fading to nothing
    `band_m` from the outline."""
    from PIL import Image
    from scipy import ndimage

    from .data import smooth

    shutil.copytree(paint_dir, out)
    meta = json.loads((out / "paint.json").read_text())
    px_m = float(meta["extent_m"]) / PX
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        _from_xcf(out / "layout.xcf", tmp)
        h = _read_height(tmp / "height.png")
        cls = _read_classes(tmp / "classes.png")
        area = cls == class_id
        if not area.any():
            raise ValueError(f"no class {class_id} painted")
        edge = np.minimum(ndimage.distance_transform_edt(area), ndimage.distance_transform_edt(~area)) * px_m
        taper = np.clip(1 - edge / band_m, 0, 1) ** 2
        # No warping near the map border, which must stay sea.
        c = (np.arange(h.shape[0]) + 0.5) * px_m
        border = np.minimum(c, h.shape[0] * px_m - c)
        taper *= np.clip((np.minimum(border[:, None], border[None, :]) - BORDER_M - amp_m) / band_m, 0, 1)
        rng = np.random.default_rng(seed)
        d = []
        for _ in range(2):  # two octaves: bays and headlands, then smaller coves
            a = np.zeros(h.shape, np.float32)
            for f in (1.0, 3.0):
                n = smooth(rng.standard_normal(h.shape).astype(np.float32), wave_m / f / 4 / px_m)
                a += n / n.std() / f
            d.append(a * amp_m / px_m * taper)
        yy, xx = np.indices(h.shape).astype(np.float32)
        at = [yy + d[0], xx + d[1]]
        layers = {"height": ndimage.map_coordinates(h, at, order=1, mode="nearest"),
                  "classes": ndimage.map_coordinates(cls, at, order=0, mode="nearest")}
        _write_height(layers["height"], tmp / "height.png")
        Image.fromarray(np.array([COLORS[i] for i in range(len(COLORS))], np.uint8)[layers["classes"]]).save(
            tmp / "classes.png")
        pngs = {"height": tmp / "height.png", "classes": tmp / "classes.png"}
        for name in ("roughness", "hold"):
            if (tmp / f"{name}.png").exists():
                g = np.asarray(Image.open(tmp / f"{name}.png").convert("L")).astype(np.float32)
                Image.fromarray(np.rint(ndimage.map_coordinates(g, at, order=1, mode="nearest")).astype(np.uint8)).save(
                    tmp / f"{name}.png")
                pngs[name] = tmp / f"{name}.png"
        replace_layers(out / "layout.xcf", pngs)
    meta.setdefault("roughened", []).append({"class": class_id, "amp_m": amp_m, "wave_m": wave_m, "band_m": band_m,
                                             "seed": seed})
    (out / "paint.json").write_text(json.dumps(meta, indent=1))
    return out


def transplant(paint_dir: Path, out: Path, region: str, release: Path, base: Path | None = None,
               cone_km: float = 12.0) -> Path:
    """A copy of a paint dir whose painted volcano is replaced by a real one. The real
    region's cone (its relief above the ground `cone_km` from its summit) is centred on
    the painted summit, scaled to the painted cone's footprint and summit height, and
    faded out at the edge of the volcano biome, so it never runs into other biomes.
    `base` is a layout.xcf of the same map without the painted cone (else the painted
    height, smoothed across the cone, is the ground). Glaciers (class 10) are redrawn on
    the volcano above the height that keeps their painted area."""
    from PIL import Image
    from scipy import ndimage

    from . import cascade
    from .data import smooth

    shutil.copytree(paint_dir, out)
    meta = json.loads((out / "paint.json").read_text())
    px_m = float(meta["extent_m"]) / PX
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        _from_xcf(out / "layout.xcf", tmp)
        h = _read_height(tmp / "height.png").astype(np.float64)
        cls = _read_classes(tmp / "classes.png")
        if base is not None:
            with tempfile.TemporaryDirectory() as tb:
                _from_xcf(base, Path(tb))
                ground = _read_height(Path(tb) / "height.png").astype(np.float64)
        else:
            ground = None
        sy, sx = np.unravel_index(int(np.argmax(np.where(np.isin(cls, VOLCANO_CLASSES), h, -np.inf))), h.shape)
        labels, _ = ndimage.label(np.isin(cls, VOLCANO_CLASSES))
        biome = labels == labels[sy, sx]
        yy, xx = np.indices(h.shape)
        r_px = np.hypot(yy - sy, xx - sx)
        if ground is None:  # ground under the cone: the painted height around it, filled inward
            far = r_px * px_m > cone_km * 500
            w = smooth(far.astype(np.float32), 2000 / px_m)
            ground = smooth(np.where(far, h, 0).astype(np.float32), 2000 / px_m) / np.maximum(w, 1e-3)
            ground = np.where(far, h, ground)
        added = h - ground
        # Painted cone radius: where its ring-mean height falls to a tenth of the summit's.
        rings = np.bincount(np.rint(r_px).astype(int).ravel(), added.ravel()) / np.maximum(
            np.bincount(np.rint(r_px).astype(int).ravel()), 1)
        r_paint = float(np.argmax(rings < 0.1 * added[sy, sx])) * px_m
        # The real cone at its own 32 m cells (GLO-30 regions are 32 m, as is the paint).
        reg = cascade.load_release(release, {region})[0]
        real = np.nan_to_num(reg.height).astype(np.float64)
        ry, rx = np.unravel_index(int(np.argmax(real)), real.shape)
        rr = np.hypot(*(np.indices(real.shape) - np.array([ry, rx])[:, None, None])) * float(reg.spacing_m)
        ring = (rr >= cone_km * 1000) & (rr < cone_km * 1000 + 1000)
        relief = np.clip(real - real[ring].mean(), 0, None)
        relief *= np.clip((cone_km * 1000 - rr) / 2000.0, 0, 1)  # taper to nothing at cone_km
        s = r_paint / (cone_km * 1000)  # painted metres per real metre
        coords = [ry + (yy - sy) * px_m / s / reg.spacing_m, rx + (xx - sx) * px_m / s / reg.spacing_m]
        cone = ndimage.map_coordinates(relief, coords, order=1, mode="constant", cval=0.0)
        cone *= added[sy, sx] / max(float(relief.max()), 1.0)
        inside = ndimage.distance_transform_edt(biome) - ndimage.distance_transform_edt(~biome)
        fade = np.clip(0.5 + inside * px_m / VOLCANO_FADE_M, 0, 1)
        new = ground + cone * fade * fade * (3 - 2 * fade)
        # Glaciers keep their painted area on the volcano's highest ground, reaching lower
        # on pole-facing slopes and in gullies, as snow does (cover.py).
        ice = cls == GLACIER
        if ice.any():
            from .cover import SNOW_ASPECT_M, SNOW_HOLLOW_M, SNOW_NOISE_M, noise_field, terrain_features

            south = str(meta["settings"].get("hemisphere", "north")).startswith("s")
            _, pole, hollow = terrain_features(new.astype(np.float32), px_m, -1.0 if south else 1.0)
            noise = ndimage.zoom(noise_field(float(meta["extent_m"]), 5.0, 1), h.shape[0] / 1025, order=1)
            score = new + SNOW_ASPECT_M * pole + SNOW_HOLLOW_M * hollow + SNOW_NOISE_M * noise[: h.shape[0], : h.shape[1]]
            on = biome & np.isin(cls, (10, 11))
            line = np.quantile(score[on], 1 - ice[on].sum() / on.sum())
            cls = np.where(on & (score >= line), GLACIER, np.where(on & ice, 11, cls)).astype(np.uint8)
        _write_height(new, tmp / "height.png")
        Image.fromarray(np.array([COLORS[i] for i in range(len(COLORS))], np.uint8)[cls]).save(tmp / "classes.png")
        replace_layers(out / "layout.xcf", {"height": tmp / "height.png", "classes": tmp / "classes.png"})
    meta["volcano"] = {"region": region, "release": str(release.resolve()), "cone_km": cone_km,
                       "base": str(base.resolve()) if base else None, "summit_px": [int(sy), int(sx)],
                       "painted_radius_m": round(r_paint), "scale": round(s, 3)}
    (out / "paint.json").write_text(json.dumps(meta, indent=1))
    print(f"volcano from {region}: painted cone radius {r_paint:.0f} m, scale {s:.2f}, "
          f"summit {new[sy, sx]:.0f} m (painted {h[sy, sx]:.0f} m)")
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
    r = sub.add_parser("relabel", help="copy a paint dir with classes renamed (for example to legend v3 subclasses)")
    r.add_argument("paint", type=Path)
    r.add_argument("out", type=Path)
    r.add_argument("--map", default="", help="from:to class pairs, comma separated (2:13,8:16)")
    r.add_argument("--small", default="", help="from:to:km2 - patches of a class below km2 become the other (11:17:3)")
    r.add_argument("--hemisphere", choices=["north", "south"])
    v = sub.add_parser("transplant", help="copy a paint dir with its painted volcano replaced by a real one")
    v.add_argument("paint", type=Path)
    v.add_argument("out", type=Path)
    v.add_argument("--region", default="fuji", help="context region with the real volcano")
    v.add_argument("--release", type=Path, default=Path("data/datasets/context-v5/release.json"))
    v.add_argument("--base", type=Path, help="layout.xcf of the same map before the cone was painted")
    v.add_argument("--cone-km", type=float, default=12.0, help="radius of the real cone")
    g = sub.add_parser("roughen", help="copy a paint dir with the outline of one class warped by noise")
    g.add_argument("paint", type=Path)
    g.add_argument("out", type=Path)
    g.add_argument("--class", dest="class_id", type=int, required=True)
    g.add_argument("--amp-m", type=float, default=2000.0)
    g.add_argument("--wave-m", type=float, default=6000.0)
    g.add_argument("--band-m", type=float, default=3000.0)
    g.add_argument("--seed", type=int, default=0)
    args = p.parse_args(argv)
    if args.out.exists():
        print("error: output directory exists", file=sys.stderr)
        return 2
    if args.command == "roughen":
        print(roughen(args.paint, args.out, args.class_id, args.amp_m, args.wave_m, args.band_m, args.seed))
        return 0
    if args.command == "transplant":
        print(transplant(args.paint, args.out, args.region, args.release, args.base, args.cone_km))
        return 0
    if args.command == "relabel":
        mapping = dict(tuple(int(v) for v in pair.split(":")) for pair in args.map.split(",") if pair)
        small = [(int(a), int(b), float(k)) for a, b, k in (s.split(":") for s in args.small.split(",") if s)]
        print(relabel(args.paint, args.out, mapping, small, args.hemisphere))
        return 0
    print(export(args.l0, args.out) if args.command == "export" else import_(args.paint, args.out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
