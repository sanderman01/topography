# Shell scripting

Applies to Bash in `scripts/` and wrappers around Cargo, GDAL/PROJ, PDAL, and
Python jobs. Keep scripts focused on small orchestration tasks, following
[scripts/README.md](../../scripts/README.md).

## Arguments and failure handling

- Use an explicit Bash shebang and `set -euo pipefail`, as `scripts/check.sh`
  does. Handle expected nonzero statuses in explicit conditionals; do not rely
  on `set -e` alone to define recovery or cleanup behavior. Consult Bash's
  [set builtin](https://www.gnu.org/s/bash/manual/html_node/The-Set-Builtin.html)
  for its exceptions.
- Quote variable expansions and forward argument lists with `"$@"`. Use arrays
  for commands with optional arguments; avoid `eval`, generated command strings,
  and parsing the output of `ls`. See the
  [Bash quoting rules](https://www.gnu.org/software/bash/manual/html_node/Quoting.html).
- Use descriptive variable names such as `project_root` and `staging_dir`.
  Do not repurpose `PATH`, `HOME`, or other shell/environment control variables.
- Resolve the repository root from the script location when appropriate so a
  script works outside the checkout directory. Document whether other input and
  output paths are relative to the caller, a manifest, or the repository.
- Check required tools and inputs before starting expensive work. Preserve the
  subprocess exit status and include the failing operation in diagnostics.
  Send progress and diagnostics to stderr if stdout carries a machine result.

## Artifacts and restart behavior

- Require explicit input/output locations. Use a dedicated staging directory or
  `mktemp` for temporary outputs. Quote paths and use `--` for option termination
  where the invoked tool supports it.
- Limit cleanup traps to temporary files owned by the current run. A trap must
  never remove source assets, accepted worlds, or an unvalidated caller-supplied
  directory. Source deletion and cache cleanup are separate explicit commands.
- Validate downloads and processing results before publishing them. Treat
  cross-filesystem moves as potentially non-atomic; stage final publication on
  the destination filesystem when atomic rename is required.
- Retry only transient failures, with a bound and a delay. Keep partial
  downloads identifiable and verify source identity and checksums on resume.
- Pass credentials through the configured environment or credential mechanism.
  Avoid shell tracing or logged command lines that expose secrets or signed URLs.

## Keep scripts small and verifiable

Move persistent catalog state, provider policies, dependency tracking, and
semantic validation into Rust when a wrapper grows into domain logic. Keep
Python responsible for model jobs and use existing geospatial tools for their
numeric operations, per [architecture and contracts](architecture-and-contracts.md).

Run `bash -n` on changed scripts and execute their actual workflow, including
paths containing spaces and relevant failures. `bash scripts/check.sh` is the
current development check; it requires no GIS or GPU environment. ShellCheck
can be adopted when useful, but is not currently a project dependency or check.
