"""Versioned desktop contract. Legacy routes remain available."""
import json
import shutil
import threading
from typing import Literal

from fastapi import APIRouter
from fastapi.responses import FileResponse
from pydantic import BaseModel, Field

from . import store, runtime, catalog
from .guided import TOO_SHORT
from .schemas import TrainInput
from .studio.pipeline import digest

router = APIRouter(prefix='/api/v1')
lock = threading.RLock()
BASE = {'name': 'Qwen3 0.6B', 'source': 'hub', 'repo': 'Qwen/Qwen3-0.6B',
        'revision': 'c1899de289a04d12100db370d81485cdf75e47ca', 'status': 'registered'}


class Prepare(BaseModel):
    source_ids: list[str] = Field(min_length=1, max_length=1000)
    name: str = Field(default='Meu modelo', min_length=1, max_length=100)
    preset: Literal['auto', 'documents', 'conversations', 'code'] = 'auto'


class Run(BaseModel):
    dataset_id: str
    request_id: str = Field(min_length=8, max_length=100)
    context: Literal[512, 1024, 2048, 4096] = 1024
    model_id: str = catalog.DEFAULT_MODEL
    recipe_id: Literal['quick', 'recommended'] = 'quick'
    project_id: str | None = None
    rank: Literal[8, 16, 32] = 16
    learning_rate: float = Field(default=1e-4, ge=1e-6, le=0.001)
    max_steps: int | None = Field(default=None, ge=1, le=2000)
    max_minutes: int = Field(default=30, ge=1, le=1440)


class Project(BaseModel):
    id: str = Field(pattern=r'^[a-f0-9]{32}$')
    name: str = Field(min_length=1, max_length=100)
    source_ids: list[str] = Field(default_factory=list, max_length=1000)
    dataset_id: str | None = None
    preparation_id: str | None = None
    model_id: str = catalog.DEFAULT_MODEL
    recipe_id: Literal['quick', 'recommended'] = 'quick'
    preset: Literal['auto', 'documents', 'conversations', 'code'] = 'auto'
    context: Literal[512, 1024, 2048, 4096] = 1024
    rank: Literal[8, 16, 32] = 16
    learning_rate: float = Field(default=1e-4, ge=1e-6, le=0.001)
    max_minutes: int = Field(default=30, ge=1, le=1440)


class ImportModel(BaseModel):
    source: Literal['hub', 'local']
    location: str = Field(min_length=1, max_length=1000)


class Comparison(BaseModel):
    request_id: str = Field(min_length=8, max_length=100)
    prompt: str = Field(min_length=1, max_length=8000)


@router.post('/runs/{ident}/compare')
def compare(ident: str, body: Comparison):
    with lock:
        source = store.get('jobs', ident)
        if source['kind'] != 'workflow' or source['status'] != 'completed':
            raise ValueError('Conclua o treinamento antes de comparar.')
        if source['config'].get('runtime') != runtime.runtime_id():
            raise ValueError('Use a versão original do Studio e do runtime desta execução para comparar.')
        config = {'run_id': ident, 'prompt': body.prompt, 'system': 'Responda em português de forma clara.',
                  'max_tokens': 200, 'temperature': 0}
        fingerprint = digest(config)
        old = next((j for j in store.all_records('jobs') if j.get('request_id') == body.request_id), None)
        if old:
            if old.get('request_fingerprint') != fingerprint:
                raise ValueError('Esta solicitação já pertence a outra comparação.')
            return old
        job = store.create('jobs', {'kind': 'comparison', 'name': source['name'], 'config': config,
                'project_id': source.get('project_id'), 'request_id': body.request_id,
                'request_fingerprint': fingerprint, 'status': 'queued', 'cancel_requested': False})
        return job


@router.get('/models')
def models():
    return catalog.available()


@router.post('/models/import')
def import_model(body: ImportModel):
    with lock:
        try:
            return catalog.inspect_import(body.source, body.location)
        except (OSError, KeyError) as exc:
            raise ValueError('Não foi possível conferir a base. Verifique a pasta ou o identificador do Hugging Face, a conexão e o acesso ao modelo.') from exc


@router.get('/projects')
def projects():
    return store.all_records('projects')


@router.get('/sources')
def sources():
    return [{'id': s['id'], 'name': s.get('name', s.get('filename', 'Arquivo'))} for s in store.all_records('sources')]


@router.post('/projects')
def save_project(body: Project):
    with lock:
        catalog.resolve(body.model_id)
        for ident in body.source_ids:
            store.get('sources', ident)
        if body.dataset_id:
            store.get('datasets', body.dataset_id)
        if body.preparation_id:
            store.get('jobs', body.preparation_id)
        record = body.model_dump()
        try:
            old = store.get('projects', body.id)
            versions = list(old.get('dataset_versions', []))
            if body.dataset_id and body.dataset_id not in versions:
                versions.append(body.dataset_id)
            return store.update('projects', body.id, record | {'dataset_versions': versions})
        except KeyError:
            return store.create('projects', record | {'dataset_versions': [body.dataset_id] if body.dataset_id else []})


