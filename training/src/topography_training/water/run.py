"""Hydrology v3 drivers: the 1 m water on a world's kept raw surface, a test window, or one
coarse level (32 m or 4 m) from a prepared grid.

python -m topography_training.water.run world --source <l4 world> --out <new world> [--tile 2048] [--threads 4]
python -m topography_training.water.run window --source <world> --level {1,4,32} --centre E,N --size N --out <dir>
python -m topography_training.water.run level --run <dir with grid/> [--coarse <coarser out>] [--river-km2 X]

The Rust `topography world water` runs the exact graph passes into the work directory
(resumable), `world water-finish` checks the carved ground and publishes `hydrology/1m`, and
the world's layers are reflinked from it. Plan A ships the carved ground: there is no bank
relaxation. Rain is the chain world's real 128 m moisture field; a run without it is refused.
Rerunning the same command resumes; the binary is pinned in the run directory, so delete
`hydrology/bin` (and with it every cache) to run a new build. Never writes to the source.
Exit codes: 0 passed, 1 checks failed (outputs kept, manifest-failed.json), 2 refused or failed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path

import numpy as np

from ..diffusion import cascade, hydrology
from . import grids, review

RAW = "elevation-raw.f32"
# Disk per cell while running: the work directory (breached, carved, body, m2/m3 rasters,
# water layers, tile caches) measured 52-53 B on l4v2 windows (Task 16), plus the world's
# water.u8; published files are reflinks. Required free: what the work directory still lacks
# of this, times 1.1, plus 10 GB.
DISK_BYTES = 54
DISK_MARGIN = 1.1
DISK_SPARE = 10e9
# A launcher's own files in a fresh run directory (AGENTS.md long jobs).
LAUNCH_FILES = {"run.log", "run.pid", "exit_code"}
MOISTURE_KEYS =("file", "size", "spacing_m", "nw_m", "scale_mm_h")


class Refused(Exception):
    """A run that cannot start or continue (exit 2)."""


def _json(path: Path) -> dict:
    return json.loads(path.read_text())


def _write_if_changed(path: Path, value: dict) -> None:
    """Write JSON only when it differs: the Rust resume stamp digests grid.json."""
    text = json.dumps(value, indent=1)
    if not path.exists() or path.read_text() != text:
        tmp = path.with_name(path.name + ".tmp")
        tmp.write_text(text)
        tmp.replace(path)


def _moisture(src: Path, manifest: dict) -> tuple[Path, dict, float]:
    """The real rain: the moisture field, its grid.json entry and the mean land rain from the
    4 m hydrology of the chain world (the source itself, or the source's source world). Never
    falls back to uniform rain."""
    for world in (src, Path(manifest.get("source_world") or src)):
        grid4 = world / "hydrology" / "4m" / "grid.json"
        if not grid4.exists():
            continue
        g = _json(grid4)
        entry = g.get("moisture") or {}
        mean = (g.get("discharge") or {}).get("mean_land_mm_h")
        missing = [k for k in MOISTURE_KEYS if k not in entry] + ([] if mean else ["discharge.mean_land_mm_h"])
        if missing:
            raise Refused(f"{grid4} lacks {', '.join(missing)}: no real rain")
        field = grid4.parent / entry["file"]
        if not field.exists():
            raise Refused(f"no moisture field {field}: no real rain")
        return field, dict(entry), float(mean)
    raise Refused(f"no hydrology/4m/grid.json in {src} or its source world: no real rain")


def _symlink(target: Path | str, link: Path) -> None:
    if link.is_symlink() and Path(link.readlink()) == Path(target):
        return
    link.unlink(missing_ok=True)
    link.symlink_to(target)


def _origin(m: dict, n: int, spacing: float) -> list[float]:
    return list(_json(Path(m["world_input"]) / "world.json").get("origin", [0.0, n * spacing]))


def build_world_grid(src: Path, out: Path) -> Path:
    """The 1 m grid of an L4 world (whole or a square): the reflinked raw surface, the chain's
    4 m classes and provenance (cropped for a square), the real moisture field."""
    m = _json(src / "manifest.json")
    n = m["size"][0]
    field, entry, mean = _moisture(src, m)
    chain = Path(m["source_world"])
    n4 = _json(chain / "manifest.json")["size"][0]
    c4 = n // 4
    col4, row4 = (m.get("params") or {}).get("window_4m") or [(n4 - c4) // 2] * 2
    classes = chain / "classes.u8"
    if not classes.exists() or classes.stat().st_size != n4 * n4:
        raise Refused(f"{classes} is not the {n4}² 4 m class map of {chain}")
    prov = chain / "provenance.u8"
    if not prov.exists():
        raise Refused(f"{chain} has no provenance.u8 (a v3 chain world writes it)")
    grid = out / "hydrology" / "grid"
    grid.mkdir(parents=True, exist_ok=True)
    _symlink(Path("..", "..", RAW), grid / "elevation.f32")
    extra = {}
    if c4 == n4:
        _symlink(classes.resolve(), grid / "classes.u8")
        _symlink(prov.resolve(), grid / "provenance.u8")
        extra["provenance_cell_m"] = 32.0
    else:
        cls = np.fromfile(classes, np.uint8).reshape(n4, n4)[row4 : row4 + c4, col4 : col4 + c4]
        _write_if_bytes_differ(grid / "classes.u8", np.ascontiguousarray(cls))
        p32 = np.fromfile(prov, np.uint8).reshape(n4 // 8, n4 // 8)
        _write_if_bytes_differ(grid / "provenance.u8",
                               grids.provenance_window(p32, col4 * 4.0, row4 * 4.0, c4, 4.0))
        extra["provenance_cell_m"] = 4.0
    _symlink(field.resolve(), grid / "moisture.f32")
    # The 1 m snow/ice layer: cold cells give only meltwater and freeze standing water.
    snow = _snow(src, n)
    _symlink(snow.resolve(), grid / "snow-ice.u8")
    _write_if_changed(grid / "grid.json", {
        "size": [n, n], "spacing_m": 1.0, "origin": _origin(m, n, 1.0), "classes_cell_m": 4.0, **extra,
        "snow_cell_m": 1.0,
        "moisture": entry | {"file": "moisture.f32"}, "discharge": {"mean_land_mm_h": mean},
        "inputs": _inputs(classes, field, snow),
    })
    return grid


def _snow(src: Path, n: int) -> Path:
    """The raw world's n² 1 m `snow-ice.u8` (2: snow/ice)."""
    snow = src / "snow-ice.u8"
    if not snow.exists() or snow.stat().st_size != n * n:
        raise Refused(f"{snow} is not the {n}² 1 m snow/ice layer of {src}")
    return snow


def _write_if_bytes_differ(path: Path, a: np.ndarray) -> None:
    b = a.tobytes()
    if not path.exists() or path.is_symlink() or path.read_bytes() != b:
        tmp = path.with_suffix(path.suffix + ".tmp")
        tmp.write_bytes(b)
        tmp.replace(path)


def _inputs(classes: Path, moisture: Path, snow: Path | None = None) -> dict:
    """What the grid was built from, so a changed class map, moisture field or snow layer
    changes grid.json (and with it the driver's resume key)."""
    def ident(p: Path) -> dict:
        st = p.stat()
        return {"file": str(p.resolve()), "bytes": st.st_size, "modified_ns": st.st_mtime_ns}
    out = {"classes": ident(classes),
           "moisture": {"file": str(moisture.resolve()), "sha256": hydrology.sha256(moisture)}}
    if snow is not None:
        out["snow"] = ident(snow)
    return out


def _mode_classes(c4: np.ndarray, k: int) -> np.ndarray:
    """The most common class in each k × k block (ties to the lower class)."""
    h, w = c4.shape[0] // k, c4.shape[1] // k
    blocks = c4.reshape(h, k, w, k)
    best, count = np.zeros((h, w), np.uint8), np.full((h, w), -1)
    for c in np.unique(c4):
        n = (blocks == c).sum(axis=(1, 3))
        more = n > count
        best[more], count[more] = c, n[more]
    return best


def _read_window(path: Path, dtype, n: int, row: int, col: int, h: int, w: int) -> np.ndarray:
    """Rows [row, row + h) and columns [col, col + w) of an n-wide raster, read row by row
    (a memmap would map whole 65,536-cell rows)."""
    dtype = np.dtype(dtype)
    out = np.empty((h, w), dtype)
    with path.open("rb") as f:
        for i in range(h):
            f.seek(((row + i) * n + col) * dtype.itemsize)
            out[i] = np.frombuffer(f.read(w * dtype.itemsize), dtype)
    return out


def build_window_grid(src: Path, level: int, centre: tuple[float, float], size: int, dest: Path) -> Path:
    """A test window at level 1 (the L4 raw surface), 4 (the chain world's ground) or 32 (the
    chain's 32 m `l2_raw`), centred on (east, north) metres. Kept as is when already built
    for the same window."""
    m = _json(src / "manifest.json")
    field_m, entry, mean = _moisture(src, m)
    if level == 1:
        n, spacing, k = m["size"][0], 1.0, 4
    elif level == 4:
        n, spacing, k = m["size"][0], 4.0, 1
    elif level == 32:
        n, spacing, k = m["size"][0] // 8, 32.0, 1
    else:
        raise ValueError(f"level {level}: use 1, 4 or 32")
    if size > n or size % k:
        raise Refused(f"window {size}: at most {n} and a multiple of {k} at level {level}")
    origin = _origin(m, n, spacing)
    col = int(round((centre[0] - origin[0]) / spacing - size / 2)) // k * k
    row = int(round((origin[1] - centre[1]) / spacing - size / 2)) // k * k
    col, row = min(max(col, 0), n - size), min(max(row, 0), n - size)
    chain = Path(m["source_world"]) if level == 1 else src
    classes_src = chain / "classes.u8"
    # Where the source sits in the chain, in 4 m cells (an L4 square is a part of it).
    col4 = row4 = 0
    n4 = n // 4 if level == 1 else n if level == 4 else n * 8  # the chain's 4 m cells
    if level == 1:
        n4 = _json(chain / "manifest.json")["size"][0] if (chain / "manifest.json").exists() else n4
        col4, row4 = (m.get("params") or {}).get("window_4m") or [(n4 - n // 4) // 2] * 2
    prov_src = chain / "provenance.u8"
    if not prov_src.exists():
        print(f"note: no provenance.u8 in {chain}; the window runs without flags "
              "(all below-sea basins floored)", flush=True)
    # The climate's snow/ice layer exists only at 1 m.
    snow_src = _snow(src, n) if level == 1 else None
    manifest = {
        "size": [size, size], "spacing_m": spacing, "origin": origin,
        "offset_m": [col * spacing, row * spacing], "classes_cell_m": spacing * k,
        **({"provenance_cell_m": spacing} if prov_src.exists() else {}),
        **({"snow_cell_m": spacing} if snow_src else {}),
        "moisture": entry | {"file": "moisture.f32"}, "discharge": {"mean_land_mm_h": mean},
        "window": {"source": str(src.resolve()), "level": level, "centre_m": list(centre)},
        "inputs": _inputs(classes_src, field_m, snow_src),
    }
    sizes = {"elevation.f32": size * size * 4, "classes.u8": (size // k) ** 2}
    if prov_src.exists():
        sizes["provenance.u8"] = size * size
    if snow_src:
        sizes["snow-ice.u8"] = size * size
    if ((dest / "grid.json").exists() and _json(dest / "grid.json") == manifest
            and all((dest / f).exists() and (dest / f).stat().st_size == b for f, b in sizes.items())):
        return dest
    dest.mkdir(parents=True, exist_ok=True)
    (dest / "grid.json").unlink(missing_ok=True)
    if level == 1:
        field = _read_window(src / RAW, "<f4", n, row, col, size, size)
        classes = _read_window(classes_src, np.uint8, n4, row4 + row // 4, col4 + col // 4, size // 4, size // 4)
    elif level == 4:
        # A v3 chain keeps the raw L3 surface; v2's ground is its elevation.
        ground = src / "l3-raw.f32" if (src / "l3-raw.f32").exists() else src / "elevation.f32"
        field = _read_window(ground, "<f4", n, row, col, size, size)
        classes = _read_window(src / "classes.u8", np.uint8, n, row, col, size, size)
    else:
        l2 = np.load(src / "arrays.npz")["l2_raw"]
        if l2.shape != (n, n):
            raise Refused(f"{src / 'arrays.npz'} l2_raw is {l2.shape}, not the {n}² 32 m grid")
        field = l2[row : row + size, col : col + size]
        c4 = _read_window(src / "classes.u8", np.uint8, n * 8, row * 8, col * 8, size * 8, size * 8)
        classes = _mode_classes(c4, 8)
    layers = [("elevation.f32", field, "<f4"), ("classes.u8", classes, np.uint8)]
    if snow_src:
        layers.append(("snow-ice.u8", _read_window(snow_src, np.uint8, n, row, col, size, size), np.uint8))
    else:
        (dest / "snow-ice.u8").unlink(missing_ok=True)
    if prov_src.exists():
        p32 = np.fromfile(prov_src, np.uint8).reshape(n4 // 8, n4 // 8)
        layers.append(("provenance.u8", grids.provenance_window(p32, col4 * 4.0 + col * spacing,
                                                                 row4 * 4.0 + row * spacing, size, spacing), np.uint8))
    else:
        (dest / "provenance.u8").unlink(missing_ok=True)
    # Collage regions for the blend-zone figures at the erosion levels (read at provenance_cell_m).
    regions = _region32(chain) if level != 1 else None
    if regions is not None and prov_src.exists():
        layers.append(("region.u8", grids.provenance_window(regions, col * spacing, row * spacing, size, spacing),
                       np.uint8))
    else:
        (dest / "region.u8").unlink(missing_ok=True)
    for name, a, dtype in layers:
        np.ascontiguousarray(a, dtype).tofile(dest / (name + ".tmp"))
        (dest / (name + ".tmp")).replace(dest / name)
    _symlink(field_m.resolve(), dest / "moisture.f32")
    _write_if_changed(dest / "grid.json", manifest)
    return dest


def _region32(chain: Path) -> np.ndarray | None:
    """The collage's 32 m region ids (`region32.u8`) beside the chain world or in its collage."""
    m = _json(chain / "manifest.json") if (chain / "manifest.json").exists() else {}
    n32 = m.get("size", [0])[0] // 8
    for d in (chain, Path(m["collage"]) if m.get("collage") else None):
        if d is not None and (d / "region32.u8").exists() and (d / "region32.u8").stat().st_size == n32 * n32 > 0:
            return np.fromfile(d / "region32.u8", np.uint8).reshape(n32, n32)
    return None


def _pin(bin_dir: Path) -> tuple[Path, str]:
    """The run's pinned binary: the copy already in `bin_dir` on a resume, else a fresh copy of
    the release build. Every command of the run uses it (the resume stamp holds its digest)."""
    pins = sorted(p for p in bin_dir.glob("topography-*") if p.suffix != ".tmp")
    if len(pins) > 1:
        raise Refused(f"{bin_dir} holds {len(pins)} pinned binaries; keep one")
    if pins:
        digest = hydrology.sha256(pins[0])
        if pins[0].name != f"topography-{digest[:12]}":
            raise Refused(f"{pins[0]} does not match its digest {digest[:12]}")
        hydrology.use_binary(pins[0])
        path = pins[0]
    else:
        # Copy the release build, not a pin an earlier run in this process selected.
        hydrology.use_binary(None)
        path, digest = hydrology.pin_binary(bin_dir)
    print(f"water binary: {path} (sha256 {digest[:12]})", flush=True)
    return path, digest


def _child_peak_mb() -> int:
    """The largest peak RSS of any finished child so far (the Rust commands)."""
    import resource

    return resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss // 1024


def _rss_anon_mb() -> int:
    """The driver's anonymous resident memory now (file-backed pages excluded), MB."""
    try:
        for line in Path("/proc/self/status").read_text().splitlines():
            if line.startswith("RssAnon:"):
                return int(line.split()[1]) // 1024
    except OSError:
        pass
    return 0


class AnonPeak:
    """Samples RssAnon every 0.2 s while open: the driver's real allocation peak (ru_maxrss also
    counts mapped file pages)."""

    def __enter__(self):
        import threading

        self.peak, self._stop = _rss_anon_mb(), threading.Event()
        self._thread = threading.Thread(target=self._watch, daemon=True)
        self._thread.start()
        return self

    def _watch(self) -> None:
        while not self._stop.wait(0.2):
            self.peak = max(self.peak, _rss_anon_mb())

    def __exit__(self, *exc) -> None:
        self._stop.set()
        self._thread.join()
        self.peak = max(self.peak, _rss_anon_mb())


def _spots_arg() -> str:
    """The review spots for `water-finish --spots` (the `dam_drops_at_spots` figure)."""
    return ";".join(f"{e},{n}" for e, n in review.SPOTS)


def _rust(cmd: list[str]) -> int:
    print("$", " ".join(cmd), flush=True)
    return subprocess.run(cmd).returncode


def _used(path: Path) -> int:
    """Bytes allocated under `path`."""
    return sum(p.stat().st_blocks * 512 for p in path.rglob("*") if p.is_file() and not p.is_symlink()) \
        if path.exists() else 0


def _check_disk(where: Path, cells: int, work: Path) -> None:
    need = max(cells * DISK_BYTES - _used(work), 0) * DISK_MARGIN + DISK_SPARE
    try:
        hydrology.check_disk(where, need, f"the water run ({cells} cells, {DISK_BYTES} B a cell)")
    except SystemExit as e:
        raise Refused(str(e)) from None


class Progress:
    """progress.json of a run directory: its source, pinned binary and finished steps."""

    def __init__(self, path: Path, source: Path):
        self.path = path
        self.state = _json(path) if path.exists() else {"source": str(source.resolve()), "steps": {}}
        if self.state["source"] != str(source.resolve()):
            raise Refused(f"{path.parent} was started from {self.state['source']}")

    def key(self, digest: str, tile: int, grid: Path, work: Path) -> None:
        """A new pinned binary, tile size or grid.json (window, moisture, class map) invalidates
        every step after the raw copy and the work directory (Rust refuses a foreign one)."""
        key = {"binary": digest, "tile": tile, "grid": hydrology.sha256(grid / "grid.json")}
        if {k: self.state.get(k) for k in key} != key:
            if work.exists():
                print(f"inputs or settings changed: removing {work}", flush=True)
                shutil.rmtree(work)
            self.state["steps"] = {k: v for k, v in self.state["steps"].items() if k == "raw"}
            self.state |= key
            self.save()

    def done(self, step: str) -> dict | None:
        return self.state["steps"].get(step)

    def mark(self, step: str, **info) -> None:
        self.state["steps"][step] = info
        self.save()

    def save(self) -> None:
        cascade.save_json(self.path, self.state)


def _layout(src: Path) -> Path | None:
    """The L0 layout of a source world (its manifest's, else its source world's), whose
    snowline turns ice lakes into glaciers in `water-finish`; None without one."""
    m = _json(src / "manifest.json")
    for m in (m, _json(Path(m["source_world"]) / "manifest.json") if m.get("source_world") else {}):
        path = m.get("layout") or (m.get("cover") or {}).get("layout")
        if path and (Path(path) / "snowline.f32").exists():
            return Path(path)
    return None


def _water(progress: Progress, binary: Path, grid: Path, work: Path, out1m: Path, tile: int, threads: int,
           river_km2: float | None = None, layout: Path | None = None) -> int:
    """`world water` then `world water-finish` on the carved ground, each skipped when done.
    `layout` (finish only, outside the work's key) gives the snowline for glaciers.
    Returns finish's 0 (passed) or 1 (checks failed); a failed command raises Refused."""
    flags = ["--grid", str(grid), "--work", str(work), "--tile", str(tile), "--threads", str(threads)]
    if river_km2 is not None:
        flags += ["--river-km2", str(river_km2)]
    if not progress.done("water"):
        started = time.time()
        if _rust([str(binary), "world", "water", *flags]) != 0:
            raise Refused("world water failed")
        progress.mark("water", seconds=round(time.time() - started, 1), peak_mb=_child_peak_mb())
    if not (step := progress.done("finish")):
        if out1m.exists():
            shutil.rmtree(out1m)
        started = time.time()
        glaciers = ["--layout", str(layout)] if layout is not None else []
        code = _rust([str(binary), "world", "water-finish", *flags, "--ground", str(work / "carved.f32"),
                      "--out", str(out1m), "--spots", _spots_arg(), *glaciers])
        if code not in (0, 1) or not (out1m / "report.json").exists():
            raise Refused(f"world water-finish failed (exit {code})")
        step = {"code": code, "seconds": round(time.time() - started, 1), "peak_mb": _child_peak_mb()}
        progress.mark("finish", **step)
    return step["code"]


def _reflink(src: Path, dst: Path, replace: bool = False) -> None:
    """A btrfs reflink of `src` at `dst` (R18, N12): no space, and nothing written to one can
    reach the other. Fails loudly where the file system cannot reflink: a 17 GB copy is never
    made silently. Kept unless `replace`."""
    if dst.exists() and not replace:
        return
    tmp = dst.with_suffix(dst.suffix + ".tmp")
    done = subprocess.run(["cp", "--reflink=always", str(src), str(tmp)], capture_output=True, text=True)
    if done.returncode != 0:
        tmp.unlink(missing_ok=True)
        raise Refused(f"cannot reflink {src} to {tmp} ({done.stderr.strip()}); "
                      "the world must live on the same btrfs file system as its source")
    tmp.replace(dst)


def _water_u8(surface: Path, body: Path, bodies: Path, dest: Path, n: int) -> str:
    """255 where the water surface is finite and not ice (a frozen lake); returns its sha256."""
    s = np.memmap(surface, "<f4", "r", shape=(n, n))
    b = np.memmap(body, "<u4", "r", shape=(n, n))
    ice = np.array([e["id"] for e in _json(bodies)["bodies"] if e.get("kind") == "ice"], np.uint32)
    tmp = dest.with_suffix(".tmp")
    h = hashlib.sha256()
    with tmp.open("wb") as f:
        for y in range(0, n, 1024):
            liquid = np.isfinite(s[y : y + 1024]) & ~np.isin(b[y : y + 1024], ice)
            rows = np.where(liquid, 255, 0).astype(np.uint8)
            f.write(rows.tobytes())
            h.update(rows.tobytes())
    tmp.replace(dest)
    return h.hexdigest()


def _pictures(out: Path, n: int) -> None:
    from ..diffusion.cascade_chain import _png

    e = np.memmap(out / "elevation.f32", "<f4", "r", shape=(n, n))
    wm = np.memmap(out / "water.u8", np.uint8, "r", shape=(n, n))
    k = 8 if n >= 8192 else 1
    over = np.concatenate([cascade.block_mean(np.asarray(e[y : y + 1024]), k) for y in range(0, n, 1024)])
    wover = np.concatenate([cascade.block_mean(np.asarray(wm[y : y + 1024], np.float32), k)
                            for y in range(0, n, 1024)]) > 127
    _png(out / "overview.png", over, float(k), wover, min(2048, n // k))
    mid, r = n // 2, min(512, n // 2)
    _png(out / "detail.png", np.asarray(e[mid - r : mid + r, mid - r : mid + r]), 1.0,
         np.asarray(wm[mid - r : mid + r, mid - r : mid + r]) == 255, 2 * r)


def _assemble(src: Path, out: Path, m: dict, out1m: Path, moisture: tuple[Path, dict, float],
              binary: Path, digest: str, raw_sha256: str, tile: int, threads: int) -> int:
    """The world: water layers reflinked from hydrology/1m, the rest from the source."""
    n = m["size"][0]
    report = _json(out1m / "report.json")
    published = _json(out1m / "grid.json")["provenance"]["outputs"]
    names = ("elevation.f32", "water-surface.f32", "water-body.u32", "bodies.json")
    for name in names:
        _reflink(out1m / name, out / name, replace=True)
    digests = {name: published[name] for name in names}
    # Glacier ice (game layers read it as snow/ice); elevation.f32 is its surface.
    glacier = out1m / "glacier.u8"
    if glacier.exists():
        _reflink(glacier, out / "glacier.u8", replace=True)
        digests["glacier.u8"] = published["glacier.u8"]
    else:
        (out / "glacier.u8").unlink(missing_ok=True)
    digests["water.u8"] = _water_u8(out / "water-surface.f32", out / "water-body.u32", out / "bodies.json",
                                    out / "water.u8", n)
    for name in ("snow-ice.u8", "vegetation.u8"):
        _reflink(src / name, out / name)
        digests[name] = (m.get("layers") or {}).get(name) or hydrology.sha256(out / name)
    field, entry, _ = moisture
    _reflink(field, out / "moisture.f32", replace=True)
    digests["moisture.f32"] = hydrology.sha256(out / "moisture.f32")
    if (src / "moisture.f32").exists() and hydrology.sha256(src / "moisture.f32") != digests["moisture.f32"]:
        print(f"warning: {src / 'moisture.f32'} differs from the chain's {field}; the water used the chain's",
              flush=True)
    world_input = out / "world-input"
    world_input.mkdir(exist_ok=True)
    shutil.copy(Path(m["world_input"]) / "world.json", world_input / "world.json")
    extra = {}
    if "description" in m:
        for name in ("description.json", "description.md"):
            if (src / name).exists():
                shutil.copy(src / name, out / name)
        extra["description"] = "description.json"
        if (out / "description.json").exists():
            digests["description.json"] = hydrology.sha256(out / "description.json")
    _pictures(out, n)
    # Next to the Rust output, never inside hydrology/1m (its provenance lists its files).
    reviewed = _review(out, out1m, _coarse_water(Path(m["source_world"]) / "hydrology" / "4m"),
                       out / "hydrology" / "review")
    # The coarse guide is a figure (owner rule 2026-11-08): only the water's checks decide.
    passed = report["passed"]
    failures = report["failures"]
    manifest = {
        "schema_version": 1, "kind": "world_preview", "world_input": str(world_input.resolve()),
        "spacing_m": 1.0, "size": [n, n], "class_id": m.get("class_id"),
        "method": "hydrology v3 1 m water over the kept L4 raw surface of the source world",
        "water_fill": hydrology.WATER_FILL, "moisture": entry, **extra,
        **({"glacier": {"file": "glacier.u8", "value": "255 glacier ice (elevation.f32 is its rough surface; the water routes over "
                                 "the smooth one), 0 none",
                        "bodies": "bodies.json kind glacier"}} if glacier.exists() else {}),
        **{k: m[k] for k in ("layout",) if k in m},
        "source_world": m["source_world"], "hydrology_source": str(src.resolve()),
        "hydrology_input": {"file": RAW, "sha256": raw_sha256,
                            "rerun": "python -m topography_training.water.run world --source <source> --out <this world>"},
        "water": {"passed": passed, "failures": failures, "relaxation": "none",
                  "model": "hydrology v3", "report": "hydrology/1m/report.json", "ground": "carved.f32",
                  "binary": str(binary.relative_to(out)), "binary_sha256": digest,
                  "tile": tile, "threads": threads,
                  **reviewed},
        "layers": digests,
    }
    name = "manifest.json" if passed else "manifest-failed.json"
    cascade.save_json(out / name, manifest)
    (out / ("manifest-failed.json" if passed else "manifest.json")).unlink(missing_ok=True)
    print(f"done: {out} ({'passed' if passed else 'FAILED: ' + ', '.join(failures)})", flush=True)
    return 0 if passed else 1


def _coarse_water(coarse: Path) -> Path:
    """`coarse` if it holds the coarser level's water; else the run is refused."""
    if not (coarse / "water-body.u32").exists():
        raise Refused(f"no water-body.u32 in {coarse}: run the coarser level first")
    return coarse


def _review(src: Path, fine: Path, coarse: Path | None, dest: Path) -> dict:
    """The review figures after water-finish, made afresh in `dest`: the coarse guide of `fine`
    against the next coarser level's water `coarse` (when it has water) with a figure per
    mismatch, and the crops of `src` (a world or window run). The guide is a figure (owner rule
    2026-11-08). Never raises: a failure is logged and recorded as {"error": ...} and the run
    goes on. Returns the manifest entries."""
    import resource
    import traceback

    started = time.time()
    entry: dict = {"coarse_guide": None, "review": None}
    with AnonPeak() as anon:
        try:
            if dest.exists():
                shutil.rmtree(dest)
            dest.mkdir(parents=True)
            if coarse is not None and (coarse / "water-body.u32").exists():
                guide = review.coarse_guide(src, coarse, fine_dir=fine)
                cascade.save_json(dest / "coarse-guide.json", guide)
                figures = review.guide_figures(src, coarse, guide, dest / "coarse-guide")
                entry["coarse_guide"] = {k: guide[k] for k in review.GUIDE_SUMMARY} | {
                    "file": str((dest / "coarse-guide.json").relative_to(src)),
                    "figures": [str(p.relative_to(src)) for p in figures]}
        except Exception as e:
            traceback.print_exc()
            entry["coarse_guide"] = {"error": f"{type(e).__name__}: {e}"}
        try:
            entry["review"] = [str(p.relative_to(src)) for p in review.crops(src, dest)]
        except Exception as e:
            traceback.print_exc()
            entry["review"] = {"error": f"{type(e).__name__}: {e}"}
    entry["review_seconds"] = round(time.time() - started, 1)
    # The driver's peaks so far (the review is its largest step after the Rust commands):
    # anonymous memory during the review, and ru_maxrss (which also counts mapped file pages).
    entry["review_driver_anon_peak_mb"] = anon.peak
    entry["review_driver_peak_mb"] = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss // 1024
    figures = len(entry["review"]) if isinstance(entry["review"], list) else entry["review"]
    print(f"review: {figures} figures in {dest} ({entry['review_seconds']} s, driver anonymous peak "
          f"{entry['review_driver_anon_peak_mb']} MB, max RSS {entry['review_driver_peak_mb']} MB); "
          f"coarse guide {entry['coarse_guide']}", flush=True)
    return entry


def _guard(out: Path, src: Path, m: dict) -> None:
    """Refuse an output at or inside the source or its source world, and an existing non-empty
    directory that is not a run of this driver (another world, say)."""
    o = out.resolve()
    for p in {src.resolve(), *([Path(m["source_world"]).resolve()] if m.get("source_world") else [])}:
        if o == p or p in o.parents:
            raise Refused(f"--out {out} lies in the source {p}")
    if out.exists() and any(p.name not in LAUNCH_FILES for p in out.iterdir()):
        progress = out / "progress.json"
        if not progress.exists() or "source" not in _json(progress):
            raise Refused(f"{out} exists and is not a run of this driver (no progress.json)")


def _run_world(src: Path, out: Path, tile: int, threads: int) -> int:
    m = _json(src / "manifest.json")
    n = m["size"][0]
    if m.get("spacing_m") != 1.0 or not (src / RAW).exists():
        raise Refused(f"{src} is not a 1 m world with a kept {RAW}")
    _guard(out, src, m)
    moisture = _moisture(src, m)
    out.mkdir(parents=True, exist_ok=True)
    progress = Progress(out / "progress.json", src)
    work = out / "hydrology" / "work"
    _check_disk(out, n * n, work)
    if not (raw := progress.done("raw")):
        _reflink(src / RAW, out / RAW)
        sha = hydrology.sha256(out / RAW)
        want = (m.get("hydrology_input") or {}).get("sha256")
        if want and sha != want:
            raise Refused(f"{RAW} does not match the source's recorded sha256")
        progress.mark("raw", sha256=sha)
        raw = progress.done("raw")
    grid = build_world_grid(src, out)
    binary, digest = _pin(out / "hydrology" / "bin")
    progress.key(digest, tile, grid, work)
    out1m = out / "hydrology" / "1m"
    _water(progress, binary, grid, work, out1m, tile, threads, layout=_layout(src))
    if not (step := progress.done("world")):
        started = time.time()
        code = _assemble(src, out, m, out1m, moisture, binary, digest, raw["sha256"], tile, threads)
        step = {"code": code, "seconds": round(time.time() - started, 1)}
        progress.mark("world", **step)
    return step["code"]


def _run_window(src: Path, level: int, centre: tuple[float, float], size: int, out: Path, tile: int,
                threads: int) -> int:
    _guard(out, src, _json(src / "manifest.json"))
    out.mkdir(parents=True, exist_ok=True)
    progress = Progress(out / "progress.json", src)
    work = out / "work"
    _check_disk(out, size * size, work)
    grid = build_window_grid(src, level, centre, size, out / "grid")
    binary, digest = _pin(out / "bin")
    progress.key(digest, tile, grid, work)
    code = _water(progress, binary, grid, work, out / "out", tile, threads,
                  layout=_layout(src) if level == 1 else None)
    m = _json(src / "manifest.json")
    coarse = Path(m["source_world"]) / "hydrology" / "4m" if level == 1 and m.get("source_world") else None
    # A test window's guide is a figure: its exit code is the water's own.
    progress.mark("review", **_review(out, out / "out", coarse, out / "review"))
    print(f"done: {out / 'out'} ({'passed' if code == 0 else 'FAILED'})", flush=True)
    return code


def run_level(run: Path, tile: int = 2048, threads: int = 4, coarse: Path | None = None,
              river_km2: float | None = None) -> int:
    """One coarse level in `run` (grid/ built by the caller): rain if the grid has none, the water
    model with erosion, finish, review and the coarse guide (a figure) against `coarse`."""
    grid = run / "grid"
    if not (grid / "grid.json").exists():
        raise Refused(f"{grid} has no grid.json")
    if coarse is not None:
        _coarse_water(coarse)
    progress = Progress(run / "progress.json", grid)
    m = _json(grid / "grid.json")
    work = run / "work"
    _check_disk(run, m["size"][0] * m["size"][1], work)
    binary, digest = _pin(run / "bin")
    if "moisture" not in m:
        if _rust([str(binary), "world", "rain", "--grid", str(grid)]) != 0:
            raise Refused("world rain failed")
    progress.key(digest, tile, grid, work)
    code = _water(progress, binary, grid, work, run / "out", tile, threads, river_km2)
    # The guide depends on the coarse level too: redo the review when it changes.
    report_c = coarse / "report.json" if coarse is not None else None
    against = {"coarse": str(coarse.resolve()) if coarse is not None else None,
               "coarse_report_sha256": hydrology.sha256(report_c) if report_c and report_c.exists() else None}
    step = progress.done("review")
    if not step or {k: step.get(k) for k in against} != against:
        step = _review(run, run / "out", coarse, run / "review") | against
        progress.mark("review", **step)
    report = _json(run / "out" / "report.json")
    passed = code == 0 and report["passed"]
    level = {"passed": passed, "failures": report["failures"],
             "report": "out/report.json", "coarse_guide": step.get("coarse_guide"), "review": step.get("review")}
    cascade.save_json(run / "level.json", level)
    print(f"level {run}: {'passed' if passed else 'FAILED: ' + ', '.join(level['failures'])}", flush=True)
    return 0 if passed else 1


def run_world(src: Path, out: Path, tile: int, threads: int) -> int:
    """The whole 1 m world into a new world directory: 0 passed, 1 checks failed, 2 refused."""
    try:
        return _run_world(src, out, tile, threads)
    except Refused as e:
        print(f"error: {e}", file=sys.stderr, flush=True)
        return 2


def run_window(src: Path, level: int, centre: tuple[float, float], size: int, out: Path, tile: int,
               threads: int) -> int:
    """A test window (grid, work, out under `out`): 0 passed, 1 checks failed, 2 refused."""
    try:
        return _run_window(src, level, centre, size, out, tile, threads)
    except Refused as e:
        print(f"error: {e}", file=sys.stderr, flush=True)
        return 2


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = p.add_subparsers(dest="mode", required=True)
    w = sub.add_parser("world", help="the whole 1 m world into a new world directory")
    w.add_argument("--source", type=Path, required=True, help="L4 world with a kept elevation-raw.f32")
    w.add_argument("--out", type=Path, required=True, help="new world directory (resumed if it exists)")
    w.add_argument("--tile", type=int, default=2048, help="2048: ~1.8 GB for 4 threads; 4096 needs ~5.7 GB")
    w.add_argument("--threads", type=int, default=4)
    v = sub.add_parser("window", help="a test window at 1 m, 4 m or 32 m")
    v.add_argument("--source", type=Path, required=True, help="1 m: the L4 world; 4 and 32 m: the chain world")
    v.add_argument("--level", type=int, choices=(1, 4, 32), required=True)
    v.add_argument("--centre", required=True, help="E,N in metres")
    v.add_argument("--size", type=int, required=True, help="cells along a side")
    v.add_argument("--out", type=Path, required=True, help="run directory (resumed if it exists)")
    v.add_argument("--tile", type=int, help="cells along a tile side (default: the whole window)")
    v.add_argument("--threads", type=int, default=4)
    lv = sub.add_parser("level", help="one coarse level (32 m or 4 m) from a prepared grid")
    lv.add_argument("--run", type=Path, required=True, help="run directory holding grid/")
    lv.add_argument("--coarse", type=Path, help="the next coarser level's out/ (coarse guide)")
    lv.add_argument("--tile", type=int, default=2048)
    lv.add_argument("--threads", type=int, default=4)
    lv.add_argument("--river-km2", type=float)
    a = p.parse_args(argv)
    started = time.time()
    try:
        if a.mode == "world":
            code = run_world(a.source, a.out, a.tile, a.threads)
        elif a.mode == "level":
            try:
                code = run_level(a.run, a.tile, a.threads, a.coarse, a.river_km2)
            except Refused as e:
                print(f"error: {e}", file=sys.stderr, flush=True)
                code = 2
        else:
            e, n = (float(t) for t in a.centre.split(","))
            code = run_window(a.source, a.level, (e, n), a.size, a.out, a.tile or a.size, a.threads)
    except Exception:
        # Exit 1 means only "checks failed, outputs kept".
        import traceback

        traceback.print_exc()
        code = 2
    import resource

    print(f"exit {code} after {time.time() - started:.0f} s; peak RSS driver "
          f"{resource.getrusage(resource.RUSAGE_SELF).ru_maxrss // 1024} MB, topography {_child_peak_mb()} MB",
          flush=True)
    return code


if __name__ == "__main__":
    sys.exit(main())
