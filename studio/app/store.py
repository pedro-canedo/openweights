"""Small durable registry shared by the API and isolated worker processes."""
import json
import os
import sqlite3
import uuid
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(os.environ.get('LAB_DATA', './data')).resolve()


class ClosingConnection(sqlite3.Connection):
    """sqlite's normal context manager commits but does not close on Windows."""
    def __exit__(self, *args):
        try:
            return super().__exit__(*args)
        finally:
            self.close()


def now():
    return datetime.now(timezone.utc).isoformat()


def connect():
    ROOT.mkdir(parents=True, exist_ok=True)
    db = sqlite3.connect(ROOT / 'lab.sqlite', timeout=30, factory=ClosingConnection)
    db.execute('CREATE TABLE IF NOT EXISTS records (kind TEXT, id TEXT PRIMARY KEY, payload TEXT NOT NULL)')
    return db


def create(kind, data):
    record = {'id': uuid.uuid4().hex, 'created_at': now(), **data}
    with connect() as db:
        db.execute('INSERT INTO records VALUES (?,?,?)', (kind, record['id'], json.dumps(record, ensure_ascii=False)))
    return record


def get(kind, ident):
    with connect() as db:
        row = db.execute('SELECT payload FROM records WHERE kind=? AND id=?', (kind, ident)).fetchone()
    if not row:
        raise KeyError(ident)
    return json.loads(row[0])


def all_records(kind):
    with connect() as db:
        rows = db.execute('SELECT payload FROM records WHERE kind=? ORDER BY rowid DESC', (kind,)).fetchall()
    return [json.loads(r[0]) for r in rows]


def update(kind, ident, changes):
    with connect() as db:
        db.execute('BEGIN IMMEDIATE')
        row = db.execute('SELECT payload FROM records WHERE kind=? AND id=?', (kind, ident)).fetchone()
        if not row:
            raise KeyError(ident)
        record = json.loads(row[0]) | changes | {'updated_at': now()}
        db.execute('UPDATE records SET payload=? WHERE id=?', (json.dumps(record, ensure_ascii=False), ident))
    return record


def write_json(path, obj):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_suffix(path.suffix + '.tmp')
    temp.write_text(json.dumps(obj, ensure_ascii=False, indent=2), encoding='utf-8')
    temp.replace(path)


def run_dir(ident):
    # IDs originate in the registry, never from user filesystem paths.
    get('jobs', ident)
    p = ROOT / 'runs' / ident
    p.mkdir(parents=True, exist_ok=True)
    return p


def reset_all():
    """Remove all persisted workspace data and recreate an empty registry."""
    import shutil
    if ROOT.exists():
        for child in ROOT.iterdir():
            if child.is_dir():
                shutil.rmtree(child, ignore_errors=False)
            else:
                child.unlink()
    ROOT.mkdir(parents=True, exist_ok=True)
    connect().close()
