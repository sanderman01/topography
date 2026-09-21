"""Supervise one bounded CPU image-codec experiment; completion is not promotion."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import resource
import signal
import subprocess
import sys
import time

THREADS = 2
TIMEOUT_SECONDS = 1800
ADDRESS_SPACE_BYTES = 6 * 1024**3
LOG_BYTES = 1024**2


def supervise(command: list[str], log: Path, timeout: float, log_bytes: int,
              *, disk_bytes: int | None = None) -> None:
    """Run one process group with a wall-clock deadline and bounded diagnostic reads."""
    environment = dict(os.environ, OMP_NUM_THREADS="2", OPENBLAS_NUM_THREADS="2",
                       MKL_NUM_THREADS="2", NUMEXPR_NUM_THREADS="2",
                       CUDA_VISIBLE_DEVICES="", HIP_VISIBLE_DEVICES="",
                       HF_HUB_OFFLINE="1", HF_HUB_DISABLE_TELEMETRY="1",
                       PYTHONNOUSERSITE="1", PYTHONUNBUFFERED="1")
    previous = signal.getsignal(signal.SIGTERM)
    def terminated(signum, frame):
        raise RuntimeError("codec supervisor terminated")
    signal.signal(signal.SIGTERM, terminated)
    child = None
    try:
        with log.open("xb") as stream:
            child = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=stream,
                                     stderr=stream, env=environment, start_new_session=True)
            started = time.monotonic()
            while True:
                status = child.poll()
                if log.stat().st_size > log_bytes:
                    raise RuntimeError("codec worker log exceeded its limit")
                if disk_bytes is not None and sum(p.stat().st_size for p in log.parent.rglob('*') if p.is_file()) > disk_bytes:
                    raise RuntimeError("codec worker artifact disk limit exceeded")
                if time.monotonic() - started > timeout:
                    raise RuntimeError("codec worker exceeded its time limit")
                if status is not None:
                    if status:
                        with log.open("rb") as reader:
                            detail = reader.read(4096).decode(errors="replace")
                        raise RuntimeError(f"codec worker failed ({status}): {detail}")
                    return
                time.sleep(0.05)
    finally:
        # SIGTERM must use this path too: the child has its own process group.
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        try:
            if child is not None:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.wait()
        finally:
            signal.signal(signal.SIGTERM, previous)


def publish_result(output: Path, result: dict) -> None:
    """Flush a private completion file before atomically publishing without replacement."""
    from .numeric import write_json
    pending = output / ".result.pending"
    write_json(pending, result)
    os.link(pending, output / "result.json")
    # Completion is published; failure to remove the private link is only leftover staging.
    try:
        pending.unlink()
    except OSError:
        pass


def execute(job: Path, output: Path) -> Path:
    """Retain partial outputs on failure, publishing result.json only after verification."""
    if output.exists():
        raise ValueError("codec output already exists; choose a new directory")
    # Lightweight input checking precedes output creation and model allocation.
    from .reconstruction import check_inputs, verify_output
    _, _, _, identities = check_inputs(job)
    output = output.absolute()
    for source in identities["input_roots"]:
        if Path(source) in output.resolve().parents:
            raise ValueError("codec output must be outside immutable input directories")
    output.mkdir(parents=True)
    try:
        supervise([sys.executable, "-m", "topography_training.codec", "--worker",
                   str(job.absolute()), str(output)], output / "worker.log", TIMEOUT_SECONDS, LOG_BYTES)
        result = verify_output(job, output)
        publish_result(output, result)
    except BaseException as error:
        with (output / "failure.json").open("x") as stream:
            json.dump({"schema_version": 1, "kind": "incomplete_codec_experiment",
                       "error": str(error)[:4096]}, stream, indent=2)
        raise
    return output / "result.json"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("job", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        if args.worker:
            # Set the limit before importing NumPy, Torch or Diffusers.
            signal.signal(signal.SIGALRM, signal.SIG_DFL)
            signal.alarm(TIMEOUT_SECONDS)
            resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_BYTES, ADDRESS_SPACE_BYTES))
            resource.setrlimit(resource.RLIMIT_FSIZE, (64 * 1024**2, 64 * 1024**2))
            from .reconstruction import reconstruct
            reconstruct(args.job.absolute(), args.output.absolute())
        else:
            print(execute(args.job.absolute(), args.output.absolute()))
    except (ValueError, RuntimeError, OSError, MemoryError, KeyError, TypeError) as error:
        print(f"Codec evaluation failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
