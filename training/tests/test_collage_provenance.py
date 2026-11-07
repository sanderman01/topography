import unittest

import numpy as np

from topography_training.diffusion import collage as c


def _surface(n=96):
    """Two regions split at column 48 with a 20-cell blend; one piece covers region 0's west
    60 columns (feathered over its last 8), nothing covers region 1 (fill); a painted dip at
    (70, 20); sea outside rows >= 88; a 45° wall at columns 10-11."""
    x = np.arange(n, dtype=np.float32)[None, :].repeat(n, 0)
    w0 = np.clip((58 - x) / 20, 0, 1)
    w1 = 1 - w0
    piece = np.clip((60 - x) / 8, 0, 1)
    h = np.full((n, n), 100.0, np.float32)
    h[:, :10] = 100.0 + 32.0 * 2
    h[:, 10] = 100.0 + 32.0
    blended = h.copy()
    h[15:25, 65:75] -= 30.0  # painted dip
    region32 = np.where(x < 48, 1, 2).astype(np.uint8)
    region32[88:] = 0
    h[88:] = -50.0
    blended[88:] = -50.0
    cls = np.full((n, n), 18, np.uint8)
    cls[:, 80:] = 8  # dunes in the east
    z = np.zeros((n, n), np.float32)
    return c.Surface(h32=h, blended=blended, cls32=cls, region32=region32, forest32=z, ice=z > 0,
                     dips={}, pre=h, cal=None, sds=[], land_w=[w0, w1],
                     covered=[piece > 1e-3, np.zeros((n, n), bool)],
                     wmax=[piece, np.zeros((n, n), np.float32)],
                     npieces=[(piece >= c.PIECE_MIN_W).astype(np.uint8), np.zeros((n, n), np.uint8)])


class ProvenanceTest(unittest.TestCase):
    def test_kinds_and_flags(self):
        p = c.provenance(_surface(), hot_dry=[8], coast_km=1.0)
        kind = p & c.KIND_MASK
        self.assertEqual(kind[50, 2], c.INTERIOR, "past the feather and 1 km from any blend")
        self.assertEqual(kind[50, 20], c.BLEND, "within 1 km (32 cells) of the region blend at x >= 39")
        self.assertEqual(kind[20, 70], c.PAINTED)
        self.assertEqual(kind[50, 85], c.FILL, "no piece covers region 1")
        self.assertEqual(kind[90, 50], c.SEA_P)
        self.assertTrue(p[50, 10] & c.STEEP, "the 45° wall is steep source")
        self.assertFalse(p[50, 40] & c.STEEP)
        self.assertTrue(p[30, 90] & c.BELOW_SEA_OK, "dunes over 1 km from the sea")
        self.assertFalse(p[86, 90] & c.BELOW_SEA_OK, "dunes beside the sea")
        self.assertFalse(p[30, 60] & c.BELOW_SEA_OK, "plain is not hot-dry")

    def test_a_wide_single_piece_has_an_interior(self):
        s = _surface(n=192)
        n = 192
        x = np.arange(n, dtype=np.float32)[None, :].repeat(n, 0)
        one = np.ones((n, n), np.float32)
        s.land_w = [one, np.zeros((n, n), np.float32)]
        s.wmax = [one, np.zeros((n, n), np.float32)]
        s.npieces = [np.ones((n, n), np.uint8), np.zeros((n, n), np.uint8)]
        s.covered = [np.ones((n, n), bool), np.zeros((n, n), bool)]
        s.region32 = np.ones((n, n), np.uint8)
        s.h32 = np.full((n, n), 100.0, np.float32)
        s.blended = s.h32.copy()
        s.cls32 = np.full((n, n), 18, np.uint8)
        kind = c.provenance(s, hot_dry=[], coast_km=5.0) & c.KIND_MASK
        self.assertEqual(kind[96, 96], c.INTERIOR)
        self.assertTrue((kind == c.INTERIOR).all(), "no other kind anywhere: all interior")
        del x

    def test_slope_deg_is_horn(self):
        h = (np.arange(8, dtype=np.float32)[None, :] * 32.0).repeat(8, 0)
        self.assertAlmostEqual(float(c.slope_deg(h, 32.0)[4, 4]), 45.0, places=4)


if __name__ == "__main__":
    unittest.main()
