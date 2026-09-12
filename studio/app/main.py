import asyncio
import io
import json
import os
import re
import shutil
import subprocess
import threading
import zipfile
from contextlib import asynccontextmanager
from pathlib import Path
from urllib.parse import urlsplit

import psutil
from fastapi import FastAPI, File, Form, HTTPException, Request, UploadFile
from fastapi.exceptions import RequestValidationError
from fastapi.responses import FileResponse, JSONResponse
from fastapi.staticfiles import StaticFiles
from . import data, store, runtime
from .jobs import queue
from .schemas import GenerateInput, GgufExportInput, ModelInput, TrainInput

mutation_lock = threading.Lock()


@asynccontextmanager
async def lifespan(app):
    from .gpu_lease import exclusive_file
    with exclusive_file(store.ROOT/'service.lock', 'O Studio já está aberto para estes dados. Feche a outra instância antes de continuar.'):
        for p in ('datasets', 'runs', 'exports'):
            (store.ROOT / p).mkdir(parents=True, exist_ok=True)
        queue.start()
        try:
            yield
        finally:
            await asyncio.to_thread(queue.stop)


app = FastAPI(title='Open Weights Studio', version='2.2.1', lifespan=lifespan)


@app.post('/session')
async def session(request: Request):
    import secrets
    supplied = (await request.json()).get('token', '')
    expected = os.environ.get('OW_STUDIO_TOKEN', '')
    if not expected or not isinstance(supplied, str) or not secrets.compare_digest(supplied, expected):
        raise HTTPException(401, 'Abra o endereço informado pelo Studio para conectar.')
    response = JSONResponse({'ok': True})
    response.set_cookie('studio_session', expected, httponly=True, samesite='strict')
    return response


@app.middleware('http')
async def local_access(request: Request, call_next):
    # Local-only binding plus Origin/Host checks protect a localhost training service.
    host = request.headers.get('host', '').split(':')[0]
    if host not in ('localhost', '127.0.0.1', 'testserver'):
        return JSONResponse({'detail': 'Host nÃ£o autorizado.'}, status_code=403)
    import secrets
    token = os.environ.get('OW_STUDIO_TOKEN')
    if token and request.url.path.startswith('/api/') and request.url.path != '/api/health':
        supplied = request.headers.get('authorization', '').removeprefix('Bearer ') or request.cookies.get('studio_session', '')
        if not secrets.compare_digest(supplied, token):
            return JSONResponse({'code': 'unauthorized', 'message': 'Abra o Studio pelo OpenWeights para conectar com segurança.', 'next_action': 'Reabrir Studio'}, status_code=401)
    origin = request.headers.get('origin')
    try:
        if int(request.headers.get('content-length', '0')) > (258 if request.url.path == '/api/studio/sources' else 34) * 1024**2:
            return JSONResponse({'detail': 'ImportaÃ§Ã£o excede o limite de 32 MB de conteÃºdo.'}, status_code=413)
    except ValueError:
        return JSONResponse({'detail': 'Content-Length invÃ¡lido.'}, status_code=400)
    if request.method not in ('GET', 'HEAD', 'OPTIONS') and origin and urlsplit(origin).netloc != request.headers.get('host'):
        return JSONResponse({'detail': 'Origem nÃ£o autorizada.'}, status_code=403)
    response = await call_next(request)
    response.headers['X-Content-Type-Options'] = 'nosniff'
    response.headers['X-Frame-Options'] = 'DENY'
    return response


@app.exception_handler(KeyError)
async def missing(request, exc):
    return JSONResponse({'detail': 'Registro nÃ£o encontrado.'}, status_code=404)


@app.exception_handler(ValueError)
async def invalid(request, exc):
    return JSONResponse({'detail': str(exc), 'code': 'invalid_request', 'message': str(exc),
                         'next_action': 'Confira a orientação e tente novamente.'}, status_code=422)


@app.exception_handler(RequestValidationError)
async def validation_error(request, exc):
    labels = {'source_ids': 'arquivos', 'dataset_id': 'dados preparados', 'request_id': 'solicitação',
              'context': 'tamanho do contexto', 'name': 'nome', 'preset': 'tipo de conteúdo'}
    fields = list(dict.fromkeys(labels.get(str(error['loc'][-1]), 'configuração') for error in exc.errors()))
    message = 'Confira ' + ', '.join(fields) + ' e tente novamente.'
    return JSONResponse({'code': 'invalid_fields', 'message': message, 'detail': message,
                         'next_action': 'Corrigir os campos indicados.'}, status_code=422)


@app.get('/api/health')
def health():
    return {'ok': True, 'version': '2.2.1'}


