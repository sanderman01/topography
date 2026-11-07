"""World driver end to end on a tiny fake L4 world (needs the release topography binary)."""

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import numpy as np

# The driver reflinks (`cp --reflink=always`, N12), which /tmp (tmpfs) cannot: test worlds
# live in the repository's scratch directory, on the btrfs /home.
SCRATCH = Path(__file__).resolve().parents[2] / "scratch" / "test-water"
SCRATCH.mkdir(parents=True, exist_ok=True)

from topography_training.water import run as W  # noqa: E402


def fake_world(root: Path, n: int = 256, n4: int | None = None, window_4m: list[int] | None = None) -> Path:
    """An L4-shaped world over a chain world: elevation-raw.f32, the chain's 4 m classes,
    provenance and its 4 m water (no lakes) with grid.json and moisture (real rain),
    snow/vegetation, world-input. With `n4` > n // 4 the world is a square of the chain at
    `window_4m` (col, row in 4 m cells), and the chain's classes and provenance are patterned."""
    chain = root / "chain"
    h4 = chain / "hydrology" / "4m"
    h4.mkdir(parents=True)
    n4 = n4 or n // 4
    if n4 == n // 4:
        np.full((n4, n4), 18, np.uint8).tofile(chain / "classes.u8")
        np.full((n4 // 8, n4 // 8), 2, np.uint8).tofile(chain / "provenance.u8")  # blend
    else:
        y, x = np.mgrid[0:n4, 0:n4]
        ((x + 3 * y) % 23 + 1).astype(np.uint8).tofile(chain / "classes.u8")
        np.arange((n4 // 8) ** 2, dtype=np.uint8).tofile(chain / "provenance.u8")
    (chain / "manifest.json").write_text(json.dumps({"size": [n4, n4]}))
    moisture = {"file": "moisture.f32", "size": [2, 2], "spacing_m": n4 * 2.0, "nw_m": [0.0, n4 * 4.0],
                "scale_mm_h": 4.0, "dtype": "float32"}
    np.array([[1.0, 0.6], [1.4, 0.8]], "<f4").tofile(h4 / "moisture.f32")
    (h4 / "grid.json").write_text(json.dumps(
        {"size": [n4, n4], "spacing_m": 4.0, "origin": [0.0, n4 * 4.0], "moisture": moisture,
         "discharge": {"mean_land_mm_h": 2.0}}))
    np.zeros((n4, n4), "<u4").tofile(h4 / "water-body.u32")
    np.zeros((n4, n4), "<f4").tofile(h4 / "discharge.f32")
    np.full((n4, n4), 40.0, "<f4").tofile(h4 / "elevation.f32")
    (h4 / "bodies.json").write_text(json.dumps({"bodies": []}))
    (h4 / "report.json").write_text(json.dumps({"passed": True, "failures": []}))
    col4, row4 = window_4m or [0, 0]
    world = root / "l4"
    (world / "world-input").mkdir(parents=True)
    y, x = np.mgrid[0:n, 0:n].astype(np.float32)
    z = 40.0 - 0.15 * x + 3.0 * np.sin(x / 9.0) * np.cos(y / 11.0) - 6.0 * np.exp(-((x - 80) ** 2 + (y - 80) ** 2) / 300.0)
    z.astype("<f4").tofile(world / "elevation-raw.f32")
    np.array([[1.0, 0.6], [1.4, 0.8]], "<f4").tofile(world / "moisture.f32")
    for name in ("snow-ice.u8", "vegetation.u8"):
        np.zeros((n, n), np.uint8).tofile(world / name)
    origin = [col4 * 4.0, (n4 - row4) * 4.0]  # the square's own NW corner, as cascade_l4 writes it
    (world / "world-input" / "world.json").write_text(json.dumps({"name": "fake", "origin": origin}))
    manifest = {
        "schema_version": 1, "kind": "world_preview", "spacing_m": 1.0, "size": [n, n],
        "world_input": str(world / "world-input"), "source_world": str(chain), "class_id": "per cell",
        "moisture": moisture, "hydrology_input": {"file": "elevation-raw.f32"}, "layers": {},
        **({"params": {"window_4m": [col4, row4]}} if window_4m else {}),
    }
    (world / "manifest.json").write_text(json.dumps(manifest))
    return world


class RunTest(unittest.TestCase):
    def test_a_world_runs_resumes_and_is_assembled(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as tmp:
            root = Path(tmp)
            src = fake_world(root)
            out = root / "v3"
            code = W.run_world(src, out, tile=64, threads=2)
            self.assertIn(code, (0, 1))
            manifest = out / ("manifest.json" if code == 0 else "manifest-failed.json")
            m = json.loads(manifest.read_text())
            self.assertEqual(m["source_world"], str(root / "chain"))
            self.assertEqual(m["water"]["ground"], "carved.f32")
            for name in ("elevation.f32", "water-surface.f32", "water-body.u32", "bodies.json", "water.u8",
                         "snow-ice.u8", "vegetation.u8", "moisture.f32", "overview.png", "detail.png"):
                self.assertTrue((out / name).exists(), name)
                self.assertFalse((out / name).is_symlink(), name)
            self.assertTrue((out / "hydrology" / "1m" / "report.json").exists())
            # Review figures (Task 17); the fake chain's 4 m water has no lakes, so the guide passes.
            self.assertEqual(m["water"]["coarse_guide"]["hard_failures"], 0)
            self.assertNotIn("coarse_guide", m["water"]["failures"])
            self.assertIn("hydrology/review/overview.png", m["water"]["review"])
            for name in m["water"]["review"]:
                self.assertTrue(name.startswith("hydrology/review/") and (out / name).exists(), name)
            self.assertGreaterEqual(m["water"]["review_seconds"], 0)
            grid = json.loads((out / "hydrology" / "grid" / "grid.json").read_text())
            self.assertEqual(grid["moisture"]["scale_mm_h"], 4.0, "the chain's real moisture entry")
            self.assertEqual(grid["classes_cell_m"], 4.0)
            raw = np.fromfile(out / "elevation-raw.f32", "<f4")
            self.assertTrue(np.array_equal(raw, np.fromfile(src / "elevation-raw.f32", "<f4")), "raw kept")
            # The binary is pinned in the run directory.
            pins = list((out / "hydrology" / "bin").glob("topography-*"))
            self.assertEqual(len(pins), 1)
            # A rerun resumes and changes nothing.
            before = (out / "elevation.f32").read_bytes()
            stamp = (out / "hydrology" / "1m" / "report.json").stat().st_mtime_ns
            self.assertEqual(W.run_world(src, out, tile=64, threads=2), code)
            self.assertEqual((out / "elevation.f32").read_bytes(), before)
            self.assertEqual((out / "hydrology" / "1m" / "report.json").stat().st_mtime_ns, stamp, "finish not rerun")

    def test_a_failing_review_never_withholds_the_manifest(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as tmp:
            root = Path(tmp)
            src = fake_world(root)
            # 4 m water whose grid.json is corrupt (no size): the coarse guide fails.
            g4 = root / "chain" / "hydrology" / "4m" / "grid.json"
            g4.write_text(json.dumps({k: v for k, v in json.loads(g4.read_text()).items() if k != "size"}))
            out = root / "v3"
            with mock.patch.object(W.review, "crops", side_effect=RuntimeError("boom")):
                code = W.run_world(src, out, tile=64, threads=2)
            report = json.loads((out / "hydrology" / "1m" / "report.json").read_text())
            self.assertEqual(code, 1, "a coarse guide that cannot run fails the world (B15)")
            m = json.loads((out / "manifest-failed.json").read_text())
            self.assertIn("KeyError", m["water"]["coarse_guide"]["error"])
            self.assertIn("coarse_guide", m["water"]["failures"])
            self.assertEqual(m["water"]["review"], {"error": "RuntimeError: boom"})
            self.assertTrue(set(report["failures"]) <= set(m["water"]["failures"]), "the water's own failures kept")
            win = root / "win"
            with mock.patch.object(W.review, "crops", side_effect=RuntimeError("boom")):
                self.assertIn(W.run_window(src, 1, (128.0, 128.0), 128, win, 64, 2), (0, 1))
            step = json.loads((win / "progress.json").read_text())["steps"]["review"]
            self.assertEqual(step["review"], {"error": "RuntimeError: boom"})

    def test_a_world_without_real_moisture_is_refused(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as tmp:
            root = Path(tmp)
            src = fake_world(root)
            (root / "chain" / "hydrology" / "4m" / "moisture.f32").unlink()
            self.assertEqual(W.run_world(src, root / "v3", tile=64, threads=2), 2)
            self.assertFalse((root / "v3" / "hydrology" / "work").exists())

    def test_an_out_inside_a_source_is_refused(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as tmp:
            root = Path(tmp)
            src = fake_world(root)
            for out in (src / "v3", root / "chain" / "v3", src):
                self.assertEqual(W.run_world(src, out, tile=64, threads=2), 2, out)
                self.assertEqual(W.run_window(src, 1, (128.0, 128.0), 64, out, 64, 2), 2, out)
            self.assertFalse((src / "v3").exists())
            self.assertFalse((root / "chain" / "v3").exists())

    def test_an_out_inside_l4v2_is_refused(self):
        l4v2 = Path(__file__).resolve().parents[2] / "data" / "worlds" / "fujix-55-l4v2"
        if not (l4v2 / "manifest.json").exists():
            self.skipTest("no l4v2 world")
        out = l4v2 / "hydrology" / "v3-test"
        self.assertEqual(W.run_world(l4v2, out, tile=2048, threads=2), 2)
        self.assertEqual(W.run_window(l4v2, 1, (49249.0, 16381.0), 64, out, 64, 2), 2)
        self.assertFalse(out.exists())

    def test_another_existing_world_is_refused(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as tmp:
            root = Path(tmp)
            src = fake_world(root)
            other = fake_world(root / "other")
            before = sorted(p.name for p in other.iterdir())
            self.assertEqual(W.run_world(src, other, tile=64, threads=2), 2)
            self.assertEqual(W.run_window(src, 1, (128.0, 128.0), 64, other, 64, 2), 2)
            self.assertEqual(sorted(p.name for p in other.iterdir()), before)
            # A fresh run directory holding only its launcher's log is accepted.
            fresh = root / "fresh"
            fresh.mkdir()
            (fresh / "run.log").write_text("")
            self.assertIn(W.run_window(src, 1, (128.0, 128.0), 64, fresh, 64, 2), (0, 1))

    def test_an_unexpected_error_exits_2(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as tmp:
            self.assertEqual(W.main(["world", "--source", str(Path(tmp) / "none"), "--out", str(Path(tmp) / "o")]), 2)

    def test_a_changed_window_reruns(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as tmp:
            root = Path(tmp)
            src = fake_world(root)
            out = root / "win"
            self.assertIn(W.run_window(src, 1, (128.0, 128.0), 128, out, 64, 2), (0, 1))
            first = json.loads((out / "progress.json").read_text())
            self.assertIn(W.run_window(src, 1, (96.0, 160.0), 128, out, 64, 2), (0, 1))
            second = json.loads((out / "progress.json").read_text())
            self.assertNotEqual(first["grid"], second["grid"])
            published = json.loads((out / "out" / "grid.json").read_text())
            self.assertEqual(published["offset_m"], [32.0, 32.0], "finish ran on the new window")
            self.assertTrue((out / "review" / "overview.png").exists())

    def test_a_window_grid_is_cut_from_the_raw_surface(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as tmp:
            src = fake_world(Path(tmp))
            grid = W.build_window_grid(src, 1, (128.0, 128.0), 64, Path(tmp) / "g")
            g = json.loads((grid / "grid.json").read_text())
            self.assertEqual(g["size"], [64, 64])
            self.assertEqual(g["offset_m"], [96.0, 96.0])
            z = np.fromfile(grid / "elevation.f32", "<f4").reshape(64, 64)
            raw = np.fromfile(src / "elevation-raw.f32", "<f4").reshape(256, 256)
            self.assertTrue(np.array_equal(z, raw[96:160, 96:160]))
            self.assertEqual(np.fromfile(grid / "classes.u8", np.uint8).size, 16 * 16)
            self.assertTrue((np.fromfile(grid / "provenance.u8", np.uint8) == 2).all())
            # Rebuilding the same window leaves its files alone (the Rust stamp reads mtimes).
            mtime = (grid / "elevation.f32").stat().st_mtime_ns
            W.build_window_grid(src, 1, (128.0, 128.0), 64, Path(tmp) / "g")
            self.assertEqual((grid / "elevation.f32").stat().st_mtime_ns, mtime)


class SquareTest(unittest.TestCase):
    """An L4 square (256 m) at 4 m cells (32, 48) of a 128² chain: grids take its part of the
    chain's classes and provenance."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(dir=SCRATCH)
        root = Path(self.tmp.name)
        self.src = fake_world(root, 256, n4=128, window_4m=[32, 48])
        self.c4 = np.fromfile(root / "chain" / "classes.u8", np.uint8).reshape(128, 128)
        self.p32 = np.fromfile(root / "chain" / "provenance.u8", np.uint8).reshape(16, 16)
        self.out = root / "v3"

    def tearDown(self):
        self.tmp.cleanup()

    def test_the_world_grid_crops_classes_and_provenance(self):
        grid = W.build_world_grid(self.src, self.out)
        g = json.loads((grid / "grid.json").read_text())
        self.assertEqual((g["classes_cell_m"], g["provenance_cell_m"]), (4.0, 4.0))
        cls = np.fromfile(grid / "classes.u8", np.uint8).reshape(64, 64)
        np.testing.assert_array_equal(cls, self.c4[48:112, 32:96])
        i = np.arange(64)
        want = self.p32[np.ix_((192 + 4 * i) // 32, (128 + 4 * i) // 32)]
        np.testing.assert_array_equal(np.fromfile(grid / "provenance.u8", np.uint8).reshape(64, 64), want)

    def test_a_1m_window_takes_the_square_offset(self):
        # Square origin E128 N320; window cells col 68, row 108 of the square.
        grid = W.build_window_grid(self.src, 1, (228.0, 180.0), 64, self.out / "win")
        g = json.loads((grid / "grid.json").read_text())
        self.assertEqual(g["offset_m"], [68.0, 108.0])
        cls = np.fromfile(grid / "classes.u8", np.uint8).reshape(16, 16)
        np.testing.assert_array_equal(cls, self.c4[48 + 27 : 48 + 43, 32 + 17 : 32 + 33])
        i = np.arange(64)
        want = self.p32[np.ix_((192 + 108 + i) // 32, (128 + 68 + i) // 32)]
        np.testing.assert_array_equal(np.fromfile(grid / "provenance.u8", np.uint8).reshape(64, 64), want)


if __name__ == "__main__":
    unittest.main()
