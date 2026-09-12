"""Collect conda build recipes and corresponding copyleft sources for redistribution."""
import argparse
import hashlib
import json
import shutil
import subprocess
import urllib.request
import zipfile
from pathlib import Path
import yaml


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--environment', required=True)
    parser.add_argument('--cache', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    manifest = []
    for metadata in sorted((Path(args.environment)/'conda-meta').glob('*.json')):
        record = json.loads(metadata.read_text())
        package = f"{record['name']}-{record['version']}-{record['build']}"
        directories = list(Path(args.cache).rglob(package+'/info/recipe'))
        if not directories:
            if 'GPL' in record.get('license', ''): raise ValueError(f'Missing recipe: {package}')
            continue
        recipe = directories[0]
        shutil.copytree(recipe, output/'recipes'/package, dirs_exist_ok=True)
        if 'GPL' not in record.get('license', ''): continue
        rendered = recipe/'meta.yaml'
        if not rendered.exists(): rendered = recipe/'recipe.yaml'
        text = rendered.read_text(encoding='utf-8').replace('${{ version }}', record['version'])
        text = text.replace('${{ major_minor }}', '.'.join(record['version'].split('.')[:2]))
        document = yaml.safe_load(text)
        sources = document.get('source', [])
        if isinstance(sources, dict): sources = [sources]
        for source in sources:
            if 'url' not in source: continue
            urls = source['url'] if isinstance(source['url'], list) else [source['url']]
            expected = source.get('sha256')
            if not expected: raise ValueError(f'Missing source SHA256: {package}')
            destination = output/'sources'/package/urls[0].rsplit('/', 1)[-1]
            destination.parent.mkdir(parents=True, exist_ok=True)
            last_error = None
            for url in urls:
                try:
                    print(f'Downloading source: {url}', flush=True)
                    subprocess.run(['curl.exe', '--fail', '--location', '--retry', '2', '--output', str(destination), url], check=True, capture_output=True)
                    with destination.open('rb') as stream: actual = hashlib.file_digest(stream, 'sha256').hexdigest()
                    if actual != expected: raise ValueError(f'Source hash mismatch: {package}')
                    manifest.append({'package': package, 'url': url, 'sha256': actual})
                    last_error = None
                    break
                except Exception as exc: last_error = exc
            if last_error: raise last_error
    (output/'sources.json').write_text(json.dumps(manifest, indent=2))
    shutil.make_archive(str(output), 'zip', output)
    print(json.dumps(manifest))


if __name__ == '__main__': main()
