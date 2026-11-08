"""Ease a finished chain world's kept l3-raw.f32 at the 32 m lakes' outlet passes (CPU only).

`cascade_chain` runs this ease itself once L3 is done. This entry point is for a world whose L3
ran without it (`progress.json` without `l3_sills_eased`). The ease is not affine and runs only
once: back up l3-raw.f32 first, then rerun the world's 4 m level (terrain-cascade.md).
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
