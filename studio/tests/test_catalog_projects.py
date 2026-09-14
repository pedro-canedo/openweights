import json
import pytest
from fastapi.testclient import TestClient
from app import store, catalog, v1, runtime
from app.main import app
from app.studio.pipeline import digest


@pytest.fixture
def root(tmp_path, monkeypatch):
    monkeypatch.setattr(store, 'ROOT', tmp_path)
    return tmp_path


def test_models_are_pinned_and_larger_models_require_more_resources():
    a, b = catalog.MODELS
    assert len(a['revision']) == len(b['revision']) == 40
    assert not b['tested']
    assert catalog.resources(b, 1024, 16)['ram_required_bytes'] > catalog.resources(a, 1024, 16)['ram_required_bytes']
    assert catalog.resources(a, 4096, 32)['vram_estimated_mb'] > catalog.resources(a, 512, 8)['vram_estimated_mb']


def test_projects_persist_without_changing_dataset_versions(root):
    store.create('datasets', {'id': 'a'})
    p = v1.Project(id='a'*32, name='Livro', dataset_id='a')
    v1.save_project(p)
    v1.save_project(p.model_copy(update={'model_id': 'qwen3-1.7b', 'dataset_id': None}))
    result = v1.projects()[0]
    assert result['model_id'] == 'qwen3-1.7b'
    assert result['dataset_versions'] == ['a']


def test_preview_never_returns_reserved_examples(root):
    ident = 'b'*32
    store.create('datasets', {'id': ident})
    store.write_json(root/'datasets'/f'{ident}.json', [{'text': split, 'split': split} for split in ('train', 'validation', 'test')])
    store.write_json(root/'datasets'/ident/'manifest.json', {})
    assert v1.preview(ident)['examples'] == [{'text': 'train', 'split': 'train'}]


def test_run_freezes_selected_model_and_recipe_and_rejects_idempotency_collision(root, monkeypatch):
    rows = [{'text': s, 'split': s} for s in ('train', 'validation', 'test')]
    store.create('datasets', {'id': 'data', 'name': 'Livro', 'kind': 'text', 'sha256': digest(rows)})
    store.write_json(root/'datasets/data.json', rows)
    body = v1.Run(dataset_id='data', request_id='same-request', model_id='qwen3-1.7b', recipe_id='recommended', context=2048)
    monkeypatch.setattr(v1, 'preflight', lambda b: {'recipe': catalog.recipe(b, {}, catalog.resolve(b.model_id))})
    first = v1.create_run(body)
    assert v1.create_run(body)['id'] == first['id']
    snapshot = json.loads((store.run_dir(first['id'])/'config.json').read_text())
    assert snapshot['model']['repo'] == 'Qwen/Qwen3-1.7B'
    assert snapshot['max_steps'] == 500
    assert snapshot['runtime'] == runtime.runtime_id()
    with pytest.raises(ValueError, match='outra receita'):
        v1.create_run(body.model_copy(update={'rank': 32}))
    assert len(store.all_records('jobs')) == 1


def test_unsupported_local_model_is_rejected_without_copy(root, tmp_path):
    original = tmp_path/'original'
    original.mkdir()
    (original/'config.json').write_text(json.dumps({'model_type': 'unknown'}))
    (original/'tokenizer_config.json').write_text('{}')
    with pytest.raises(ValueError, match='Qwen3'):
        catalog.inspect_import('local', str(original))
    assert (original/'config.json').exists()
    assert not (root/'models').exists()


def test_import_makes_immutable_private_copy(root, tmp_path):
    original = tmp_path/'original'
    original.mkdir()
    for name, text in {'config.json':'{"model_type":"qwen3"}', 'tokenizer_config.json':'{}', 'tokenizer.json':'{}', 'model.safetensors':'weights'}.items():
        (original/name).write_text(text)
    record = catalog.inspect_import('local', str(original))
    assert record['repo'] != str(original)
    assert record['file_hashes']['model.safetensors']
    (original/'model.safetensors').write_text('changed')
    from pathlib import Path
    assert (Path(record['repo'])/'model.safetensors').read_text() == 'weights'


def test_comparison_requires_finished_run_and_is_idempotent(root):
    store.create('jobs', {'id': 'a', 'kind': 'workflow', 'status': 'running', 'config': {'runtime': runtime.runtime_id()}})
    body = v1.Comparison(request_id='comparison-id', prompt='Exemplo próprio')
    with pytest.raises(ValueError):
        v1.compare('a', body)
    store.update('jobs', 'a', {'status': 'completed', 'name': 'Livro'})
    first = v1.compare('a', body)
    assert v1.compare('a', body)['id'] == first['id']
    with pytest.raises(ValueError):
        v1.compare('a', body.model_copy(update={'prompt': 'Outra pergunta'}))
