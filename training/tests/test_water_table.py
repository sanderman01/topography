import unittest

from topography_training.water import table


class WaterTableTest(unittest.TestCase):
    def test_reads_r_hot_dry_and_levels(self):
        self.assertEqual(table.r(19), 0.05)
        self.assertEqual(table.r(200), table.r(0))
        self.assertEqual(sorted(table.hot_dry_classes()), [2, 7, 8, 13, 16, 19])
        self.assertEqual(table.lake_min_m2(), 2000.0)
        self.assertEqual(table.below_sea_coast_km(), 5.0)
        self.assertEqual(table.level(4.0)["wall_deg"], 35.0)
        self.assertEqual(table.level(32.0)["erosion"]["routings"], 5)
        self.assertNotIn("erosion", table.level(1.0))
        self.assertEqual(table.level(2.0)["river_km2"], 0.05, "the largest level not coarser")


if __name__ == "__main__":
    unittest.main()