@app.get('/api/system')
def system():
    gpu = None
    error = None
    try:
        result = subprocess.run(['nvidia-smi', '--query-gpu=name,memory.total,memory.used,utilization.gpu,temperature.gpu,power.draw', '--format=csv,noheader,nounits'], capture_output=True, text=True, timeout=4, check=True, **({'creationflags': subprocess.CREATE_NO_WINDOW} if os.name == 'nt' else {}))
        cols = [x.strip() for x in result.stdout.splitlines()[0].split(',')]
        def number(x):
            try: return float(x)
            except ValueError: return None
        gpu = dict(zip(('name','total_mb','used_mb','utilization','temperature','power_w'), [cols[0]]+[number(x) for x in cols[1:]]))
    except Exception:
        error = 'GPU indisponÃ­vel no container. Confira Docker Desktop, WSL2 e driver NVIDIA.'
    disk = shutil.disk_usage(store.ROOT)
    mem = psutil.virtual_memory()
    gguf = {'available': (runtime.converter().is_file() and
                          runtime.quantizer().is_file()),
            'ref': os.getenv('LLAMA_CPP_REF', 'b95502ba9aa0eb73a2f4fc8878d7fbe6a847a0b9')}
    return {'gpu': gpu, 'gpu_error': error, 'gguf': gguf, 'ram_used': mem.used, 'ram_total': mem.total,
            'disk_free': disk.free, 'cpu_percent': psutil.cpu_percent(), 'token_configured': bool(os.getenv('HF_TOKEN')),
            'llm_configured': bool(os.getenv('LLM_API_KEY') or os.getenv('OPENROUTER_API_KEY'))}


@app.post('/api/reset')
def reset_workspace(request: Request):
    # Deliberately explicit endpoint; the UI adds a second confirmation.
    if request.headers.get('x-reset-confirmation') != 'RESET_WORKSPACE':
        raise HTTPException(400, 'Invalid confirmation. Use RESET_WORKSPACE.')
    with mutation_lock:
        for job in store.all_records('jobs'):
            if job.get('status') in ('queued', 'running', 'preparing'):
                try:
                    store.update('jobs', job['id'], {'cancel_requested': True})
                except Exception:
                    pass
        store.reset_all()
    return {'ok': True, 'message': 'Workspace limpo.'}


@app.get('/api/datasets')
def datasets():
    return store.all_records('datasets')


def load_rows(ident):
    record = store.get('datasets', ident)
    return json.loads((store.ROOT / 'datasets' / f'{ident}.json').read_text(encoding='utf-8'))


def save_dataset(name, rows, ident=None):
    with mutation_lock:
        if ident and store.get('datasets', ident).get('immutable'):
            raise ValueError('Dataset versionado Ã© imutÃ¡vel. Crie uma nova preparaÃ§Ã£o no Data Studio.')
        previous = load_rows(ident) if ident else []
        unique, duplicates = data.dedupe(previous + rows)
        meta = {'name': name.strip(), 'count': len(unique), 'kind': 'text' if 'text' in unique[0] else 'chat',
                'characters': sum(len(json.dumps(r, ensure_ascii=False)) for r in unique), 'last_duplicates': duplicates}
        if not meta['name']:
            raise ValueError('Informe um nome para o dataset.')
        record = store.update('datasets', ident, meta) if ident else store.create('datasets', meta)
        store.write_json(store.ROOT / 'datasets' / f"{record['id']}.json", unique)
    return record


@app.post('/api/datasets/import')
async def import_dataset(name: str = Form(..., max_length=100), content: str = Form(''),
                         format: str = Form('text'), dataset_id: str = Form(''), files: list[UploadFile] = File(default=[])):
    rows = []
    total = len(content.encode())
    if content.strip():
        extension = {'text': '.txt', 'json': '.json', 'jsonl': '.jsonl'}.get(format)
        if not extension:
            raise ValueError('Formato literal invÃ¡lido.')
        rows += await asyncio.to_thread(data.parse, content.encode(), 'literal'+extension)
    for f in files:
        raw = await f.read(data.MAX_BYTES+1)
        total += len(raw)
        if total > data.MAX_BYTES:
            raise HTTPException(413, 'Limite de 32 MB por importaÃ§Ã£o.')
        rows += await asyncio.to_thread(data.parse, raw, f.filename or 'arquivo.txt')
    if not rows:
        raise ValueError('Cole conteÃºdo ou selecione arquivos.')
    if total > data.MAX_BYTES:
        raise HTTPException(413, 'Limite de 32 MB por importaÃ§Ã£o.')
    return await asyncio.to_thread(save_dataset, name, rows, dataset_id or None)


