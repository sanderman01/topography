"""Ease a finished chain world's kept l3-raw.f32 at the 32 m lakes' exit sills (CPU only).

For worlds whose L3 ran before `cascade_chain` eased the sills itself: the ease is affine in
the raw surface, so applying it afterwards matches applying it per band. Back up l3-raw.f32
first; the world's 4 m level must then be rerun (terrain-cascade.md, Rerunning from a stage).
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from .cascade_chain import reease_sills


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--world", type=Path, required=True, help="cascade_chain output directory")
    args = p.parse_args(argv)
    print(json.dumps(reease_sills(args.world)), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
