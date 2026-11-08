import json
import tempfile
import unittest
from pathlib import Path

import numpy as np

from topography_training.diffusion import cascade, cascade_chain

N32 = 64  # 32 m cells
EXIT = (40, 32)  # (x, y) of the lake's exit sill cell; the lake is west, the receiver east


def _ground32() -> np.ndarray:
    """A bowl at (32, 32) spilling east over a saddle at EXIT (116 m), then falling away."""
    y, x = np.mgrid[:N32, :N32].astype(np.float32)
    h = 100.0 + 2.0 * np.hypot(x - 32, y - 32)
    h[32, 33:] = np.minimum(h[32, 33:], 116.0 - 1.0 * (np.arange(33, N32) - 40).clip(0))
    return h.astype(np.float32)


def _world(tmp: Path, raw: np.ndarray) -> Path:
    """A finished L3 world with the 32 m output of _ground32 (lake 1 west of EXIT, which drains
    east) and the given l3-raw.f32."""
    out = tmp / "w"
    o32 = out / "hydrology" / "32m"
    o32.mkdir(parents=True)
    h = _ground32()
    h.tofile(o32 / "elevation.f32")
    lake = h < 116.0
    np.where(lake, 116.0, np.nan).astype("<f4").tofile(o32 / "water-surface.f32")
    np.where(lake, 1, 0).astype("<u4").tofile(o32 / "water-body.u32")
    flow = np.zeros(h.shape, np.uint8)
    flow[EXIT[1], EXIT[0]] = 4  # east
    flow.tofile(o32 / "flow.u8")
    bodies = [{"id": 1, "kind": "lake", "endorheic": False, "exit_cell": list(EXIT), "level_m": 116.0},
              {"id": 2, "kind": "lake", "endorheic": True, "exit_cell": None, "level_m": 90.0}]
    (o32 / "bodies.json").write_text(json.dumps({"bodies": bodies}))
    (o32 / "grid.json").write_text(json.dumps({"size": [N32, N32], "spacing_m": 32.0}))
    raw.astype("<f4").tofile(out / "l3-raw.f32")
    params = {"l3_km": None, "l3_centre": None, "l3_band_m": 8192, "l3_halo_m": 1024}
    (out / "progress.json").write_text(json.dumps({"params": params, "l3_bands_done": 1}))
    return out


def _coarse() -> np.ndarray:
    return cascade.refine_surface(cascade.LEVELS["l3"], _ground32())


def _pass(h: np.ndarray) -> float:
    x, y = EXIT
    return cascade_chain._corridor_pass(h, ((x - 1) * 8, y * 8), (x * 8, y * 8), ((x + 1) * 8, y * 8))


def _ease(raw: np.ndarray) -> tuple[np.ndarray, dict, Path]:
    tmp = Path(tempfile.mkdtemp())
    out = _world(tmp, raw)
    stats = cascade_chain.reease_sills(out)
    return np.fromfile(out / "l3-raw.f32", "<f4").reshape(raw.shape), stats, out


def _near(raw: np.ndarray, dz: float) -> np.ndarray:
    """raw with dz added smoothly around the exit (L3 raised or lowered the sill area)."""
    x, y = EXIT
    yy, xx = np.mgrid[: raw.shape[0], : raw.shape[1]]
    w = np.exp(-(((xx - x * 8 - 4) / 40.0) ** 2 + ((yy - y * 8 - 4) / 40.0) ** 2))
    return (raw + dz * w).astype(np.float32)


class SillEaseTest(unittest.TestCase):
    def test_the_corridor_runs_from_the_lake_cell_through_the_exit_to_its_receiver(self):
        h = _ground32()
        body = np.where(h < 116.0, 1, 0)
        flow = np.zeros(h.shape, np.uint8)
        flow[EXIT[1], EXIT[0]] = 4
        sills = cascade_chain._sills([{"id": 1, "kind": "lake", "exit_cell": list(EXIT)},
                                      {"id": 2, "kind": "lake", "exit_cell": None}], h, body, flow, body > 0)
        self.assertEqual(len(sills), 1)
        self.assertEqual((sills[0].lake, sills[0].exit, sills[0].recv), ((39, 32), EXIT, (41, 32)))
        self.assertEqual(sills[0].v, 116.0)
        # A river through the lake carries its own id: the wet cell draining into the exit is the lake side.
        body = np.where(body == 1, 9, 0)
        flow[31, 39] = 8  # SE into the exit
        sills = cascade_chain._sills([{"id": 1, "kind": "lake", "exit_cell": list(EXIT)}], h, body, flow, body > 0)
        self.assertEqual(sills[0].lake, (39, 31))

    def test_a_raised_sill_is_cut_to_the_32_m_spill_keeping_its_shape(self):
        raw = _near(_coarse(), 10.0)
        eased, stats, out = _ease(raw)
        self.assertAlmostEqual(_pass(eased), 116.0, delta=0.01)
        x, y = EXIT
        core = (eased - raw)[y * 8 : y * 8 + 8, x * 8 : x * 8 + 8]
        self.assertLess(np.ptp(core), 1e-3, "one offset over the exit cell: L3's shape kept")
        self.assertLess(core.mean(), -5.0)
        self.assertEqual(stats["eased"], 1)
        self.assertEqual(stats["unconverged"], 0)
        with self.assertRaises(SystemExit):  # not affine: never twice
            cascade_chain.reease_sills(out)

    def test_a_notch_cut_below_the_spill_gets_a_bump_and_stays_a_notch(self):
        raw = _near(_coarse(), 10.0)
        x, y = EXIT
        notch = np.s_[y * 8 + 3 : y * 8 + 5, (x - 1) * 8 : (x + 2) * 8]
        raw[notch] = 110.0  # L3 cut a channel 6 m below the 32 m spill
        self.assertAlmostEqual(_pass(raw), 110.0, delta=0.01)
        eased, stats, _ = _ease(raw)
        self.assertAlmostEqual(_pass(eased), 116.0, delta=0.01)
        cell = np.s_[y * 8 : y * 8 + 8, x * 8 : x * 8 + 8]
        # The banks keep their height above the channel: the notch stays a notch.
        self.assertLess(np.ptp((eased - raw)[cell]), 1e-3)
        self.assertGreater((eased[cell].max() - eased[notch].max()), 5.0)
        self.assertGreater(stats["offset_m"]["max"], 5.0)
        self.assertEqual(stats["offsets_over_5m"], 1)

    def test_the_bump_is_smooth(self):
        raw = _near(_coarse(), 10.0)
        eased, stats, _ = _ease(raw)
        d = eased - raw
        off = abs(float(d[EXIT[1] * 8 + 4, EXIT[0] * 8 + 4]))
        steps = max(np.abs(np.diff(d, axis=0)).max(), np.abs(np.diff(d, axis=1)).max())
        self.assertLess(steps, 0.45 * off)
        self.assertEqual(np.abs(d[:128]).max(), 0.0, "far from the sill nothing moves")

    def test_neighbouring_sills_do_not_add_up(self):
        off = np.full((12, 12), np.nan, np.float32)
        off[6, 5] = off[6, 6] = 5.0
        bump = cascade_chain._bump(off)
        self.assertLessEqual(bump.max(), 5.0 + 1e-4)
        self.assertAlmostEqual(float(bump[6 * 8 + 4, 5 * 8 + 4]), 5.0, delta=1e-3)


if __name__ == "__main__":
    unittest.main()