@app.get('/api/datasets/{ident}')
def dataset_detail(ident: str):
    return store.get('datasets', ident) | {'examples': load_rows(ident)[:20]}


@app.get('/api/datasets/{ident}/export')
def export_dataset(ident: str):
    store.get('datasets', ident)
    return FileResponse(store.ROOT / 'datasets' / f'{ident}.json', filename='dataset.json')


@app.get('/api/models')
def models():
    return store.all_records('models')


@app.get('/api/local-models')
def local_models():
    root = runtime.models_dir()
    return [p.parent.relative_to(root).as_posix() for p in root.glob('*/config.json') if p.is_file()] if root.exists() else []


@app.post('/api/models')
def add_model(body: ModelInput):
    if body.source == 'hub':
        from huggingface_hub.utils import validate_repo_id
        validate_repo_id(body.repo)
    else:
        root = runtime.models_dir().resolve()
        target = (root / body.repo).resolve()
        if not target.is_relative_to(root) or not (target / 'config.json').is_file():
            raise ValueError('Escolha uma pasta vÃ¡lida dentro de models, contendo config.json.')
        if not list(target.glob('*.safetensors')):
            raise ValueError('Modelo local precisa de pesos .safetensors.')
    return store.create('models', body.model_dump() | {'status': 'ready' if body.source == 'local' else 'registered'})


def new_job(kind, name, config):
    return store.create('jobs', {'kind': kind, 'name': name, 'config': config, 'status': 'queued', 'cancel_requested': False})


@app.post('/api/models/{ident}/download')
def download_model(ident: str):
    model = store.get('models', ident)
    if model['source'] != 'hub':
        raise ValueError('Modelo local jÃ¡ disponÃ­vel.')
    if any(j['kind'] == 'download' and j['config'].get('model_id') == ident and j['status'] in ('queued','running') for j in store.all_records('jobs')):
        raise ValueError('Download jÃ¡ estÃ¡ na fila.')
    return new_job('download', 'Download Â· '+model['name'], {'model_id': ident})


@app.post('/api/train')
def train(body: TrainInput):
    dataset = store.get('datasets', body.dataset_id)
    if body.mode == 'qlora' and dataset['kind'] != 'chat':
        raise ValueError('QLoRA de instruÃ§Ãµes usa conversas. Escolha um dataset de conversas.')
    if body.mode in ('continued', 'scratch', 'scratch_moe') and dataset['kind'] != 'text':
        raise ValueError('ContinuaÃ§Ã£o, treino denso do zero e MoE do zero usam um dataset de texto livre.')
    model = store.get('models', body.model_id) if body.model_id else None
    rows = load_rows(body.dataset_id)
    if dataset.get('immutable'):
        from .studio.pipeline import digest
        if digest(rows) != dataset['sha256']:
            raise ValueError('Integridade do dataset invÃ¡lida.')
        train_rows = [r for r in rows if r['split'] == 'train']
        val_rows = [r for r in rows if r['split'] == 'validation']
        if not train_rows or not val_rows:
            raise ValueError('Dataset nÃ£o contÃ©m treino e validaÃ§Ã£o.')
    else:
        train_rows, val_rows = data.split_rows(rows, body.validation, body.seed)
    job = store.create('jobs', {'kind':'train','name':body.name,'config':body.model_dump(), 'status':'preparing','cancel_requested':False})
    folder = store.run_dir(job['id'])
    store.write_json(folder / 'dataset.json', {'train':train_rows,'validation':val_rows})
    store.write_json(folder / 'config.json', body.model_dump() | {'dataset_name':dataset['name'], 'model':model})
    return store.update('jobs', job['id'], {'status':'queued', 'split_counts':{'train':len(train_rows),'validation':len(val_rows)}})


@app.post('/api/generate')
def generate(body: GenerateInput):
    if body.model_id:
        store.get('models', body.model_id)
    else:
        run = store.get('jobs', body.run_id)
        if run['kind'] != 'train' or run['status'] != 'completed':
            raise ValueError('Escolha um treinamento concluÃ­do.')
    return new_job('generate', 'Teste de geraÃ§Ã£o', body.model_dump())


@app.get('/api/jobs')
def jobs():
    return store.all_records('jobs')


