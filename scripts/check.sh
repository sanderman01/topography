#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd -- "$project_root"

cargo fmt --all --check
cargo build --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo run --quiet --locked --offline -p topography-cli -- --help
cargo run --quiet --locked --offline -p topography-cli -- --version

cli=(cargo run --quiet --locked --offline --manifest-path "$project_root/Cargo.toml" -p topography-cli --)

check_success() {
  local expected="$1"
  shift
  local output
  output="$("${cli[@]}" "$@")"
  if [[ "$output" != *"$expected"* ]]; then
    printf 'Expected output containing: %s\nActual output: %s\n' "$expected" "$output" >&2
    return 1
  fi
}

check_failure() {
  local expected_status="$1" expected_message="$2"
  shift 2
  local output command_status
  if output="$("${cli[@]}" "$@" 2>&1)"; then
    printf 'Expected failure for: %s\n' "$*" >&2
    return 1
  else
    command_status=$?
  fi
  if [[ "$command_status" != "$expected_status" || "$output" != *"$expected_message"* ]]; then
    printf 'Unexpected status %s or diagnostic: %s\n' "$command_status" "$output" >&2
    return 1
  fi
}

check_success 'Extent: 4096 x 4096 m' validate world configs/worlds/pilot.toml
check_success '4294967296 bytes' validate world configs/worlds/standard.toml
check_success 'Final corner tile: 257 x 257 vertices' validate world configs/worlds/exact-32km.toml
check_success 'Extent: 32000 x 16000 m' validate world 'tests/fixtures/contracts/rectangular world.toml'
check_success '9 classes, 0 retired IDs' validate legend catalog/labels/terrain-v1.toml
check_success 'excluded from prepared and training pools' validate source examples/manifests/source-v1.toml
# Resolve storage against the config file, even when invoked outside the checkout.
(
  cd -- /tmp
  check_success "raw: $project_root/configs/../data/raw" validate runtime "$project_root/configs/runtime.example.toml"
)

check_failure 1 'generation.stride_cells' validate world tests/fixtures/contracts/invalid-stride.toml
check_failure 1 'grid.spacing_m' validate world tests/fixtures/contracts/nonfinite-spacing.toml
check_failure 1 '64-bit byte count' validate world tests/fixtures/contracts/overflow-grid.toml
check_failure 1 'unknown field' validate world tests/fixtures/contracts/unknown-field.toml
check_failure 1 'classes.id' validate legend tests/fixtures/contracts/duplicate-class.toml
check_failure 1 'only version 1' validate legend tests/fixtures/contracts/unsupported-version.toml
check_failure 1 'license.training_permission' validate source tests/fixtures/contracts/missing-license-evidence.toml
check_failure 1 'hardware.gpu_vram_budget_gb' validate runtime tests/fixtures/contracts/over-budget.toml
check_failure 1 'cannot read' validate world tests/fixtures/contracts/does-not-exist.toml
check_failure 2 'Unsupported arguments' validate world
check_failure 2 'Unsupported arguments' validate unknown configs/worlds/pilot.toml
printf 'Contract workflows passed.\n'

if command -v python3 >/dev/null && command -v gdalinfo >/dev/null && command -v gdal_translate >/dev/null && command -v gdaldem >/dev/null; then
  cli_binary="$(cargo metadata --no-deps --locked --offline --format-version 1 | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"] + "/debug/topography")')"
  python3 scripts/check_acquisition.py "$cli_binary" --local-only
  python3 scripts/check_curation.py "$cli_binary"
  python3 scripts/check_batch_gallery.py "$cli_binary" --local-only
  python3 scripts/check_quality.py "$cli_binary"
  python3 scripts/check_preparation.py "$cli_binary"
  if [[ -x training/.venv/bin/python ]] && training/.venv/bin/python -c 'import topography_training.numeric' >/dev/null 2>&1; then
    training/.venv/bin/python scripts/check_numeric.py
    training/.venv/bin/python scripts/check_codec.py
    python3 scripts/check_datasets.py "$cli_binary" --python "$project_root/training/.venv/bin/python"
  else
    python3 scripts/check_datasets.py "$cli_binary"
    printf 'Numeric worker checks skipped: install the training package in training/.venv.\n'
  fi
  if command -v pdal >/dev/null; then
    python3 scripts/check_ground_support.py --self-check
  else
    printf 'PDAL ground-support probe check skipped: PDAL is not installed.\n'
  fi
else
  printf 'GDAL acquisition workflow skipped: Python 3.11+ and GDAL command-line tools are required.\n'
fi

if [[ -x data/models/codec-environment-v1/.venv/bin/python ]]; then
  data/models/codec-environment-v1/.venv/bin/python scripts/check_scalar_fit.py
else
  printf 'Scalar fitting checks skipped: the optional CPU Torch environment is not installed.\n'
fi
