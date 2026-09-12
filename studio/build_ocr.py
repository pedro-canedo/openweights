"""Assemble a private Windows OCR bundle from a locked conda-forge environment."""
import argparse
import hashlib
import json
import shutil
import subprocess
import urllib.request
import zipfile
from pathlib import Path

TESSDATA_REV = '87416418657359cb625c412a48b6e1d6d41c29bd'


def build(environment, cache, output, version):
    environment, cache, output = map(lambda p: Path(p).resolve(), (environment, cache, output))
    output.mkdir(parents=True, exist_ok=False)
    for name in ('bin', 'share', 'etc', 'ssl'):
        if (environment/'Library'/name).exists():
            shutil.copytree(environment/'Library'/name, output/name)
    # CRT dependencies can live outside Library/bin in conda environments.
    for item in environment.glob('*.dll'):
        shutil.copy2(item, output/'bin'/item.name)
    packages = []
    licenses = output/'licenses'
    licenses.mkdir()
    for metadata in sorted((environment/'conda-meta').glob('*.json')):
        record = json.loads(metadata.read_text())
        packages.append({k: record.get(k) for k in ('name', 'version', 'build', 'url', 'sha256', 'license')})
    for directory in cache.rglob('info/licenses'):
        shutil.copytree(directory, licenses/directory.parent.parent.name, dirs_exist_ok=True)
    models = {}
    for language in ('por', 'eng'):
        path = output/'share/tessdata'/f'{language}.traineddata'
        url = f'https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/{TESSDATA_REV}/{path.name}'
        urllib.request.urlretrieve(url, path)
        models[language] = {'url': url, 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}
    urllib.request.urlretrieve(f'https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/{TESSDATA_REV}/LICENSE', licenses/'tessdata-LICENSE')
    for executable in ('tesseract.exe', 'pdftoppm.exe'):
        result = subprocess.run([str(output/'bin'/executable), '--version' if executable == 'tesseract.exe' else '-v'], capture_output=True, check=True)
        print((result.stdout + result.stderr).decode('utf-8', errors='replace'))
    (output/'runtime.json').write_text(json.dumps({'version': version, 'packages': packages, 'models': models}, indent=2))
    archive = output.parent/f'openweights-studio-ocr-{version}-windows-x64.zip'
    files = sorted(p for p in output.rglob('*') if p.is_file())
    with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED, compresslevel=6) as target:
        for file in files:
            target.write(file, file.relative_to(output).as_posix())
    with archive.open('rb') as stream:
        sha = hashlib.file_digest(stream, 'sha256').hexdigest()
    catalog = {'version': version, 'url': f'https://github.com/pedro-canedo/openweights/releases/download/studio-runtime-v1/{archive.name}',
               'size': archive.stat().st_size, 'expanded_size': sum(p.stat().st_size for p in files), 'sha256': sha}
    (output.parent/'ocr-windows-x64.json').write_text(json.dumps(catalog, indent=2))
    print(json.dumps(catalog))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    for name in ('environment', 'cache', 'output', 'version'):
        parser.add_argument('--'+name, required=True)
    build(**vars(parser.parse_args()))
