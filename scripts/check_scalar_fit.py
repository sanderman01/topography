#!/usr/bin/env python3
"""Small scalar fitting checks: excluded cells, held-out roles and retained weights."""
import importlib.util
from pathlib import Path
import tempfile

assert importlib.util.find_spec('topography_training.scalar_model') is not None, 'scalar model workflow is not implemented'
import torch
from safetensors.torch import save_file, load_file
from topography_training.scalar_model import build_model, masked_loss, window_schedule

torch.set_num_threads(2)
torch.set_num_interop_threads(1)
torch.manual_seed(0)
torch.use_deterministic_algorithms(True)
model = build_model()
x = torch.linspace(-0.2, 0.2, 16 * 20).reshape(1, 1, 16, 20)
y = model(x)
assert y.shape == x.shape and y.dtype == torch.float32
# Loss must ignore excluded cells, including NaNs there, and gradients there are zero.
pred = torch.tensor([1., 100., 5., float('nan')], requires_grad=True)
target = torch.tensor([3., -100., 1., float('nan')])
valid = torch.tensor([True, False, True, False])
loss = masked_loss(pred, target, valid)
assert loss.item() == 10.0
loss.backward()
assert torch.equal(pred.grad, torch.tensor([-2., 0., 4., 0.]))
try:
    masked_loss(pred, target, torch.zeros(4, dtype=torch.bool))
except ValueError:
    pass
else:
    raise AssertionError('all-masked loss accepted')
regions = [{'prepared_sha256': 'a' * 64, 'role': 'calibration', 'grid': {'size': [1024, 1024]}},
           {'prepared_sha256': 'b' * 64, 'role': 'validation', 'grid': {'size': [1024, 1024]}}]
schedule = window_schedule(regions)
assert len(schedule) == 200 and all(r['prepared_sha256'] == 'a' * 64 for r in schedule)
assert all(0 <= r['column'] <= 512 and 0 <= r['row'] <= 512 for r in schedule)
assert schedule == window_schedule(regions)
try:
    window_schedule(regions[1:])
except ValueError:
    pass
else:
    raise AssertionError('validation-only fitting accepted')
# A real optimizer update changes weights; reload reconstructs exactly, no pickle.
optimizer = torch.optim.Adam(model.parameters(), lr=0.001)
optimizer.zero_grad()
masked_loss(y, x, torch.ones_like(x, dtype=torch.bool)).backward()
optimizer.step()
with torch.inference_mode():
    updated = model(x)
assert not torch.equal(updated, y)
with tempfile.TemporaryDirectory(prefix='scalar checkpoint ') as name:
    path = Path(name) / 'weights.safetensors'
    save_file(model.state_dict(), path)
    restored = build_model()
    restored.load_state_dict(load_file(path))
    with torch.inference_mode():
        assert torch.equal(restored(x), updated)
print('Scalar model checks passed: scalar shape, masked gradients, fit-only windows and checkpoint reload.')

assert importlib.util.find_spec('topography_training.fitting') is not None, 'scalar batch workflow is not implemented'
import numpy as np
from topography_training.fitting import measurements
height = np.arange(80 * 80, dtype=np.float32).reshape(80, 80)
valid = np.ones(height.shape, bool)
valid[0, 0] = False
water = ~valid
decoded = height.copy()
decoded[0, 0] = 90000
stats = measurements(height, decoded, valid, water, [0.5, 1.0])
assert stats['full']['max_height_error_m'] == 0 and stats['water_cells'] == 1
try:
    measurements(height, decoded * np.nan, valid, water, [0.5, 1.0])
except ValueError:
    pass
else:
    raise AssertionError('nonfinite output accepted')
print('Scalar batch metric checks passed.')

from topography_training.codec import supervise
import sys
with tempfile.TemporaryDirectory(prefix='scalar disk bound ') as name:
    output = Path(name)
    try:
        supervise([sys.executable, '-c', "from pathlib import Path; import sys; Path(sys.argv[1]).write_bytes(b'x' * 4096)", str(output / 'overflow')],
                  output / 'worker.log', 10, 1024, disk_bytes=2048)
    except RuntimeError as error:
        assert 'disk' in str(error)
    else:
        raise AssertionError('worker disk overflow accepted')
print('Scalar disk limit check passed.')