@app.get('/api/jobs/{ident}')
def job_detail(ident: str):
    job = store.get('jobs', ident)
    folder = store.run_dir(ident)
    log = folder / 'worker.log'
    text = ''
    if log.exists():
        with log.open('rb') as f:
            f.seek(max(0, log.stat().st_size - 30000))
            text = f.read().decode('utf-8', errors='replace')
    metrics = []
    mpath = folder / 'metrics.jsonl'
    if mpath.exists():
        with mpath.open(encoding='utf-8') as f:
            from collections import deque
            for line in deque(f, maxlen=2000):
                try: metrics.append(json.loads(line))
                except json.JSONDecodeError: pass
    checkpoints = sorted((p.name for p in folder.glob('checkpoint-*') if (p/'trainer_state.json').exists()), key=lambda x:int(x.split('-')[-1]))
    if job['kind'] == 'workflow':
        checkpoints = [name for name in checkpoints if (folder/name/'complete.json').is_file()]
    return job | {'log':text,'metrics':metrics,'checkpoints':checkpoints,
                  'can_retry_export': (folder/'training-complete.json').is_file() and job['status'] in ('failed', 'cancelled', 'interrupted')}


@app.post('/api/jobs/{ident}/cancel')
def cancel(ident: str):
    job = store.get('jobs', ident)
    if job['status'] not in ('queued','running','cancelling'):
        raise ValueError('Esse trabalho nÃ£o estÃ¡ em execuÃ§Ã£o ou na fila.')
    return store.update('jobs', ident, {'cancel_requested':True,'status':'cancelled' if job['status']=='queued' else 'cancelling'})


@app.post('/api/jobs/{ident}/resume')
def resume(ident: str):
    job = job_detail(ident)
    if job['kind'] != 'train' or job['status'] not in ('cancelled','failed','interrupted') or not job['checkpoints']:
        raise ValueError('Retomada exige treino interrompido e checkpoint vÃ¡lido.')
    return store.update('jobs', ident, {'status':'queued','cancel_requested':False,'resume':job['checkpoints'][-1], 'error':None})


@app.get('/api/jobs/{ident}/export')
def export_run(ident: str):
    job = store.get('jobs', ident)
    if job['kind'] != 'train' or job['status'] != 'completed':
        raise ValueError('Somente treinamentos concluÃ­dos podem ser exportados.')
    folder = store.run_dir(ident)
    target = store.ROOT / 'exports' / f'{ident}.zip'
    if not target.exists():
        with zipfile.ZipFile(target, 'w', zipfile.ZIP_STORED) as z:
            for p in (folder/'artifact').rglob('*'):
                if p.is_file(): z.write(p, p.relative_to(folder))
            for name in ('config.json','metrics.jsonl','manifest.json'):
                if (folder/name).exists(): z.write(folder/name, name)
    return FileResponse(target, filename=f'llm-{ident[:8]}.zip')


@app.post('/api/jobs/{ident}/gguf')
def export_gguf(ident: str, body: GgufExportInput):
    run = store.get('jobs', ident)
    if run['kind'] != 'train' or run['status'] != 'completed':
        raise ValueError('GGUF exige um treinamento concluÃ­do.')
    if run['config'].get('mode') == 'scratch':
        raise ValueError('A LLM do zero usa um tokenizer BPE prÃ³prio que o llama.cpp nÃ£o reconhece. Exporte o artefato safetensors.')
    if run['config'].get('mode') == 'scratch_moe':
        raise ValueError('O MoE customizado ainda precisa de um conversor GGUF prÃ³prio. Exporte o artefato safetensors.')
    if not runtime.converter().is_file() or not runtime.quantizer().is_file():
        raise ValueError('Ferramentas GGUF nÃ£o estÃ£o disponÃ­veis na imagem. Reconstrua o container com docker compose up -d --build.')
    quant = body.quantization.upper()
    if any(j['kind'] == 'gguf' and j['config'].get('run_id') == ident and
           j['config'].get('quantization') == quant and j['status'] in ('queued', 'running')
           for j in store.all_records('jobs')):
        raise ValueError('Esta exportaÃ§Ã£o GGUF jÃ¡ estÃ¡ na fila.')
    return new_job('gguf', f'GGUF Â· {quant}', {'run_id': ident, 'quantization': quant})


@app.get('/api/jobs/{ident}/download')
def download_job(ident: str):
    job = store.get('jobs', ident)
    if job['kind'] != 'gguf' or job['status'] != 'completed' or not job.get('artifact'):
        raise ValueError('O GGUF ainda nÃ£o estÃ¡ disponÃ­vel.')
    path = Path(job['artifact']).resolve()
    root = store.ROOT.resolve()
    if not path.is_relative_to(root) or not path.is_file():
        raise ValueError('Artefato GGUF invÃ¡lido.')
    return FileResponse(path, filename=path.name, media_type='application/octet-stream')


from .studio.api import router as studio_router
from .studio.limits import BodyLimit
app.add_middleware(BodyLimit)
app.include_router(studio_router)
from .v1 import router as v1_router
app.include_router(v1_router)
app.mount('/', StaticFiles(directory=Path(__file__).parent/'static', html=True), name='ui')