@router.get('/datasets/{ident}/preview')
def preview(ident: str):
    store.get('datasets', ident)
    rows = json.loads((store.ROOT/'datasets'/f'{ident}.json').read_text(encoding='utf-8'))
    # Never expose held-out examples in the tuning UI.
    return {'examples': [r for r in rows if r['split'] == 'train'][:3],
            'manifest': json.loads((store.ROOT/'datasets'/ident/'manifest.json').read_text(encoding='utf-8'))}


class ImportLegacy(BaseModel):
    path: str


@router.post('/import-legacy')
def import_legacy(body: ImportLegacy):
    from .migration import import_legacy as copy
    from .jobs import queue
    with lock:
        if any(j['status'] in ('queued', 'running', 'cancelling', 'preparing') for j in store.all_records('jobs')):
            raise ValueError('Aguarde os trabalhos atuais terminarem antes de importar.')
        queue.stop()
        try:
            return copy(body.path)
        finally:
            queue.stopping.clear()
            queue.start()


@router.get('/capabilities')
def capabilities():
    from .main import system
    store.ROOT.mkdir(parents=True, exist_ok=True)
    return system() | {'api_version': 1, 'studio_contract': 2, 'runtime': runtime.runtime_id(), 'base': BASE,
                       'recipes': catalog.RECIPES, 'presets': ['documents', 'conversations', 'code']}


@router.post('/prepare')
def prepare(body: Prepare):
    with lock:
        config = body.model_dump()
        for ident in body.source_ids:
            if store.get('sources', ident)['status'] != 'ready':
                raise ValueError('Aguarde a importação dos arquivos terminar.')
        key = digest(config)
        old = next((j for j in store.all_records('jobs') if j.get('request_key') == key and j['status'] not in ('failed', 'cancelled', 'interrupted')), None)
        if old:
            return old
        return store.create('jobs', {'kind': 'guided_prepare', 'name': body.name, 'config': config,
                                    'request_key': key, 'status': 'queued', 'cancel_requested': False})


@router.get('/datasets')
def datasets():
    return store.all_records('datasets')


@router.post('/preflight')
def preflight(body: Run):
    dataset = store.get('datasets', body.dataset_id)
    counts = dataset.get('split_counts', {})
    if not dataset.get('immutable') or not all(counts.get(p) for p in ('train', 'validation', 'test')):
        raise ValueError(TOO_SHORT)
    model = catalog.resolve(body.model_id)
    if dataset['kind'] == 'chat' and model.get('chat_template') is False:
        raise ValueError('Esta base não tem template de conversa. Escolha outra base para conversas.')
    if body.project_id:
        store.get('projects', body.project_id)
    estimated = catalog.resources(model, body.context, body.rank)
    caps = capabilities()
    if not caps['gpu']:
        raise ValueError('Não encontramos uma GPU NVIDIA disponível. Confira o driver NVIDIA.')
    gpu = caps['gpu']
    if gpu['total_mb'] - gpu['used_mb'] < estimated['vram_estimated_mb']:
        raise ValueError('A GPU está com pouca memória livre. Feche outros programas que usam a GPU e tente novamente.')
    if caps['disk_free'] < estimated['disk_required_bytes']:
        raise ValueError(f"Libere {estimated['disk_required_bytes']/1024**3:.1f} GB no disco para esta base e exportação.")
    import psutil
    if psutil.virtual_memory().available < estimated['ram_required_bytes']:
        raise ValueError(f"Libere {estimated['ram_required_bytes']/1024**3:.1f} GB de RAM ou escolha uma base menor.")
    if not caps['gguf']['available']:
        raise ValueError('Instale ou repare as ferramentas GGUF do módulo de treinamento.')
    return {'ready': True, 'base': model, 'context': body.context, 'benchmark_required': True,
            **estimated, 'recipe': catalog.recipe(body, dataset, model), 'dataset': dataset}


