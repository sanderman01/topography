"""Hydrology v3 at 1 m: the water model on a world's kept raw surface (or a test window).

python -m topography_training.water.run world --source <l4 world> --out <new world> [--tile 2048] [--threads 4]
python -m topography_training.water.run window --source <world> --level {1,4,32} --centre E,N --size N --out <dir>

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
from . import review

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
    """The 1 m grid over the whole world: links to the reflinked raw surface, the chain's 4 m
    classes and the real moisture field."""
    m = _json(src / "manifest.json")
    n = m["size"][0]
    field, entry, mean = _moisture(src, m)
    chain = Path(m["source_world"])
    classes = chain / "classes.u8"
    if not classes.exists() or classes.stat().st_size != (n // 4) ** 2:
        raise Refused(f"{classes} is not the {n // 4}² 4 m class map of {src}")
    grid = out / "hydrology" / "grid"
    grid.mkdir(parents=True, exist_ok=True)
    _symlink(Path("..", "..", RAW), grid / "elevation.f32")
    _symlink(classes.resolve(), grid / "classes.u8")
    _symlink(field.resolve(), grid / "moisture.f32")
    _write_if_changed(grid / "grid.json", {
        "size": [n, n], "spacing_m": 1.0, "origin": _origin(m, n, 1.0), "classes_cell_m": 4.0,
        "moisture": entry | {"file": "moisture.f32"}, "discharge": {"mean_land_mm_h": mean},
        "inputs": _inputs(classes, field),
    })
    return grid


def _inputs(classes: Path, moisture: Path) -> dict:
    """What the grid was built from, so a changed class map or moisture field changes grid.json
    (and with it the driver's resume key)."""
    st = classes.stat()
    return {"classes": {"file": str(classes.resolve()), "bytes": st.st_size, "modified_ns": st.st_mtime_ns},
            "moisture": {"file": str(moisture.resolve()), "sha256": hydrology.sha256(moisture)}}


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
    classes_src = (Path(m["source_world"]) if level == 1 else src) / "classes.u8"
    manifest = {
        "size": [size, size], "spacing_m": spacing, "origin": origin,
        "offset_m": [col * spacing, row * spacing], "classes_cell_m": spacing * k,
        "moisture": entry | {"file": "moisture.f32"}, "discharge": {"mean_land_mm_h": mean},
        "window": {"source": str(src.resolve()), "level": level, "centre_m": list(centre)},
        "inputs": _inputs(classes_src, field_m),
    }
    sizes = {"elevation.f32": size * size * 4, "classes.u8": (size // k) ** 2}
    if ((dest / "grid.json").exists() and _json(dest / "grid.json") == manifest
            and all((dest / f).exists() and (dest / f).stat().st_size == b for f, b in sizes.items())):
        return dest
    dest.mkdir(parents=True, exist_ok=True)
    (dest / "grid.json").unlink(missing_ok=True)
    if level == 1:
        field = _read_window(src / RAW, "<f4", n, row, col, size, size)
        classes = _read_window(classes_src, np.uint8, n // 4, row // 4, col // 4, size // 4, size // 4)
    elif level == 4:
        field = _read_window(src / "elevation.f32", "<f4", n, row, col, size, size)
        classes = _read_window(src / "classes.u8", np.uint8, n, row, col, size, size)
    else:
        l2 = np.load(src / "arrays.npz")["l2_raw"]
        if l2.shape != (n, n):
            raise Refused(f"{src / 'arrays.npz'} l2_raw is {l2.shape}, not the {n}² 32 m grid")
        field = l2[row : row + size, col : col + size]
        c4 = _read_window(src / "classes.u8", np.uint8, n * 8, row * 8, col * 8, size * 8, size * 8)
        classes = _mode_classes(c4, 8)
    for name, a, dtype in (("elevation.f32", field, "<f4"), ("classes.u8", classes, np.uint8)):
        np.ascontiguousarray(a, dtype).tofile(dest / (name + ".tmp"))
        (dest / (name + ".tmp")).replace(dest / name)
    _symlink(field_m.resolve(), dest / "moisture.f32")
    _write_if_changed(dest / "grid.json", manifest)
    return dest


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


def _water(progress: Progress, binary: Path, grid: Path, work: Path, out1m: Path, tile: int, threads: int) -> int:
    """`world water` then `world water-finish` on the carved ground, each skipped when done.
    Returns finish's 0 (passed) or 1 (checks failed); a failed command raises Refused."""
    flags = ["--grid", str(grid), "--work", str(work), "--tile", str(tile), "--threads", str(threads)]
    if not progress.done("water"):
        started = time.time()
        if _rust([str(binary), "world", "water", *flags]) != 0:
            raise Refused("world water failed")
        progress.mark("water", seconds=round(time.time() - started, 1), peak_mb=_child_peak_mb())
    if not (step := progress.done("finish")):
        if out1m.exists():
            shutil.rmtree(out1m)
        started = time.time()
        code = _rust([str(binary), "world", "water-finish", *flags, "--ground", str(work / "carved.f32"),
                      "--out", str(out1m)])
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


def _water_u8(surface: Path, dest: Path, n: int) -> str:
    """255 where the water surface is finite; returns its sha256."""
    s = np.memmap(surface, "<f4", "r", shape=(n, n))
    tmp = dest.with_suffix(".tmp")
    h = hashlib.sha256()
    with tmp.open("wb") as f:
        for y in range(0, n, 1024):
            rows = np.where(np.isfinite(s[y : y + 1024]), 255, 0).astype(np.uint8)
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
    digests["water.u8"] = _water_u8(out / "water-surface.f32", out / "water.u8", n)
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
    guide, shots = _review(out, out1m, Path(m["source_world"]) / "hydrology" / "4m", out1m / "review")
    manifest = {
        "schema_version": 1, "kind": "world_preview", "world_input": str(world_input.resolve()),
        "spacing_m": 1.0, "size": [n, n], "class_id": m.get("class_id"),
        "method": "hydrology v3 1 m water over the kept L4 raw surface of the source world",
        "water_fill": hydrology.WATER_FILL, "moisture": entry, **extra,
        **{k: m[k] for k in ("layout",) if k in m},
        "source_world": m["source_world"], "hydrology_source": str(src.resolve()),
        "hydrology_input": {"file": RAW, "sha256": raw_sha256,
                            "rerun": "python -m topography_training.water.run world --source <source> --out <this world>"},
        "water": {"model": "hydrology v3", "report": "hydrology/1m/report.json", "passed": report["passed"],
                  "failures": report["failures"], "ground": "carved.f32",
                  "relaxation": "none: Plan A ships the carved ground",
                  "binary": str(binary.relative_to(out)), "binary_sha256": digest,
                  "tile": tile, "threads": threads,
                  # A figure in Plan A (R17): recorded, explained at gate 2, never withholds manifest.json.
                  "coarse_guide": ({k: guide[k] for k in review.GUIDE_SUMMARY}
                                   | {"file": "hydrology/1m/review/coarse-guide.json"}) if guide else None,
                  "review": [str(p.relative_to(out)) for p in shots]},
        "layers": digests,
    }
    name = "manifest.json" if report["passed"] else "manifest-failed.json"
    cascade.save_json(out / name, manifest)
    (out / ("manifest-failed.json" if report["passed"] else "manifest.json")).unlink(missing_ok=True)
    print(f"done: {out} ({'passed' if report['passed'] else 'FAILED: ' + ', '.join(report['failures'])})",
          flush=True)
    return 0 if report["passed"] else 1


def _review(src: Path, fine: Path, coarse: Path | None, dest: Path) -> tuple[dict | None, list[Path]]:
    """The review figures after water-finish, made afresh: crops of `src` (a world or window run)
    and the coarse guide of `fine` against the 4 m hydrology `coarse` when it has water."""
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    guide = None
    if coarse is not None and (coarse / "water-body.u32").exists():
        guide = review.coarse_guide(src, coarse, fine_dir=fine)
        cascade.save_json(dest / "coarse-guide.json", guide)
    return guide, review.crops(src, dest)


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
    _water(progress, binary, grid, work, out1m, tile, threads)
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
    code = _water(progress, binary, grid, work, out / "out", tile, threads)
    m = _json(src / "manifest.json")
    coarse = Path(m["source_world"]) / "hydrology" / "4m" if level == 1 and m.get("source_world") else None
    guide, shots = _review(out, out / "out", coarse, out / "review")
    shots.append(review.window_pair(out, out / "review" / "window.png"))
    summary = {k: guide[k] for k in review.GUIDE_SUMMARY} if guide else "none"
    print(f"review: {len(shots)} figures in {out / 'review'}; coarse guide {summary}", flush=True)
    print(f"done: {out / 'out'} ({'passed' if code == 0 else 'FAILED'})", flush=True)
    return code


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
    a = p.parse_args(argv)
    started = time.time()
    try:
        if a.mode == "world":
            code = run_world(a.source, a.out, a.tile, a.threads)
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
