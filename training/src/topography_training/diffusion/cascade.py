"""Cascade levels trained on context regions (see terrain-cascade-v1.md).

L1 (128 m) refines the smooth mid-surface of an L0-like layout envelope; L2 (32 m)
refines the 128 m surface, both from GLO-30 regions at 32 m. L3 (4 m) refines the 32 m
surface from lidar regions at 4 m. L4 (1 m) refines the 4 m surface of the 1 m library
tiles (diffusion training releases). Each generates a residual over the level above.
Regions are row-major and north-up; context regions are split whole (set in the
release), library tiles by geographic bands inside each tile.

Terrain continues under water: GLO-30 sea (0 m) is replaced by a seabed joined to the
coast (see `join_seabed`), so sea is simply ground below 0 m. Water layers keep only
water above sea level.
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from pathlib import Path

import numpy as np

from .data import NUM_CLASSES, _window_sum, smooth

# L0 layout scale: the envelope is soft so the model, not the painter, draws ranges,
# valleys and coastlines.
ENVELOPE_M = 2048
MIN_RANGE_M = 1.0  # unit-range shape channel floor for flat windows
# Seabed join: GEBCO (~450 m) is not trusted this near the coast; there the land slope
# continues down until it meets the deepest GEBCO nearby, then GEBCO takes over.
NEARSHORE_M = 1000.0
COAST_BAND_M = 300.0  # land whose slope continues under water
SEA_SLOPE_DEG = (1.0, 35.0)
SEA_MIN_DEPTH_M = 10.0  # GLO-30 ground at 0 m is sea only where GEBCO reaches this depth


@dataclass(frozen=True)
class Level:
    name: str
    cell_m: int
    conditioning: int  # layout id stored in run configs
    region_m: int  # cell size of the regions it trains on
    block: int = 0  # refining levels: coarse block size in cells
    smooth_cells: float = 0.0  # refining levels: smoothing after upsampling
    margin: int = 16  # refining levels: context cells around a window
    stride: int = 8  # spacing of candidate training windows, in cells
    water_in_loss: bool = True  # False where water cells hold a surveyed surface, not ground


# Smoothing is half a block, like the 1 m model's 16 m on 32 m blocks.
# Layout 4 (land/sea channel, flat sea) is retired; L1 runs before 2026-10-02 used it.
# L1 layout 6 adds landmark channels (valleys, peaks) to layout 5; chains run either.
LEVELS = {
    "l1": Level("l1", 128, 6, 32),
    "l2": Level("l2", 32, 3, 32, block=4, smooth_cells=2.0),
    "l3": Level("l3", 4, 3, 4, block=8, smooth_cells=4.0, margin=32, stride=32),
    "l4": Level("l4", 1, 3, 1, block=4, smooth_cells=2.0, margin=16, stride=16, water_in_loss=False),
}
# Layout 3 channels set to 0 when dropped in training, for guidance at generation.
# Coarse dropout hides shape, slopes and absolute height: absolute height alone carries
# the whole coarse surface (L2 pilot of 2026-10-04 learned no class with it left in).
# Per conditioning layout: (coarse channels, cover channels). L1 (layout 6) hides its
# envelope mid surface and relief, so a painted, featureless envelope still gets class style.
DROP_CHANNELS = {3: ((0, 1, 2, 3), (5, 6)), 6: ((0, 1, 2, 3, 4), ())}
SHAPE_CHANNELS, COVER_CHANNELS = DROP_CHANNELS[3]
MIN_VALID = 0.9  # share of surveyed cells a training window needs


@dataclass
class Region:
    name: str
    class_id: int
    split: str
    spacing_m: int
    height: np.ndarray  # float32 metres, NaN where not surveyed; seabed under the sea
    water: np.ndarray  # bool; water above sea level where a seabed is joined
    snow: np.ndarray  # bool, or uint8 soft coverage for library tiles
    vegetation: np.ndarray  # bool, or uint8 soft coverage for library tiles
    cells: np.ndarray | None = None  # per-cell split of library tiles (data.TRAIN, data.VALIDATION)
    fields: Fields | None = None  # built once, shared by the train and validation samplers
    source: tuple | None = None  # lazy regions: (release manifest path, region entry); arrays load on demand


def _read(release: Path, layer: dict, shape) -> np.ndarray:
    data = (release / layer["path"]).read_bytes()
    if hashlib.sha256(data).hexdigest() != layer["sha256"]:
        raise ValueError(f"release layer hash mismatch: {layer['path']}")
    dtype = {"float32_le": "<f4", "uint8": "u1"}[layer["dtype"]]
    return np.frombuffer(data, dtype=dtype).reshape(shape)


def load_regions(manifest_path: Path, lazy: bool = False) -> list[Region]:
    """Regions of a context release, or the tiles of a 1 m library release (L4); `lazy`
    applies to context releases (see `load_release`)."""
    if json.loads(manifest_path.read_text()).get("kind") != "diffusion_training_release":
        return load_release(manifest_path, lazy=lazy)
    from .data import load_release as load_tiles

    return [Region(t.name, t.class_id, "tile", 1, t.height, t.water, t.snow, t.vegetation, t.split)
            for t in load_tiles(manifest_path)]


def load_release(manifest_path: Path, ids: set[str] | None = None, lazy: bool = False) -> list[Region]:
    """Regions of a context release; `ids` reads only those. `lazy` returns regions without
    arrays: samplers build their fields from disk one region at a time and keep them
    memory-mapped (see `region_fields`), so a large release need not fit in RAM."""
    manifest = json.loads(manifest_path.read_text())
    if manifest.get("kind") != "context_training_release" or manifest.get("schema_version") != 1:
        raise ValueError("unsupported context release")
    out = []
    for r in manifest["regions"]:
        if ids is not None and r["id"] not in ids:
            continue
        if lazy:
            out.append(Region(r["id"], int(r["class_id"]), r["split"], int(r["spacing_m"]), None, None, None,
                              None, source=(manifest_path, r)))
        else:
            out.append(_load_region(manifest_path, r))
    return out


def _load_region(manifest_path: Path, r: dict) -> Region:
    shape = (r["size"][1], r["size"][0])
    layers = {k: _read(manifest_path.parent, v, shape) for k, v in r["layers"].items()}
    height = layers["height"].astype(np.float32)
    if "validity" in layers:
        height = np.where(layers["validity"] == 255, height, np.nan).astype(np.float32)
    water = layers["water"] == 255
    if "seabed" in layers:
        height, sea = join_seabed(height, water, layers["seabed"], float(r["spacing_m"]))
        water &= ~sea
    return Region(r["id"], int(r["class_id"]), r["split"], int(r["spacing_m"]), height,
                  water, layers["snow-ice"] == 2, layers["vegetation"] == 2)


# Bump when field construction changes, so cached fields are rebuilt.
FIELDS_VERSION = 1


def region_fields(r: Region, level: "Level", make) -> Fields:
    """Fields of a region. Lazy regions are built once per release and level into
    data/cache/cascade-fields/<release>/ and memory-mapped from there."""
    if r.source is None:
        f = make(r)
        r.water = r.snow = r.vegetation = None
        return f
    manifest_path, entry = r.source
    release = Path(manifest_path).resolve().parent
    key = hashlib.sha256(json.dumps([FIELDS_VERSION, level.name, level.block, level.smooth_cells,
                                     sorted((k, v["sha256"]) for k, v in entry["layers"].items())]).encode()).hexdigest()
    cache = release.parents[1] / "cache" / "cascade-fields" / release.name / f"{entry['id']}-{level.name}"
    meta = cache / "fields.json"
    if not (meta.exists() and json.loads(meta.read_text()).get("key") == key):
        f = make(_load_region(Path(manifest_path), entry))
        if f.marks is not None:  # landmarks are small and not cached
            r.height = f.target
            return f
        tmp = cache.with_name(cache.name + ".tmp")
        import shutil

        shutil.rmtree(tmp, ignore_errors=True)
        tmp.mkdir(parents=True)
        np.save(tmp / "target.npy", f.target)
        if f.coarse is not None:
            np.save(tmp / "coarse.npy", f.coarse)
        for i, e in enumerate(f.extra):
            np.save(tmp / f"extra-{i}.npy", e)
        (tmp / "fields.json").write_text(json.dumps({"key": key, "extra": len(f.extra), "coarse": f.coarse is not None,
                                                    "class_id": f.class_id}))
        del f
        shutil.rmtree(cache, ignore_errors=True)
        tmp.rename(cache)
    m = json.loads(meta.read_text())
    load = lambda name: np.load(cache / name, mmap_mode="r")  # noqa: E731
    f = Fields(load("target.npy"), load("coarse.npy") if m["coarse"] else None,
               [load(f"extra-{i}.npy") for i in range(m["extra"])], int(m["class_id"]))
    r.height = f.target
    return f


def join_seabed(height: np.ndarray, water: np.ndarray, seabed: np.ndarray, cell_m: float):
    """GLO-30 with its sea (0 m) replaced by ground continuing under water.

    Near the coast the local land slope carries on down to the deepest GEBCO depth within
    NEARSHORE_M; GEBCO itself takes over from 1 to 3 km out. Returns (surface, sea)."""
    from scipy import ndimage

    # GLO-30 sea is 0 m; the water layer misses marinas and booms drawn as land.
    candidate = np.abs(np.nan_to_num(height, nan=1.0)) <= 0.5
    labels, _ = ndimage.label(candidate)
    deep = np.unique(labels[candidate & (seabed < -SEA_MIN_DEPTH_M)])
    sea = np.isin(labels, deep[deep > 0])
    if not sea.any():
        return height, sea
    dist, (iy, ix) = ndimage.distance_transform_edt(sea, return_indices=True)
    dist_m = dist * cell_m
    # Land slope next to the coast, averaged along it, read at each sea cell's nearest shore.
    gy, gx = np.gradient(np.nan_to_num(height), cell_m)
    band = (~sea & (ndimage.distance_transform_edt(~sea) * cell_m <= COAST_BAND_M)).astype(np.float32)
    sigma = COAST_BAND_M / cell_m
    num, den = smooth(np.hypot(gx, gy).astype(np.float32) * band, sigma), smooth(band, sigma)
    slope = np.where(den > 1e-3, num / np.maximum(den, 1e-3), 0.0)[iy, ix]
    lo, hi = np.tan(np.radians(SEA_SLOPE_DEG))
    extrap = -dist_m * np.clip(slope, lo, hi)
    near = 2 * int(NEARSHORE_M / cell_m) + 1
    deepest = ndimage.minimum_filter(seabed, size=near, mode="nearest")
    t = np.clip((dist_m - NEARSHORE_M) / (2 * NEARSHORE_M), 0, 1)
    t = t * t * (3 - 2 * t)
    floor = (1 - t) * deepest + t * seabed
    # Always a little below sea level and still deepening away from the coast.
    ceiling = -np.minimum(0.5 + 0.01 * dist_m, 20.0)
    bed = np.minimum(np.maximum(extrap, floor), ceiling).astype(np.float32)
    # Soften the kink where the slope meets the GEBCO floor, within the sea only.
    w = sea.astype(np.float32)
    bed = smooth(bed * w, 2.0) / np.maximum(smooth(w, 2.0), 1e-3)
    bed = np.minimum(bed, ceiling)
    return np.where(sea, bed, height).astype(np.float32), sea


def block_mean(a: np.ndarray, k: int) -> np.ndarray:
    h, w = a.shape[0] // k * k, a.shape[1] // k * k
    return a[:h, :w].reshape(h // k, k, w // k, k).mean(axis=(1, 3), dtype=np.float64).astype(np.float32)


def upsample(a: np.ndarray, k: int) -> np.ndarray:
    """Bicubic k-times upsampling to cell centres."""
    import torch
    import torch.nn.functional as F

    t = torch.from_numpy(a.astype(np.float64))[None, None]
    return F.interpolate(t, scale_factor=k, mode="bicubic", align_corners=False)[0, 0].numpy().astype(np.float32)


def _extreme(a: np.ndarray, size: int, maximum: bool) -> np.ndarray:
    import torch
    import torch.nn.functional as F

    t = torch.from_numpy(a if maximum else -a)[None, None]
    r = size // 2
    t = F.max_pool2d(F.pad(t, (r, r, r, r), mode="replicate"), size, stride=1)
    out = t[0, 0, : a.shape[0], : a.shape[1]].numpy()
    return out if maximum else -out


def envelope(h128: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """L0 envelope from a 128 m surface: local low and high over ~2 km, smoothed at 2 km."""
    cells = ENVELOPE_M // 128
    lo = smooth(_extreme(h128, cells, False), cells)
    hi = smooth(_extreme(h128, cells, True), cells)
    return lo, np.maximum(hi, lo)


def unit_range(c: np.ndarray) -> np.ndarray:
    lo, hi = float(c.min()), float(c.max())
    return ((c - lo) / max(hi - lo, MIN_RANGE_M) * 2 - 1).astype(np.float32)


def slopes(c: np.ndarray, cell_m: float) -> tuple[np.ndarray, np.ndarray]:
    """East and north gradients (m/m) of a north-up surface, clipped to +-2."""
    gy, gx = np.gradient(c, cell_m)
    return np.clip(gx, -2, 2).astype(np.float32), np.clip(-gy, -2, 2).astype(np.float32)


def soft(mask: np.ndarray, sigma_cells: float) -> np.ndarray:
    return (np.clip(smooth(mask.astype(np.float32), sigma_cells), 0, 1) * 2 - 1).astype(np.float32)


def soft_u8(mask: np.ndarray, sigma_cells: float) -> np.ndarray:
    """Soft coverage stored as uint8 (0 absent .. 255 present); see `decode`."""
    return np.rint(np.clip(smooth(mask.astype(np.float32), sigma_cells), 0, 1) * 255).astype(np.uint8)


def decode(e: np.ndarray) -> np.ndarray:
    """A conditioning channel as float32; uint8 coverage maps to -1 .. 1."""
    return e.astype(np.float32) / 127.5 - 1 if e.dtype == np.uint8 else e


@dataclass
class Fields:
    """Full-region inputs of one level; channel 0 holds raw coarse metres."""

    target: np.ndarray  # true surface at the level's cell size
    coarse: np.ndarray | None  # None: computed per window
    extra: list[np.ndarray]  # conditioning channels after shape, slopes and absolute height
    class_id: int
    marks: tuple | None = None  # L1 layout 6 landmarks (see `landmarks`)


# Landmarks (L1 layout 6): deep valleys and prominent peaks of the true surface, so an
# author's canyon line or summit has a learned meaning. Training windows are told about a
# random subset, so a missing mark never means "no valley here".
VALLEY_KM2 = 20.0  # drainage area of a marked valley
VALLEY_INCISION_M = 250.0  # below the highest ground within 1 km
VALLEY_DECAY_M = 300.0
PEAK_RADIUS_M = 1500.0  # highest point within this radius
PEAK_PROMINENCE_M = 300.0  # above the lowest ground within 3 km
PEAK_DECAY_M = 400.0
MARK_KEEP = 0.4  # share of valley networks and peaks a training window is told about
MARK_NONE = 0.2  # share of training windows told about none


def landmarks(h: np.ndarray, cell_m: float) -> tuple:
    """(valley value, valley labels, peak value, peak labels): values in km of incision or
    prominence on the marked cells, labels for choosing subsets."""
    from scipy import ndimage

    from . import hydrology

    h = np.nan_to_num(h, nan=0.0).astype(np.float32)
    sea = open_sea(h)
    _, parent, order = hydrology.flood(h, sea)
    area = hydrology.accumulate(parent, order).reshape(h.shape) * cell_m * cell_m / 1e6
    r = int(1000 / cell_m)
    incision = ndimage.maximum_filter(h, size=2 * r + 1) - h
    valley = (area >= VALLEY_KM2) & (incision >= VALLEY_INCISION_M) & ~sea
    vlab, _ = ndimage.label(valley, structure=np.ones((3, 3)))
    rp, rm = int(PEAK_RADIUS_M / cell_m), int(3000 / cell_m)
    prominence = h - ndimage.minimum_filter(h, size=2 * rm + 1)
    peak = (h == ndimage.maximum_filter(h, size=2 * rp + 1)) & (prominence >= PEAK_PROMINENCE_M) & (h > 0)
    plab, _ = ndimage.label(peak)
    return (np.where(valley, incision / 1000, 0).astype(np.float32), vlab,
            np.where(peak, prominence / 1000, 0).astype(np.float32), plab, cell_m)


def mark_channels(marks: tuple, sl=np.s_[:, :], rng: np.random.Generator | None = None) -> list[np.ndarray]:
    """Valley and peak channels of a window: each mark's value decaying with distance.
    With `rng` (training) only a random subset of marks is kept."""
    from scipy import ndimage

    vval, vlab, pval, plab, cell_m = marks
    out = []
    for val, lab, decay in ((vval, vlab, VALLEY_DECAY_M), (pval, plab, PEAK_DECAY_M)):
        v, lb = val[sl], lab[sl]
        keep = lb > 0
        if rng is not None:
            ids = np.unique(lb[keep])
            chosen = ids[rng.random(len(ids)) < MARK_KEEP] if rng.random() >= MARK_NONE else ids[:0]
            keep &= np.isin(lb, chosen)
        if not keep.any():
            out.append(np.zeros(v.shape, np.float32))
            continue
        d, (iy, ix) = ndimage.distance_transform_edt(~keep, return_indices=True)
        out.append((v[iy, ix] * np.exp(-d * cell_m / decay)).astype(np.float32))
    return out

def _nan_blocks(a: np.ndarray, k: int) -> np.ndarray:
    """Block means over surveyed cells; blocks under a quarter surveyed are filled."""
    from .data import fill_nan

    h, w = a.shape[0] // k * k, a.shape[1] // k * k
    b = a[:h, :w].reshape(h // k, k, w // k, k)
    count = (~np.isnan(b)).sum(axis=(1, 3))
    if count.min() == k * k:
        return b.mean(axis=(1, 3), dtype=np.float64).astype(np.float32)
    with np.errstate(invalid="ignore"):
        mean = (np.nansum(b, axis=(1, 3), dtype=np.float64) / count).astype(np.float32)
    mean[count < k * k // 4] = np.nan
    return fill_nan(mean)


def refine_coarse(level: Level, h: np.ndarray, y: int, x: int, size: int, oy: int = 0, ox: int = 0) -> np.ndarray:
    """Coarse surface of one window: block means on a grid shifted by (oy, ox) cells,
    upsampled and smoothed. Cells beyond the region repeat its edge."""
    k, m = level.block, level.margin
    n = -(-(size + 2 * m + k) // k) * k
    rows = np.clip(np.arange(y - m - oy, y - m - oy + n), 0, h.shape[0] - 1)
    cols = np.clip(np.arange(x - m - ox, x - m - ox + n), 0, h.shape[1] - 1)
    c = smooth(upsample(_nan_blocks(h[np.ix_(rows, cols)], k), k), level.smooth_cells)
    return c[m + oy : m + oy + size, m + ox : m + ox + size]


def refine_surface(level: Level, coarse: np.ndarray) -> np.ndarray:
    """Coarse surface of a whole field from the level above (generation)."""
    return smooth(upsample(coarse, level.block), level.smooth_cells)


def refine_fields(r: Region) -> Fields:
    """Per-cell inputs; the coarse surface is computed per window (see refine_coarse)."""
    # Stored as uint8 per region; full-size float32 copies would not fit in memory at 4 m.
    def cover(m):  # library tiles arrive with soft coverage already
        return m if m.dtype == np.uint8 else soft_u8(m, 1.0)

    extra = [np.where(r.water, 255, 0).astype(np.uint8), cover(r.snow), cover(r.vegetation)]
    return Fields(r.height, None, extra, r.class_id)


def l1_fields(r: Region) -> Fields:
    """The layout: envelope mid-surface and relief. Sea is wherever it lies below 0 m."""
    h = block_mean(r.height, 4)
    lo, hi = envelope(h)
    mid = (lo + hi) / 2
    marks = landmarks(h, 128.0) if LEVELS["l1"].conditioning == 6 else None
    return Fields(h, mid.astype(np.float32), [((hi - lo) / 2048.0).astype(np.float32)], r.class_id, marks)


def refine_mask(mask: np.ndarray, k: int, sigma_cells: float) -> np.ndarray:
    """A coarse mask enlarged k times with its block corners rounded."""
    big = np.repeat(np.repeat(mask, k, axis=0), k, axis=1)
    return smooth(big.astype(np.float32), sigma_cells) > 0.5


def open_sea(h: np.ndarray) -> np.ndarray:
    """Ground below sea level connected to the field edge: the sea of a generated surface.
    Closed basins below 0 m stay land (filled as lakes by drainage)."""
    from scipy import ndimage

    labels, _ = ndimage.label(h < 0)
    edge = np.unique(np.concatenate([labels[0], labels[-1], labels[:, 0], labels[:, -1]]))
    return np.isin(labels, edge[edge > 0])


def condition(level: Level, coarse: np.ndarray, extra: list[np.ndarray], classes: np.ndarray) -> np.ndarray:
    """Conditioning for one window. Channel 0 is raw coarse metres here; `normalise`
    turns it into the per-window unit-range shape channel (training and generation)."""
    ex, ny = slopes(coarse, level.cell_m)
    parts = [coarse[None], ex[None], ny[None], (coarse / 4096.0)[None]] + [e[None] for e in extra]
    parts.append(np.eye(NUM_CLASSES, dtype=np.float32)[classes].transpose(2, 0, 1))
    return np.concatenate(parts).astype(np.float32)


def drop(cond: np.ndarray, rng: np.random.Generator, coarse: float, cover: float, conditioning: int = 3) -> np.ndarray:
    """Zero the coarse or cover channels of some windows, in place."""
    n = len(cond)
    shape, cover_chans = DROP_CHANNELS[conditioning]
    for rate, chans in ((coarse, shape), (cover, cover_chans)):
        hit = rng.random(n) < rate
        for c in chans:
            cond[hit, c] = 0
    return cond


class LazyField:
    """Conditioning of a big field assembled per window (see model.sample_field), so host
    memory holds the coarse surface, its slopes and uint8 layers, not every channel."""

    def __init__(self, level: Level, coarse: np.ndarray, extra: list[np.ndarray], classes: int | np.ndarray):
        self.coarse, self.extra, self.classes = coarse, extra, classes
        self.ex, self.ny = slopes(coarse, level.cell_m)
        self.shape, self.device = coarse.shape, "cpu"

    def windows(self, boxes, size, device):
        import torch

        out = np.zeros((len(boxes), 4 + len(self.extra) + NUM_CLASSES, size, size), np.float32)
        for i, (y, x) in enumerate(boxes):
            sl = np.s_[y : y + size, x : x + size]
            c = self.coarse[sl]
            out[i, 0], out[i, 1], out[i, 2], out[i, 3] = c, self.ex[sl], self.ny[sl], c / 4096.0
            for j, e in enumerate(self.extra):
                out[i, 4 + j] = decode(e[sl])
            one_hot(out[i, 4 + len(self.extra) :], self.classes if isinstance(self.classes, int) else self.classes[sl])
        return torch.from_numpy(out).to(device)


def one_hot(channels: np.ndarray, classes: int | np.ndarray) -> None:
    """Set the class channels (NUM_CLASSES x h x w, zeroed) for one class or a class per cell."""
    if isinstance(classes, (int, np.integer)):
        channels[int(classes)] = 1.0
    else:
        np.put_along_axis(channels, classes[None].astype(np.intp), 1.0, axis=0)


def normalise(cond: np.ndarray) -> np.ndarray:
    cond = cond.copy()
    cond[0] = unit_range(cond[0])
    return cond


class Sampler:
    """Random windows of one split with the CropSampler interface of the 1 m model."""

    def __init__(self, regions: list[Region], split: str, level: Level, size: int, stride: int | None = None,
                 balanced: bool = False, mix: float = 0.0, blur: float = 0.0):
        """`balanced` draws a class, then a region of it, then a window (not by area);
        `mix` is the share of windows joined with a window of another class along a
        soft border, so the model sees class borders; `blur` is the share of refining
        windows whose coarse surface is smoothed further (conditioning augmentation), so
        a smooth coarse input does not imply smooth terrain."""
        self.balanced, self.mix, self.blur = balanced, mix, blur
        self.level, self.size = level, size
        stride = stride or level.stride
        role = {"train": 1, "validation": 2}[split]  # data.TRAIN, data.VALIDATION
        self.regions = [r for r in regions if r.split == split or (r.cells is not None and (r.cells == role).any())]
        if not self.regions:
            raise ValueError(f"no {split} regions")
        if any(r.spacing_m != level.region_m for r in self.regions):
            raise ValueError(f"{level.name} trains on {level.region_m} m regions")
        make = l1_fields if level.name == "l1" else refine_fields
        for r in self.regions:  # the fields hold everything the windows need
            if r.fields is None:
                r.fields = region_fields(r, level, make)
        self.fields = [r.fields for r in self.regions]
        self.positions = []
        for i, (r, f) in enumerate(zip(self.regions, self.fields)):
            rows, cols = f.target.shape
            ok = _window_sum(~np.isnan(f.target), size) >= MIN_VALID * size * size
            if r.cells is not None:
                ok &= _window_sum(r.cells == role, size) == size * size
            ys, xs = np.nonzero(ok[: rows - size + 1 : stride, : cols - size + 1 : stride])
            self.positions += [(i, int(y) * stride, int(x) * stride) for y, x in zip(ys, xs)]
        if not self.positions:
            raise ValueError("no window of this size has enough surveyed ground")

    def window(self, index: int, rng: np.random.Generator | None = None):
        """(true surface, coarse, cond with raw channel 0) of one window. Refining levels
        draw a random block offset so the model cannot learn a fixed alignment."""
        i, y, x = self.positions[index]
        f, s = self.fields[i], self.size
        sl = np.s_[y : y + s, x : x + s]
        if f.coarse is None:
            k = self.level.block
            oy, ox = (int(v) for v in rng.integers(k, size=2)) if rng is not None else (0, 0)
            c = refine_coarse(self.level, f.target, y, x, s, oy, ox)
            if rng is not None and self.blur and rng.random() < self.blur:
                # Target stays truth minus this coarse, so the model learns to add the lost relief.
                c = smooth(c, float(rng.uniform(0.5, 2.0)) * k)
        else:
            c = f.coarse[sl]
        classes = np.full((s, s), f.class_id, np.int64)
        extra = [decode(e[sl]) for e in f.extra] + (mark_channels(f.marks, sl, rng) if f.marks else [])
        return f.target[sl], c, condition(self.level, c, extra, classes)

    def crop(self, index: int, scale: float, rng: np.random.Generator, augment: bool, partner: int | None = None):
        h, c, cond = self.window(index, rng)
        if partner is not None:
            h, c, cond = _join((h, c, cond), self.window(partner, rng), rng)
        valid = ~np.isnan(h)
        if not self.level.water_in_loss:
            valid &= cond[4] < 0  # water channel: 1 water, -1 dry
        target = np.where(valid, (h - c) / scale, 0.0)[None].astype(np.float32)
        cond = normalise(cond)
        mask = valid[None].astype(np.float32)
        if augment:
            k, flip = rng.integers(4), rng.integers(2)
            target, cond, mask = (_dihedral(a, k, flip) for a in (target, cond, mask))
            cond = _rotate_slopes(cond, int(k), int(flip))
        return target, cond, mask

    def _groups(self):
        if not hasattr(self, "_by_class"):
            self._by_class: dict[int, dict[int, list[int]]] = {}
            for idx, (ri, _, _) in enumerate(self.positions):
                self._by_class.setdefault(self.fields[ri].class_id, {}).setdefault(ri, []).append(idx)
        return self._by_class

    def pick(self, rng: np.random.Generator, exclude_class: int | None = None) -> int:
        groups = {c: g for c, g in self._groups().items() if c != exclude_class}
        if not self.balanced and exclude_class is None:
            return int(rng.integers(len(self.positions)))
        if not groups:
            return int(rng.integers(len(self.positions)))
        classes = sorted(groups)
        regions = groups[classes[int(rng.integers(len(classes)))]]
        idxs = regions[sorted(regions)[int(rng.integers(len(regions)))]]
        return idxs[int(rng.integers(len(idxs)))]

    def batch(self, n: int, scale: float, rng: np.random.Generator, augment=True):
        parts = []
        for _ in range(n):
            p = self.pick(rng)
            partner = None
            if self.mix and rng.random() < self.mix:
                partner = self.pick(rng, exclude_class=self.fields[self.positions[p][0]].class_id)
            parts.append(self.crop(p, scale, rng, augment, partner))
        return tuple(np.stack(a) for a in zip(*parts))


def _join(a, b, rng: np.random.Generator):
    """Two windows joined along a random soft line: heights, coarse surfaces and
    continuous channels blend over about an eighth of the window; classes and water
    switch at the line."""
    (h1, c1, k1), (h2, c2, k2) = a, b
    s = h1.shape[0]
    angle, offset = rng.uniform(0, np.pi), rng.uniform(-0.25, 0.25) * s
    yy, xx = np.mgrid[0:s, 0:s] - (s - 1) / 2
    d = xx * np.cos(angle) + yy * np.sin(angle) - offset
    t = np.clip(0.5 + d / (s / 8), 0, 1)
    m = (t * t * (3 - 2 * t)).astype(np.float32)
    h = m * h1 + (1 - m) * h2
    c = m * c1 + (1 - m) * c2
    cond = m * k1 + (1 - m) * k2
    hard = d > 0
    first = class_start_of(k1)
    cond[first:] = np.where(hard, k1[first:], k2[first:])
    cond[4] = np.where(hard, k1[4], k2[4])  # water (L2-L4) or relief (L1): switch, not blend
    return h.astype(np.float32), c.astype(np.float32), cond.astype(np.float32)


def class_start_of(cond: np.ndarray) -> int:
    return cond.shape[0] - NUM_CLASSES


def _dihedral(a: np.ndarray, k: int, flip: int) -> np.ndarray:
    a = np.rot90(a, k, axes=(1, 2))
    return np.ascontiguousarray(a[:, :, ::-1] if flip else a)


def _rotate_slopes(cond: np.ndarray, k: int, flip: int) -> np.ndarray:
    """Recompute slope channels after rotating the grid so they stay east/north."""
    # Channel 0 is already unit-range; its gradients point the same way as the metre field's.
    ex, ny = cond[1].copy(), cond[2].copy()
    for _ in range(k):  # np.rot90 turns counter-clockwise: east becomes north
        ex, ny = -ny, ex
    if flip:
        ex = -ex
    cond[1], cond[2] = ex, ny
    return cond


def residual_scale(sampler: Sampler, windows: int = 400) -> float:
    """Fixed metre scale: twice the residual std over training windows, to 0.5 m."""
    rng = np.random.default_rng(0)
    parts = []
    for p in rng.choice(len(sampler.positions), size=min(windows, len(sampler.positions)), replace=False):
        h, c, _ = sampler.window(int(p))
        parts.append((h - c)[~np.isnan(h)])
    std = float(np.concatenate(parts).std())
    return max(0.5, round(2 * std * 2) / 2)
