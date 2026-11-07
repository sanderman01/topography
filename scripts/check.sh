#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd -- "$project_root"

# Run offline: re-exec in a network namespace with only loopback, so any web request fails.
if [[ -z "${TOPOGRAPHY_CHECK_NETNS:-}" ]]; then
  if command -v unshare >/dev/null && unshare -rn true 2>/dev/null; then
    export TOPOGRAPHY_CHECK_NETNS=1
    exec unshare -rn bash "${BASH_SOURCE[0]}" "$@"
  elif [[ "${TOPOGRAPHY_CHECK_ALLOW_NETWORK:-}" == 1 ]]; then
    printf 'Warning: running without network isolation (TOPOGRAPHY_CHECK_ALLOW_NETWORK=1).\n' >&2
  else
    printf 'check.sh needs unprivileged network namespaces (unshare -rn) to block web requests.\n' >&2
    printf 'Set TOPOGRAPHY_CHECK_ALLOW_NETWORK=1 to run without isolation.\n' >&2
    exit 1
  fi
elif [[ "$TOPOGRAPHY_CHECK_NETNS" == 1 ]]; then
  if command -v ip >/dev/null; then
    ip link set lo up
  else
    python3 - <<'EOF'
import fcntl, socket, struct
s = socket.socket()
req = struct.pack("16sH22x", b"lo", 0)
flags = struct.unpack("16sH22x", fcntl.ioctl(s, 0x8913, req))[1]  # SIOCGIFFLAGS
fcntl.ioctl(s, 0x8914, struct.pack("16sH22x", b"lo", flags | 1))  # SIOCSIFFLAGS, IFF_UP
EOF
  fi
fi

cargo fmt --all --check
cargo build --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --locked --offline -p topography-core --quiet hydrology
cargo test --locked --offline -p topography-core --quiet world::water
cargo test --locked --offline -p topography-core --quiet export
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
# Hydrology v3 on a 64² fixture (lakes, rivers and a dam to breach): the commands run, find
# them and write their outputs.
water_tmp="$(mktemp -d)"
trap 'rm -rf "$water_tmp"' EXIT
water_grid=tests/fixtures/water/grid
water_args=(--tile 16 --threads 2 --water "$water_grid/water.toml" --river-km2 0.00001)
water_out="$("${cli[@]}" world water --grid "$water_grid" --work "$water_tmp/work" "${water_args[@]}" 2>&1)" ||
  { printf 'world water failed: %s\n' "$water_out" >&2; exit 1; }
[[ "$water_out" =~ :\ [1-9][0-9]*\ lakes,\ [1-9][0-9]*\ rivers,\ [1-9][0-9]*\ breaches,.*next:\ water-finish ]] ||
  { printf 'world water found no lakes, rivers or breaches: %s\n' "$water_out" >&2; exit 1; }
water_out="$("${cli[@]}" world water-finish --grid "$water_grid" --work "$water_tmp/work" \
  --ground "$water_tmp/work/carved.f32" --out "$water_tmp/out" "${water_args[@]}" 2>&1)" ||
  { printf 'world water-finish failed: %s\n' "$water_out" >&2; exit 1; }
[[ "$water_out" =~ water\ passed:\ [1-9][0-9]*\ lakes,\ [1-9][0-9]*\ rivers ]] ||
  { printf 'world water-finish did not pass with lakes and rivers: %s\n' "$water_out" >&2; exit 1; }
for f in elevation.f32 water-surface.f32 water-body.u32 discharge.f32 flow.u8 bodies.json report.json grid.json; do
  test -s "$water_tmp/out/$f" || { printf 'world water-finish wrote no %s\n' "$f" >&2; exit 1; }
done
grep -q '"passed": true' "$water_tmp/out/report.json" ||
  { printf 'water report not passed\n' >&2; exit 1; }
grep -Eq '"breaches": [1-9]' "$water_tmp/out/report.json" ||
  { printf 'water report lists no breach\n' >&2; exit 1; }
rain_grid="$water_tmp/rain"
cp -r "$water_grid" "$rain_grid"
jq 'del(.moisture, .discharge) | .spacing_m = 32 | .wind_from_deg = 270' "$water_grid/grid.json" > "$rain_grid/grid.json"
rm "$rain_grid/moisture.f32"
"${cli[@]}" world rain --grid "$rain_grid" >/dev/null ||
  { printf 'world rain failed\n' >&2; exit 1; }
test -s "$rain_grid/moisture.f32" || { printf 'world rain wrote no moisture\n' >&2; exit 1; }
check_failure 2 'Unsupported arguments' world rain --grid
check_failure 2 'Unsupported arguments' world water --grid
check_failure 2 'Unsupported arguments' world water-finish --grid g --work w
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
  python3 scripts/check_automatic_preparation.py "$cli_binary"
  python3 scripts/check_quality.py "$cli_binary"
  python3 scripts/check_surface_cover.py "$cli_binary"
  python3 scripts/check_cover_acquisition.py "$cli_binary"
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
