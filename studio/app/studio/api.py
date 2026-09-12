import asyncio
import json
import os
import threading

from fastapi import APIRouter, File, Form, HTTPException, UploadFile
from fastapi.responses import FileResponse

from .. import store
from .extractors import FORMATS
from .pipeline import VERSION, digest
from .schemas import Publish, Recipe, RemoteSource
from .storage import save_source

router = APIRouter(prefix='/api/studio', tags=['Data Studio'])
publish_lock = threading.Lock()


@router.get('/capabilities')
def capabilities():
    import shutil
    return {'version': VERSION, 'formats': sorted(FORMATS), 'file_mb': 32, 'batch_mb': 256,
            'ocr': bool(shutil.which('pdftoppm') and shutil.which('tesseract')),
            'llm_configured': bool(os.getenv('LLM_API_KEY') or os.getenv('OPENROUTER_API_KEY')),
            'recipe_schema': Recipe.model_json_schema()}


@router.get('/sources')
def sources():
    return [s for s in store.all_records('sources') if s['status'] == 'ready']


@router.post('/sources')
async def upload(files: list[UploadFile] = File(default=[]), content: str = Form(''),
                 name: str = Form('literal.txt', max_length=512), license: str = Form('', max_length=200)):
    if len(files) > 1000:
        raise HTTPException(413, 'Limite de 1000 arquivos por envio. Use ZIP para pastas maiores.')
    result, total = [], len(content.encode())
    if content.strip():
        result.append(await asyncio.to_thread(save_source, name, content.encode(), license=license))
    for file in files:
        raw = await file.read(32 * 1024**2 + 1)
        total += len(raw)
        if total > 256 * 1024**2:
            raise HTTPException(413, 'Lote excede 256 MiB. As fontes anteriores foram preservadas na biblioteca.')
        result.append(await asyncio.to_thread(save_source, file.filename or 'arquivo.txt', raw, license=license))
        await file.close()
    if not result:
        raise ValueError('Selecione arquivos, uma pasta ou cole texto.')
    return result


def enqueue(kind, name, config):
    return store.create('jobs', {'kind': kind, 'name': name, 'config': config, 'status': 'queued', 'cancel_requested': False})


@router.post('/remote')
def remote(body: RemoteSource):
    from .network import public_target, repository_url
    for url in body.urls:
        public_target(repository_url(url, body.revision) if body.repository else url)
    return enqueue('acquire', 'Importar fontes públicas', body.model_dump())


@router.post('/prepare')
def prepare(body: Recipe):
    if body.tokenizer_model_id:
        store.get('models', body.tokenizer_model_id)
    for ident in body.source_ids:
        if store.get('sources', ident)['status'] != 'ready':
            raise ValueError('Fonte ainda não está disponível.')
    if body.parent_id:
        store.get('datasets', body.parent_id)
    return enqueue('prepare', body.name, body.model_dump())


@router.get('/preparations')
def preparations():
    return [r for r in store.all_records('jobs') if r['kind'] in ('prepare', 'acquire')]


@router.get('/preparations/{ident}')
def preview(ident: str, offset: int = 0, limit: int = 20, q: str = '', split: str = '', role: str = ''):
    job = store.get('jobs', ident)
    if job['kind'] != 'prepare':
        raise ValueError('Escolha uma preparação de dataset.')
    folder = store.run_dir(ident)
    manifest = folder/'data-manifest.json'
    if job['status'] != 'completed' or not manifest.exists():
        reports = folder/'files.json'
        return {'job': job, 'rows': [], 'total': 0, 'files': json.loads(reports.read_text(encoding='utf-8')) if reports.exists() else []}
    rows = json.loads((folder/'preview.json').read_text(encoding='utf-8'))
    rows = [r for r in rows if (not split or r['split'] == split) and (not role or (r.get('metadata') or {}).get('role') == role)
            and (not q or q.lower() in json.dumps(r, ensure_ascii=False).lower())]
    offset, limit = max(0, offset), max(1, min(100, limit))
    return {'job': job, 'manifest': json.loads(manifest.read_text(encoding='utf-8')), 'rows': rows[offset:offset+limit], 'total': len(rows)}


@router.post('/preparations/{ident}/resume')
def resume(ident: str):
    job = store.get('jobs', ident)
    if job['kind'] != 'prepare' or job['status'] not in ('failed', 'cancelled', 'interrupted'):
        raise ValueError('Retomada exige uma preparação interrompida.')
    return store.update('jobs', ident, {'status': 'queued', 'cancel_requested': False, 'error': None})


@router.post('/preparations/{ident}/publish')
def publish(ident: str, body: Publish):
    with publish_lock:
        job = store.get('jobs', ident)
        if job['kind'] != 'prepare' or job['status'] != 'completed':
            raise ValueError('A preparação precisa terminar antes de publicar.')
        existing = next((d for d in store.all_records('datasets') if d.get('preparation_id') == ident), None)
        if existing:
            return existing
        folder = store.run_dir(ident)
        original = json.loads((folder/'preview.json').read_text(encoding='utf-8'))
        excluded = set(body.excluded_ids)
        if excluded - {r['id'] for r in original}:
            raise ValueError('Há exemplos desconhecidos na seleção de exclusão.')
        rows = [r for r in original if r['id'] not in excluded]
        counts = {key: sum(r['split'] == key for r in rows) for key in ('train', 'validation', 'test')}
        if not counts['train'] or not counts['validation'] or (job['config']['test'] and not counts['test']):
            raise ValueError('A revisão deixou um conjunto vazio. Desfaça exclusões ou crie outra receita.')
        manifest = json.loads((folder/'data-manifest.json').read_text(encoding='utf-8'))
        parent = job['config'].get('parent_id')
        parent_record = store.get('datasets', parent) if parent else None
        lineage = parent_record.get('lineage', parent) if parent_record else ident
        versions = [d.get('version', 1) for d in store.all_records('datasets') if d.get('lineage') == lineage]
        version = max(versions, default=0) + 1
        manifest.update(sha256=digest(rows), counts=counts, examples=len(rows), excluded_ids=sorted(excluded), version=version,
                        pipeline_version=VERSION, parent_id=parent, lineage=lineage, published_at=store.now())
        # Persist files before publishing the registry record so training never sees partial data.
        dataset_id = digest(['published-dataset', ident])[:32]
        target = store.ROOT/'datasets'/dataset_id
        target.mkdir(parents=True, exist_ok=True)
        store.write_json(store.ROOT/'datasets'/f'{dataset_id}.json', rows)
        store.write_json(target/'manifest.json', manifest)
        temporary = target/'dataset.jsonl.tmp'
        with temporary.open('w', encoding='utf-8') as stream:
            for row in rows:
                stream.write(json.dumps(row, ensure_ascii=False)+'\n')
        temporary.replace(target/'dataset.jsonl')
        record = store.create('datasets', {'id': dataset_id, 'name': job['config']['name'], 'kind': manifest['kind'],
            'count': len(rows), 'characters': sum(len(json.dumps(r, ensure_ascii=False)) for r in rows), 'last_duplicates': manifest['duplicates'],
            'preparation_id': ident, 'immutable': True, 'version': version, 'parent_id': parent, 'lineage': lineage,
            'split_counts': counts, 'sha256': manifest['sha256']})
        return record


@router.get('/datasets/{ident}/{artifact}')
def download(ident: str, artifact: str):
    record = store.get('datasets', ident)
    if not record.get('immutable') or artifact not in ('manifest.json', 'dataset.jsonl'):
        raise ValueError('Artefato indisponível.')
    return FileResponse(store.ROOT/'datasets'/ident/artifact, filename=artifact)
