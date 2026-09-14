"""Pinned training bases, conservative resource estimates and immutable imports."""
import json
import math
import re
import shutil
from pathlib import Path

from . import store, runtime
from .studio.pipeline import digest

DEFAULT_MODEL = 'qwen3-0.6b'
MODELS = [
    dict(id=DEFAULT_MODEL, name='Qwen3 0.6B', source='hub', repo='Qwen/Qwen3-0.6B',
         revision='c1899de289a04d12100db370d81485cdf75e47ca', parameters_b=.6,
         download_bytes=1_200_000_000, license='apache-2.0', tested=True),
    dict(id='qwen3-1.7b', name='Qwen3 1.7B', source='hub', repo='Qwen/Qwen3-1.7B',
         revision='70d244cc86ccca08cf5af4e1e306ecf908b1ad5e', parameters_b=1.7,
         download_bytes=3_500_000_000, license='apache-2.0', tested=False),
]
RECIPES = [dict(id='quick', name='Teste rápido', max_steps=50),
           dict(id='recommended', name='Treino recomendado', max_steps=500)]


def resolve(ident):
    model = next((dict(m) for m in MODELS if m['id'] == ident), None)
    if model is None:
        model = store.get('training_models', ident)
    return dict(model, status='registered', architecture='qwen3', max_context=8192,
                gguf=True)


def available():
    result = []
    for record in MODELS + store.all_records('training_models'):
        model = resolve(record['id'])
        downloaded = model['source'] == 'local'
        if not downloaded:
            try:
                from huggingface_hub import snapshot_download
                path = Path(snapshot_download(model['repo'], revision=model['revision'], local_files_only=True))
                # A config-only metadata cache is not a downloaded model.
                index = path/'model.safetensors.index.json'
                weights = set(json.loads(index.read_text())['weight_map'].values()) if index.exists() else {'model.safetensors'}
                downloaded = all((path/f).is_file() for f in weights | {'config.json', 'tokenizer.json', 'tokenizer_config.json'})
            except (OSError, ValueError, KeyError):
                pass
        result.append(model | {'downloaded': downloaded, 'validation': 'tested' if model.get('tested') else 'benchmark_required'})
    return result


def recipe(body, dataset, model):
    selected = next((r for r in RECIPES if r['id'] == body.recipe_id), None)
    if selected is None:
        raise ValueError('Escolha uma receita disponível.')
    # Exact token/step count is established by the tokenizer in the worker.
    steps = body.max_steps or selected['max_steps']
    return dict(context=body.context, max_steps=steps, learning_rate=body.learning_rate,
                rank=body.rank, batch=1, accumulation=16, save_steps=min(5, steps),
                max_minutes=body.max_minutes, recipe_id=body.recipe_id,
                one_epoch=True, runtime=runtime.runtime_id(), benchmark=True)


def resources(model, context, rank):
    size = model['download_bytes']
    return dict(disk_required_bytes=max(8 * 1024**3, math.ceil(size * 5)),
                ram_required_bytes=max(4 * 1024**3, math.ceil(size * 2.5)),
                vram_estimated_mb=math.ceil(2048 + model['parameters_b'] * 1100 + context * rank / 16))


def inspect_import(source, location):
    """Only architectures supported by the pinned converter; never execute model code."""
    from huggingface_hub import HfApi, hf_hub_download
    if source == 'hub':
        if not re.fullmatch(r'[\w.-]+/[\w.-]+', location):
            raise ValueError('Informe o identificador Hugging Face, por exemplo Qwen/Qwen3-1.7B.')
        info = HfApi().model_info(location, files_metadata=True, timeout=20)
        revision = info.sha
        files = {f.rfilename: f.size or 0 for f in info.siblings}
        def read(name):
            return json.loads(Path(hf_hub_download(location, name, revision=revision)).read_text(encoding='utf-8'))
        license_name = (info.card_data.to_dict() if info.card_data else {}).get('license', 'não informada')
        repo = location
    else:
        root = Path(location).resolve()
        if not root.is_dir() or not (root/'config.json').is_file():
            raise ValueError('Selecione uma pasta com config.json, tokenizer e pesos safetensors. GGUF é usado no chat.')
        files = {p.name: p.stat().st_size for p in root.iterdir() if p.is_file()}
        def read(name):
            return json.loads((root/name).read_text(encoding='utf-8'))
        license_name, repo, revision = 'confira a licença da origem', str(root), ''
    config, tokenizer = read('config.json'), read('tokenizer_config.json')
    if config.get('model_type') != 'qwen3' or config.get('quantization_config') or config.get('auto_map') or tokenizer.get('auto_map'):
        raise ValueError('Esta versão aceita bases Qwen3 densas em safetensors, sem código remoto ou quantização prévia.')
    weights = {n for n in files if n.endswith('.safetensors')}
    if not weights or 'tokenizer.json' not in files:
        raise ValueError('Faltam pesos safetensors ou tokenizer.json na base.')
    if 'model.safetensors.index.json' in files:
        if not set(read('model.safetensors.index.json')['weight_map'].values()) <= weights:
            raise ValueError('A base está incompleta: faltam partes dos pesos.')
    size = sum(files[n] for n in weights)
    if size <= 0:
        raise ValueError('Não foi possível conferir o tamanho dos pesos desta base.')
    record = dict(name=Path(location).name, source=source, repo=repo, revision=revision,
                  download_bytes=size, parameters_b=round(size / 2e9, 2), license=str(license_name),
                  tested=False, chat_template=bool(tokenizer.get('chat_template') or 'chat_template.jinja' in files))
    if source == 'local':
        from .workflow import file_hash
        names = weights | {n for n in files if n.endswith('.json') or n in ('LICENSE', 'NOTICE')}
        hashes = {n: file_hash(root/n) for n in sorted(names)}
        record['revision'] = digest(hashes)
        target = store.ROOT/'models'/'imports'/record['revision']
        if not target.exists():
            target.parent.mkdir(parents=True, exist_ok=True)
            required = sum(files[n] for n in names)
            if shutil.disk_usage(target.parent).free < required + 512*1024**2:
                raise ValueError('Libere espaço no disco para copiar a base sem alterar os arquivos originais.')
            import tempfile
            stage = Path(tempfile.mkdtemp(prefix='import-', dir=target.parent))
            try:
                for name in names:
                    shutil.copy2(root/name, stage/name)
                    if file_hash(stage/name) != hashes[name]:
                        raise ValueError('A base mudou durante a cópia. Tente importar novamente.')
                stage.replace(target)
            finally:
                if stage.exists():
                    shutil.rmtree(stage)
        record.update(repo=str(target), file_hashes=hashes)
    record['id'] = digest([record['source'], record['repo'], record['revision']])[:32]
    try:
        return store.get('training_models', record['id'])
    except KeyError:
        return store.create('training_models', record)
