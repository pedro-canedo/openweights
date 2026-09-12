import hashlib
import json
from pathlib import Path

import pytest

from app import store, workflow


def test_publication_is_hidden_until_verified_and_idempotent(tmp_path, monkeypatch):
    models = tmp_path/'models'
    source = tmp_path/'run'
    source.mkdir()
    artifact = source/'model.gguf'
    artifact.write_bytes(b'GGUF-model-content')
    (source/'manifest.json').write_text(json.dumps({'runtime': 'test'}))
    monkeypatch.setenv('OW_MODELS_DIR', str(models))
    updates = []
    monkeypatch.setattr(store, 'update', lambda *args: updates.append(args))
    original_hash = workflow.file_hash
    def check_hidden(path):
        if '.studio-staging' in Path(path).parts:
            assert not list(models.rglob('*.gguf'))
        return original_hash(path)
    monkeypatch.setattr(workflow, 'file_hash', check_hidden)
    result = {'artifact': str(artifact), 'sha256': hashlib.sha256(artifact.read_bytes()).hexdigest()}
    workflow.publish({'id': 'run-id'}, source, result)
    published = models/'studio/run-id/model.gguf'
    assert published.read_bytes() == artifact.read_bytes()
    assert updates[-1][2]['chat_model'] == 'model.gguf'
    monkeypatch.setattr(workflow, 'file_hash', original_hash)
    workflow.publish({'id': 'run-id'}, source, result)
    assert not (tmp_path/'.studio-staging/run-id').exists()
    assert len(list(models.rglob('*.gguf'))) == 1


def test_corrupt_copy_never_appears_in_library(tmp_path, monkeypatch):
    source = tmp_path/'run'
    source.mkdir()
    artifact = source/'model.gguf'
    artifact.write_bytes(b'GGUF-test')
    models = tmp_path/'models'
    monkeypatch.setenv('OW_MODELS_DIR', str(models))
    with pytest.raises(ValueError, match='verificação'):
        workflow.publish({'id': 'run-id'}, source, {'artifact': str(artifact), 'sha256': 'incorrect'})
    assert not list(models.rglob('*.gguf'))
