"""Run one approved scalar fitting trial in a bounded CPU worker."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import resource
import signal
import subprocess
import sys

from .codec import ADDRESS_SPACE_BYTES, LOG_BYTES, TIMEOUT_SECONDS, publish_result, supervise


def current_status(job: dict, request: dict) -> dict:
    """Rust owns source admission and currentness; bind its checked grids/roles to the payload."""
    completed = subprocess.run([job['cli']['path'], 'dataset', 'fit-verify', job['runtime']['path'],
                                job['fitting_release']['path']], capture_output=True, timeout=600, check=True)
    status = json.loads(completed.stdout)
    expected = [{k: r[k] for k in ['prepared_sha256', 'group', 'role', 'grid']} for r in request['regions']]
    if (status.get('manifest_sha256') != job['fitting_release']['sha256'] or status.get('current') is not True
            or status.get('purpose') != 'representation_fit_only' or status.get('trial') != 'scalar_codec_fit_v1'
            or status.get('regions') != expected):
        raise ValueError('current fitting release does not match numeric payload')
    return status


def execute(job_path: Path, output: Path) -> Path:
    from .fitting import DISK_BYTES, check_inputs, verify_output
    from .numeric import write_json
    if output.exists():
        raise ValueError('fitting output already exists; choose a new directory')
    job, request, _, identities = check_inputs(job_path)
    output = output.absolute()
    if any(Path(root) in output.resolve().parents for root in identities['input_roots']):
        raise ValueError('output must be outside immutable input directories')
    before = current_status(job, request)
    output.mkdir(parents=True)
    try:
        write_json(output / 'source-before.json', before)
        with (output / 'job.json').open('xb') as stream:
            stream.write(job_path.read_bytes())
        supervise([sys.executable, '-m', 'topography_training.scalar_fit', '--worker',
                   str(job_path.absolute()), str(output)], output / 'worker.log', TIMEOUT_SECONDS, LOG_BYTES,
                  disk_bytes=DISK_BYTES)
        result = verify_output(job_path, output)
        after = current_status(job, request)
        if before != after:
            raise ValueError('fitting source status changed during experiment')
        write_json(output / 'source-after.json', after)
        result['source_current_at_completion'] = True
        publish_result(output, result)
    except BaseException as error:
        with (output / 'failure.json').open('x') as stream:
            json.dump({'schema_version': 1, 'kind': 'incomplete_scalar_fit', 'error': str(error)[:4096]}, stream, indent=2)
        raise
    return output / 'result.json'


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--worker', action='store_true', help=argparse.SUPPRESS)
    parser.add_argument('job', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    try:
        if args.worker:
            signal.signal(signal.SIGALRM, signal.SIG_DFL)
            signal.alarm(TIMEOUT_SECONDS)
            resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_BYTES, ADDRESS_SPACE_BYTES))
            resource.setrlimit(resource.RLIMIT_FSIZE, (64 * 1024**2, 64 * 1024**2))
            from .fitting import fit
            fit(args.job.absolute(), args.output.absolute())
        else:
            print(execute(args.job.absolute(), args.output.absolute()))
    except (ValueError, RuntimeError, OSError, MemoryError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print(f'Scalar fitting failed: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