@router.post('/runs')
def create_run(body: Run):
    with lock:
        old = next((j for j in store.all_records('jobs') if j.get('request_id') == body.request_id), None)
        if old:
            if old.get('request_fingerprint', digest({'dataset_id': old['config']['dataset_id'], 'context': old['config']['context']})) != digest(body.model_dump(exclude={'request_id'})):
                raise ValueError('Esta solicitação já pertence a outra receita. Inicie uma nova solicitação.')
            return old
        validation = preflight(body)
        dataset = store.get('datasets', body.dataset_id)
        rows = json.loads((store.ROOT/'datasets'/f'{body.dataset_id}.json').read_text(encoding='utf-8'))
        if digest(rows) != dataset['sha256']:
            raise ValueError('O dataset mudou no disco. Prepare uma nova versão.')
        base = catalog.resolve(body.model_id)
        model_id = digest(base)[:32]
        try:
            model = store.get('models', model_id)
        except KeyError:
            model = store.create('models', base | {'id': model_id})
        config = TrainInput(name=dataset['name'], mode='qlora' if dataset['kind'] == 'chat' else 'continued',
                            dataset_id=body.dataset_id, model_id=model_id, **{k:v for k,v in validation['recipe'].items() if k in TrainInput.model_fields}).model_dump()
        config.update(validation['recipe'])
        config['project_id'] = body.project_id
        job = store.create('jobs', {'kind': 'workflow', 'name': dataset['name'], 'config': config,
                                  'request_id': body.request_id, 'request_fingerprint': digest(body.model_dump(exclude={'request_id'})),
                                  'project_id': body.project_id, 'status': 'preparing', 'cancel_requested': False})
        folder = store.run_dir(job['id'])
        snapshot = {p: [r for r in rows if r['split'] == p] for p in ('train', 'validation', 'test')}
        store.write_json(folder/'dataset.json', snapshot)
        store.write_json(folder/'config.json', config | {'model': model})
        store.write_json(folder/'integrity.json', {'dataset': digest(snapshot), 'config': digest(config | {'model': model})})
        return store.update('jobs', job['id'], {'status': 'queued'})


@router.get('/runs')
def runs():
    return [j for j in store.all_records('jobs') if j['kind'] in ('workflow', 'guided_prepare', 'comparison')]


@router.get('/runs/{ident}')
def detail(ident: str):
    from .main import job_detail
    return job_detail(ident)


@router.get('/runs/{ident}/download')
def download(ident: str):
    import os
    from pathlib import Path
    from .workflow import file_hash
    job = store.get('jobs', ident)
    if job['kind'] != 'workflow' or job['status'] != 'completed' or not job.get('chat_model'):
        raise ValueError('Aguarde a exportação terminar para baixar o modelo.')
    root = Path(os.environ.get('OW_MODELS_DIR', str(store.ROOT/'exports'))).resolve()
    path = Path(job['artifact']).resolve()
    if not path.is_relative_to(root) or not path.is_file() or file_hash(path) != job['result']['sha256']:
        raise ValueError('O modelo não passou na verificação. Exporte novamente uma cópia íntegra.')
    return FileResponse(path, filename=path.name, media_type='application/octet-stream')


@router.get('/runs/{ident}/manifest')
def manifest(ident: str):
    job = store.get('jobs', ident)
    if job['kind'] != 'workflow' or job['status'] != 'completed':
        raise ValueError('Aguarde a conclusão para baixar o manifesto.')
    return FileResponse(store.run_dir(ident)/'manifest.json', filename=f'{ident}-manifest.json')


@router.post('/runs/{ident}/cancel')
def cancel(ident: str):
    from .main import cancel as legacy_cancel
    return legacy_cancel(ident)


@router.post('/preparations/{ident}/resume')
def resume_preparation(ident: str):
    if store.get('jobs', ident)['kind'] != 'guided_prepare':
        raise ValueError('Escolha uma preparação interrompida.')
    return resume(ident)


@router.post('/runs/{ident}/resume')
def resume(ident: str):
    with lock:
        job = detail(ident)
        if job['status'] not in ('failed', 'cancelled', 'interrupted'):
            raise ValueError('Este trabalho não está interrompido.')
        if job['kind'] == 'guided_prepare':
            return store.update('jobs', ident, {'status': 'queued', 'cancel_requested': False, 'error': None})
        if job['config'].get('runtime') != runtime.runtime_id():
            raise ValueError('Use a versão original do Studio e do runtime para retomar este treino. Os checkpoints permanecem preservados.')
        folder = store.run_dir(ident)
        integrity = json.loads((folder/'integrity.json').read_text(encoding='utf-8'))
        for name in ('config', 'dataset'):
            if digest(json.loads((folder/f'{name}.json').read_text(encoding='utf-8'))) != integrity[name]:
                raise ValueError('O snapshot foi alterado. Inicie um novo treinamento com uma versão íntegra.')
        if (folder/'training-complete.json').is_file():
            return store.update('jobs', ident, {'status': 'queued', 'cancel_requested': False, 'resume_export': True, 'error': None})
        if not job['checkpoints']:
            raise ValueError('Ainda não existe checkpoint completo. Inicie um novo treinamento.')
        from .workflow import file_hash
        checkpoint = folder/job['checkpoints'][-1]
        marker = json.loads((checkpoint/'complete.json').read_text(encoding='utf-8'))
        if marker['runtime'] != runtime.runtime_id():
            raise ValueError('Instale o runtime original do checkpoint para retomar.')
        for name, expected in marker['sha256'].items():
            target = (checkpoint/name).resolve()
            if not target.is_relative_to(checkpoint.resolve()) or not target.is_file() or file_hash(target) != expected:
                raise ValueError('O checkpoint está incompleto ou foi alterado. Use um checkpoint íntegro.')
        return store.update('jobs', ident, {'status': 'queued', 'cancel_requested': False,
                                          'resume': job['checkpoints'][-1], 'error': None})
