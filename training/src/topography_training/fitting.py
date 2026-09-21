"""Single approved scalar-codec trial: immutable inputs, fixed fit and checked outputs."""
from __future__ import annotations

import json
import math
from pathlib import Path
import platform
import resource
import sys
import time

import numpy as np

from .codec import ADDRESS_SPACE_BYTES, THREADS, TIMEOUT_SECONDS
from .metrics import compare_terrain
from .numeric import write_json
from .payload import digest, fields, load_region, read_document
from .reconstruction import cases, check_numeric_run, file_hash, pinned_document, save_array
from .scalar_model import window_schedule

SIZE = 512
MARGIN = 32
DISK_BYTES = 512 * 1024**2
SOURCES = {'scalar_fit.py', 'scalar_model.py', 'fitting.py', 'codec.py',
           'reconstruction.py', 'metrics.py', 'numeric.py', 'payload.py'}
SETTINGS = {'trial': 'scalar_codec_fit_v1', 'steps': 200, 'seed': 0, 'batch_size': 1,
            'optimizer': 'Adam', 'learning_rate': 0.001, 'betas': [0.9, 0.999],
            'epsilon': 1e-8, 'weight_decay': 0.0, 'precision': 'float32', 'device': 'cpu',
            'gpu_used': False, 'threads': THREADS, 'interop_threads': 1,
            'timeout_seconds': TIMEOUT_SECONDS, 'address_space_bytes': ADDRESS_SPACE_BYTES,
            'artifact_disk_limit_bytes': DISK_BYTES, 'window_cells': SIZE,
            'interior_margin_cells': MARGIN, 'normalization': {'offset_m': 0, 'scale_m': 2048},
            'architecture': 'scalar_conv_16_32_latent4_bilinear_v1',
            'checkpoint_scope': 'inference weights only; resuming is not supported'}


def check_inputs(job_path: Path) -> tuple[dict, dict, Path, dict]:
    job, data = read_document(job_path)
    fields(job, {'schema_version', 'kind', 'numeric_run', 'fitting_release', 'runtime', 'cli'}, 'scalar fitting job')
    if type(job['schema_version']) is not int or job['schema_version'] != 1 or job['kind'] != 'scalar_codec_fit_job':
        raise ValueError('unsupported scalar fitting job')
    fit_path, release, _ = pinned_document(job['fitting_release'])
    fields(release, {'schema_version', 'kind', 'purpose', 'dataset_sha256', 'approval_document', 'topography_version'}, 'fitting release')
    if (release['schema_version'] != 1 or release['kind'] != 'representation_fit_release'
            or release['purpose'] != 'representation_fit_only'
            or fit_path.parent.name != job['fitting_release']['sha256']):
        raise ValueError('separate fitting-only release required')
    parent_path = fit_path.parent.parent / release['dataset_sha256'] / 'manifest.json'
    _, parent, _ = pinned_document({'path': str(parent_path), 'sha256': release['dataset_sha256']})
    selection = json.loads(parent['selection_document'])
    request, root, request_sha = check_numeric_run(job['numeric_run'], release['dataset_sha256'])
    expected = [(member, g['id'], g['role']) for g in selection['groups'] for member in g['prepared']]
    actual = [(r['prepared_sha256'], r['group'], r['role']) for r in request['regions']]
    if actual != expected or len(actual) != 12 or sum(r[2] == 'calibration' for r in actual) != 6:
        raise ValueError('fitting requires the approved twelve-region membership and six/six split')
    schedule = window_schedule(request['regions'])
    for r in request['regions']:
        height, valid, _ = load_region(r, root)
        if min(height.shape) < SIZE:
            raise ValueError('fitting requires native 512-cell windows')
        for window in [w for w in schedule if w['prepared_sha256'] == r['prepared_sha256']]:
            row, col = window['row'], window['column']
            if not np.any(valid[row:row + SIZE, col:col + SIZE]):
                raise ValueError('all-masked scheduled fitting window')
    for _, height, valid, _ in cases(request, root, SIZE):
        if not valid[MARGIN:-MARGIN, MARGIN:-MARGIN].any():
            raise ValueError('all-masked evaluation interior')
    for name in ['runtime', 'cli']:
        ref = job[name]
        fields(ref, {'path', 'sha256'}, name)
        if not isinstance(ref['path'], str) or not Path(ref['path']).is_absolute() or file_hash(Path(ref['path'])) != ref['sha256']:
            raise ValueError(f'{name} identity mismatch')
    return job, request, root, {'job_sha256': digest(data), 'request_sha256': request_sha,
            'dataset_sha256': release['dataset_sha256'],
            'input_roots': [str(root.resolve()), str(fit_path.parent.resolve()), str(parent_path.parent.resolve())]}