if len(sys.argv) == 3 and sys.argv[1] == '--job':
    import copy
    import json
    from topography_training.fitting import (SETTINGS, artifact, check_inputs, source_hashes,
                                              verify_output, SIZE)
    from topography_training.numeric import write_json
    from topography_training.reconstruction import cases, save_array
    from topography_training.scalar_fit import current_status, execute
    job_path = Path(sys.argv[2]).absolute()
    job, request, root, identities = check_inputs(job_path)
    status = current_status(job, request)
    assert status['current'] and len(status['regions']) == 12
    with tempfile.TemporaryDirectory(prefix='scalar completion fixture ') as name:
        output = Path(name)
        fixture_schedule = window_schedule(request['regions'])
        write_json(output / 'schedule.json', fixture_schedule)
        write_json(output / 'loss-history.json', [dict(row, loss_normalized_mse=0.0) for row in fixture_schedule])
        for stage in ['initial', 'final']:
            save_file(model.state_dict(), output / f'{stage}.safetensors')
        rows = []
        # Identity output is a completion-contract fixture, never a model-quality claim.
        for info, height, valid, water in cases(request, root, SIZE):
            arrays = {'original': height.astype('<f4'), 'valid': valid.astype('u1'),
                      'water': water.astype('u1'), 'initial': height.astype('<f4'), 'final': height.astype('<f4')}
            stats = measurements(height, height, valid, water, info['spacing_m'])
            rows.append(dict(info, artifacts={n: save_array(output / f"{info['id']}-{n}.bin", a) for n, a in arrays.items()},
                             metrics={n: copy.deepcopy(stats) for n in ['direct_scalar', 'initial', 'final']}))
        result = {'schema_version': 1, 'kind': 'scalar_codec_fit_worker_result',
                  'job_sha256': identities['job_sha256'], 'fitting_release_sha256': job['fitting_release']['sha256'],
                  'numeric_run_sha256': job['numeric_run']['sha256'], 'settings': SETTINGS,
                  'source_sha256': source_hashes(), 'cases': rows,
                  'environment': {'python': 'fixture', 'numpy': 'fixture', 'torch': 'fixture', 'safetensors': 'fixture',
                                  'platform': 'fixture', 'elapsed_seconds': 1, 'max_self_rss_kib': 1, 'rss_scope': 'fixture'},
                  'artifacts': {n: artifact(output / n) for n in ['schedule.json', 'loss-history.json',
                                                                'initial.safetensors', 'final.safetensors']}}
        result_path = output / 'worker-result.json'
        write_json(result_path, result)
        assert verify_output(job_path, output)['kind'] == 'verified_scalar_codec_fit'
        def rejected(call):
            try:
                call()
            except (ValueError, KeyError, OSError):
                return
            raise AssertionError('invalid fitting completion accepted')
        for edit in [lambda r: r['cases'].pop(), lambda r: r.update(environment={}),
                     lambda r: r['cases'][0]['metrics']['final']['full'].update(height_rmse_m=99),
                     lambda r: r['cases'][0]['artifacts']['final'].update(path='../escape.bin'),
                     lambda r: r['settings'].update(steps=201)]:
            broken = copy.deepcopy(result)
            edit(broken)
            result_path.write_text(json.dumps(broken))
            rejected(lambda: verify_output(job_path, output))
        result_path.write_text(json.dumps(result))
        schedule_path = output / 'schedule.json'
        altered = copy.deepcopy(fixture_schedule)
        altered[0]['prepared_sha256'] = next(r['prepared_sha256'] for r in request['regions'] if r['role'] == 'validation')
        schedule_path.write_text(json.dumps(altered))
        changed = copy.deepcopy(result)
        changed['artifacts']['schedule.json'] = artifact(schedule_path)
        result_path.write_text(json.dumps(changed))
        rejected(lambda: verify_output(job_path, output))
        # An evaluation release cannot authorize optimizer use, even with a correct reference hash.
        bad_job = copy.deepcopy(job)
        parent = Path(job['fitting_release']['path']).parent.parent / identities['dataset_sha256'] / 'manifest.json'
        bad_job['fitting_release'] = {'path': str(parent), 'sha256': identities['dataset_sha256']}
        bad_path = output / 'bad-job.json'
        write_json(bad_path, bad_job)
        rejected(lambda: execute(bad_path, output / 'should-not-exist'))
        assert not (output / 'should-not-exist').exists()
    print('Scalar real-input preflight passed: scoped current release, complete outputs and invalid completions.')
