"""Banded L4 generation: refine a 4 m world (cascade chain output) to 1 m.

python -m topography_training.diffusion.cascade_l4 --run <l4 run dir> --world <4 m world dir> --out <dir>
    [--km N] [options]

The 1 m field is generated in bands of rows with a halo above and below, so host memory
stays bounded at any world size. Bands share their starting noise where they overlap
and are cross-faded over the halo. Each band then gets its water from the Rust
`topography world hydrology` at 1 m, north to south: it holds the world's 4 m network fixed
(`--coarse`) and takes the previous band's rows (`--north`); the seam between them is
checked and a failure stops the run. Water cells keep the refined 4 m ground (L4 learned
land only) under the hydrology surface; bands and windows whose 4 m ground is all deep sea
water keep it without diffusion. `--guidance-classes` limits class guidance to windows
holding those classes. `--km` with `--centre` places the square anywhere. Output layers are
memory-mapped and filled band by band; rerunning with the same arguments resumes after the
last finished band.
Writes a 1 m world output (manifest.json + layers, water-surface.f32, water-body.u32 and
bodies.json) for `topography export heightmap`, plus overview.png and detail.png hillshades.
The diffused surface (elevation-raw.f32) is kept as the hydrology input unless --drop-raw:
`--out <world> --hydrology-only` reruns every band's hydrology from it without diffusion
(resumable), and `--out <world> --seam-audit` checks the band seams of a finished world, as
every run does when it finishes.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
import time
from pathlib import Path
from types import SimpleNamespace

import numpy as np

from . import cascade, hydrology
from .data import smooth, soft_cover

MARGIN_4M = 16  # 4 m context cells around a band for the smoothing
WATER_SIGMA_M = 3.0  # rounds 4 m water corners at 1 m
# Body fields that hold for the whole world (the rest are per band grid). Dry basins (not in
# the raster) keep their largest band area; tiny ones stay out (hydrology.world_bodies).
BODY_FIELDS = ("id", "kind", "level_m", "coarse", "parent", "over_budget", "endorheic", "ice")
# Disk per 1 m cell: elevation, the raw surface (kept as the hydrology input), water surface
# and body (4 B each), water, snow and vegetation (1 B each).
DISK_BYTES = 19
# The diffused surface hydrology carves: kept, so hydrology can be rerun without diffusion.
RAW = "elevation-raw.f32"
# Seam audit, as the per-band hydrology check (report.rs): what hydrology changed may differ
# across a band boundary by SEAM_STEP_M plus the side slope per row over SEAM_CHECK_ROWS rows,
# and a cell's change may not lie SLOT_M below both neighbours there.
SEAM_STEP_M = 5.0
SEAM_CHECK_ROWS = 4
SLOT_M = 5.0
SIDE_SLOPE = float(np.tan(np.radians(35.0)))  # m per 1 m row
# The manifest of a world whose seam audit failed (not manifest.json: it is not finished).
MANIFEST_FAILED = "manifest-failed.json"


class SeamAuditFailed(RuntimeError):
    """A finished world failed its seam audit (manifest-failed.json written)."""


def _code_digest() -> str:
    """Digest of the Python side of band hydrology (hydrology.py and `_hydro_band`): a change
    restarts a rerun, as a new binary does."""
    import hashlib
    import inspect

    h = hashlib.sha256(Path(hydrology.__file__).read_bytes())
    h.update(inspect.getsource(_hydro_band).encode())
    h.update(repr(BODY_FIELDS).encode())
    return h.hexdigest()


def _pin(c: SimpleNamespace) -> dict:
    """Pin the hydrology binary (TOPOGRAPHY_BIN, else the release build) into the world's
    hydrology/bin for the whole run; its manifest entry."""
    path, digest = hydrology.pin_binary(c.hyd / "bin")
    print(f"hydrology binary: {path} (sha256 {digest[:12]})", flush=True)
    return {"sha256": digest, "file": str(path.relative_to(c.out))}


def _rows_4m(a: np.ndarray, r0: int, r1: int, c0: int, c1: int) -> np.ndarray:
    """A 4 m block with indices beyond the field repeating its edge."""
    rows = np.clip(np.arange(r0, r1), 0, a.shape[0] - 1)
    cols = np.clip(np.arange(c0, c1), 0, a.shape[1] - 1)
    return a[np.ix_(rows, cols)]


def _hydro_band(c: SimpleNamespace, j: int, state: dict) -> None:
    """1 m hydrology of band j from the raw surface (its rows and halos final), checked against
    band j - 1, into the output layers; `state` counts finished bands, times and bodies."""
    t = time.time()
    # Only band j - 1 is needed (as the north); others are left by a crash or an earlier rerun.
    for d in c.hyd.glob("band-[0-9][0-9][0-9]"):
        if d.name != f"band-{j - 1:03d}":
            shutil.rmtree(d)
    a, b = c.bands[j]
    top, bot = max(a - c.halo_m, 0), min(b + c.halo_m, c.n)
    cls = c.classes4[top // 4 : bot // 4] if c.per_cell else np.full(((bot - top) // 4, c.c4), c.class_id, np.uint8)
    grid = hydrology.write_grid(c.hyd / "grid", np.asarray(c.raw[top:bot]), 1.0, c.origin, cls,
                                offset_m=(c.east, c.south + top), core_rows=(a - top, b - top), classes_cell_m=4.0)
    out, north = c.hyd / f"band-{j:03d}", (c.hyd / f"band-{j - 1:03d}" if j else None)
    hydrology.run(grid, out, coarse=c.coarse_dir, north=north)
    if north is not None:
        hydrology.seam(north, out)
    shutil.copy(out / "report.json", c.hyd / f"report-{j:03d}.json")
    o = hydrology.read(out, slice(a - top, b - top))
    c.elev[a:b], c.surface_out[a:b], c.body_out[a:b] = o.ground, o.surface, o.body
    c.water_out[a:b] = np.where(np.isfinite(o.surface), 255, 0)
    for mm in (c.elev, c.surface_out, c.body_out, c.water_out):
        mm.flush()
    # A body's parent is where it really ends: the band whose core holds its mouth or exit.
    core, bodies = range(a - top, b - top), state["bodies"]
    for entry in o.bodies:
        fields = {k: entry[k] for k in BODY_FIELDS if k in entry}
        key = str(entry["id"])
        if entry["kind"] == "dry":  # areas count the band grid; keep the largest
            area = max(entry.get("area_km2", 0.0), bodies.get(key, {}).get("area_km2", 0.0))
            if area >= hydrology.WORLD_DRY_MIN_KM2:
                bodies[key] = fields | {"area_km2": area}
            continue
        end = entry.get("mouth_cell") or entry.get("exit_cell")
        if end is not None and end[1] in core:
            bodies[key] = fields
        else:
            bodies.setdefault(key, fields)
    state["hydrology_done"] = j + 1
    state["hydrology_s"].append(time.time() - t)
    c.save()
    if north is not None:  # only the latest band is needed, as the next band's north
        shutil.rmtree(north)
    wet = np.isfinite(o.surface).mean()
    print(f"hydrology {j + 1}/{len(c.bands)} rows {a}-{b} in {state['hydrology_s'][-1]:.0f} s, water {wet:.2%}",
          flush=True)


def _seam_audit(out: Path, n: int, bands: list[tuple[int, int]], step_m: float = SEAM_STEP_M) -> dict:
    """Steps and slots at each band boundary of a 1 m world, as the per-band hydrology check
    (`seam_steps`, `seam_slots`) counts them. With d = what shows (the water surface where wet,
    else the ground) less the raw surface: a column steps where d in one of the first
    SEAM_CHECK_ROWS rows of the band differs from d in the last row of the band above by more
    than `step_m` plus the side slope per further row, unless both cells are water of one body;
    it holds a slot where d lies SLOT_M below both neighbours (north and south, or west and
    east) in those rows. Without the raw surface (dropped) only steps of what shows over
    `step_m` and 4 times the rows beside count."""
    def mm(name, dtype):
        return np.memmap(out / name, dtype=dtype, mode="r", shape=(n, n))

    ground, surface, body = mm("elevation.f32", "<f4"), mm("water-surface.f32", "<f4"), mm("water-body.u32", "<u4")
    raw = mm(RAW, "<f4") if (out / RAW).exists() else None
    k = SEAM_CHECK_ROWS
    boundaries = []
    for a, _ in bands[1:]:
        if a < 2 or a + k + 1 >= n:
            continue
        rows = slice(a - 2, a + k + 1)  # a - 2 .. a + k: d for a - 1 .. a + k - 1 with neighbours
        s = np.asarray(surface[rows], np.float64)
        vis = np.where(np.isfinite(s), s, np.asarray(ground[rows], np.float64))
        bd, wet = np.asarray(body[rows]), np.isfinite(s)
        slots = np.zeros(n, bool)
        if raw is not None:
            d = vis - np.asarray(raw[rows], np.float64)
            bad, step = np.zeros(n, bool), np.zeros(n)
            for r in range(k):  # rows a + r against row a - 1 (index 1)
                same = wet[1] & wet[2 + r] & (bd[1] == bd[2 + r])
                st = np.abs(d[2 + r] - d[1])
                hit = (st > step_m + SIDE_SLOPE * r) & ~same
                bad |= hit
                step = np.where(hit, np.maximum(step, st), step)
                mid = d[2 + r]
                ns = np.minimum(d[1 + r], d[3 + r]) - mid
                we = np.zeros(n)
                we[1:-1] = np.minimum(mid[:-2], mid[2:]) - mid[1:-1]
                slots |= np.maximum(ns, we) > SLOT_M
        else:
            same = wet[1] & wet[2] & (bd[1] == bd[2])
            step = np.abs(vis[2] - vis[1])
            beside = np.maximum(np.abs(vis[3] - vis[2]), np.abs(vis[1] - vis[0]))
            bad = (step > step_m) & (step > 4 * beside) & ~same
        bad |= slots
        cols = int(bad.sum())
        boundaries.append({"row": a, "columns": cols, "slot_columns": int(slots.sum()),
                           "max_step_m": round(float(step[bad].max()), 2) if cols else 0.0,
                           "at_columns": np.flatnonzero(bad)[:3].tolist()})
    total = sum(b["columns"] for b in boundaries)
    return {"step_m": step_m, "rule": "hydrology change (shown - raw)" if raw is not None else "slopes (no raw)",
            "columns": total, "boundaries_with_steps": sum(1 for b in boundaries if b["columns"]),
            "worst": sorted(boundaries, key=lambda b: -b["columns"])[:10], "boundaries": boundaries}


def _finish(c: SimpleNamespace, state: dict, extra: dict) -> None:
    """World body list, pictures, world input and manifest of a finished 1 m world."""
    # World body list: areas from the delivered cells; dry basins keep no area.
    count = np.zeros(1, np.int64)
    for y in range(0, c.n, 1024):
        cnt = np.bincount(np.asarray(c.body_out[y : y + 1024]).ravel())
        count = np.pad(count, (0, max(len(cnt) - len(count), 0)))
        count[: len(cnt)] += cnt
    listed = []
    for entry in state["bodies"].values():
        if entry["kind"] == "dry":
            listed.append(entry)
            continue
        cells = int(count[entry["id"]]) if entry["id"] < len(count) else 0
        if cells:
            listed.append(entry | {"area_km2": cells / 1e6})
    (c.out / "bodies.json").write_text(json.dumps(hydrology.world_bodies(
        listed, "merged from the 1 m bands: water areas count delivered cells, dry basin areas their "
        "largest band grid; parents from the band holding each end"), indent=1))

    from .cascade_chain import _png

    # Pictures: the whole area averaged to 8 m and a 1,024 m square at 1 m in the middle.
    n = c.n
    over = np.concatenate([cascade.block_mean(np.asarray(c.elev[y : y + 1024]), 8) for y in range(0, n, 1024)])
    wover = np.concatenate([cascade.block_mean(np.asarray(c.water_out[y : y + 1024], np.float32), 8)
                            for y in range(0, n, 1024)]) > 127
    _png(c.out / "overview.png", over, 8.0, wover, 2048)
    m, r = n // 2, min(512, n // 2)
    _png(c.out / "detail.png", np.asarray(c.elev[m - r : m + r, m - r : m + r]), 1.0,
         np.asarray(c.water_out[m - r : m + r, m - r : m + r]) == 255, 1024)

    src = json.loads((Path(c.world["world_input"]) / "world.json").read_text())
    ox, oy = src["origin"][0] + c.col4 * 4.0, src["origin"][1] - c.row4 * 4.0
    world_input = c.out / "world-input"
    world_input.mkdir(exist_ok=True)
    (world_input / "world.json").write_text(json.dumps(
        src | {"name": f"{src.get('name', 'world')}-l4", "origin": [ox, oy]}, indent=1))
    names = ("elevation.f32", "water.u8", "water-surface.f32", "water-body.u32", "bodies.json",
             "snow-ice.u8", "vegetation.u8")
    digests = {name: hydrology.sha256(c.out / name) for name in names}
    if (c.world_dir / "moisture.f32").exists():  # 128 m moisture, as the 4 m world's
        digests |= hydrology.link(c.world_dir, c.out, ("moisture.f32",))
        extra["moisture"] = c.world["moisture"]
    if "description" in c.world:  # carried through as the chain measured it
        shutil.copy(c.world_dir / c.world["description"], c.out / "description.json")
        shutil.copy(c.world_dir / "description.md", c.out / "description.md")
        digests["description.json"] = hydrology.sha256(c.out / "description.json")
        extra["description"] = "description.json"
    if c.raw_kept:  # the diffused surface hydrology reads: a rerun needs no diffusion
        extra["hydrology_input"] = {"file": RAW, "sha256": hydrology.sha256(c.out / RAW),
                                    "rerun": "cascade_l4 --hydrology-only --out <this world>"}
    audit = _seam_audit(c.out, n, c.bands)
    (c.hyd / "seam-audit.json").write_text(json.dumps(audit, indent=1))
    passed = not audit["columns"]
    extra["seam_audit"] = {k: audit[k] for k in ("step_m", "rule", "columns", "boundaries_with_steps")} | {
        "passed": passed}
    # An earlier world's game layers: kept, marked stale (they follow the ground).
    game, game_layers = extra.pop("game", None), extra.pop("game_layers", {})
    if game is not None:
        extra["game"] = game | {"stale": f"hydrology rerun {extra.get('hydrology_rerun')}: rerun game_layers"}
        digests |= game_layers
    manifest = {
        "schema_version": 1, "kind": "world_preview", "world_input": str(world_input.resolve()),
        "spacing_m": 1.0, "size": [n, n], "class_id": c.class_id,
        "method": "banded L4 over a cascade chain 4 m world, 1 m hydrology per band",
        "water_fill": hydrology.WATER_FILL, **extra,
        "source_world": str(c.world_dir.resolve()), "params": c.params, "band_s": c.band_s,
        "hydrology_s": state["hydrology_s"], "layers": digests}
    # A world that failed its audit gets no manifest.json (export and resumes see it unfinished).
    name = "manifest.json" if passed else MANIFEST_FAILED
    cascade.save_json(c.out / name, manifest)
    if passed:
        (c.out / MANIFEST_FAILED).unlink(missing_ok=True)
    else:
        raise SeamAuditFailed(f"seam audit: {audit['columns']} columns step or hold a slot at "
                           f"{audit['boundaries_with_steps']} band boundaries (hydrology/seam-audit.json; "
                           f"{MANIFEST_FAILED}); fix hydrology and rerun with --hydrology-only")


def _setup(params: dict, out: Path) -> SimpleNamespace:
    """What the bands need from the run's parameters (as `main` checked them)."""
    world_dir = Path(params["world"])
    world = json.loads((world_dir / "manifest.json").read_text())
    coarse_dir = (world_dir / world["hydrology"]["4m"]).resolve()
    class_id = params["class_id"]
    per_cell = class_id == "per cell"
    n4 = world["size"][0]
    c4 = n4 if params["km"] is None else params["km"] * 256
    col4, row4 = params.get("window_4m", [(n4 - c4) // 2] * 2)
    g4 = json.loads((coarse_dir / "grid.json").read_text())
    off_m = g4.get("offset_m") or [0.0, 0.0]
    n = c4 * 4

    def read4(name, dtype):
        # Memory-mapped: a band reads only its rows.
        mm = np.memmap(world_dir / name, dtype=dtype, mode="r", shape=(n4, n4))
        return mm[row4 : row4 + c4, col4 : col4 + c4]

    # Band grids sit in the 4 m hydrology grid's plane, offset by the crop.
    return SimpleNamespace(
        out=out, hyd=out / "hydrology", world_dir=world_dir, world=world, coarse_dir=coarse_dir,
        class_id=class_id, per_cell=per_cell, n4=n4, c4=c4, col4=col4, row4=row4, n=n, params=params,
        bands=cascade.bands(n, params["band_m"], params["halo_m"]), halo_m=params["halo_m"],
        origin=g4["origin"], east=off_m[0] + col4 * 4.0, south=off_m[1] + row4 * 4.0, read4=read4,
        classes4=read4("classes.u8", np.uint8) if per_cell else None)


def _layers(c: SimpleNamespace, mode: str, raw_mode: str | None = None) -> None:
    def layer(name, dtype, m=mode):
        return np.memmap(c.out / name, dtype=dtype, mode=m, shape=(c.n, c.n))

    c.elev, c.water_out = layer("elevation.f32", "<f4"), layer("water.u8", np.uint8)
    c.surface_out, c.body_out = layer("water-surface.f32", "<f4"), layer("water-body.u32", "<u4")
    c.snow_out, c.veg_out = layer("snow-ice.u8", np.uint8), layer("vegetation.u8", np.uint8)
    c.raw = layer(RAW, "<f4", raw_mode or mode) if mode == "w+" or (c.out / RAW).exists() else None


def _hydrology_only(out: Path, drop_raw: bool, restart: bool) -> int:
    """Rerun every band's hydrology from the kept raw surface of an L4 world (finished, failed
    or with hydrology stopped part-way), without diffusion. Resumes after the last band it
    finished while the hydrology binary and Python code are the same; a change (a code fix),
    `restart` or an attempt that failed its seam audit starts again from band 0."""
    progress_path = out / "progress.json"
    if not progress_path.exists() or not (out / RAW).exists():
        print(f"error: {out} has no progress.json or {RAW} (worlds made before it was kept need diffusion again)",
              file=sys.stderr)
        return 2
    progress = json.loads(progress_path.read_text())
    if progress["bands_done"] < len(cascade.bands(*_shape(progress["params"]))):
        print("error: diffusion is not finished; rerun the full command to resume it", file=sys.stderr)
        return 2
    c = _setup(progress["params"], out)
    _layers(c, "r+", raw_mode="r")  # hydrology never writes the raw surface
    c.raw_kept = not drop_raw
    pinned = _pin(c)
    binary, code = pinned["sha256"], _code_digest()
    state = progress.get("rerun")
    if state is not None:
        done = state["hydrology_done"]
        why = ("--restart" if restart else "the hydrology binary changed" if state.get("binary") != binary
               else "the hydrology Python code changed" if state.get("code") != code
               else "the last attempt failed its seam audit" if state.get("audit_failed")
               else "the band to resume from is gone"
               if 0 < done < len(c.bands) and not (c.hyd / f"band-{done - 1:03d}").exists()
               else None)
        if why:
            print(f"hydrology only: starting again from band 0 ({why})", flush=True)
            state = None
    if state is None:
        # The manifest of the world as it was (moved aside: its layers change from now on); after
        # an audit failure, the failed one.
        old = progress.get("rerun", {}).get("manifest")
        for name in ("manifest.json", MANIFEST_FAILED):
            if old is None and (out / name).exists():
                old = json.loads((out / name).read_text())
        state = progress["rerun"] = {"hydrology_done": 0, "hydrology_s": [], "bodies": {}, "binary": binary,
                                     "code": code, "manifest": old}
    # The raw surface is what diffusion wrote: checked against its recorded digest.
    want = ((state.get("manifest") or {}).get("hydrology_input") or {}).get("sha256") or state.get("raw_sha256")
    print(f"hydrology only: checking {RAW}", flush=True)
    got = hydrology.sha256(out / RAW)
    if want is not None and got != want:
        print(f"error: {RAW} does not match its recorded sha256 {want}; it is not the diffused surface",
              file=sys.stderr)
        return 2
    state["raw_sha256"] = got

    def save():
        cascade.save_json(progress_path, progress)

    c.save, c.band_s = save, progress["band_s"]
    save()
    (out / "manifest.json").unlink(missing_ok=True)
    print(f"hydrology only: {len(c.bands)} bands; resuming after band {state['hydrology_done']}", flush=True)
    while state["hydrology_done"] < len(c.bands):
        _hydro_band(c, state["hydrology_done"], state)
    manifest = state.get("manifest") or {}
    # Fields of the diffusion that stay true; game layers follow the ground and are marked stale.
    extra = {k: manifest[k] for k in ("peak_vram_gb", "deep_sea_bands", "windows", "game") if k in manifest}
    for k in ("deep_sea_bands", "windows"):  # a world whose inline hydrology stopped has no manifest
        if k not in extra and k in progress:
            extra[k] = progress[k]
    extra["game_layers"] = {k: v for k, v in manifest.get("layers", {}).items() if k.startswith("game/")}
    extra["hydrology_rerun"] = time.strftime("%Y-%m-%dT%H:%M:%S")
    extra["hydrology_binary"] = pinned
    try:
        _finish(c, state, extra)
    except SeamAuditFailed:
        # Only an audit failure restarts the next rerun; any other failure here just finishes again.
        state["audit_failed"] = True
        save()
        raise
    progress["hydrology_s"], progress["bodies"] = state["hydrology_s"], state["bodies"]
    progress["hydrology_done"] = len(c.bands)
    progress["hydrology_binary"], progress["hydrology_binaries"] = pinned, [binary]
    del progress["rerun"]
    save()
    _drop(c, drop_raw)
    if "game" in manifest:
        print("note: rerun game_layers, the game layers follow the ground", flush=True)
    print(f"done: hydrology of {out} in {sum(state['hydrology_s']) / 60:.0f} min", flush=True)
    return 0


def _shape(params: dict) -> tuple[int, int, int]:
    world = json.loads((Path(params["world"]) / "manifest.json").read_text())
    c4 = world["size"][0] if params["km"] is None else params["km"] * 256
    return c4 * 4, params["band_m"], params["halo_m"]


def _drop(c: SimpleNamespace, drop_raw: bool) -> None:
    """Remove what a finished run no longer needs (the raw surface only when asked)."""
    if drop_raw:
        c.raw = None
        (c.out / RAW).unlink(missing_ok=True)
    cascade.drop_overlaps(c.out, 1 << 30)
    for d in (c.hyd / f"band-{len(c.bands) - 1:03d}", c.hyd / "grid"):  # band reports stay
        shutil.rmtree(d, ignore_errors=True)


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--run", type=Path, help="L4 run directory")
    p.add_argument("--class-runs", type=Path, help="directory of per-class expert runs (c1, c2, ...) mixed over --run")
    p.add_argument("--world", type=Path, help="4 m world output (cascade_chain)")
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--km", type=int, help="square side in units of 1,024 m (power of two); default whole world")
    p.add_argument("--centre", help="X,Y: centre of the --km square in map metres from the north-west corner "
                   "(y south); default the 4 m world's centre")
    p.add_argument("--class-id", type=int, help="terrain class when the world manifest has none")
    p.add_argument("--band-m", type=int, default=1024, help="band height; 1,024 m keeps a 64 km run under about 10 GB")
    p.add_argument("--halo-m", type=int, default=256)
    p.add_argument("--steps", type=int, default=12)
    p.add_argument("--stride", type=int, default=192)
    p.add_argument("--batch", type=int, default=16)
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--guidance", type=float, default=1.0, help="class guidance scale (1: off)")
    p.add_argument("--guidance-classes", type=int, nargs="+",
                   help="legend classes whose windows get --guidance; other windows none (default: all)")
    p.add_argument("--hydrology-only", action="store_true",
                   help=f"rerun every band's hydrology of --out from its {RAW} (no GPU); resumes while the "
                        "hydrology binary and Python code are unchanged, else starts again from band 0; the "
                        "binary (TOPOGRAPHY_BIN, else the release build) is copied to hydrology/bin")
    p.add_argument("--restart", action="store_true", help="with --hydrology-only: start again from band 0")
    p.add_argument("--seam-audit", action="store_true",
                   help="only audit the band seams of a finished --out (exit 1 on a step)")
    p.add_argument("--drop-raw", action="store_true", help=f"remove {RAW} when finished (then no hydrology rerun)")
    args = p.parse_args(argv)
    if args.seam_audit:
        progress = json.loads((args.out / "progress.json").read_text())
        n, band_m, halo_m = _shape(progress["params"])
        audit = _seam_audit(args.out, n, cascade.bands(n, band_m, halo_m))
        print(json.dumps({k: audit[k] for k in ("step_m", "rule", "columns", "boundaries_with_steps", "worst")},
                         indent=1))
        return 1 if audit["columns"] else 0
    if args.hydrology_only:
        return _hydrology_only(args.out, args.drop_raw, args.restart)
    if args.run is None or args.world is None:
        p.error("--run and --world are needed (but for --hydrology-only and --seam-audit)")
    if args.centre is not None and args.km is None:
        p.error("--centre needs --km")

    import torch

    from .cascade_chain import _load
    from .model import plan_windows, sample_field, world_noise

    world = json.loads((args.world / "manifest.json").read_text())
    if world.get("kind") != "world_preview" or world.get("spacing_m") != 4.0:
        print("error: --world must be a 4 m world output", file=sys.stderr)
        return 2
    if "4m" not in world.get("hydrology", {}):
        print("error: the world has no 4 m hydrology output; regenerate it with cascade_chain", file=sys.stderr)
        return 2
    coarse_dir = (args.world / world["hydrology"]["4m"]).resolve()
    class_id = args.class_id if args.class_id is not None else world.get("class_id")
    per_cell = class_id is None and (args.world / "classes.u8").exists()
    if per_cell:
        class_id = "per cell"
    if class_id is None:
        print("error: the world has no class_id or classes.u8; pass --class-id", file=sys.stderr)
        return 2
    # Hydrology halos are as wide as the diffusion halo and must lie in final rows.
    if args.band_m % 64 or args.halo_m % 64 or args.halo_m * 3 // 2 > args.band_m:
        print("error: band and halo must be multiples of 64 m, the halo at most 2/3 of a band", file=sys.stderr)
        return 2
    n4 = world["size"][0]
    c4 = n4 if args.km is None else args.km * 256
    if c4 > n4 or c4 & (c4 - 1):
        print("error: --km must give a power-of-two square inside the world", file=sys.stderr)
        return 2
    # The world's 4 m grid in the map plane (an L3 square sits at its offset).
    g4 = json.loads((coarse_dir / "grid.json").read_text())
    off_m = g4.get("offset_m") or [0.0, 0.0]
    col4 = row4 = (n4 - c4) // 2
    if args.centre is not None:
        try:
            cx, cy = (float(v) for v in args.centre.split(","))
        except ValueError:
            print("error: --centre takes X,Y", file=sys.stderr)
            return 2
        col4, row4 = (int(np.clip(round((c - o) / 4.0 - c4 / 2), 0, n4 - c4)) for c, o in ((cx, off_m[0]), (cy, off_m[1])))
    n = c4 * 4

    params = {"run": str(args.run.resolve()), "world": str(args.world.resolve()), "km": args.km,
              "class_id": class_id, "band_m": args.band_m, "halo_m": args.halo_m, "steps": args.steps,
              "stride": args.stride, "seed": args.seed}
    if args.guidance != 1.0:
        params["guidance"] = args.guidance
        if args.guidance_classes:
            params["guidance_classes"] = sorted(args.guidance_classes)
    if args.centre is not None:
        params["window_4m"] = [col4, row4]
    if args.class_runs is not None:
        params["class_runs"] = str(args.class_runs.resolve())
    progress_path = args.out / "progress.json"
    if args.out.exists():
        if (args.out / "manifest.json").exists():
            print("error: output is already complete (--hydrology-only reruns its hydrology)", file=sys.stderr)
            return 2
        if not progress_path.exists() or json.loads(progress_path.read_text())["params"] != params:
            print("error: output directory exists with other arguments", file=sys.stderr)
            return 2
        progress = json.loads(progress_path.read_text())
        if "rerun" in progress:  # its layers are part rerun: finishing here would mix bands
            print("error: a --hydrology-only rerun is in progress or failed; continue it with "
                  "--hydrology-only --out <this world>", file=sys.stderr)
            return 2
        mode = "r+"
    else:
        # Plus two band directories of about 22 B a cell while the run goes.
        band_cells = n * (args.band_m + 2 * args.halo_m)
        hydrology.check_disk(args.out, DISK_BYTES * n * n + 2 * 22 * band_cells, "the L4 run")
        args.out.mkdir(parents=True)
        (args.out / "hydrology").mkdir()
        progress = {"params": params, "bands_done": 0, "band_s": [], "hydrology_done": 0, "hydrology_s": [],
                    "bodies": {}}
        mode = "w+"

    c = _setup(params, args.out)
    # Diffusion writes the raw surface (cross-faded between bands); hydrology carves it. It is
    # kept, so hydrology can be rerun without diffusion (`--hydrology-only`).
    _layers(c, mode)
    c.raw_kept = not args.drop_raw
    raw, h4 = c.raw, c.read4("elevation.f32", "<f4")
    water4, snow4, veg4 = c.read4("water.u8", np.uint8), c.read4("snow-ice.u8", np.uint8), c.read4("vegetation.u8", np.uint8)
    classes4 = c.classes4

    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    model, config = _load(args.run, device, args.class_runs)
    level = cascade.LEVELS["l4"]
    if config.get("level") != "l4":
        print("error: --run is not an L4 run", file=sys.stderr)
        return 2
    scale, window = config["residual_scale_m"], config["crop"]
    from . import legend

    n_classes, lut = legend.model_lut(config)
    m = MARGIN_4M
    bands = c.bands
    done = progress["bands_done"]
    print(f"{n} x {n} at 1 m in {len(bands)} bands; resuming after band {done}" if done else
          f"{n} x {n} at 1 m in {len(bands)} bands", flush=True)
    east, south = c.east, c.south

    def save():
        cascade.save_json(progress_path, progress)

    c.save, c.band_s = save, progress["band_s"]
    failed: list[Exception] = []

    def hydro_until(k: int):
        # A failed band (any failed check, the seam checks included) stops hydrology only:
        # diffusion does not read it, so the GPU keeps working and a rerun after the fix
        # resumes hydrology at that band.
        while not failed and progress["hydrology_done"] < k:
            try:
                _hydro_band(c, progress["hydrology_done"], progress)
            except Exception as e:  # any hydrology error, not only failed checks
                failed.append(e)
                print(f"hydrology stopped, diffusion continues: {e}", flush=True)

    if progress["hydrology_done"] < len(bands):
        # Every binary this run's bands were made with (a resume may bring a new one).
        used = progress.setdefault("hydrology_binaries", [])
        pinned = _pin(c)
        if not used or used[-1] != pinned["sha256"]:
            used.append(pinned["sha256"])
        progress["hydrology_binary"] = pinned
        save()
    hydro_until(done - 1)
    for k in range(done, len(bands)):
        t = time.time()
        a, b = bands[k]
        g0, g1 = max(a - args.halo_m, 0), min(b + args.halo_m, n)
        # 4 m rows covering the band with a margin; the 1 m fields are cropped back to it.
        r0, r1 = g0 // 4 - m, g1 // 4 + m
        crop = np.s_[m * 4 : -m * 4, m * 4 : -m * 4]
        ground4 = _rows_4m(h4, r0, r1, -m, c4 + m)
        # A band of deep sea only (with its margin) keeps the refined 4 m ground: L4 learned land only.
        deep = bool((ground4 < cascade.DEEP_SEA_M).all())
        coarse = cascade.refine_surface(level, ground4)[crop]
        del ground4
        water = cascade.refine_mask(_rows_4m(water4, r0, r1, -m, c4 + m) == 255, 4, WATER_SIGMA_M,
                                    keep_thin=True)[crop]

        def cover(mask):
            rows = _rows_4m(mask, r0, r1, -m, c4 + m) == 2
            return soft_cover(np.repeat(np.repeat(rows, 4, 0), 4, 1))[crop]

        snow, veg = cover(snow4), cover(veg4)
        classes = class_id
        if per_cell:
            classes = np.repeat(np.repeat(_rows_4m(classes4, r0, r1, -m, c4 + m), 4, 0), 4, 1)[crop]
        guide = None
        if args.guidance != 1.0 and args.guidance_classes:
            guide = np.isin(classes, args.guidance_classes)
            guide = (None if guide else False) if guide.ndim == 0 else guide
        # Legend v3 classes as the run's model classes (older runs see subclasses as parents).
        classes = int(lut[classes]) if isinstance(classes, (int, np.integer)) else lut[classes]
        extra = [np.where(water, 255, 0).astype(np.uint8), snow, veg]
        if deep:
            h = coarse
        elif "forms" in config:  # form channels from the run's per-class table (forms.py)
            from . import forms

            cmap = np.full(coarse.shape, classes, np.uint8) if np.ndim(classes) == 0 else classes.astype(np.uint8)
            extra += list(forms.from_classes(cmap, config["forms"]["table"], None,
                                             config["forms"]["sigma"] * window).astype(np.float16))
            del cmap
        note = " (deep sea, not diffused)"
        if not deep:
            # L4 learned land only: water keeps the refined 4 m ground, with a soft shore; windows
            # of deep water only are not diffused.
            land = 1.0 - np.clip(smooth(water.astype(np.float32), 2.0), 0, 1)
            skip = (coarse < cascade.DEEP_SEA_M) & (land < 1e-3)
            plan = plan_windows(*coarse.shape, window, args.stride, skip, guide if guide is not False else None)
            stats = progress.setdefault("windows", {"total": 0, "skipped": 0, "guided": 0})
            stats["total"] += len(plan[1])
            stats["skipped"] += len(plan[2])
            g = 1.0 if guide is False else args.guidance
            stats["guided"] += 0 if g == 1.0 else len(plan[1]) if plan[3] is None else len(plan[3] - plan[2])
            note = f" ({len(plan[2])}/{len(plan[1])} windows deep sea)"
            field = cascade.LazyField(level, coarse, extra, classes, n_classes)
            # Noise of the map's 1 m grid, so a square previews the whole map at its place.
            wy, wx = int(south) + g0, int(east)
            res = sample_field(model, field, steps=args.steps, window=window, stride=args.stride, batch=args.batch,
                               noise=world_noise(args.seed, wy, wy + g1 - g0, wx, wx + n), guidance=g,
                               plan=plan)[0, 0].numpy()
            h = (coarse + res * scale * land).astype(np.float32)
            del res, field, land, skip
        else:
            progress.setdefault("deep_sea_bands", []).append(k)

        cascade.write_band(raw, h, bands, k, g0, args.halo_m, args.out)
        c.snow_out[a:b] = np.where(snow[a - g0 : b - g0] >= 128, 2, 1)
        c.veg_out[a:b] = np.where(veg[a - g0 : b - g0] >= 128, 2, 1)
        for mm in (c.snow_out, c.veg_out):
            mm.flush()
        progress["band_s"].append(time.time() - t)
        progress["bands_done"] = k + 1
        save()
        cascade.drop_overlaps(args.out, k)
        print(f"band {k + 1}/{len(bands)} rows {a}-{b} in {progress['band_s'][-1]:.0f} s{note}", flush=True)
        # Free the band's fields before Rust runs; the band above is now final down to its halo.
        del coarse, water, snow, veg, classes, extra, h, guide
        hydro_until(k)
    hydro_until(len(bands))
    if failed:
        raise failed[0]

    extra = {}
    if torch.cuda.is_available():  # this process only; a resumed run counts its last part
        extra["peak_vram_gb"] = round(torch.cuda.max_memory_reserved() / 2**30, 2)
    extra["deep_sea_bands"] = progress.get("deep_sea_bands", [])
    extra["windows"] = progress.get("windows", {})  # of diffused bands: deep-sea skipped, guided
    if "hydrology_binary" in progress:
        extra["hydrology_binary"] = progress["hydrology_binary"] | (
            {"mixed": progress["hydrology_binaries"]} if len(progress.get("hydrology_binaries", [])) > 1 else {})
    _finish(c, progress, extra)
    # Only now the run cannot be resumed any more: drop the last band (and the raw surface
    # with --drop-raw).
    _drop(c, args.drop_raw)
    print(f"done: wrote {args.out} in {(sum(progress['band_s']) + sum(progress['hydrology_s'])) / 60:.0f} min",
          flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