def measurements(height: np.ndarray, decoded: np.ndarray, valid: np.ndarray,
                 water: np.ndarray, spacing: list[float]) -> dict:
    if decoded.dtype != np.float32 or decoded.shape != height.shape or not np.isfinite(decoded).all():
        raise ValueError('invalid scalar reconstruction')
    interior = (slice(MARGIN, -MARGIN), slice(MARGIN, -MARGIN))
    return {'valid_cells': int(valid.sum()), 'water_cells': int(water.sum()),
            'invalid_land_cells': int((~valid & ~water).sum()),
            'full': compare_terrain(height, decoded, valid, spacing),
            'interior': compare_terrain(height[interior], decoded[interior], valid[interior], spacing),
            'mean_bias_m': float(np.mean(decoded[valid].astype(np.float64) - height[valid])),
            'decoded_range_m': [float(decoded[valid].min()), float(decoded[valid].max())]}


def artifact(path: Path) -> dict:
    return {'path': path.name, 'bytes': path.stat().st_size, 'sha256': file_hash(path)}


def source_hashes() -> dict:
    return {name: file_hash(Path(__file__).with_name(name)) for name in sorted(SOURCES)}


def fit(job_path: Path, output: Path) -> None:
    """Called only after worker resource limits are installed; no validation-driven tuning."""
    started = time.monotonic()
    job, request, root, identities = check_inputs(job_path)
    sources = source_hashes()
    import torch
    import safetensors
    from safetensors.torch import save_file
    from .scalar_model import build_model, masked_loss
    if torch.version.cuda is not None or torch.version.hip is not None:
        raise ValueError('scalar fitting requires a CPU-only Torch build')
    torch.set_num_threads(THREADS)
    torch.set_num_interop_threads(1)
    torch.manual_seed(0)
    torch.use_deterministic_algorithms(True)
    model = build_model().to('cpu')
    schedule = window_schedule(request['regions'])
    write_json(output / 'schedule.json', schedule)
    rows = []

    def evaluate(stage):
        model.eval()
        save_file(model.state_dict(), output / f'{stage}.safetensors')
        for index, (info, height, valid, water) in enumerate(cases(request, root, SIZE)):
            data = (height / np.float32(2048))[None, None].copy()
            with torch.inference_mode():
                tensor = torch.from_numpy(data)
                decoded = model(tensor)[0, 0].numpy().copy() * np.float32(2048)
                direct = (tensor[0, 0] * 2048).numpy().copy()
            if stage == 'initial':
                arrays = {'original': height.astype('<f4'), 'valid': valid.astype('u1'), 'water': water.astype('u1')}
                rows.append(dict(info, metrics={'direct_scalar': measurements(height, direct, valid, water, info['spacing_m'])},
                                 artifacts={n: save_array(output / f"{info['id']}-{n}.bin", a) for n, a in arrays.items()}))
            rows[index]['metrics'][stage] = measurements(height, decoded, valid, water, info['spacing_m'])
            rows[index]['artifacts'][stage] = save_array(output / f"{info['id']}-{stage}.bin", decoded.astype('<f4'))
        print(f'{stage} evaluation complete', flush=True)

    evaluate('initial')
    model.train()
    optimizer = torch.optim.Adam(model.parameters(), lr=0.001, betas=(0.9, 0.999),
                                 eps=1e-8, weight_decay=0, foreach=False)
    # Six native regions fit comfortably within the approved cap; no validation tensor enters loss.
    fitting = {r['prepared_sha256']: load_region(r, root)[:2] for r in request['regions'] if r['role'] == 'calibration'}
    history = []
    for window in schedule:
        height, valid = fitting[window['prepared_sha256']]
        row, col = window['row'], window['column']
        crop = (slice(row, row + SIZE), slice(col, col + SIZE))
        data = torch.from_numpy((height[crop] / np.float32(2048))[None, None].copy())
        mask = torch.from_numpy(valid[crop][None, None].copy())
        optimizer.zero_grad(set_to_none=True)
        loss = masked_loss(model(data), data, mask)
        loss.backward()
        optimizer.step()
        history.append(dict(window, loss_normalized_mse=float(loss.detach())))
        if window['step'] % 20 == 0:
            print(f"step {window['step']}/200, normalized MSE {history[-1]['loss_normalized_mse']:.9g}", flush=True)
    write_json(output / 'loss-history.json', history)
    evaluate('final')
    environment = {'python': sys.version, 'numpy': np.__version__, 'torch': torch.__version__,
                   'safetensors': safetensors.__version__, 'platform': platform.platform(),
                   'elapsed_seconds': time.monotonic() - started,
                   'max_self_rss_kib': resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
                   'rss_scope': 'worker only; Linux KiB; excludes supervisor and Rust checks'}
    write_json(output / 'worker-result.json', {'schema_version': 1, 'kind': 'scalar_codec_fit_worker_result',
               'job_sha256': identities['job_sha256'], 'fitting_release_sha256': job['fitting_release']['sha256'],
               'numeric_run_sha256': job['numeric_run']['sha256'], 'settings': SETTINGS,
               'source_sha256': sources, 'environment': environment,
               'artifacts': {name: artifact(output / name) for name in ['schedule.json', 'loss-history.json',
                                                                      'initial.safetensors', 'final.safetensors']},
               'cases': rows})


