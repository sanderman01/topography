"""Water for the cascade.

The cascade's water (sea, lakes, rivers as a surface height) comes from the Rust
`topography world hydrology`; `write_grid`, `run`, `read` and `seam` call it. What stays here
is the priority flood (Barnes et al. 2014) for analysis (collage, description, exemplar,
cascade) and `drain`, which makes the 128 m L1 surface drain before L2.
"""

from __future__ import annotations

import heapq
import json
import shutil
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path

import numpy as np

EPSILON_M = 1e-3  # gradient across filled flats so every cell drains
ROOT = Path(__file__).resolve().parents[4]
# One water table for Rust and Python.
WATER_TABLE = ROOT / "configs" / "hydrology" / "water-v1.toml"


@dataclass(frozen=True)
class Water:
    """How much water a terrain class gets (configs/hydrology/water-v1.toml): lakes kept
    (shares of the land), small lakes on finer levels, the drainage area where streams begin
    and their width (m per sqrt(km2)), closed dry basins, and whether streams sink (karst)."""

    lake_share: float
    tarn_share: float
    stream_km2: float
    stream_width: float = 2.5
    dry_share: float = 0.0
    sinks: bool = False


def load_water(path: Path = WATER_TABLE) -> dict[int, Water]:
    """Water by legend v3 class id; classes missing from the table take class 0."""
    table = tomllib.loads(path.read_text())["classes"]
    return {int(k): Water(**v) for k, v in table.items()}


WATER = load_water()

# Dry basins kept by `drain`: at least 2 m deep and compact (bounding-box side squared over area).
DRY_MIN_DEPTH_M = 2.0
DRY_MAX_ELONGATION = 4.0


def flood(h: np.ndarray, outlet: np.ndarray):
    """Filled surface, downstream neighbour per cell (-1 at outlets) and visit order.

    Outlets are sea cells and the field edge; every other cell drains to one of them."""
    rows, cols = h.shape
    flat = h.ravel().astype(np.float64)
    filled = flat.copy()
    parent = np.full(flat.size, -1, np.int64)
    seen = np.zeros(flat.size, bool)
    edge = np.zeros(h.shape, bool)
    edge[0, :] = edge[-1, :] = edge[:, 0] = edge[:, -1] = True
    heap = []
    for i in np.flatnonzero((outlet | edge).ravel()):
        seen[i] = True
        heap.append((filled[i], int(i)))
    heapq.heapify(heap)
    order = []
    offsets = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (-1, 1), (1, -1), (1, 1)]
    push, pop = heapq.heappush, heapq.heappop
    while heap:
        z, i = pop(heap)
        order.append(i)
        r, c = divmod(i, cols)
        for dr, dc in offsets:
            rr, cc = r + dr, c + dc
            if 0 <= rr < rows and 0 <= cc < cols:
                j = rr * cols + cc
                if not seen[j]:
                    seen[j] = True
                    parent[j] = i
                    if filled[j] <= z:
                        filled[j] = z + EPSILON_M
                    push(heap, (filled[j], j))
    return filled.reshape(h.shape), parent, np.array(order, np.int64)


def accumulate(parent: np.ndarray, order: np.ndarray) -> np.ndarray:
    """Upstream cell count per cell (itself included)."""
    acc = np.ones(parent.size, np.float64)
    for i in order[::-1]:
        p = parent[i]
        if p >= 0:
            acc[p] += acc[i]
    return acc


def _maxpool3(a: np.ndarray) -> np.ndarray:
    p = np.pad(a, 1, mode="edge")
    return np.max([p[dy : dy + a.shape[0], dx : dx + a.shape[1]] for dy in range(3) for dx in range(3)], axis=0)


def carve(h: np.ndarray, outlet: np.ndarray) -> np.ndarray:
    """Priority-flood carving (Soille 2004): instead of filling a pit, lower the path from
    its spill cell back to the outlet so every cell drains."""
    rows, cols = h.shape
    hc = h.ravel().astype(np.float64).copy()
    parent = np.full(hc.size, -1, np.int64)
    seen = np.zeros(hc.size, bool)
    edge = np.zeros(h.shape, bool)
    edge[0, :] = edge[-1, :] = edge[:, 0] = edge[:, -1] = True
    heap = [(hc[i], int(i)) for i in np.flatnonzero((outlet | edge).ravel())]
    for _, i in heap:
        seen[i] = True
    heapq.heapify(heap)
    offsets = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (-1, 1), (1, -1), (1, 1)]
    push, pop = heapq.heappush, heapq.heappop
    while heap:
        _, i = pop(heap)
        r, c = divmod(i, cols)
        for dr, dc in offsets:
            rr, cc = r + dr, c + dc
            if 0 <= rr < rows and 0 <= cc < cols:
                j = rr * cols + cc
                if seen[j]:
                    continue
                seen[j] = True
                parent[j] = i
                target = hc[j] - EPSILON_M
                k = i
                while k >= 0 and hc[k] > target:
                    hc[k] = target
                    target -= EPSILON_M
                    k = parent[k]
                push(heap, (hc[j], j))
    return hc.reshape(h.shape)


