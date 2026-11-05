"""Bank relaxation: lowering only, slope limits, determinism and tile overlaps."""

import math
import unittest

import numpy as np

from topography_training.water import relax as R

TAN = math.tan(math.radians(R.LIMIT_DEG))


def channel_case(w=60, h=40):
    """Flat banks at 10 m, a 4-cell channel cut to 7.9 m under water at 8 m, corridor around."""
    ground = np.full((h, w), 10.0, np.float32)
    raw = ground.copy()
    surface = np.full((h, w), np.nan, np.float32)
    corridor = np.zeros((h, w), np.uint8)
    ground[:, 28:32] = 7.9
    surface[:, 28:32] = 8.0
    corridor[:, 20:40] = 1
    corridor[:, 28:32] = 2
    flow = np.zeros((h, w), np.uint8)
    flow[:, 20:28] = 4  # banks drain east, toward the channel
    flow[:, 32:40] = 3  # and west
    return ground, raw, surface, corridor, flow


def horn_deg(z):
    """Horn slope in degrees of the inner cells (as the Rust `walls` check computes it)."""
    z = z.astype(np.float64)
    gx = (z[:-2, 2:] + 2 * z[1:-1, 2:] + z[2:, 2:] - z[:-2, :-2] - 2 * z[1:-1, :-2] - z[2:, :-2]) / 8
    gy = (z[2:, :-2] + 2 * z[2:, 1:-1] + z[2:, 2:] - z[:-2, :-2] - 2 * z[:-2, 1:-1] - z[:-2, 2:]) / 8
    return np.degrees(np.arctan(np.hypot(gx, gy)))


