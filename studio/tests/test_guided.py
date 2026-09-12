import json
from fastapi.testclient import TestClient
import pytest
from app import store
from app.main import app
from app.guided import prepare, TOO_SHORT
from app.studio.storage import save_source
from app.v1 import Prepare, prepare as enqueue


@pytest.fixture
def root(tmp_path, monkeypatch):
    monkeypatch.setattr(store, 'ROOT', tmp_path)
    return tmp_path


def prepared(content, name='book.txt'):
    source = save_source(name, content.encode())
    job = enqueue(Prepare(source_ids=[source['id']]))
    prepare(job, store.run_dir(job['id']), lambda: None)
    result = store.get('jobs', job['id'])['result']
    rows = json.loads((store.ROOT/'datasets'/f"{result['id']}.json").read_text())
    return result, rows


def test_short_source_is_prepared_but_not_trainable(root):
    result, rows = prepared('Um pequeno texto para aprender.')
    assert len(rows) == 1
    assert result['trainable'] is False
    assert result['next_action'] == TOO_SHORT


def test_one_book_has_ordered_disjoint_splits(root):
    text = ' '.join(f'Trecho {i}: uma descoberta peculiar {i*i} aconteceu na cidade {i+18}. ' for i in range(800))
    result, rows = prepared(text)
    assert result['trainable']
    splits = [row['split'] for row in rows]
    assert splits == sorted(splits, key=['train', 'validation', 'test'].index)
    assert len({r['text'] for r in rows}) == len(rows)
    assert 'Trecho 799:' in rows[-1]['text']


def test_conversations_are_not_sliced(root):
    messages = [{'messages': [{'role': 'user', 'content': f'Pergunta {i}'}, {'role': 'assistant', 'content': f'Resposta {i*i}'}]} for i in range(20)]
    result, rows = prepared(json.dumps(messages), 'messages.json')
    assert result['kind'] == 'chat'
    assert result['trainable']
    assert all(len(r['messages']) == 2 for r in rows)


def test_authentication_protects_api(root, monkeypatch):
    monkeypatch.setenv('OW_STUDIO_TOKEN', 'private-session')
    client = TestClient(app)
    assert client.get('/api/v1/datasets').status_code == 401
    assert client.get('/api/v1/datasets', headers={'Authorization': 'Bearer private-session'}).status_code == 200
    assert client.get('/api/health').status_code == 200


def test_preparation_is_idempotent(root):
    source = save_source('tiny.txt', b'A short source.')
    body = Prepare(source_ids=[source['id']])
    assert enqueue(body)['id'] == enqueue(body)['id']


def test_pdf_reads_every_native_page_and_reuses_checkpoints(root, monkeypatch):
    from app import pdf_text
    class Page:
        def __init__(self, number): self.number = number
        def extract_text(self): return f'Página {self.number} com texto nativo.'
    class Reader:
        is_encrypted = False
        pages = [Page(n) for n in range(1, 301)]
    monkeypatch.setattr(pdf_text, 'PdfReader', lambda _: Reader())
    monkeypatch.setattr(pdf_text.subprocess, 'run', lambda *a, **kw: pytest.fail('OCR não deve rodar em texto nativo'))
    progress = []
    rows, kind = pdf_text.extract_pdf(b'%PDF-test', root/'pages', lambda n, total: progress.append((n, total)), lambda: None)
    assert 'Página 300 ' in rows[0]['text']
    assert progress[-1] == (300, 300)
    monkeypatch.setattr(Page, 'extract_text', lambda _: pytest.fail('Deve reutilizar checkpoint'))
    assert pdf_text.extract_pdf(b'%PDF-test', root/'pages', lambda *args: None, lambda: None)[0] == rows


def test_gpu_lock_is_exclusive_and_released(root, monkeypatch):
    from app.gpu_lease import lease
    monkeypatch.setenv('OW_GPU_LOCK', str(root/'gpu.lock'))
    with lease():
        with pytest.raises(ValueError, match='usando a GPU'):
            with lease(): pass
    with lease(): pass


def test_second_service_cannot_recover_running_jobs(root, monkeypatch):
    from app.gpu_lease import exclusive_file
    from app.main import queue
    monkeypatch.setattr(queue, 'start', lambda: pytest.fail('Second service must not touch the queue'))
    with exclusive_file(root/'service.lock', 'busy'):
        with pytest.raises(ValueError, match='já está aberto'):
            with TestClient(app): pass


def test_copy_import_preserves_source_and_identifiers(root, tmp_path):
    import sqlite3
    import hashlib
    from app.migration import import_legacy
    legacy = root.parent/(root.name+'-legacy')
    (legacy/'data/runs/old/checkpoint-1').mkdir(parents=True)
    weight = legacy/'data/runs/old/checkpoint-1/model.safetensors'
    weight.write_bytes(b'checkpoint bytes')
    with sqlite3.connect(legacy/'data/lab.sqlite') as conn:
        conn.execute('CREATE TABLE records(kind TEXT, id TEXT PRIMARY KEY, payload TEXT NOT NULL)')
        conn.execute('INSERT INTO records VALUES(?,?,?)', ('jobs', 'old', json.dumps({'id': 'old', 'status': 'running', 'artifact': '/data/runs/old'})))
    original = (legacy/'data/lab.sqlite').read_bytes()
    result = import_legacy(legacy)
    assert result['files'] == 1
    assert (legacy/'data/lab.sqlite').read_bytes() == original
    assert (root/'runs/old/checkpoint-1/model.safetensors').read_bytes() == weight.read_bytes()
    assert store.get('jobs', 'old')['status'] == 'interrupted'
    assert store.get('jobs', 'old')['artifact'].replace('\\', '/') == f'{root.as_posix()}/runs/old'


def test_session_cookie_requires_correct_token_and_origin(root, monkeypatch):
    monkeypatch.setenv('OW_STUDIO_TOKEN', 'secret-token')
    client = TestClient(app)
    assert client.post('/session', json={'token': 'wrong'}).status_code == 401
    assert client.post('/session', json={'token': 'secret-token'}, headers={'Origin': 'https://example.org'}).status_code == 403
    response = client.post('/session', json={'token': 'secret-token'})
    assert response.status_code == 200
    assert 'HttpOnly' in response.headers['set-cookie']
    assert client.get('/api/v1/datasets').status_code == 200
