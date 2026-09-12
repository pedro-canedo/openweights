"""Optional OpenAI-compatible classification. API keys never enter recipes or manifests."""
import hashlib
import json
import os
import socket
import ssl
import time
from urllib.parse import quote

from .network import AGENT, public_target

BATCH = 8
MAX_ITEMS = 10000
SNIPPET = 1500
LIMIT = 512 * 1024
TIMEOUT = 45
ROLES = {'body', 'front_matter', 'boilerplate', 'metadata'}
LANGUAGES = {'pt', 'en', 'es', 'unknown'}


def digest(value):
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, sort_keys=True).encode()).hexdigest()


SYSTEM = (
    'Classify snippets for dataset curation. Reply with JSON only: '
    '{"items":[{"id":"...","quality":0,"role":"body","language":"pt","topic":"..."}]}. '
    'quality is 0-5. role is body, front_matter, boilerplate, or metadata. '
    'language is pt, en, es, or unknown. topic is at most four words. '
    'Do not rewrite the text. Do not invent questions or answers.'
)


def api_key():
    return (os.getenv('LLM_API_KEY') or os.getenv('OPENROUTER_API_KEY') or '').strip()


def completions_url(base):
    url = (base or '').strip().rstrip('/')
    if url.endswith('/chat/completions'):
        return url
    return url + '/chat/completions'


def redact(message):
    key = api_key()
    text = str(message)
    return text.replace(key, '***') if key else text


def chat_completions(url, payload, timeout=TIMEOUT):
    key = api_key()
    if not key:
        raise ValueError('Classificação externa exige LLM_API_KEY ou OPENROUTER_API_KEY no ambiente.')
    public_target(url)
    body = json.dumps(payload).encode()
    u, ip = public_target(url)
    conn = http_client(u, ip, timeout)
    headers = {
        'User-Agent': AGENT,
        'Accept': 'application/json',
        'Content-Type': 'application/json',
        'Authorization': f'Bearer {key}',
        'HTTP-Referer': 'http://localhost:7860',
        'X-Title': 'Open Weights Studio',
    }
    try:
        conn.request('POST', quote(u.path or '/', safe='/%:@-._~!$&()*+,;='), body=body, headers=headers)
        response = conn.getresponse()
        chunks, size, started = [], 0, time.monotonic()
        while True:
            part = response.read(min(65536, LIMIT + 1 - size))
            if not part:
                break
            size += len(part)
            if size > LIMIT or time.monotonic() - started > timeout:
                raise ValueError('Resposta do classificador excedeu tamanho ou duração permitidos.')
            chunks.append(part)
        raw = b''.join(chunks)
        if response.status != 200:
            raise ValueError(f'Classificador respondeu HTTP {response.status}.')
        try:
            data = json.loads(raw.decode('utf-8'))
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise ValueError('Classificador devolveu JSON inválido.') from exc
        content = data.get('choices', [{}])[0].get('message', {}).get('content', '')
        if not isinstance(content, str) or not content.strip():
            raise ValueError('Classificador não devolveu conteúdo.')
        return content
    finally:
        conn.close()


def http_client(parsed, ip, timeout):
    import http.client
    conn = http.client.HTTPSConnection(parsed.hostname, timeout=timeout)
    raw = socket.create_connection((ip, 443), timeout=timeout)
    conn.sock = ssl.create_default_context().wrap_socket(raw, server_hostname=parsed.hostname)
    return conn


def parse_items(content):
    text = content.strip()
    if text.startswith('```'):
        text = re_strip_fence(text)
    try:
        data = json.loads(text)
    except json.JSONDecodeError:
        start, end = text.find('{'), text.rfind('}')
        if start < 0 or end <= start:
            raise ValueError('Classificador devolveu JSON inválido.')
        data = json.loads(text[start:end + 1])
    items = data.get('items', data if isinstance(data, list) else [])
    if not isinstance(items, list):
        raise ValueError('Classificador devolveu JSON inválido.')
    return items


def re_strip_fence(text):
    lines = text.splitlines()
    if lines and lines[0].startswith('```'):
        lines = lines[1:]
    if lines and lines[-1].strip() == '```':
        lines = lines[:-1]
    return '\n'.join(lines).strip()


def apply_item(row, item):
    meta = row.setdefault('metadata', {})
    if 'quality' in item:
        try:
            quality = float(item['quality'])
        except (TypeError, ValueError):
            quality = None
        if quality is not None:
            meta['quality'] = round(max(0.0, min(5.0, quality)) / 5.0, 3)
    role = str(item.get('role', '')).strip()
    if role in ROLES:
        meta['role'] = role
    language = str(item.get('language', '')).strip()
    if language in LANGUAGES:
        meta['language'] = language
    topic = str(item.get('topic', '')).strip()
    if topic:
        meta['topic'] = topic[:80]


def classify_rows(rows, recipe, folder, check_cancel=lambda: None, started=None, complete=None):
    warnings = []
    if not recipe.llm_classify:
        return rows, warnings
    key = api_key()
    if not key:
        raise ValueError('Classificação externa exige LLM_API_KEY ou OPENROUTER_API_KEY no ambiente.')
    url = completions_url(recipe.llm_base_url)
    public_target(url)
    cache = folder / 'classified'
    cache.mkdir(exist_ok=True)
    pending = []
    sender = complete or chat_completions
    begin = time.monotonic() if started is None else started
    limit = rows[:MAX_ITEMS]
    if len(rows) > MAX_ITEMS:
        warnings.append(f'Classificação limitada aos primeiros {MAX_ITEMS} exemplos.')
    for row in limit:
        check_cancel()
        text = row.get('text') or '\n'.join(m['content'] for m in row['messages'])
        ident = digest([text, recipe.llm_model, url])
        path = cache / f'{ident}.json'
        if path.exists():
            apply_item(row, json.loads(path.read_text(encoding='utf-8')))
        else:
            pending.append((row, ident, path, text))
    for offset in range(0, len(pending), BATCH):
        check_cancel()
        if time.monotonic() - begin > recipe.timeout_seconds:
            raise ValueError('Preparação excedeu o tempo configurado.')
        batch = pending[offset:offset + BATCH]
        payload = {
            'model': recipe.llm_model.strip(),
            'temperature': 0,
            'messages': [
                {'role': 'system', 'content': SYSTEM},
                {'role': 'user', 'content': json.dumps({
                    'items': [{'id': ident, 'text': text[:SNIPPET]} for _, ident, _, text in batch]
                }, ensure_ascii=False)},
            ],
        }
        try:
            content = sender(url, payload)
            items = {str(item.get('id')): item for item in parse_items(content) if isinstance(item, dict)}
        except Exception as exc:
            message = redact(exc)
            if recipe.invalid == 'fail':
                raise ValueError(message) from exc
            warnings.append('Classificação externa falhou em um lote e os exemplos mantiveram a nota local.')
            continue
        for row, ident, path, _text in batch:
            item = items.get(ident)
            if not isinstance(item, dict):
                continue
            path.write_text(json.dumps(item, ensure_ascii=False), encoding='utf-8')
            apply_item(row, item)
    return rows, warnings
