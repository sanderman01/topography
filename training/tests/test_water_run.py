"""World driver end to end on a tiny fake L4 world (needs the release topography binary)."""

import json
import tempfile
import unittest
from pathlib import Path

import numpy as np

# The driver reflinks (`cp --reflink=always`, N12), which /tmp (tmpfs) cannot: test worlds
# live in the repository's scratch directory, on the btrfs /home.
SCRATCH = Path(__file__).resolve().parents[2] / "scratch" / "test-water"
SCRATCH.mkdir(parents=True, exist_ok=True)

from topography_training.water import run as W  # noqa: E402


def fake_world(root: Path, n: int = 256) -> Path:
    """An L4-shaped world over a chain world: elevation-raw.f32, the chain's 4 m classes and its
    4 m hydrology grid.json and moisture (real rain), snow/vegetation, world-input."""
    chain = root / "chain"
    (chain / "hydrology" / "4m").mkdir(parents=True)
    np.full((n // 4, n // 4), 18, np.uint8).tofile(chain / "classes.u8")
    moisture = {"file": "moisture.f32", "size": [2, 2], "spacing_m": n / 2, "nw_m": [0.0, float(n)],
                "scale_mm_h": 4.0, "dtype": "float32"}
    np.array([[1.0, 0.6], [1.4, 0.8]], "<f4").tofile(chain / "hydrology" / "4m" / "moisture.f32")
    (chain / "hydrology" / "4m" / "grid.json").write_text(json.dumps(
        {"moisture": moisture, "discharge": {"mean_land_mm_h": 2.0}}))
    world = root / "l4"
    (world / "world-input").mkdir(parents=True)
    y, x = np.mgrid[0:n, 0:n].astype(np.float32)
    z = 40.0 - 0.15 * x + 3.0 * np.sin(x / 9.0) * np.cos(y / 11.0) - 6.0 * np.exp(-((x - 80) ** 2 + (y - 80) ** 2) / 300.0)
    z.astype("<f4").tofile(world / "elevation-raw.f32")
    np.array([[1.0, 0.6], [1.4, 0.8]], "<f4").tofile(world / "moisture.f32")
    for name in ("snow-ice.u8", "vegetation.u8"):
        np.zeros((n, n), np.uint8).tofile(world / name)
    (world / "world-input" / "world.json").write_text(json.dumps({"name": "fake", "origin": [0.0, float(n)]}))
    manifest = {
        "schema_version": 1, "kind": "world_preview", "spacing_m": 1.0, "size": [n, n],
        "world_input": str(world / "world-input"), "source_world": str(chain), "class_id": "per cell",
        "moisture": moisture, "hydrology_input": {"file": "elevation-raw.f32"}, "layers": {},
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
            # Rebuilding the same window leaves its files alone (the Rust stamp reads mtimes).
            mtime = (grid / "elevation.f32").stat().st_mtime_ns
            W.build_window_grid(src, 1, (128.0, 128.0), 64, Path(tmp) / "g")
            self.assertEqual((grid / "elevation.f32").stat().st_mtime_ns, mtime)


if __name__ == "__main__":
    unittest.main()
