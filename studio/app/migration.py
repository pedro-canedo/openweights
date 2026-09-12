"""Copy-only legacy import. Source files and SQLite are never modified."""
import json
import os
import shutil
import sqlite3
from pathlib import Path

from . import store
from .workflow import file_hash


def import_legacy(source):
    source = Path(source).resolve(strict=True)
    if not (source/'data/lab.sqlite').is_file():
        raise ValueError('Escolha a pasta do MVP que contém data/lab.sqlite.')
    if source == store.ROOT or source.is_relative_to(store.ROOT) or store.ROOT.is_relative_to(source):
        raise ValueError('A origem precisa estar fora do diretório de dados atual.')
    if any(store.all_records(kind) for kind in ('datasets', 'jobs', 'sources', 'models')):
        raise ValueError('Importe o MVP em um Studio vazio para preservar os identificadores existentes.')
    stage = store.ROOT/'migration-staging'
    if stage.exists():
        raise ValueError('Há uma importação interrompida em migration-staging. Preserve-a e confira o relatório antes de tentar novamente.')
    stage.mkdir(parents=True)
    hashes = {}
    for directory in ('data', 'models'):
        base = source/directory
        if not base.exists():
            continue
        for path in base.rglob('*'):
            if path.is_symlink() or not path.resolve().is_relative_to(base.resolve()):
                raise ValueError('A origem contém links externos. Importe uma cópia sem links.')
            if not path.is_file() or path.name.startswith('lab.sqlite'):
                continue
            relative = path.relative_to(base)
            target = stage/('models' if directory == 'models' else '')/relative
            target.parent.mkdir(parents=True, exist_ok=True)
            before = file_hash(path)
            shutil.copy2(path, target)
            if file_hash(target) != before or file_hash(path) != before:
                raise ValueError('Um arquivo mudou durante a cópia. Pare o MVP antes de importar.')
            hashes[f'{directory}/{relative.as_posix()}'] = before
    with sqlite3.connect(f'{(source / "data/lab.sqlite").as_uri()}?mode=ro', uri=True) as old, sqlite3.connect(stage/'lab.sqlite') as new:
        old.backup(new)
        if new.execute('PRAGMA integrity_check').fetchone()[0] != 'ok':
            raise ValueError('O banco original falhou na verificação de integridade.')
        roots = [(str(source/'data'), str(store.ROOT)), ('/data', str(store.ROOT)),
                 (str(source/'models'), str(store.ROOT/'models')), ('/models', str(store.ROOT/'models'))]
        def translate(value):
            if isinstance(value, dict):
                return {k: translate(v) for k, v in value.items()}
            if isinstance(value, list):
                return [translate(v) for v in value]
            if isinstance(value, str):
                normalized = value.replace('\\', '/')
                for prefix, target in roots:
                    prefix = prefix.replace('\\', '/')
                    if normalized == prefix or normalized.startswith(prefix+'/'):
                        return target + normalized[len(prefix):]
            return value
        for ident, payload in new.execute('SELECT id, payload FROM records').fetchall():
            record = translate(json.loads(payload))
            if record.get('status') in ('queued', 'running', 'preparing', 'cancelling'):
                record.update(status='interrupted', error='Importado do MVP. Confira a compatibilidade do runtime antes de retomar.')
            new.execute('UPDATE records SET payload=? WHERE id=?', (json.dumps(record, ensure_ascii=False), ident))
    old.close()
    new.close()
    # Snapshots stay byte-identical; original absolute references are preserved as lineage.
    store.write_json(stage/'migration-report.json', {'source': str(source), 'sha256': hashes,
                     'snapshot_policy': 'preserved', 'imported_at': store.now()})
    for path in stage.iterdir():
        if path.name != 'lab.sqlite':
            target = store.ROOT/path.name
            if target.exists():
                if target.is_dir() and not any(target.iterdir()):
                    target.rmdir()
                else:
                    raise ValueError('O destino já contém arquivos. A cópia original foi preservada.')
            path.replace(target)
    (stage/'lab.sqlite').replace(store.ROOT/'lab.sqlite')
    stage.rmdir()
    return {'imported': True, 'files': len(hashes)}
