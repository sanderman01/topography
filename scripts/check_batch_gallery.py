#!/usr/bin/env python3
"""Exercise list-driven acquisition and gallery decisions through public consumers."""
import argparse
import copy
import json
from pathlib import Path
import shutil
import tempfile
import http.client
import re
import subprocess

from check_acquisition import FIXTURES, ROOT, run, runtime_file, sha


def check(cli, directory, browser=None, local_only=False):
    runtime = runtime_file(directory)
    source = directory / 'source.toml'
    shutil.copyfile(FIXTURES / 'source.toml', source)
    shutil.copyfile(FIXTURES / 'metadata.json', directory / 'metadata.json')
    asset = directory / 'ramp.tif'
    run('gdal_translate', '-q', '-of', 'GTiff', '-ot', 'Float32', '-a_srs', 'EPSG:32619', FIXTURES / 'ramp.asc', asset)
    receipt = Path(run(cli, 'import', source, asset, runtime, 1048576))
    queue = {'schema_version': 1, 'max_total_bytes': 2097152, 'sources': [
        {'id': 'ramp', 'source': 'source.toml', 'existing_receipt': str(receipt), 'max_bytes': 1048576,
         'crops': [{'id': 'first', 'title': 'First <terrain>', 'window_cells': [0, 0, 3, 3], 'primary_class_id': 1},
                   {'id': 'second', 'title': 'Second terrain', 'window_cells': [1, 1, 3, 3], 'primary_class_id': 2}]}]}
    path = directory / 'queue.json'
    path.write_text(json.dumps(queue))
    run(cli, 'batch', 'validate', path)
    # Failed acquisition must not silently adopt replacement metadata on resume.
    metadata = directory/'metadata.json'; original_metadata = metadata.read_bytes()
    metadata.write_bytes(b'{"changed":true}')
    failed_output = directory/'failed-metadata-run'
    run(cli,'batch','download',path,runtime,failed_output,expected=1)
    metadata.write_bytes(original_metadata)
    run(cli,'batch','download',path,runtime,failed_output,expected=1,message='metadata')
    for change, message in [
        (lambda q: q['sources'].append(copy.deepcopy(q['sources'][0])), 'duplicate'),
        (lambda q: q.update(max_total_bytes=1), 'budget'),
        (lambda q: q['sources'][0]['crops'][0].update(id='../escape'), 'identifier'),
        (lambda q: q.update(typo=True), 'unknown field')]:
        bad = copy.deepcopy(queue); change(bad)
        invalid = directory / 'invalid.json'; invalid.write_text(json.dumps(bad))
        run(cli, 'batch', 'validate', invalid, expected=1, message=message)
    output = directory / 'batch run'
    for _ in range(2):
        report = json.loads(run(cli, 'batch', 'download', path, runtime, output))
        assert report['failed'] == 0 and report['completed'] == 1
    for _ in range(2):
        report = json.loads(run(cli, 'batch', 'review', path, runtime, output))
        assert report['failed'] == 0 and report['completed'] == 2
    inputs = json.loads((output / 'gallery-input.json').read_text())
    assert len(inputs['tiles']) == 2
    changed = source.read_bytes(); source.write_bytes(changed + b'\n# replacement\n')
    run(cli, 'batch', 'download', path, runtime, output, expected=1, message='differs')
    source.write_bytes(changed)
    review = Path(inputs['tiles'][0]['review'])
    preview = review.parent / 'preview.html'; before = preview.read_bytes(); preview.write_bytes(b'corrupt')
    report = json.loads(run(cli, 'batch', 'review', path, runtime, output, expected=1))
    assert report['failed'] == 1 and report['completed'] == 1
    preview.write_bytes(before)
    run(cli, 'batch', 'review', path, runtime, output)
    inputs = json.loads((output / 'gallery-input.json').read_text())
    draft = directory / 'annotation.json'
    run(cli,'curate','draft',runtime,receipt,inputs['tiles'][0]['review'],ROOT/'catalog/labels/terrain-v1.toml',draft)
    annotation = json.loads(draft.read_text())
    evidence = directory/'evidence.json'; evidence.write_text('{"synthetic":true}')
    annotation['claims'].update(actor={'kind':'assistant','name':'synthetic checker'},reason='Retain technical evidence',primary_class_id=1)
    annotation['evidence']=[{'path':str(evidence),'evidence':{'id':'fixture','description':'Synthetic evidence','source_url':None,'sha256':sha(evidence)}}]
    annotation['claims']['findings']['coordinates']={'state':'pass','note':'Synthetic coordinates checked','evidence_ids':['fixture']}
    draft.write_text(json.dumps(annotation)); run(cli,'curate','record',draft,runtime)
    gallery = directory / 'gallery'
    manifest = Path(run(cli, 'gallery', 'build', output / 'gallery-input.json', ROOT / 'catalog/labels/terrain-v1.toml', gallery))
    gallery_id = sha(manifest)
    # Thin native crops must not turn a thumbnail into a large raster job.
    thin_review = Path(run(cli,'review',receipt,directory/'thin-review','0,0,1,4'))
    thin_input=directory/'thin.json';thin_input.write_text(json.dumps({'schema_version':1,'title':'Thin fixture','tiles':[{'id':'thin','title':'Thin','raw':str(receipt),'review':str(thin_review),'primary_class_id':1}]}))
    run(cli,'gallery','build',thin_input,ROOT/'catalog/labels/terrain-v1.toml',directory/'thin-gallery')
    thumbnail_size = json.loads(run('gdalinfo','-json',directory/'thin-gallery/thumb-0.png'))['size']
    if local_only:
        assert max(thumbnail_size)<=192
        assert len(json.loads(manifest.read_text())['tiles']) == 2
        return runtime, output / 'gallery-input.json'
    process = subprocess.Popen([str(cli), 'gallery', 'serve', str(runtime), str(manifest), '0'], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    raw_asset = directory/'raw/objects'/json.loads(receipt.read_text())['asset_sha256']
    retained_asset = directory/'temporarily-retained-raw'
    try:
        url = process.stdout.readline().strip()
        assert url.startswith('http://127.0.0.1:'), (url, process.stderr.read())
        authority = url.removeprefix('http://').rstrip('/')
        def request(method, target, body=None, headers=None):
            connection = http.client.HTTPConnection(authority, timeout=10)
            connection.request(method, target, body=body, headers=headers or {})
            response = connection.getresponse(); data = response.read(); code = response.status
            connection.close()
            return code, data
        code, html = request('GET', '/'); assert code == 200
        token = re.search(r'"token":"([a-f0-9]+)"', html.decode())[1]
        code, data = request('GET', '/api/tiles'); assert code == 200
        state = json.loads(data); assert len(state['tiles']) == 2
        tile = state['tiles'][0]; assert tile['decision'] == 'pending'
        # The session verified this source already. Browsing/voting must use its
        # captured preview and journal, not reopen a potentially enormous raw raster.
        raw_asset.rename(retained_asset)
        assert request('GET','/api/tiles')[0] == 200, 'gallery state reread raw raster'
        assert request('GET', '/preview/0')[0] == 200
        assert request('GET', '/thumb/0')[0] == 200
        assert request('GET', '/../source.toml')[0] == 404
        assert request('GET', '/', headers={'Host':'attacker.invalid'})[0] == 400
        vote = {'index':0,'gallery_sha256':gallery_id,'review_sha256':tile['review_sha256'], 'previous_record_sha256':tile['head'],'decision':'accept'}
        headers = {'Origin':url.rstrip('/'),'Content-Type':'application/json','X-Topography-Token':token}
        assert request('POST','/api/decision',json.dumps(vote))[0] == 400
        bad_headers = dict(headers, Origin='https://attacker.invalid')
        assert request('POST','/api/decision',json.dumps(vote),bad_headers)[0] == 400
        code, data = request('POST','/api/decision',json.dumps(vote),headers); assert code == 200, data
        saved = json.loads(data); assert saved['decision'] == 'accept'
        code, data = request('GET','/api/tiles'); assert json.loads(data)['tiles'][0]['decision'] == 'accept'
        stale = dict(vote, decision='reject')
        assert request('POST','/api/decision',json.dumps(stale),headers)[0] == 409
        stale['previous_record_sha256'] = saved['head']
        code, data = request('POST','/api/decision',json.dumps(stale),headers); assert code == 200, data
        assert json.loads(data)['decision'] == 'reject'
        run(cli,'verify',receipt,expected=1)  # Numeric integrity still requires raw bytes.
        retained_asset.rename(raw_asset)
        current = json.loads(run(cli,'curate','status',runtime,tile['review_sha256']))
        assert current['claims']['decision'] == 'reject'
        assert current['claims']['findings'] == annotation['claims']['findings']
        assert current['evidence'][0]['sha256'] == sha(evidence)
        assert current['admission'] == 'not admitted'
        if browser:
            run('node', ROOT/'scripts/check_gallery_browser.mjs', browser, url, directory/'gallery.png')
    finally:
        if retained_asset.exists(): retained_asset.rename(raw_asset)
        process.terminate(); process.communicate(timeout=10)
    assert max(thumbnail_size)<=192
    image = gallery/'thumb-0.png'; image.write_bytes(b'corrupt')
    run(cli,'gallery','serve',runtime,manifest,0,expected=1,message='thumbnail integrity')
    return runtime, output / 'gallery-input.json'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--browser', type=Path, help='Optional Chromium executable for actual UI checks')
    parser.add_argument('--local-only', action='store_true', help='Skip HTTP serving when loopback sockets are unavailable')
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='topography batch gallery ') as temporary:
        check(args.binary.resolve(), Path(temporary), args.browser, args.local_only)
    print('Batch/gallery workflows passed' + (' (local artifacts only).' if args.local_only else ' (including HTTP decisions' + (' and Chromium).' if args.browser else ').')))


if __name__ == '__main__':
    main()
