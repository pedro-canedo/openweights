import json
import pytest
from fastapi.testclient import TestClient
from app import store
from app.main import app


@pytest.fixture
def client(tmp_path, monkeypatch):
    monkeypatch.setattr(store, 'ROOT', tmp_path)
    return TestClient(app)  # No lifespan: queue intentionally disabled in unit tests.


def test_import_snapshot_and_export(client):
    rows = [{'text':f'Documento número {i} com conteúdo para aprender.'} for i in range(12)]
    response=client.post('/api/datasets/import',data={'name':'Study','format':'json','content':json.dumps(rows)})
    assert response.status_code==200,response.text
    dataset=response.json()
    training=client.post('/api/train',json={'name':'test','mode':'scratch','dataset_id':dataset['id'],'max_steps':2,'context':128})
    assert training.status_code==200,training.text
    job=training.json()
    snapshot=(store.run_dir(job['id'])/'dataset.json').read_text()
    client.post('/api/datasets/import',data={'name':'Study','dataset_id':dataset['id'],'format':'text','content':'New document'})
    assert (store.run_dir(job['id'])/'dataset.json').read_text()==snapshot
    assert len(client.get(f"/api/datasets/{dataset['id']}/export").json())==13
    assert client.post(f"/api/jobs/{job['id']}/cancel").json()['status']=='cancelled'
    assert client.post(f"/api/jobs/{job['id']}/resume").status_code==422


def test_type_guards_and_local_access(client):
    response=client.post('/api/models',json={'name':'bad','source':'local','repo':'../../etc'})
    assert response.status_code==422
    assert client.get('/api/health',headers={'host':'attacker.example'}).status_code==403
    assert client.post('/api/train',headers={'origin':'https://attacker.example'},json={}).status_code==403
    assert client.post('/api/generate',json={'prompt':'test'}).status_code==422


def test_file_upload(client):
    response=client.post('/api/datasets/import',data={'name':'file'}, files={'files':('demo.json',b'[{"instruction":"a","output":"b"},{"instruction":"c","output":"d"}]','application/json')})
    assert response.status_code==200,response.text
    assert response.json()['kind']=='chat'


def test_moe_mode_accepts_text_dataset(client):
    response = client.post('/api/datasets/import', data={
        'name': 'MoE text', 'format': 'text',
        'content': 'Primeiro bloco de texto.\n\nSegundo bloco de texto.'
    })
    assert response.status_code == 200
    dataset = response.json()
    training = client.post('/api/train', json={
        'name': 'moe', 'mode': 'scratch_moe', 'dataset_id': dataset['id'],
        'context': 128, 'max_steps': 2, 'num_experts': 4, 'top_k': 2,
    })
    assert training.status_code == 200, training.text
    invalid = client.post('/api/train', json={
        'name': 'bad moe', 'mode': 'scratch_moe', 'dataset_id': dataset['id'],
        'top_k': 5, 'num_experts': 4,
    })
    assert invalid.status_code == 422


def test_gguf_rejects_custom_moe_until_converter_exists(client):
    dataset = client.post('/api/datasets/import', data={
        'name': 'GGUF MoE', 'format': 'text',
        'content': 'Bloco um para o experimento.\n\nBloco dois para validação.'
    }).json()
    training = client.post('/api/train', json={
        'name': 'moe concluído', 'mode': 'scratch_moe', 'dataset_id': dataset['id'],
        'context': 128, 'max_steps': 2,
    }).json()
    store.update('jobs', training['id'], {'status': 'completed'})
    response = client.post(f"/api/jobs/{training['id']}/gguf", json={'quantization': 'q4_k_m'})
    assert response.status_code == 422
    assert 'MoE customizado' in response.json()['detail']


def test_gguf_rejects_scratch_tokenizer(client):
    dataset = client.post('/api/datasets/import', data={
        'name': 'GGUF LLM', 'format': 'text',
        'content': 'Bloco um para o experimento.\n\nBloco dois para validação.'
    }).json()
    training = client.post('/api/train', json={
        'name': 'llm concluído', 'mode': 'scratch', 'dataset_id': dataset['id'],
        'context': 128, 'max_steps': 2,
    }).json()
    store.update('jobs', training['id'], {'status': 'completed'})
    response = client.post(f"/api/jobs/{training['id']}/gguf", json={'quantization': 'Q4_K_M'})
    assert response.status_code == 422
    assert 'tokenizer BPE' in response.json()['detail']