class RelaxTest(unittest.TestCase):
    def test_banks_are_cut_back_to_the_limit_and_never_raised(self):
        g, raw, s, c, f = channel_case()
        out = R.relax(g, raw, s, c, f)
        self.assertTrue((out <= g + 1e-6).all(), "raised")
        self.assertTrue((out[c == 0] == g[c == 0]).all(), "moved outside the corridor")
        self.assertTrue((out[c == 2] == g[c == 2]).all(), "moved a channel bed")
        self.assertTrue((out >= raw - R.MAX_LOWER_M - 1e-5).all())
        self.assertTrue((out[:, 27] >= 8.0 - 1e-5).all(), "a bank below the water beside it")
        # The wall above the channel is no steeper than the limit.
        self.assertTrue(((out[:, 27] - out[:, 28]) <= TAN + 1e-3).all(), out[0, 20:30])
        # Every lowered bank cell stays at least 1 mm above its receiver (I7).
        moved = (out < g) & (c == 1)
        east = np.roll(out, -1, axis=1)
        west = np.roll(out, 1, axis=1)
        recv = np.where(f == 4, east, np.where(f == 3, west, -np.inf))
        self.assertTrue((out[moved] >= recv[moved] + R.RECEIVER_M - 1e-6).all())

    def test_a_raw_cliff_is_never_steepened(self):
        g, raw, s, c, f = channel_case()
        cliff = np.where(np.arange(60) < 24, 14.0, 10.0).astype(np.float32)[None, :].repeat(40, 0)
        g, raw = np.minimum(g, cliff) + (cliff - 10.0), raw + (cliff - 10.0)
        out = R.relax(g, raw, s, c, f)
        raw_step = raw[:, 23] - raw[:, 24]
        self.assertTrue(((out[:, 23] - out[:, 24]) <= raw_step + 1e-4).all())

    def test_a_changed_bed_is_not_steepened_into_a_wall(self):
        # A channel along x with a fixed high north bank (3.4 m) and a movable south bank (2 m):
        # cutting the south bank down to the water would tilt the bed past 40 degrees (walls).
        h, w = 30, 40
        raw = np.full((h, w), 2.0, np.float32)
        raw[:14] = 3.4
        g = raw.copy()
        g[14] = 1.0  # carved bed, 1 m below raw
        s = np.full((h, w), np.nan, np.float32)
        s[14] = 1.1
        c = np.zeros((h, w), np.uint8)
        c[14] = 2
        c[15:23] = 1
        f = np.zeros((h, w), np.uint8)
        f[15:23] = 1  # the south bank drains north, into the channel
        out = R.relax(g, raw, s, c, f)
        after, before, rw = horn_deg(out), horn_deg(g), horn_deg(raw)
        changed = np.abs(out - raw)[1:-1, 1:-1] > 0.01
        allowed = np.maximum(np.maximum(40.0, rw), before) + 0.01
        bad = changed & (after > allowed)
        self.assertFalse(bad.any(), (after[13, 5:10], allowed[13, 5:10]))

    def test_steep_noisy_banks_pass_walls(self):
        # Banks rising at 35 degrees with 0.6 m of noise beside a 0.5 m cut: the per-neighbour
        # cone alone leaves 40 changed cells steeper than max(40, raw); the repair puts them back.
        rng = np.random.default_rng(7)
        g, raw, s, c, f = channel_case()
        rise = np.tan(np.radians(35.0)) * np.clip(np.abs(np.arange(60) - 29.5) - 2.0, 0, None)
        raw = (raw + rise[None, :] + rng.random(raw.shape) * 0.6).astype(np.float32)
        g = raw.copy()
        g[:, 28:32] -= 0.5
        s[:, 28:32] = g[:, 28:32] + 0.1
        out = R.relax(g, raw, s, c, f)
        self.assertGreater(int((out < g - 0.01).sum()), 400, "banks moved")
        after, before, rw = horn_deg(out), horn_deg(g), horn_deg(raw)
        changed = np.abs(out - raw)[1:-1, 1:-1] > 0.01
        allowed = np.maximum(np.maximum(40.0, rw), before) + 0.01  # beds as carved may be steeper
        self.assertFalse((changed & (after > allowed)).any())

    def test_the_1m_detail_survives_so_no_facets_form(self):
        # Noisy banks on a 10 degree slope beside a 0.5 m cut: smoothing the ground itself
        # would plane them off (281 planar windows against raw's 7) and fail the Rust `facets`
        # check (planar 5 x 5 windows on changed cells, at most raw's count).
        rng = np.random.default_rng(5)
        g, raw, s, c, f = channel_case()
        tilt = np.tan(np.radians(10.0)) * np.abs(np.arange(60) - 29.5)[None, :]
        raw = (raw + tilt + rng.random(raw.shape) * 0.1).astype(np.float32)
        g = raw.copy()
        g[:, 28:32] -= 0.5
        s[:, 28:32] = g[:, 28:32] + 0.1
        out = R.relax(g, raw, s, c, f)
        self.assertTrue((out < g - 0.01).any(), "banks moved")

        def planar(z):
            win = np.lib.stride_tricks.sliding_window_view(z.astype(np.float64), (5, 5))
            yy, xx = np.mgrid[-2:3, -2:3]
            a = (win * xx).sum((-2, -1)) / 50
            b = (win * yy).sum((-2, -1)) / 50
            res = win - win.mean((-2, -1), keepdims=True) - a[..., None, None] * xx - b[..., None, None] * yy
            rms = np.sqrt((res**2).mean((-2, -1)))
            return (rms < 0.02) & (np.degrees(np.arctan(np.hypot(a, b))) > 5.0)

        changed = (np.abs(out - raw) > 0.01)[2:-2, 2:-2]
        self.assertLessEqual(int((planar(out) & changed).sum()), int((planar(raw) & changed).sum()))

    def test_runs_are_deterministic_and_tiles_agree(self):
        rng = np.random.default_rng(3)
        h, w = 50, 30 + R.HALO + 100
        raw = (10.0 + rng.random((h, w), np.float32) * 0.3).astype(np.float32)
        g = raw.copy()
        s = np.full((h, w), np.nan, np.float32)
        c = np.ones((h, w), np.uint8)
        c[:2] = 0
        c[-2:] = 0
        f = np.zeros((h, w), np.uint8)
        # A channel 0.5 m deep along the window: its noisy banks move along its whole length,
        # right up to the window cut at 30 + HALO, so a halo too small changes the core (N9).
        # (A channel across the window would shield the core: its cells never move.)
        g[24:26] -= 0.5
        s[24:26] = g[24:26] + 0.1
        c[24:26] = 2
        whole = R.relax(g, raw, s, c, f)
        self.assertTrue(np.array_equal(whole, R.relax(g, raw, s, c, f)))
        cut = 30 + R.HALO
        self.assertTrue((whole[:, cut - 10 : cut] < g[:, cut - 10 : cut]).any(), "cells move near the cut")
        core = slice(0, 30)
        win = slice(0, cut)
        part = R.relax(g[:, win], raw[:, win], s[:, win], c[:, win], f[:, win])
        self.assertTrue(np.array_equal(part[:, core], whole[:, core]), "core differs")
        small = R.relax(*[a[:, :38] for a in (g, raw, s, c, f)])
        self.assertFalse(np.array_equal(small[:, core], whole[:, core]), "an 8-cell halo should not do")
        strip = slice(30, 30 + R.STRIP)
        self.assertLessEqual(float(np.abs(part[:, strip] - whole[:, strip]).max()), 1e-3)


if __name__ == "__main__":
    unittest.main()
