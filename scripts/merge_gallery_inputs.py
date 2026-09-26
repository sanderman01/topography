#!/usr/bin/env python3
"""Merge published galleries and batch gallery inputs into one gallery input.

usage: merge_gallery_inputs.py [--drop-overlaps] [--drop-rejected] [--decided tiles.json] <new-input.json> <input>...

Inputs are published gallery.json files or batch gallery-input.json files. Earlier
inputs win on duplicate review paths, so library tiles keep their IDs and pinned
profiles. With --drop-overlaps, tiles of the first input and tiles with a saved
decision (from a gallery server's /api/tiles dump) are always kept; other tiles,
newest survey first, are dropped when they cover more than a quarter of a kept
tile's ground. With --drop-rejected, tiles the dump marks rejected are left out
(`topography curate purge-rejected` deletes their rasters). Build the result with `topography gallery build`.
"""

import argparse
import hashlib
import json
import sys
from pathlib import Path

FIELDS = ("id", "title", "raw", "review", "primary_class_id", "preparation_profile")
OVERLAP = 0.25


def footprint(review: Path):
    ring = json.loads((review.parent / "inspection.json").read_text())["wgs84Extent"]["coordinates"][0]
    xs, ys = [p[0] for p in ring], [p[1] for p in ring]
    return min(xs), min(ys), max(xs), max(ys)


def overlap(a, b) -> float:
    w = max(0.0, min(a[2], b[2]) - max(a[0], b[0]))
    h = max(0.0, min(a[3], b[3]) - max(a[1], b[1]))
    smaller = min((a[2] - a[0]) * (a[3] - a[1]), (b[2] - b[0]) * (b[3] - b[1]))
    return w * h / smaller if smaller > 0 else 0.0


def survey_end(raw: Path) -> str:
    receipt = json.loads(raw.read_text())
    source = (raw.parent.parent / "objects" / receipt["source_sha256"]).read_text()
    for line in source.splitlines():
        if line.startswith("acquisition_dates"):
            return line.split("/")[-1]
    return ""


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--drop-overlaps", action="store_true")
    parser.add_argument("--drop-rejected", action="store_true")
    parser.add_argument("--decided", type=Path, help="/api/tiles dump; decided tiles are kept")
    parser.add_argument("output", type=Path)
    parser.add_argument("inputs", nargs="+", type=Path)
    args = parser.parse_args()
    if args.output.exists():
        print(f"error: {args.output} exists", file=sys.stderr)
        return 1
    decided, rejected = set(), set()
    if args.decided:
        dump = json.loads(args.decided.read_text())["tiles"]
        decided = {t["review_sha256"] for t in dump if t["decision"] != "pending"}
        rejected = {t["review_sha256"] for t in dump if t["decision"] == "reject"}
    if args.drop_rejected and not args.decided:
        print("error: --drop-rejected needs --decided", file=sys.stderr)
        return 1
    candidates, reviews = [], set()
    for position, name in enumerate(args.inputs):
        for tile in json.loads(name.read_text())["tiles"]:
            review = Path(tile["review"]).resolve()
            if str(review) in reviews:
                continue
            reviews.add(str(review))
            entry = {k: tile[k] for k in FIELDS if tile.get(k) is not None}
            entry["review"], entry["raw"] = str(review), str(Path(tile["raw"]).resolve())
            sha = hashlib.sha256(review.read_bytes()).hexdigest()
            if args.drop_rejected and sha in rejected:
                continue
            candidates.append((position == 0 or sha in decided, entry))
    kept, dropped = [], 0
    if args.drop_overlaps:
        order = [c for c in candidates if c[0]] + sorted(
            (c for c in candidates if not c[0]),
            key=lambda c: survey_end(Path(c[1]["raw"])), reverse=True)
        boxes = []
        for keep, entry in order:
            box = footprint(Path(entry["review"]))
            if not keep and any(overlap(box, other) > OVERLAP for other in boxes):
                dropped += 1
                continue
            boxes.append(box)
            kept.append(entry)
        # Restore input order for a stable gallery.
        rank = {c[1]["review"]: i for i, c in enumerate(candidates)}
        kept.sort(key=lambda e: rank[e["review"]])
    else:
        kept = [c[1] for c in candidates]
    ids, titles = set(), set()
    for entry in kept:
        base, n = entry["id"], 2
        while entry["id"] in ids:
            entry["id"], n = f"{base}-{n}", n + 1
        title, n = entry["title"], 2
        while entry["title"] in titles:
            entry["title"], n = f"{title} #{n}", n + 1
        ids.add(entry["id"])
        titles.add(entry["title"])
    args.output.write_text(json.dumps({"schema_version": 1, "title": "Terrain library", "tiles": kept}, indent=1))
    print(f"{len(kept)} tiles, {dropped} overlapping dropped -> {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
