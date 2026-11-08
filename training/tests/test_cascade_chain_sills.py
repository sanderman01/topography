import json
import tempfile
import unittest
from pathlib import Path

import numpy as np

from topography_training.diffusion import cascade, cascade_chain

N32 = 64  # 32 m cells
EXIT = (40, 32)  # (x, y) of the lake's exit sill cell


def _ground32() -> np.ndarray:
    """A bowl at (32, 32) spilling east over a saddle at EXIT, then falling away."""
    y, x = np.mgrid[:N32, :N32].astype(np.float32)
    h = 100.0 + 2.0 * np.hypot(x - 32, y - 32)
    h[32, 33:] = np.minimum(h[32, 33:], 116.0 - 1.0 * (np.arange(33, N32) - 40).clip(0))
    return h.astype(np.float32)


def _world(tmp: Path, residual: float) -> Path:
    """A finished L3 world: the 32 m output and an l3-raw.f32 that is the refined 32 m ground
    plus `residual` metres on the exit sill (L3 raised or cut it)."""
    out = tmp / "w"
    o32 = out / "hydrology" / "32m"
    o32.mkdir(parents=True)
    h = _ground32()
    h.tofile(o32 / "elevation.f32")
    np.full_like(h, np.nan).tofile(o32 / "water-surface.f32")
    np.zeros(h.shape, "<u4").tofile(o32 / "water-body.u32")
    bodies = [{"id": 1, "kind": "lake", "endorheic": False, "exit_cell": list(EXIT), "level_m": 116.0},
              {"id": 2, "kind": "lake", "endorheic": True, "exit_cell": None, "level_m": 90.0}]
    (o32 / "bodies.json").write_text(json.dumps({"bodies": bodies}))
    (o32 / "grid.json").write_text(json.dumps({"size": [N32, N32], "spacing_m": 32.0}))
    coarse = cascade.refine_surface(cascade.LEVELS["l3"], h)
    raw = coarse.copy()
    x, y = EXIT
    raw[y * 8 - 8 : y * 8 + 16, x * 8 - 8 : x * 8 + 16] += residual
    raw.astype("<f4").tofile(out / "l3-raw.f32")
    params = {"l3_km": None, "l3_centre": None, "l3_band_m": 8192, "l3_halo_m": 1024}
    (out / "progress.json").write_text(json.dumps({"params": params, "l3_bands_done": 1}))
    return out


class SillEaseTest(unittest.TestCase):
    def test_exit_cells_are_full_lakes_exits_only(self):
        bodies = [{"kind": "lake", "exit_cell": [3, 1]}, {"kind": "lake", "exit_cell": None},
                  {"kind": "river", "exit_cell": None}]
        m = cascade_chain._exit_cells(bodies, (4, 5))
        self.assertEqual(m.sum(), 1)
        self.assertTrue(m[1, 3])

    def test_a_raised_or_cut_sill_is_eased_to_the_32_m_cell_both_ways(self):
        for residual in (10.0, -5.5):
            with tempfile.TemporaryDirectory() as tmp:
                out = _world(Path(tmp), residual)
                stats = cascade_chain.reease_sills(out)
                raw = np.fromfile(out / "l3-raw.f32", "<f4").reshape(N32 * 8, N32 * 8)
                coarse = cascade.refine_surface(cascade.LEVELS["l3"], _ground32())
                x, y = EXIT
                sill = np.s_[y * 8 : y * 8 + 8, x * 8 : x * 8 + 8]
                # The whole exit cell takes the 32 m ground it spilled over.
                self.assertLess(np.abs(raw[sill] - _ground32()[y, x]).max(), 1e-3, residual)
                # Far from the sill nothing moves.
                self.assertEqual(np.abs(raw[:128] - coarse[:128]).max(), 0.0)
                self.assertGreater(stats["cells_changed"], 64)
                # A second ease would ease again: refused.
                with self.assertRaises(SystemExit):
                    cascade_chain.reease_sills(out)

    def test_a_notch_the_refined_surface_fills_is_not_the_target(self):
        """Lake 6: a narrow 32 m notch between high cells; smoothing fills it, the ease must not."""
        h = _ground32()
        x, y = EXIT
        h[y - 1 : y + 2, x - 1 : x + 2] += 60.0
        h[y, x - 1 : x + 2] -= 60.0  # the notch through the sill row
        coarse = cascade.refine_surface(cascade.LEVELS["l3"], h)
        target, s = cascade_chain._sill_fields(h, cascade_chain._exit_cells(
            [{"kind": "lake", "exit_cell": list(EXIT)}], h.shape))
        sill = np.s_[y * 8 : y * 8 + 8, x * 8 : x * 8 + 8]
        self.assertGreater(coarse[sill].mean() - h[y, x], 5.0, "the refined surface fills the notch")
        eased = cascade_chain._ease_sills(coarse + 3.0, target, s)
        self.assertLess(np.abs(eased[sill] - h[y, x]).max(), 1e-3)
        self.assertEqual(eased.dtype, np.float32)
        # Continuous: no step between neighbouring 4 m cells beyond the surface's own.
        self.assertLess(np.abs(np.diff(eased, axis=1)).max(), np.abs(np.diff(coarse, axis=1)).max() + 15.0)


if __name__ == "__main__":
    unittest.main()
