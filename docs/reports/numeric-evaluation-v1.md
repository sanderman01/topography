# Reusable numeric pilot result

Completed 2026-09-21. The reusable Rust/Python path evaluated all twelve selected
native regions. Every **12,222,942 valid ground samples** round-tripped bit for bit
through scalar Float32 loading and fixed height ÷ 2,048 normalization/inverse.
All 359,970 water cells were excluded from height and adjacent-slope metrics.
No model, autoencoder or GPU ran.

The owner said, “I have no idea what I am looking at. I suppose its fine.” We
captured this as assent to initial evaluation, with technical judgments remaining
the implementer's responsibility. It does not certify technical details or grant
model-training, accuracy-tolerance or generated-output approval. No terrain tile
was selected again; the existing twelve acceptances and one rejection remain.

## What the measurement establishes

Float32 normalization preserved exact valid bits, including signed zero in the
workflow fixtures. Float16 absolute-height storage introduced up to **0.25 m**
error on real terrain. Across the twelve rasters, 7,803,628 originally nonflat
valid adjacent pairs became flat. The synthetic narrow ridge, 0.125 m high at
1,800 m elevation, disappeared completely in Float16. Ramps, flats and a sharper
ridge also ran with explicit height and slope measurements.

Keep Float32 authoritative elevation and scalar loading. This result concerns the
specified absolute-height storage experiment. It does not reject every use of
mixed precision in model internals or establish survey/gameplay accuracy. No
vehicle clearance or model acceptance tolerance has been invented.

The Python worker took 2.81 seconds and reported 134,072 KiB (about 131 MiB) peak
self RSS. This excludes Rust, GDAL and their current-source/integrity checks;
end-to-end processing took several minutes and was not instrumented as a whole.
The retained run occupies 125,867,650 bytes (about 120 MiB), including all decoded
inputs. Environment: Python 3.14.7, NumPy 2.3.5, Linux. Python 3.11 and Rust 1.85
are compatibility targets, not environments tested in this continuation.

## Retained identities

- Release: `data/datasets/6b7e2049f45d94bd87216ac07d96c38ece2885e8f22a00fb9d5e0686d6f07411/manifest.json`.
- Run: `data/models/numeric-pilot-v1/manifest.json`, with `request.json`, 84 hashed
  binary payloads, `metrics.json`, `worker-result.json` and `worker.log`.
- Exact selection, scoped approval and links to the original presented proposal:
  `data/review/numeric-evaluation-v1/`.
- Original proposal, page, historical arithmetic probe and exact owner response:
  `data/review/pilot-dataset-proposal-v1/`.

The release retains six area groups with calibration/validation roles, a 512 m
maximum context/buffer and 8,042.645 m minimum opposing footprint-edge separation.
It has no untouched test set, independent provider-project sample or full coarse
training context. Larger-context/model-training use requires its own release.

## Implementation verification

The [workflow](../workflows/numeric-evaluation.md) exercises strict release and
batch contracts, geographic overlap/leakage, current-source revocation, masked
fill, signed zero, physical spacing, corrupt payloads and failed jobs. A fresh
review identified incomplete-result acceptance and a fast-exit log-limit bypass.
Both were reproduced with faulty workers and fixed before the checkpoint.
Completion now requires full metric/provenance structures and consistent bounds;
diagnostic reads and logs are bounded even after process exit.

The real run's release, request, result, metrics, worker-code hashes and all 84
payload hashes were independently checked. Source files and decisions were
preserved. These checks establish numeric processing and provenance, not human
acceptance of a model or generated terrain.

## Next experiment

Follow the [bounded image-codec plan](../plans/image-codec-pilot-v1.md) to test
a public pretrained image autoencoder's reconstruction before investing in
LoRA training. The scalar loader is now available; the learned compression path
remains unevaluated. Use a small deterministic CPU experiment and the same
physical metrics. Obtain workload readiness under AGENTS.md before substantial
compute, and retain the real-patch/blending fallback if the model route fails.