def _hollows(h: np.ndarray, sea: np.ndarray):
    """Filled surface, depth and labelled closed depressions (scipy labels) of a surface."""
    from scipy import ndimage

    filled, _, _ = flood(h, sea)
    depth = filled - h
    labels, count = ndimage.label((depth > 0.05) & ~sea)
    return filled, depth, labels, count


def _pick(order, area, budget) -> list[int]:
    chosen, total = [], 0
    for k in order:
        if total + area[k] <= budget:
            chosen.append(k + 1)
            total += area[k]
    return chosen


def drain(h: np.ndarray, sea: np.ndarray, lake_share: float, widen_sigma: float,
          keep_at: np.ndarray | None = None, dry_share: float = 0.0, dry_at: np.ndarray | None = None,
          dry_keep_at: np.ndarray | None = None):
    """Make the generated L1 surface drain like real terrain.

    Depressions are kept as lakes (flat at their spill level), largest by volume, until they
    cover `lake_share` of the land. Every other depression is carved through and the cut is
    widened into a valley floor. Depressions touching `keep_at` (authored lakes) are always
    kept and count toward the share. With `dry_share`, the deepest compact remaining
    depressions centred in `dry_at` stay closed and dry (dolines, craters, interdune hollows)
    up to that share of the land; depressions centred in `dry_keep_at` (painted craters)
    always stay closed and dry. Returns (surface, lake mask, lake level, dry basin mask)."""
    from scipy import ndimage

    from .data import smooth

    filled, depth, labels, count = _hollows(h, sea)
    keep = np.zeros(h.shape, bool)
    dry = np.zeros(h.shape, bool)
    if count:
        idx = np.arange(1, count + 1)
        area = ndimage.sum(np.ones(h.shape), labels, idx)
        budget = lake_share * (~sea).sum()
        forced = [] if keep_at is None else [int(k) for k in np.unique(labels[keep_at & (labels > 0)])]
        budget -= sum(area[k - 1] for k in forced)
        order = np.argsort(-ndimage.sum(depth, labels, idx))
        if dry_at is not None:  # hollows of dry classes (craters, dolines, interdunes) stay dry, not lakes
            wet_ok = ndimage.mean(dry_at.astype(np.float32), labels, idx) <= 0.5
            order = [k for k in order if wet_ok[k]]
        lakes = _pick([k for k in order if k + 1 not in forced], area, budget) + forced
        keep = np.isin(labels, lakes)
        if dry_share > 0:
            deepest = ndimage.maximum(depth, labels, idx)
            span = np.array([max(s[0].stop - s[0].start, s[1].stop - s[1].start) for s in ndimage.find_objects(labels)])
            ok = (deepest >= DRY_MIN_DEPTH_M) & (span**2 / area <= DRY_MAX_ELONGATION)
            if dry_at is not None:
                ok &= ndimage.mean(dry_at.astype(np.float32), labels, idx) > 0.5
            order = [k for k in np.argsort(-deepest) if ok[k] and k + 1 not in lakes]
            dry = np.isin(labels, _pick(order, area, dry_share * (~sea).sum()))
        if dry_keep_at is not None:
            craters = ndimage.mean(dry_keep_at.astype(np.float32), labels, idx) > 0.5
            dry |= np.isin(labels, [k + 1 for k in np.flatnonzero(craters) if k + 1 not in lakes])
    work = np.where(keep, filled, h)
    cut = work - carve(work, sea | keep | dry)
    widened = np.maximum(cut, smooth(_maxpool3(cut).astype(np.float32), widen_sigma))
    widened[dry] = 0
    out = np.where(keep, filled, work - widened).astype(np.float32)
    return out, keep, np.where(keep, filled, np.nan).astype(np.float32), dry


# Rust `topography world hydrology` (docs/workflows/terrain-cascade.md, "Hydrology v2").

_BINARY: Path | None = None


def binary() -> Path:
    """The release `topography` binary, built (or brought up to date) once per process."""
    global _BINARY
    if _BINARY is None:
        subprocess.run(["cargo", "build", "--release", "--locked", "--offline", "-q", "-p", "topography-cli"],
                       cwd=ROOT, check=True)
        _BINARY = ROOT / "target" / "release" / "topography"
    return _BINARY


def write_table(path: Path, **overrides: float) -> Path:
    """The water table with some fields set for every class (cascade_chain overrides)."""
    lines = ['schema_version = 1', 'kind = "water_table"', 'legend = "topography terrain-v3"']
    for c, w in sorted(WATER.items()):
        fields = vars(w) | overrides
        lines.append(f"\n[classes.{c}]")
        lines += [f"{k} = {str(v).lower() if isinstance(v, bool) else float(v)}" for k, v in fields.items()]
    path.write_text("\n".join(lines) + "\n")
    return path


