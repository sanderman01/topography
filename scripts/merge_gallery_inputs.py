#!/usr/bin/env python3
"""Merge published galleries and batch gallery inputs into one gallery input.

usage: merge_gallery_inputs.py <new-input.json> <gallery.json|gallery-input.json>...

Earlier inputs win on duplicate review paths, so existing library tiles keep their
IDs and pinned profiles. Build the result with `topography gallery build`.
"""

import json
import sys
from pathlib import Path

FIELDS = ("id", "title", "raw", "review", "primary_class_id", "preparation_profile")


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    output = Path(sys.argv[1])
    if output.exists():
        print(f"error: {output} exists", file=sys.stderr)
        return 1
    tiles, reviews, ids, titles = [], set(), set(), set()
    for name in sys.argv[2:]:
        for tile in json.loads(Path(name).read_text())["tiles"]:
            review = str(Path(tile["review"]).resolve())
            if review in reviews:
                continue
            entry = {k: tile[k] for k in FIELDS if tile.get(k) is not None}
            entry["review"], entry["raw"] = review, str(Path(tile["raw"]).resolve())
            base, n = entry["id"], 2
            while entry["id"] in ids:
                entry["id"], n = f"{base}-{n}", n + 1
            title, n = entry["title"], 2
            while entry["title"] in titles:
                entry["title"], n = f"{title} #{n}", n + 1
            titles.add(entry["title"])
            reviews.add(review)
            ids.add(entry["id"])
            tiles.append(entry)
    output.write_text(json.dumps({"schema_version": 1, "title": "Terrain library", "tiles": tiles}, indent=1))
    print(f"{len(tiles)} tiles -> {output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
