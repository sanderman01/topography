import json
import tempfile
import unittest
from pathlib import Path

import numpy as np

from topography_training.diffusion import cascade_l4


class RawWorldTest(unittest.TestCase):
    def test_band_hydrology_flags_are_gone(self):
        import contextlib
        import io

        for flag in ("--hydrology-only", "--seam-audit", "--drop-raw", "--restart"):
            err = io.StringIO()
            with contextlib.redirect_stderr(err), self.assertRaises(SystemExit) as e:
                cascade_l4.main(["--run", "r", "--world", "w", "--out", "x", flag])
            self.assertEqual(e.exception.code, 2)
            self.assertIn("unrecognized arguments", err.getvalue(), flag)

    def test_the_raw_manifest_names_its_layers_and_source(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            n = 64
            for name, dt in (("elevation-raw.f32", "<f4"), ("snow-ice.u8", np.uint8), ("vegetation.u8", np.uint8)):
                np.ones((n, n), dt).tofile(out / name)
            m = cascade_l4.raw_manifest(out, n, world_input=out / "world-input", source_world=out,
                                        params={"window_4m": [4, 8]}, extra={})
            self.assertEqual(m["kind"], "world_raw")
            self.assertEqual(m["params"]["window_4m"], [4, 8])
            self.assertEqual(set(m["layers"]), {"elevation-raw.f32", "snow-ice.u8", "vegetation.u8"})
            self.assertEqual(m["hydrology_input"]["file"], "elevation-raw.f32")
            self.assertEqual(m["hydrology_input"]["sha256"], m["layers"]["elevation-raw.f32"])
            json.dumps(m)


if __name__ == "__main__":
    unittest.main()
