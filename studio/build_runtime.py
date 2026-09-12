"""Release engineering: assemble a relocatable runtime, never modify system Python.

Run with the private environment after installing runtime-lock.txt. Supply the
verified llama.cpp source and its Windows CPU build. Sign the resulting catalog
with the release minisign key outside this script; no private keys are read here.
"""
import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path


def build(output, source, binaries, version, archive=True):
    output, source, binaries = (Path(p).resolve() for p in (output, source, binaries))
    if output.exists():
        raise ValueError('Use a new empty output directory; existing runtimes are immutable.')
    project = Path(__file__).parent
    output.mkdir(parents=True)
    python = output/'python'
    shutil.copytree(sys.base_prefix, python, ignore=shutil.ignore_patterns('__pycache__', 'site-packages', 'Scripts', 'include', 'libs', 'tcl'))
    shutil.copytree(Path(sys.prefix)/'Lib/site-packages', python/'Lib/site-packages',
                    ignore=shutil.ignore_patterns('__pycache__', 'tests', 'pytest*', '_pytest', 'pip', 'pip-*'))
    (python/'python312._pth').write_text('Lib\nDLLs\nLib/site-packages\n..\n../llama.cpp\n../llama.cpp/gguf-py\nimport site\n', encoding='utf-8')
    shutil.copytree(project/'app', output/'app', ignore=shutil.ignore_patterns('__pycache__'))
    dist = project.parent/'dist'
    if not (dist/'studio.html').is_file():
        raise ValueError('Execute npm run build antes de empacotar a interface independente.')
    shutil.copy2(dist/'studio.html', output/'app/static/studio.html')
    shutil.copytree(dist/'assets', output/'app/static/assets')
    llama = output/'llama.cpp'
    llama.mkdir()
    for name in ('convert_hf_to_gguf.py', 'conversion', 'gguf-py'):
        item = source/name
        if item.is_dir():
            shutil.copytree(item, llama/name, ignore=shutil.ignore_patterns('__pycache__'))
        else:
            shutil.copy2(item, llama/name)
    for item in binaries.iterdir():
        if item.suffix in ('.dll', '.exe') and (item.suffix == '.dll' or item.name == 'llama-quantize.exe'):
            shutil.copy2(item, llama/item.name)
    shutil.copy2(project/'LICENSE', output/'LICENSE-Studio')
    shutil.copy2(project/'runtime-lock.txt', output/'runtime-lock.txt')
    shutil.copy2(source/'LICENSE', llama/'LICENSE')
    result = subprocess.run([str(python/'python.exe'), '-c',
        'import torch, peft, trl, bitsandbytes, app.main; print(torch.__version__)'],
        cwd=output, capture_output=True, text=True, check=True)
    metadata = {'version': version, 'python': sys.version, 'torch': result.stdout.strip(),
                'llama_cpp_ref': 'b95502ba9aa0eb73a2f4fc8878d7fbe6a847a0b9'}
    (output/'runtime.json').write_text(json.dumps(metadata, indent=2), encoding='utf-8')
    if archive:
        path = output.parent/f'openweights-studio-{version}-windows-x64.zip'
        files = sorted(p for p in output.rglob('*') if p.is_file() and '__pycache__' not in p.parts)
        with zipfile.ZipFile(path, 'w', zipfile.ZIP_DEFLATED, compresslevel=9) as z:
            for file in files:
                z.write(file, file.relative_to(output).as_posix())
        with path.open('rb') as stream:
            sha = hashlib.file_digest(stream, 'sha256').hexdigest()
        catalog = {'version': version, 'url': f'https://github.com/pedro-canedo/openweights/releases/download/studio-runtime-v1/{path.name}',
                   'sha256': sha, 'size': path.stat().st_size, 'expanded_size': sum(p.stat().st_size for p in files)}
        path.with_name('windows-x64.json').write_text(json.dumps(catalog, indent=2), encoding='utf-8')
    print(json.dumps(metadata))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', required=True)
    parser.add_argument('--llama-source', required=True)
    parser.add_argument('--llama-binaries', required=True)
    parser.add_argument('--version', default='1.0.0-alpha.1')
    parser.add_argument('--no-archive', action='store_true')
    args = parser.parse_args()
    build(args.output, args.llama_source, args.llama_binaries, args.version, not args.no_archive)
