# Rust

Applies to `crates/` and the Cargo workspace. The workspace uses Rust 2024,
declares Rust 1.85 as its minimum version, and forbids unsafe code.

## Workspace and dependencies

- Inherit package metadata and lints from the workspace as the existing crates
  do. Keep shared dependency declarations in the root `Cargo.toml` and commit
  intentional dependency changes with `Cargo.lock`.
- Use APIs and dependencies compatible with the declared minimum Rust version.
  A build on a newer compiler does not verify that minimum. Check Rust 1.85 when
  adding version-sensitive APIs or dependencies; treat a minimum-version increase
  as an explicit compatibility change. See
  [Cargo's Rust version reference](https://doc.rust-lang.org/cargo/reference/rust-version.html).
- Add a dependency for a concrete need and inspect its features, native build
  requirements, and compatibility. Contract validation uses Serde and TOML with
  locked compatible dependencies. Do not relax `unsafe_code = "forbid"` for
  raster algorithms; use the established tool boundary.

## Types, ownership, and errors

- Model distinct concepts such as physical spacing, grid dimensions, class IDs,
  and approval states explicitly. Use small types and enums when they prevent
  invalid combinations or accidental unit mixing.
- Accept borrowed data when ownership is unnecessary; avoid cloning complete
  rasters or manifests for convenience. Keep public APIs small and expose only
  what a caller needs. Follow the applicable
  [Rust API guidelines](https://rust-lang.github.io/api-guidelines/checklist.html).
- Return `Result` for failed I/O, provider responses, invalid configuration, and
  subprocess failures. Preserve causes and add operation, artifact, or path
  context. Keep recovery decisions distinguishable without parsing error text.
- Avoid `unwrap`, `expect`, and panics on user data or environmental failures.
  If an internal invariant justifies an assertion, document why it must hold.
  Check finite floats, integer multiplication, offsets, casts, and buffer sizes
  before operating on externally supplied raster dimensions.
- Keep paths as `Path`/`PathBuf` and process arguments as `OsStr`/`OsString` where
  appropriate. Invoke programs through `std::process::Command` argument lists;
  do not interpolate manifest values into shell command strings.

## CLI and large jobs

- Keep reusable behavior in `topography-core`; the CLI translates results into
  useful messages and exit status. Send diagnostics to stderr and requested
  output to stdout. Preserve the scaffold's help/version behavior and status 2
  for unsupported arguments until an intentional interface change.
- Bound memory and concurrency. Use windowed raster operations and bounded
  queues rather than collecting all world cells, downloads, or subprocess logs
  into memory. Handle cancellation and partial output explicitly.
- Add comments for non-obvious invariants and public documentation for units,
  masks, and error behavior. Let rustfmt determine formatting.

## Verify

Run `bash scripts/check.sh` from the repository root. It checks rustfmt, builds
the workspace, runs Clippy with warnings denied, and exercises CLI help/version
and valid/invalid contract inputs.
It uses `--locked --offline`; dependencies must already be available locally.
When adding dependencies, document any required fetch/setup step instead of
silently dropping reproducibility flags.

Exercise the changed command with representative inputs and a relevant failure
case. Formal integration tests, when requested, belong in each crate's `tests/`
directory; see [validation and review](validation-and-review.md).
