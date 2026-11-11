"""Repair an L4 raw world made before `cascade_l4.blend_residual`: under 4 m rivers keep only the
smooth part of the diffused residual, as cascade_l4 now does, without rerunning the diffusion.

    python -m topography_training.diffusion.l4_repair --source <raw world> --out <new raw world>

The residual is the raw surface minus the refined 4 m ground, recomputed exactly as the band loop
did. Land cells keep their surface; the other layers are hard-linked from the source.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
import time
from pathlib import Path

import numpy as np

from . import cascade, hydrology
from .cascade_l4 import MARGIN_4M, RAW, WATER_SIGMA_M, _rows_4m, _setup, blend_residual

TILE_M = 1024
HALO_M = 64  # context rows for the smoothing; a multiple of 4


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--source", type=Path, required=True, help="L4 raw world (cascade_l4 output)")
    p.add_argument("--out", type=Path, required=True, help="new raw world directory")
    args = p.parse_args(argv)
    src = json.loads((args.source / "manifest.json").read_text())
    params = src["params"]
    if src.get("kind") != "world_raw" or not params.get("keep_river_residual"):
        print("error: --source must be an L4 raw world that kept the river residual", file=sys.stderr)
        return 2
    if args.out.exists():
        print("error: --out exists", file=sys.stderr)
        return 2
    hydrology.check_disk(args.out.parent, 4 * src["size"][0] ** 2, "the repaired surface")
    args.out.mkdir(parents=True)

    c = _setup(params, args.out)
    n, c4, m = c.n, c.c4, MARGIN_4M
    level = cascade.LEVELS["l4"]
    raw = np.memmap(args.source / RAW, dtype="<f4", mode="r", shape=(n, n))
    new = np.memmap(args.out / RAW, dtype="<f4", mode="w+", shape=(n, n))
    h4, water4 = c.read4("elevation.f32", "<f4"), c.read4("water.u8", np.uint8)
    t0, changed = time.time(), 0
    for a in range(0, n, TILE_M):
        b = min(a + TILE_M, n)
        g0, g1 = max(a - HALO_M, 0), min(b + HALO_M, n)
        r0, r1 = g0 // 4 - m, g1 // 4 + m
        crop = np.s_[m * 4 : -m * 4, m * 4 : -m * 4]
        coarse = cascade.refine_surface(level, _rows_4m(h4, r0, r1, -m, c4 + m))[crop]
        water = cascade.refine_mask(_rows_4m(water4, r0, r1, -m, c4 + m) == 255, 4, WATER_SIGMA_M,
                                    keep_thin=True)[crop]
        lakes_sea = cascade.refine_mask(np.isin(_rows_4m(c.body4, r0, r1, -m, c4 + m), c.lake_sea_ids), 4,
                                        WATER_SIGMA_M, keep_thin=True)[crop]
        old = np.asarray(raw[g0:g1])
        h = (coarse + blend_residual(old - coarse, water, lakes_sea)).astype(np.float32)
        # Cells away from rivers keep their exact values (no float round trip).
        h = np.where(np.abs(h - old) > 1e-3, h, old)
        new[a:b] = h[a - g0 : b - g0]
        changed += int((h[a - g0 : b - g0] != old[a - g0 : b - g0]).sum())
        print(f"rows {a}-{b} ({time.time() - t0:.0f} s, {changed} cells changed)", flush=True)
    new.flush()
    del new

    for name in ("snow-ice.u8", "vegetation.u8", "moisture.f32"):
        if (args.source / name).exists():
            hydrology.link(args.source, args.out, (name,))
    for name in ("description.json", "description.md", "overview.png", "detail.png"):
        if (args.source / name).exists():
            shutil.copy2(args.source / name, args.out / name)
    shutil.copytree(args.source / "world-input", args.out / "world-input")
    digest = hydrology.sha256(args.out / RAW)
    manifest = src | {
        "world_input": str((args.out / "world-input").resolve()),
        "hydrology_input": src["hydrology_input"] | {"sha256": digest},
        "layers": src["layers"] | {RAW: digest},
        "repair": {"from": str(args.source.resolve()), "source_sha256": src["layers"][RAW],
                   "rule": "cascade_l4.blend_residual: smooth residual under 4 m rivers",
                   "cells_changed": changed},
    }
    cascade.save_json(args.out / "manifest.json", manifest)
    print(f"done: {args.out} ({changed} cells changed, {time.time() - t0:.0f} s)", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
