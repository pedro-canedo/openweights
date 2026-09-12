"""One queue owner, sequential CUDA stages, CPU export and atomic publication."""
import gc
import hashlib
import json
import os
import shutil
from pathlib import Path

from . import store
from .studio.pipeline import digest


def file_hash(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def run(job, folder, check_cancel):
    from . import worker
    import torch
    check_cancel()
    config = dict(job['config'])
    integrity = json.loads((folder/'integrity.json').read_text(encoding='utf-8'))
    for name in ('dataset', 'config'):
        if digest(json.loads((folder/f'{name}.json').read_text(encoding='utf-8'))) != integrity[name]:
            raise ValueError('O snapshot foi alterado. Prepare uma nova versão antes de treinar.')
    if not job.get('resume') and not job.get('resume_export'):
        store.update('jobs', job['id'], {'stage': 'benchmark'})
        benchmark = folder/'benchmark'
        benchmark.mkdir(exist_ok=True)
        small = dict(config, max_steps=1, save_steps=1, one_epoch=False, benchmark=False)
        saved = json.loads((folder/'config.json').read_text(encoding='utf-8'))
        store.write_json(benchmark/'config.json', saved | small)
        shutil.copy2(folder/'dataset.json', benchmark/'dataset.json')
        try:
            worker.train(dict(job, config=small), benchmark)
        except torch.cuda.OutOfMemoryError as exc:
            raise ValueError('Esta receita não coube na GPU. Escolha contexto 512 e tente novamente.') from exc
        check_cancel()
        result = store.get('jobs', job['id']).get('result', {})
        store.write_json(folder/'benchmark.json', result)
        store.update('jobs', job['id'], {'benchmark': result, 'result': None, 'artifact': None})
        gc.collect()
        torch.cuda.empty_cache()
    if job.get('resume_export'):
        complete = json.loads((folder/'training-complete.json').read_text(encoding='utf-8'))
        for name, expected in complete['artifact_sha256'].items():
            target = (folder/'artifact'/name).resolve()
            if not target.is_relative_to((folder/'artifact').resolve()) or not target.is_file() or file_hash(target) != expected:
                raise ValueError('O artefato de treino foi alterado. Restaure uma cópia íntegra antes de exportar.')
    else:
        store.update('jobs', job['id'], {'stage': 'training'})
        worker.train(job, folder)
    check_cancel()
    gc.collect()
    torch.cuda.empty_cache()
    store.update('jobs', job['id'], {'stage': 'exporting'})
    # Exporter's source is a completed train record; the workflow remains running.
    source_id = digest(['workflow-source', job['id']])[:32]
    source = store.get('jobs', job['id'])
    record = dict(source, id=source_id, kind='train', status='completed', source_workflow=job['id'])
    try:
        store.get('jobs', source_id)
        store.update('jobs', source_id, record)
    except KeyError:
        store.create('jobs', record)
    export = folder/'export'
    export.mkdir(exist_ok=True)
    worker.gguf_export(dict(job, config={'run_id': source_id, 'quantization': 'Q4_K_M'}), export)
    check_cancel()
    result = store.get('jobs', job['id'])['result']
    publish(job, folder, result)


def publish(job, folder, result):
    artifact = Path(result['artifact'])
    target_root = Path(os.environ.get('OW_MODELS_DIR', str(store.ROOT/'exports'))).resolve()
    parent = target_root/'studio'
    parent.mkdir(parents=True, exist_ok=True)
    destination = parent/job['id']
    # Same volume for atomic rename, outside the recursively scanned model library.
    staging = target_root.parent/'.studio-staging'/job['id']
    staging.mkdir(parents=True, exist_ok=True)
    copied = staging/artifact.name
    shutil.copy2(artifact, copied)
    if file_hash(copied) != result['sha256']:
        raise ValueError('A cópia do modelo falhou na verificação. Tente exportar novamente.')
    with copied.open('rb') as stream:
        if stream.read(4) != b'GGUF':
            raise ValueError('O conversor não produziu um GGUF válido.')
    manifest = json.loads((folder/'manifest.json').read_text(encoding='utf-8'))
    store.write_json(staging/'manifest.json', manifest | result | {'artifact': artifact.name, 'workflow_id': job['id']})
    if destination.exists():
        if file_hash(destination/artifact.name) != result['sha256']:
            raise ValueError('Já existe um modelo diferente neste destino. Preserve-o e inicie uma nova exportação.')
        shutil.rmtree(staging)
    else:
        staging.replace(destination)
    store.update('jobs', job['id'], {'stage': 'ready', 'artifact': str(destination/artifact.name),
                                  'chat_model': artifact.name})