def checked_artifact(output: Path, description: dict, name: str, maximum_bytes: int) -> Path:
    fields(description, {'path', 'bytes', 'sha256'}, 'fit artifact')
    path = output / name
    if (description['path'] != name or type(description['bytes']) is not int
            or not 0 < description['bytes'] <= maximum_bytes or path.is_symlink()
            or path.stat().st_size != description['bytes'] or file_hash(path) != description['sha256']):
        raise ValueError('fitting artifact integrity mismatch')
    return path


def verify_output(job_path: Path, output: Path) -> dict:
    """Recompute metrics and bind complete schedule, history, checkpoints and all scalar arrays."""
    job, request, root, identities = check_inputs(job_path)
    result, result_bytes = read_document(output / 'worker-result.json')
    fields(result, {'schema_version', 'kind', 'job_sha256', 'fitting_release_sha256', 'numeric_run_sha256',
                   'settings', 'source_sha256', 'environment', 'artifacts', 'cases'}, 'scalar fit result')
    expected = {'schema_version': 1, 'kind': 'scalar_codec_fit_worker_result',
                'job_sha256': identities['job_sha256'], 'fitting_release_sha256': job['fitting_release']['sha256'],
                'numeric_run_sha256': job['numeric_run']['sha256'], 'settings': SETTINGS,
                'source_sha256': source_hashes()}
    if any(result[k] != v for k, v in expected.items()):
        raise ValueError('scalar fit result identity/settings/source mismatch')
    env = result['environment']
    fields(env, {'python', 'numpy', 'torch', 'safetensors', 'platform', 'elapsed_seconds', 'max_self_rss_kib', 'rss_scope'}, 'fit environment')
    for name in ['python', 'numpy', 'torch', 'safetensors', 'platform', 'rss_scope']:
        if not isinstance(env[name], str) or not env[name].strip():
            raise ValueError('missing fitting software provenance')
    for name in ['elapsed_seconds', 'max_self_rss_kib']:
        if type(env[name]) not in (float, int) or not math.isfinite(env[name]) or env[name] <= 0:
            raise ValueError('invalid fitting resource observation')
    names = {'schedule.json', 'loss-history.json', 'initial.safetensors', 'final.safetensors'}
    fields(result['artifacts'], names, 'fitting artifacts')
    for name in names:
        checked_artifact(output, result['artifacts'][name], name, 1024**2)
    schedule, _ = read_document(output / 'schedule.json')
    history, _ = read_document(output / 'loss-history.json')
    if schedule != window_schedule(request['regions']) or not isinstance(history, list) or len(history) != 200:
        raise ValueError('incomplete or altered fitting schedule/history')
    for step, loss in zip(schedule, history):
        fields(loss, set(step) | {'loss_normalized_mse'}, 'loss history')
        value = loss['loss_normalized_mse']
        if (any(loss[k] != v for k, v in step.items()) or type(value) not in (float, int)
                or not math.isfinite(value) or value < 0):
            raise ValueError('invalid loss history')
    from safetensors.numpy import load_file
    shapes = {}
    for index, channels in [(0, (16, 1, 3, 3)), (2, (32, 16, 4, 4)), (4, (4, 32, 4, 4)),
                            (5, (32, 4, 3, 3)), (8, (16, 32, 3, 3)), (11, (1, 16, 3, 3))]:
        shapes[f'{index}.weight'], shapes[f'{index}.bias'] = channels, (channels[0],)
    for stage in ['initial', 'final']:
        tensors = load_file(output / f'{stage}.safetensors')
        if set(tensors) != set(shapes) or any(tensors[n].shape != shape or tensors[n].dtype != np.float32
                                            or not np.isfinite(tensors[n]).all() for n, shape in shapes.items()):
            raise ValueError('invalid scalar checkpoint')
    if not isinstance(result['cases'], list) or len(result['cases']) != len(request['regions']) + 4:
        raise ValueError('scalar fit worker omitted cases')
    for row, (info, height, valid, water) in zip(result['cases'], cases(request, root, SIZE)):
        fields(row, set(info) | {'metrics', 'artifacts'}, 'scalar fit case')
        if any(row[k] != v for k, v in info.items()):
            raise ValueError('scalar fit case identity mismatch')
        fields(row['artifacts'], {'original', 'valid', 'water', 'initial', 'final'}, 'scalar fit arrays')
        arrays = {}
        for name, description in row['artifacts'].items():
            dtype = np.dtype('u1' if name in {'valid', 'water'} else '<f4')
            path = checked_artifact(output, description, f"{info['id']}-{name}.bin", SIZE * SIZE * dtype.itemsize)
            arrays[name] = np.frombuffer(path.read_bytes(), dtype=dtype).reshape(SIZE, SIZE)
        if (arrays['original'].tobytes() != height.astype('<f4').tobytes()
                or not np.array_equal(arrays['valid'], valid.astype('u1'))
                or not np.array_equal(arrays['water'], water.astype('u1'))):
            raise ValueError('fitting original/mask arrays changed')
        stats = {stage: measurements(height, arrays[stage], valid, water, info['spacing_m']) for stage in ['initial', 'final']}
        direct = height / np.float32(2048) * np.float32(2048)
        stats['direct_scalar'] = measurements(height, direct, valid, water, info['spacing_m'])
        if row['metrics'] != stats:
            raise ValueError('fitting measurements disagree with retained arrays')
    if sum(p.stat().st_size for p in output.iterdir()) > DISK_BYTES:
        raise ValueError('fitting artifact disk budget exceeded')
    return {'schema_version': 1, 'kind': 'verified_scalar_codec_fit', 'job_sha256': identities['job_sha256'],
            'worker_result_sha256': digest(result_bytes), 'fitting_release_sha256': job['fitting_release']['sha256'],
            'numeric_run_sha256': job['numeric_run']['sha256'], 'gpu_used': False,
            'production_training_admission': 'not admitted', 'human_acceptance': None}
