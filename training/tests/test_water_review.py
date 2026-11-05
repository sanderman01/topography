"""Coarse guide on a tiny pair of worlds; crops on a small window run."""

import json
import tempfile
import unittest
from pathlib import Path

import numpy as np

from topography_training.water import review


def write(dirpath: Path, n: int, body: np.ndarray, bodies: list, discharge: np.ndarray | None = None, spacing=1.0,
          **grid):
    dirpath.mkdir(parents=True, exist_ok=True)
    body.astype("<u4").tofile(dirpath / "water-body.u32")
    (dirpath / "bodies.json").write_text(json.dumps({"bodies": bodies}))
    (dirpath / "grid.json").write_text(json.dumps({"size": [n, n], "spacing_m": spacing, **grid}))
    if discharge is not None:
        discharge.astype("<f4").tofile(dirpath / "discharge.f32")


class CoarseGuideTest(unittest.TestCase):
    def case(self, fine_level: float, split: bool = False):
        tmp = Path(tempfile.mkdtemp())
        cb = np.zeros((16, 16), np.uint32)
        cb[4:8, 4:8] = 2  # a 4 m lake
        cb[10, 2:14] = 3  # a 4 m river
        cq = np.where(cb == 3, 1.0, 0.0)
        write(tmp / "coarse", 16, cb, [{"id": 2, "kind": "lake", "level_m": 100.0}, {"id": 3, "kind": "river"}], cq, 4.0)
        fb = np.zeros((64, 64), np.uint32)
        fb[16:32, 16:32] = 5
        if split:
            fb[16:32, 24:32] = 6
        fb[41, 8:56] = 7
        bodies = [{"id": 5, "kind": "lake", "level_m": fine_level}, {"id": 6, "kind": "lake", "level_m": fine_level},
                  {"id": 7, "kind": "river"}]
        write(tmp / "fine", 64, fb, bodies)
        return review.coarse_guide(tmp / "fine", tmp / "coarse", fine_dir=tmp / "fine")

    def test_matching_lakes_and_rivers_pass(self):
        g = self.case(101.0)
        self.assertEqual(g["hard_failures"], 0)
        self.assertEqual(g["level_diff_m"]["2"], 1.0)
        self.assertEqual(g["coarse_river_share_with_fine_river_within_100m"], 1.0)

    def test_gross_level_or_doubled_lakes_fail(self):
        self.assertEqual(self.case(106.0)["lakes_over_5m"], [2])
        self.assertEqual(self.case(100.0, split=True)["lakes_doubled"], [2])

    def test_a_window_is_compared_at_its_offset(self):
        """A 1 m window over part of the 4 m world: lakes outside it are not 'gone'."""
        tmp = Path(tempfile.mkdtemp())
        cb = np.zeros((64, 64), np.uint32)
        cb[20:24, 36:40] = 2  # inside the window
        cb[50:54, 2:6] = 4  # outside it
        write(tmp / "coarse", 64, cb, [{"id": 2, "kind": "lake", "level_m": 50.0},
                                      {"id": 4, "kind": "lake", "level_m": 10.0}],
              np.zeros((64, 64)), 4.0, origin=[0.0, 256.0])
        fb = np.zeros((64, 64), np.uint32)
        fb[(20 - 16) * 4 : (24 - 16) * 4, (36 - 32) * 4 : (40 - 32) * 4] = 9
        write(tmp / "fine", 64, fb, [{"id": 9, "kind": "lake", "level_m": 52.0}], origin=[0.0, 256.0],
              offset_m=[128.0, 64.0])
        g = review.coarse_guide(tmp / "fine", tmp / "coarse", fine_dir=tmp / "fine")
        self.assertEqual(g["lakes_compared"], 1)
        self.assertEqual(g["level_diff_m"], {"2": 2.0})
        self.assertEqual(g["hard_failures"], 0)


def window_run(root: Path, n: int = 600) -> Path:
    """A driver window run directory (grid/ and out/) at E1000 N2000 + offset, with a lake and a river."""
    run = root / "run"
    (run / "grid").mkdir(parents=True)
    out = run / "out"
    y, x = np.mgrid[0:n, 0:n].astype(np.float32)
    raw = 100.0 - 0.05 * y + 2.0 * np.sin(x / 7.0)
    raw.astype("<f4").tofile(run / "grid" / "elevation.f32")
    body = np.zeros((n, n), np.uint32)
    body[100:160, 100:180] = 2
    body[160:500, 140] = 3
    write(out, n, body, [{"id": 2, "kind": "lake", "level_m": 95.0, "exit_cell": [140, 160], "endorheic": False},
                         {"id": 3, "kind": "river"}], np.where(body == 3, 1.0, 0.01), origin=[1000.0, 3000.0],
          offset_m=[0.0, 1000.0])
    (raw - (body > 0) * 0.3).astype("<f4").tofile(out / "elevation.f32")
    np.where(body > 0, raw, np.nan).astype("<f4").tofile(out / "water-surface.f32")
    flow = np.full((n, n), 2, np.uint8)  # all south
    flow[body == 3] |= 16
    flow.tofile(out / "flow.u8")
    report = {"examples": {"thalweg_beside": [[300, 300], [301, 301], [10, 590]]},
              "figures": {"longest_straight_runs": [{"from": [140, 200], "to": [140, 300], "length_m": 100.0}]}}
    (out / "report.json").write_text(json.dumps(report))
    return run


class CropsTest(unittest.TestCase):
    def test_a_window_run_gets_every_crop_inside_it(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = window_run(Path(tmp))
            # The window spans E1000-1600 N2000-1400: one owner spot inside, the rest skipped.
            spots = ((1300, 1700), (25645, 24400))
            paths = review.crops(run, Path(tmp) / "review", spots=spots, areas=(), dam=(1140, 1840))
            names = sorted(str(p.relative_to(Path(tmp) / "review")) for p in paths)
            self.assertIn("spots/E1300-N1700.png", names)
            self.assertFalse(any("E25645" in s for s in names))
            self.assertIn("dam/flow-E1140-N1840.png", names)
            self.assertEqual(len([s for s in names if s.startswith("straight-runs/")]), 1)
            self.assertEqual(len([s for s in names if s.startswith("failures/thalweg_beside")]), 2)
            self.assertEqual([s for s in names if s.startswith("lake-exits/")], ["lake-exits/sheet-01.png"])
            from PIL import Image

            with Image.open(Path(tmp) / "review" / "spots" / "E1300-N1700.png") as im:
                self.assertEqual(im.size, (2 * 512 + review.GAP_PX, 512))
                rgb = np.asarray(im.convert("RGB")).astype(int)
            # The river (cyan) shows in the result panel only.
            right = rgb[:, 512 + review.GAP_PX :]
            cyan = (right[..., 2] > right[..., 0] + 40) & (right[..., 1] > right[..., 0] + 40)
            self.assertGreater(cyan.sum(), 100)
            left = rgb[:, :512]
            self.assertEqual(((left[..., 2] > left[..., 0] + 40) & (left[..., 1] > left[..., 0] + 40)).sum(), 0)

    def test_window_pair_shows_the_whole_window(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = window_run(Path(tmp))
            path = review.window_pair(run, Path(tmp) / "pair.png")
            from PIL import Image

            with Image.open(path) as im:
                self.assertEqual(im.size, (2 * 600 + review.GAP_PX, 600))


if __name__ == "__main__":
    unittest.main()
