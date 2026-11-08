"""Owner rule (2026-11-08): the diffusion levels keep river terrain by default."""

import unittest

from topography_training.diffusion import cascade_chain, cascade_l4


class WaterDefaultsTest(unittest.TestCase):
    def test_l3_clamps_lakes_and_sea_only_unless_asked(self):
        base = ["--collage", "c", "--out", "o", "--l3-run", "r"]
        self.assertFalse(cascade_chain.parser().parse_args(base).l3_river_clamp)
        self.assertTrue(cascade_chain.parser().parse_args(base + ["--l3-river-clamp"]).l3_river_clamp)
        self.assertFalse(cascade_chain.parser().parse_args(base + ["--no-l3-river-clamp"]).l3_river_clamp)

    def test_l4_keeps_the_river_residual_unless_asked(self):
        base = ["--run", "r", "--world", "w", "--out", "x"]
        self.assertTrue(cascade_l4.parser().parse_args(base).keep_river_residual)
        self.assertFalse(cascade_l4.parser().parse_args(base + ["--zero-river-residual"]).keep_river_residual)
        self.assertTrue(cascade_l4.parser().parse_args(base + ["--keep-river-residual"]).keep_river_residual)


if __name__ == "__main__":
    unittest.main()
