"""The level runner on a tiny coarse grid (needs the release topography binary)."""

import json
import tempfile
import unittest
from pathlib import Path

import numpy as np

SCRATCH = Path(__file__).resolve().parents[2] / "scratch" / "test-water"
SCRATCH.mkdir(parents=True, exist_ok=True)

from topography_training.water import grids, review, run as W  # noqa: E402


def coarse_grid(run: Path, n: int = 64, spacing: float = 32.0) -> Path:
    y, x = np.mgrid[0:n, 0:n].astype(np.float32)
    z = 300.0 - 5.0 * x + 30.0 * np.sin(x / 7.0) * np.cos(y / 9.0) - 60.0 * np.exp(-((x - 20) ** 2 + (y - 20) ** 2) / 40.0)
    prov = np.where(x < 30, 1, 2).astype(np.uint8)
    return grids.write_grid(run / "grid", z, spacing, (0.0, n * spacing), np.full((n, n), 18, np.uint8),
                            wind_from_deg=270.0, provenance=prov, provenance_cell_m=spacing)


class LevelTest(unittest.TestCase):
    def test_a_coarse_level_rains_erodes_checks_and_reviews(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as tmp:
            run = Path(tmp) / "run-32m"
            coarse_grid(run)
            code = W.run_level(run, tile=32, threads=2, river_km2=0.05)
            self.assertIn(code, (0, 1))
            level = json.loads((run / "level.json").read_text())
            self.assertEqual(level["passed"], code == 0)
            for name in ("elevation.f32", "water-surface.f32", "water-body.u32", "bodies.json", "report.json",
                         "moisture.f32", "grid.json", "flow.u8", "discharge.f32"):
                self.assertTrue((run / "out" / name).exists(), name)
            stages = json.loads((run / "work" / "stages.json").read_text())
            self.assertEqual(stages["erosion"]["routings"], 5)
            self.assertTrue(any(p.parts[-2] == "lake-exits" for p in map(Path, level["review"])) or
                            not any(b["kind"] == "lake" for b in json.loads((run / "out" / "bodies.json").read_text())["bodies"]))
            # Resuming does nothing new.
            self.assertEqual(W.run_level(run, tile=32, threads=2, river_km2=0.05), code)

    def test_a_finer_level_takes_the_coarse_rain_and_is_guided(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as tmp:
            c = Path(tmp) / "run-32m"
            coarse_grid(c)
            W.run_level(c, tile=32, threads=2, river_km2=0.05)
            f = Path(tmp) / "run-4m"
            z32 = np.fromfile(c / "out" / "elevation.f32", "<f4").reshape(64, 64)
            z4 = np.kron(z32[16:32, 16:32], np.ones((8, 8), np.float32))  # 128² at 4 m inside it
            prov = np.fromfile(c / "grid" / "provenance.u8", np.uint8).reshape(64, 64)[16:32, 16:32]
            g = grids.write_grid(f / "grid", z4, 4.0, (0.0, 64 * 32.0), np.full((16, 16), 18, np.uint8),
                                 offset_m=(16 * 32.0, 16 * 32.0), classes_cell_m=32.0, provenance=prov,
                                 provenance_cell_m=32.0)
            grids.add_moisture(g, c / "out")
            code = W.run_level(f, tile=64, threads=2, coarse=c / "out", river_km2=0.005)
            level = json.loads((f / "level.json").read_text())
            self.assertIn("hard_failures", level["coarse_guide"])
            self.assertEqual(code == 0, level["passed"])
            if level["coarse_guide"]["hard_failures"]:
                self.assertIn("coarse_guide", level["failures"])

    def test_provenance_window_is_nearest(self):
        p = np.arange(16, dtype=np.uint8).reshape(4, 4)
        w = grids.provenance_window(p, 32.0, 64.0, 16, 4.0)  # 64 m square at (32, 64) m, 4 m cells
        self.assertEqual(w.shape, (16, 16))
        self.assertTrue((w[:8, :8] == p[2, 1]).all() and (w[8:, 8:] == p[3, 2]).all())


if __name__ == "__main__":
    unittest.main()