def write_grid(path: Path, h: np.ndarray, spacing_m: float, origin: tuple[float, float],
               classes: np.ndarray | None = None, offset_m: tuple[float, float] | None = None,
               core_rows: tuple[int, int] | None = None, classes_cell_m: float | None = None,
               wind_from_deg: float | None = None) -> Path:
    """A grid directory (replaced if present): `origin` is the world's north-west corner in
    the layout plane, `offset_m` [east, south] places this grid inside it."""
    if path.exists():
        shutil.rmtree(path)
    path.mkdir(parents=True)
    grid = {"size": [h.shape[1], h.shape[0]], "spacing_m": float(spacing_m),
            "origin": [float(origin[0]), float(origin[1])]}
    if offset_m is not None:
        grid["offset_m"] = [float(offset_m[0]), float(offset_m[1])]
    if core_rows is not None:
        grid["core_rows"] = [int(core_rows[0]), int(core_rows[1])]
    if classes_cell_m is not None:
        grid["classes_cell_m"] = float(classes_cell_m)
    if wind_from_deg is not None:
        grid["wind_from_deg"] = float(wind_from_deg)
    np.ascontiguousarray(h, dtype="<f4").tofile(path / "elevation.f32")
    if classes is not None:
        np.ascontiguousarray(classes, dtype=np.uint8).tofile(path / "classes.u8")
    (path / "grid.json").write_text(json.dumps(grid, indent=1))
    return path


def run(grid: Path, out: Path, coarse: Path | None = None, north: Path | None = None,
        wind_deg: float | None = None, water: Path | None = None, river_km2: float | None = None) -> dict:
    """Run hydrology on `grid` into `out` (replaced if present); returns report.json. Raises
    on a failed check (the outputs stay for reading)."""
    if out.exists():
        shutil.rmtree(out)
    cmd = [str(binary()), "world", "hydrology", "--grid", str(grid), "--out", str(out)]
    for flag, value in (("--coarse", coarse), ("--north", north), ("--wind-deg", wind_deg),
                        ("--water", water), ("--river-km2", river_km2)):
        if value is not None:
            cmd += [flag, str(value)]
    done = subprocess.run(cmd, capture_output=True, text=True)
    sys.stdout.write(done.stdout)
    report_path = out / "report.json"
    if done.returncode != 0:
        sys.stderr.write(done.stderr)
        failures = json.loads(report_path.read_text()).get("failures") if report_path.exists() else None
        raise RuntimeError(f"hydrology failed on {grid} (exit {done.returncode}): {failures}")
    return json.loads(report_path.read_text())


def seam(north: Path, south: Path) -> None:
    """The seam check between two band outputs; raises on a failure."""
    done = subprocess.run([str(binary()), "world", "hydrology-seam", "--north", str(north), "--south", str(south)],
                          capture_output=True, text=True)
    if done.returncode != 0:
        raise RuntimeError(f"seam check failed between {north.name} and {south.name}:\n{done.stdout}{done.stderr}")


# Hydrology layers a world output carries (beside water.u8, kept until export v2).
WORLD_LAYERS = ("elevation.f32", "water-surface.f32", "water-body.u32", "bodies.json")
WATER_FILL = ("ground continues under all water; water-surface.f32 holds the water surface "
              "(NaN dry, sea 0), water-body.u32 its body (bodies.json)")


def sha256(path: Path) -> str:
    import hashlib

    h = hashlib.sha256()
    with path.open("rb") as f:
        while chunk := f.read(1 << 24):
            h.update(chunk)
    return h.hexdigest()


def link(src: Path, dst: Path, names) -> dict[str, str]:
    """Hard-link files of a hydrology output into a world output; their SHA-256 by name."""
    for name in names:
        (dst / name).unlink(missing_ok=True)
        (dst / name).hardlink_to(src / name)
    return {name: sha256(dst / name) for name in names}


@dataclass
class Output:
    """A hydrology output: carved ground, water surface (NaN dry), body ids and the body list."""

    ground: np.ndarray
    surface: np.ndarray
    body: np.ndarray
    bodies: list[dict]

    def kind(self, kind: str) -> np.ndarray:
        """Cells of bodies of one kind (`sea`, `lake`, `river`)."""
        ids = [b["id"] for b in self.bodies if b["kind"] == kind]
        return np.isin(self.body, ids)

    @property
    def wet(self) -> np.ndarray:
        return np.isfinite(self.surface)


def read(out: Path, rows: slice | None = None) -> Output:
    """An output directory's rasters (memory-mapped; `rows` limits them)."""
    w, h = json.loads((out / "grid.json").read_text())["size"]
    rows = rows or slice(0, h)

    def raster(name, dtype):
        return np.memmap(out / name, dtype=dtype, mode="r", shape=(h, w))[rows]

    return Output(raster("elevation.f32", "<f4"), raster("water-surface.f32", "<f4"),
                  raster("water-body.u32", "<u4"), json.loads((out / "bodies.json").read_text())["bodies"])
