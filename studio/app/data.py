import csv
import hashlib
import io
import json
import re
from pathlib import Path

MAX_BYTES = 32 * 1024 * 1024
MAX_ROWS = 100000


def normalize(row):
    if isinstance(row, str):
        row = {'text': row}
    if not isinstance(row, dict):
        raise ValueError('Cada exemplo precisa ser um objeto JSON ou texto.')
    group = str(row.get('group', row.get('grupo', ''))).strip()
    if 'text' in row or 'texto' in row:
        text = row.get('text', row.get('texto'))
        if not isinstance(text, str) or not text.strip():
            raise ValueError('O campo text deve conter texto não vazio.')
        return {'text': text.strip(), 'group': group}
    messages = row.get('messages')
    if messages is None:
        prompt = row.get('prompt', row.get('instruction', row.get('pergunta')))
        answer = row.get('completion', row.get('output', row.get('resposta')))
        if isinstance(prompt, list) and isinstance(answer, list):
            messages = prompt + answer
        elif isinstance(prompt, str) and isinstance(answer, str):
            extra = row.get('input', '')
            system = row.get('system', row.get('sistema', ''))
            messages = ([{'role': 'system', 'content': system}] if system else [])
            messages += [{'role': 'user', 'content': prompt + ('\n' + str(extra) if extra else '')},
                         {'role': 'assistant', 'content': answer}]
        else:
            raise ValueError('Use text, messages, instruction/output ou prompt/completion.')
    if not isinstance(messages, list) or len(messages) < 2:
        raise ValueError('Conversa deve ter pergunta e resposta.')
    clean = []
    for i, m in enumerate(messages):
        if not isinstance(m, dict) or m.get('role') not in ('system', 'user', 'assistant'):
            raise ValueError('Papéis aceitos: system, user e assistant.')
        if not isinstance(m.get('content'), str) or not m['content'].strip():
            raise ValueError('Mensagem vazia ou não textual.')
        if m['role'] == 'system' and i != 0:
            raise ValueError('Mensagem system somente no início.')
        clean.append({'role': m['role'], 'content': m['content'].strip()})
    body = clean[1:] if clean[0]['role'] == 'system' else clean
    if any(m['role'] != ('user' if i % 2 == 0 else 'assistant') for i, m in enumerate(body)) or body[-1]['role'] != 'assistant':
        raise ValueError('Use pares user/assistant alternados, terminando com assistant.')
    return {'messages': clean, 'group': group}


def parse(content, filename='input.txt'):
    if len(content) > MAX_BYTES:
        raise ValueError('Limite de 32 MB por importação.')
    suffix = Path(filename).suffix.lower()
    if suffix == '.pdf':
        from pypdf import PdfReader
        raw = '\n\n'.join(page.extract_text() or '' for page in PdfReader(io.BytesIO(content)).pages)
        rows = [p for p in re.split(r'\n\s*\n', raw) if p.strip()]
    elif suffix == '.docx':
        import zipfile
        with zipfile.ZipFile(io.BytesIO(content)) as z:
            if sum(i.file_size for i in z.infolist()) > 128 * 1024**2:
                raise ValueError('DOCX expandido excede 128 MB.')
        from docx import Document
        rows = [p.text for p in Document(io.BytesIO(content)).paragraphs if p.text.strip()]
    else:
        try:
            raw = content.decode('utf-8-sig')
        except UnicodeDecodeError:
            raise ValueError('Salve o arquivo em UTF-8.') from None
        if suffix == '.json':
            obj = json.loads(raw)
            rows = obj if isinstance(obj, list) else obj.get('data', obj.get('examples', [obj])) if isinstance(obj, dict) else [obj]
        elif suffix == '.jsonl':
            rows = [json.loads(line) for line in raw.splitlines() if line.strip()]
        elif suffix == '.csv':
            rows = list(csv.DictReader(io.StringIO(raw)))
        elif suffix in ('.txt', '.md'):
            rows = [p for p in re.split(r'\n\s*\n', raw) if p.strip()]
        else:
            raise ValueError('Formatos: JSON, JSONL, TXT, MD, CSV, PDF e DOCX.')
    if not isinstance(rows, list) or not rows:
        raise ValueError('Nenhum exemplo encontrado. PDFs digitalizados precisam de OCR antes da importação.')
    if len(rows) > MAX_ROWS:
        raise ValueError('Limite de 100 mil exemplos por dataset.')
    result = []
    for i, row in enumerate(rows):
        try:
            result.append(normalize(row))
        except ValueError as exc:
            raise ValueError(f'Exemplo {i+1}: {exc}') from exc
    return result


def dedupe(rows):
    unique, seen = [], {}
    kinds = {'text' if 'text' in r else 'chat' for r in rows}
    if len(kinds) != 1:
        raise ValueError('Separe texto livre e conversas em datasets diferentes.')
    for row in rows:
        body = row.get('text', row.get('messages'))
        key = json.dumps(body, ensure_ascii=False, sort_keys=True)
        if key not in seen:
            seen[key] = True
            unique.append(row)
    if len(unique) > MAX_ROWS:
        raise ValueError('Dataset excede 100 mil exemplos.')
    return unique, len(rows) - len(unique)


def split_rows(rows, validation, seed):
    # Prompt identity keeps duplicate questions together; explicit groups override.
    groups = {}
    for row in rows:
        key = row.get('group') or json.dumps(row.get('text', row.get('messages', [])[:-1]), ensure_ascii=False, sort_keys=True)
        groups.setdefault(key, []).append(row)
    if len(groups) < 2:
        raise ValueError('São necessários ao menos dois exemplos/grupos independentes para treino e validação.')
    keys = sorted(groups, key=lambda x: hashlib.sha256(f'{seed}:{x}'.encode()).hexdigest())
    n = max(1, min(len(keys)-1, round(len(keys)*validation)))
    valkeys = set(keys[:n])
    train, val = [], []
    for key in keys:
        (val if key in valkeys else train).extend(groups[key])
    return train, val
