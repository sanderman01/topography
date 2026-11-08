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
        # A coarse output: its ground, 100 m and 10 m below the level under its lakes (the guide's depth).
        ground = np.full(body.shape, 100.0, np.float32)
        for b in bodies:
            if b["kind"] == "lake":
                ground[body == b["id"]] = b["level_m"] - 10.0
        ground.astype("<f4").tofile(dirpath / "elevation.f32")


class CoarseGuideTest(unittest.TestCase):
    def case(self, fine_level: float, split: bool = False, split_level: float | None = None):
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
        bodies = [{"id": 5, "kind": "lake", "level_m": fine_level},
                  {"id": 6, "kind": "lake", "level_m": fine_level if split_level is None else split_level},
                  {"id": 7, "kind": "river"}]
        write(tmp / "fine", 64, fb, bodies)
        return review.coarse_guide(tmp / "fine", tmp / "coarse", fine_dir=tmp / "fine", min_km2=0, min_depth_m=0,
                                   edge_m=0)

    def test_matching_lakes_and_rivers_pass(self):
        g = self.case(101.0)
        self.assertEqual(g["hard_failures"], 0)
        self.assertEqual(g["level_diff_m"]["2"], 1.0)
        self.assertEqual(g["coarse_river_share_with_fine_river_within_100m"], 1.0)

    def test_gross_level_or_doubled_lakes_fail(self):
        self.assertEqual(self.case(106.0)["lakes_over_5m"], [2])
        self.assertEqual(self.case(100.0, split=True, split_level=92.0)["lakes_doubled"], [2])

    def test_guide_captions_name_the_compared_levels(self):
        m = {"lake": 6, "failure": "over_5m", "coarse_level_m": 618.84, "fine_level_m": 628.24}
        c = review._guide_caption(m, 32.0, 4.0, 37376, 58288)
        self.assertEqual(c, "32 m lake 6: over_5m, 32 m level 618.84 m, 4 m level 628.24 m   centre E37376 "
                            "N58288   32 m water | 4 m water")
        self.assertIn("1 m level none", review._guide_caption(m | {"fine_level_m": None}, 4.0, 1.0, 0, 0))

    def test_a_split_at_the_coarse_level_is_a_figure(self):
        """Lake 1906: two pits joined diagonally at 32 m, both at the coarse level at 4 m."""
        g = self.case(100.0, split=True)
        self.assertEqual((g["lakes_doubled"], g["lakes_split_same_level"], g["hard_failures"]), ([], [2], 0))

    def spill_case(self, fine_level: float | None, notch_m: float | None = None, pit: bool = True,
                   side_pit: bool = False):
        """A coarse (4 m) endorheic lake at 100 m in a cone spilling at 112 m over the grid edge;
        the fine (1 m) cone the same, a supply-limited lake at `fine_level` (none: dry), its rim
        notched down to `notch_m` east of the centre, or with no pit at all."""
        tmp = Path(tempfile.mkdtemp())
        n = 32
        y, x = np.mgrid[:n, :n] + 0.5
        cz = (80.0 + 2.0 * np.hypot(x - 16, y - 16)).astype(np.float32)
        cb = np.where(cz < 100.0, 2, 0).astype(np.uint32)
        write(tmp / "coarse", n, cb, [{"id": 2, "kind": "lake", "level_m": 100.0, "endorheic": True}],
              np.zeros((n, n)), 4.0)
        cz.tofile(tmp / "coarse" / "elevation.f32")
        y, x = (np.mgrid[: n * 4, : n * 4] + 0.5) / 4
        fz = (80.0 + 2.0 * np.hypot(x - 16, y - 16)).astype(np.float32)
        if notch_m is not None:
            fz[62:66, 64:] = np.minimum(fz[62:66, 64:], notch_m)
        if not pit:
            fz = (200.0 - x).astype(np.float32)
        main = np.hypot(x - 16, y - 16) < 6.5
        if side_pit:
            # A walled side pit under the coarse lake, lower than the lake's floor, draining east
            # through its own walled trench at 95 m.
            fz[56:72, 88:128] = 120.0
            fz[60:68, 92:100] = 60.0
            fz[62:66, 100:128] = 95.0
        fb = np.zeros((n * 4, n * 4), np.uint32)
        bodies = []
        if fine_level is not None:
            fb[(fz < fine_level) & main] = 5
            bodies = [{"id": 5, "kind": "lake", "level_m": fine_level, "endorheic": True}]
        write(tmp / "fine", n * 4, fb, bodies)
        fz.tofile(tmp / "fine" / "elevation.f32")
        return review.coarse_guide(tmp / "fine", tmp / "coarse", fine_dir=tmp / "fine", min_km2=0, min_depth_m=0,
                                   edge_m=0)

    def test_supply_limited_lakes_compare_spills(self):
        """Lakes 1140/1141: the fine catchment halves, so the level drops 8 m; the spill stays."""
        g = self.spill_case(92.0)
        self.assertEqual((g["hard_failures"], g["lakes_supply"]), (0, [2]))
        self.assertLess(abs(g["spill_diff_m"]["2"]), 1.0)
        g = self.spill_case(None)  # dry at the fine level, its depression intact
        self.assertEqual((g["hard_failures"], g["lakes_gone"], g["lakes_supply"]), (0, [], [2]))

    def test_the_fine_spill_starts_at_the_fine_lake_not_a_side_pit(self):
        g = self.spill_case(92.0, side_pit=True)
        self.assertEqual((g["hard_failures"], g["lakes_supply"]), (0, [2]))

    def test_spills_found_at_the_crop_edge_are_recorded(self):
        self.assertEqual(self.spill_case(92.0)["spill_edge"], [2])  # the cone spills over the grid edge
        self.assertEqual(self.spill_case(92.0, side_pit=True)["spill_edge"], [2])

    def test_a_lowered_spill_or_a_missing_depression_stays_hard(self):
        g = self.spill_case(92.0, notch_m=104.0)  # the rim cut 8 m
        self.assertEqual((g["lakes_over_5m"], g["lakes_supply"]), ([2], []))
        g = self.spill_case(None, notch_m=104.0)
        self.assertEqual(g["lakes_gone"], [2])
        g = self.spill_case(None, pit=False)
        self.assertEqual(g["lakes_gone"], [2])

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
        g = review.coarse_guide(tmp / "fine", tmp / "coarse", fine_dir=tmp / "fine", min_km2=0, min_depth_m=0,
                                edge_m=0)
        self.assertEqual(g["lakes_compared"], 1)
        self.assertEqual(g["level_diff_m"], {"2": 2.0})
        self.assertEqual(g["hard_failures"], 0)

    def test_only_large_deep_lakes_away_from_edges_can_fail(self):
        tmp = Path(tempfile.mkdtemp())
        n = 128  # 4 m coarse cells: 512 m; footprint = the whole grid (no inner edge)
        cb = np.zeros((n, n), np.uint32)
        cb[10:12, 10:12] = 2  # 64 m²: small
        cb[40:100, 40:100] = 3  # 0.0576 km², 10 m deep: hard
        cb[20:80, 105:125] = 4  # 0.0192 km²: small
        ground = np.full((n, n), 100.0, np.float32)
        ground[40:100, 40:100] = 90.0
        bodies = [{"id": i, "kind": "lake", "level_m": 100.0} for i in (2, 3, 4)]
        write(tmp / "coarse", n, cb, bodies, np.zeros((n, n)), 4.0)
        ground.astype("<f4").tofile(tmp / "coarse" / "elevation.f32")
        write(tmp / "fine", n * 4, np.zeros((n * 4, n * 4), np.uint32), [])
        g = review.coarse_guide(tmp / "fine", tmp / "coarse", fine_dir=tmp / "fine", min_km2=0.05, min_depth_m=3.0)
        self.assertEqual(g["lakes_gone"], [3])
        self.assertEqual(g["lakes_small"], 2)
        self.assertEqual(g["hard_failures"], 1)
        shallow = review.coarse_guide(tmp / "fine", tmp / "coarse", fine_dir=tmp / "fine", min_km2=0.05,
                                      min_depth_m=12.0)
        self.assertEqual((shallow["hard_failures"], shallow["lakes_shallow"]), (0, 1))

    def test_lakes_near_a_window_edge_are_figures(self):
        tmp = Path(tempfile.mkdtemp())
        n = 1024  # 4 m coarse cells: 4,096 m
        ground = np.full((n, n), 100.0, np.float32)

        def coarse_with(r0, c0):
            cb = np.zeros((n, n), np.uint32)
            cb[r0 : r0 + 80, c0 : c0 + 80] = 5  # 0.1 km², 20 m deep
            g = ground.copy()
            g[r0 : r0 + 80, c0 : c0 + 80] = 80.0
            write(tmp / "coarse", n, cb, [{"id": 5, "kind": "lake", "level_m": 100.0}], np.zeros((n, n)), 4.0,
                  origin=[0.0, 4096.0])
            g.astype("<f4").tofile(tmp / "coarse" / "elevation.f32")
            return review.coarse_guide(tmp / "fine", tmp / "coarse", fine_dir=tmp / "fine", min_km2=0.05,
                                       min_depth_m=3.0)

        # A 1 m window over x, y in [0, 3,072) m: its east and south edges are inner, west and north are the world's.
        write(tmp / "fine", 3072, np.zeros((3072, 3072), np.uint32), [], origin=[0.0, 4096.0], offset_m=[0.0, 0.0])
        self.assertEqual(coarse_with(300, 10)["hard_failures"], 1, "beside the world's west edge: judged")
        g = coarse_with(300, 680)  # ends 32 m from the window's inner east edge
        self.assertEqual((g["hard_failures"], g["lakes_near_edge"]), (0, 1))


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
    body[300:320, 400:420] = 4  # closed: the flag wins over a stray exit cell
    write(out, n, body, [{"id": 2, "kind": "lake", "level_m": 95.0, "exit_cell": [140, 160], "endorheic": False},
                         {"id": 3, "kind": "river"},
                         {"id": 4, "kind": "lake", "level_m": 85.0, "exit_cell": [410, 299], "endorheic": True}],
          np.where(body == 3, 1.0, 0.01), origin=[1000.0, 3000.0], offset_m=[0.0, 1000.0])
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
            self.assertIn("overview.png", names)
            from PIL import Image

            with Image.open(Path(tmp) / "review" / "spots" / "E1300-N1700.png") as im:
                self.assertEqual(im.size, (3 * 512 + 2 * review.GAP_PX, 512 + review.BAR_PX))
                rgb = np.asarray(im.convert("RGB")).astype(int)
            raw, result = rgb[:512, :512], rgb[:512, 512 + review.GAP_PX : 1024 + review.GAP_PX]
            cut = rgb[:512, 1024 + 2 * review.GAP_PX :]

            def river(a):
                return (a[..., 0] < 60) & (a[..., 2] > 130) & (a[..., 2] > a[..., 1] + 60)

            # The river (dark blue, widened) shows in the result panel only.
            self.assertGreater(river(result).sum(), 3 * 300)
            self.assertEqual(river(raw).sum(), 0)
            # Rings: green at the open lake's exit, red at the closed lake's low point.
            for col in (review.OPEN_EXIT, review.CLOSED):
                self.assertGreater((result == col).all(axis=-1).sum(), 10, col)
            # The cut panel shows the 0.3 m lowering under water in red.
            self.assertGreater(((cut[..., 0] > cut[..., 2] + 40)).sum(), 1000)

    def test_the_overview_marks_the_crops(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = window_run(Path(tmp))
            review.crops(run, Path(tmp) / "review", spots=((1300, 1700),), areas=(), dam=None)
            from PIL import Image

            with Image.open(Path(tmp) / "review" / "overview.png") as im:
                self.assertEqual(im.size[1], 600 + review.BAR_PX)
                rgb = np.asarray(im.convert("RGB"))
            self.assertGreater((rgb[:600, :600] == review.MARK).all(axis=-1).sum(), 1000)

    def test_a_mismatch_gets_a_figure(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = window_run(Path(tmp))
            cb = np.zeros((150, 150), np.uint32)
            cb[25:40, 25:45] = 7  # the 1 m lake 2 at 95 m
            write(Path(tmp) / "4m", 150, cb, [{"id": 7, "kind": "lake", "level_m": 80.0}], np.zeros((150, 150)),
                  4.0, origin=[1000.0, 2000.0])
            guide = review.coarse_guide(run, Path(tmp) / "4m", fine_dir=run / "out", min_km2=0, min_depth_m=0,
                                        edge_m=0)
            self.assertEqual(guide["lakes_over_5m"], [7])
            self.assertEqual(guide["mismatches"][0]["box_m"], [1100.0, 1900.0, 1180.0, 1840.0])
            paths = review.guide_figures(run, Path(tmp) / "4m", guide, Path(tmp) / "guide")
            self.assertEqual([p.name for p in paths], ["lake-7-over_5m.png"])


class ExitCropTest(unittest.TestCase):
    def test_exit_crops_span_128_m_or_32_cells(self):
        self.assertEqual([review.exit_cells(s) * s for s in (1.0, 4.0, 32.0)], [128.0, 128.0, 1024.0])


class RowsTest(unittest.TestCase):
    def test_rows_match_array_slicing_at_any_stride(self):
        a = np.arange(37 * 29, dtype="<u4").reshape(37, 29)
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "a.u32"
            a.tofile(path)
            for win, k in [((0, 0, 37, 29), 1), ((5, 0, 9, 29), 1), ((3, 4, 20, 11), 1), ((0, 0, 37, 29), 4),
                           ((2, 3, 30, 25), 3)]:
                r0, c0, h, w = win
                np.testing.assert_array_equal(review._rows(path, "<u4", 29, win, k),
                                              a[r0 : r0 + h : k, c0 : c0 + w : k])


class WindowPairTest(unittest.TestCase):
    def test_raw_and_result_side_by_side(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = Path(tmp)
            (run / "grid").mkdir()
            (run / "work").mkdir()
            n = 64
            (run / "grid" / "grid.json").write_text(json.dumps({"size": [n, n], "spacing_m": 32.0}))
            y, x = np.mgrid[0:n, 0:n].astype(np.float32)
            (100 - x).astype("<f4").tofile(run / "grid" / "elevation.f32")
            (100 - x - (abs(y - 32) < 2) * 5).astype("<f4").tofile(run / "work" / "carved.f32")
            np.where(abs(y - 32) < 1, 96.0, np.nan).astype("<f4").tofile(run / "work" / "water-surface.f32")
            out = review.window_pair(run, run / "pair.png")
            from PIL import Image

            im = Image.open(out)
            self.assertEqual(im.size, (2 * n + review.GAP_PX, n))


if __name__ == "__main__":
    unittest.main()
