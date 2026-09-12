"""Small, deterministic local-first data path. No synthetic supervision."""
import json
import re
from collections import Counter

from . import store
from .studio.clean import repair_layout, sentences
from .studio.extractors import extract, zip_members, ignored
from .studio.pipeline import digest, body
from .studio.schemas import Recipe

TOO_SHORT = 'Este conteúdo é curto demais para treinar e conferir o resultado. Adicione mais texto.'


def ordered_split(rows):
    """Partition whole records; never manufacture held-out data by repetition."""
    n = len(rows)
    test = max(1, round(n * .1)) if n >= 3 else 0
    val = max(1, round(n * .1)) if n >= 3 else 0
    for i, row in enumerate(rows):
        row['split'] = 'test' if i >= n-test else 'validation' if i >= n-test-val else 'train'
    return rows


def prepare(job, folder, check_cancel):
    config = job['config']
    preset = config.get('preset', 'auto')
    recipe = Recipe(source_ids=config['source_ids'], min_chars=1, min_words=1,
                    quality_filter='off', ocr=True, max_pdf_pages=20000)
    rows, reports = [], []
    for source_id in config['source_ids']:
        check_cancel()
        source = store.get('sources', source_id)
        raw = (store.ROOT/'sources'/source_id/'original').read_bytes()
        if digest(raw) != source['sha256']:
            raise ValueError('O arquivo mudou no disco. Importe novamente para preparar uma versão íntegra.')
        members = zip_members(raw, recipe) if source['name'].lower().endswith('.zip') else [(source['name'], raw)]
        for name, content in members:
            check_cancel()
            if content is None or ignored(name, recipe):
                continue
            cache = folder/'extracted'/f'{digest([source_id, name, digest(content)])}.json'
            if cache.exists():
                extracted, kind = json.loads(cache.read_text(encoding='utf-8'))
            else:
                if name.lower().endswith('.pdf'):
                    from .pdf_text import extract_pdf
                    def progress(done, total):
                        store.update('jobs', job['id'], {'progress': {'stage': 'Lendo PDF', 'done': done, 'total': total}})
                    extracted, kind = extract_pdf(content, cache.with_suffix(''), progress, check_cancel)
                else:
                    extracted, kind, _ = extract(content, name, recipe)
                store.write_json(cache, [extracted, kind])
            if preset == 'code':
                kind = 'code'
            elif preset == 'documents' and kind == 'code':
                kind = 'text'
            for index, item in enumerate(extracted):
                metadata = {'source_id': source_id, 'path': name, 'index': index, 'kind': kind}
                if 'messages' in item:
                    rows.append({'messages': item['messages'], 'metadata': metadata})
                    continue
                text = item.get('text', '')
                if kind != 'code':
                    text = repair_layout(text)
                # Obvious headings delimit regions; sentences/lines preserve reading order.
                regions = re.split(r'(?im)(?=^\s*(?:cap[íi]tulo|chapter)\s+(?:[IVXLCDM]+|\d+)\b)', text)
                for region in regions:
                    units = region.splitlines(keepends=True) if kind == 'code' else sentences(region)
                    chunk = ''
                    for unit in units:
                        if chunk and len(chunk) + len(unit) > 2000:
                            rows.append({'text': chunk.strip(), 'metadata': metadata})
                            chunk = ''
                        # A single long line still needs bounded, non-overlapping pieces.
                        for start in range(0, len(unit), 2000):
                            piece = unit[start:start+2000]
                            if len(chunk) + len(piece) > 2000 and chunk:
                                rows.append({'text': chunk.strip(), 'metadata': metadata})
                                chunk = ''
                            chunk += piece + ('\n' if kind == 'code' else ' ')
                    if chunk.strip():
                        rows.append({'text': chunk.strip(), 'metadata': metadata})
            reports.append({'name': name, 'status': 'ready'})
            store.update('jobs', job['id'], {'progress': {'stage': 'Preparando dados', 'done': len(reports)}})
    if not rows:
        raise ValueError('Não encontramos texto utilizável. Confira o arquivo ou adicione outro documento.')
    kinds = {'chat' if 'messages' in row else 'text' for row in rows}
    if len(kinds) != 1:
        raise ValueError('Prepare as conversas separadamente dos livros e arquivos de código.')
    kind = kinds.pop()
    if preset == 'conversations' and kind != 'chat':
        raise ValueError('Este arquivo contém prosa. Use Livro; conversas precisam de mensagens reais.')
    # Deduplicate before assigning splits. Prompt variants remain in one group.
    unique, seen, fingerprints, postings = [], set(), [], {}
    for row in rows:
        check_cancel()
        value = body(row)
        key = digest(value)
        tokens = set(re.findall(r'\w+', value.casefold()))
        if key in seen:
            continue
        candidates = Counter(index for token in tokens for index in postings.get(token, ()))
        if any(overlap/max(1, len(tokens) + len(fingerprints[index]) - overlap) >= .9 for index, overlap in candidates.items()):
            continue
        seen.add(key)
        for token in tokens:
            postings.setdefault(token, []).append(len(fingerprints))
        fingerprints.append(tokens)
        row['id'] = digest([row['metadata'], len(unique), key])[:32]
        unique.append(row)
    if kind == 'chat':
        groups = {}
        for row in unique:
            key = digest(row['messages'][:-1])
            groups.setdefault(key, []).append(row)
        group_rows = ordered_split([{'group': key} for key in groups])
        unique = [dict(row, split=entry['split']) for entry in group_rows for row in groups[entry['group']]]
    elif all(row['metadata']['kind'] == 'code' for row in unique):
        groups = {}
        for row in unique:
            key = (row['metadata']['source_id'], row['metadata']['path'])
            groups.setdefault(key, []).append(row)
        if len(groups) >= 3:
            assignments = ordered_split([{'group': key} for key in groups])
            unique = [dict(row, split=entry['split']) for entry in assignments for row in groups[entry['group']]]
        else:
            ordered_split(unique)
    else:
        ordered_split(unique)
    counts = dict(Counter(row['split'] for row in unique))
    ready = all(counts.get(part, 0) for part in ('train', 'validation', 'test'))
    dataset_id = digest(['guided-v1', config, [r['id'] for r in unique]])[:32]
    manifest = {'version': 1, 'sha256': digest(unique), 'counts': counts, 'kind': kind,
                'sources': config['source_ids'], 'recipe': config, 'files': reports,
                'warnings': ['Um conjunto pequeno demonstra o fluxo; não comprova um modelo útil.'] if len(unique) < 100 else [],
                'trainable': bool(ready), 'next_action': None if ready else TOO_SHORT}
    store.write_json(store.ROOT/'datasets'/f'{dataset_id}.json', unique)
    store.write_json(store.ROOT/'datasets'/dataset_id/'manifest.json', manifest)
    record = {'id': dataset_id, 'name': config['name'], 'kind': kind, 'count': len(unique),
              'immutable': True, 'version': 1, 'sha256': manifest['sha256'], 'split_counts': counts,
              'trainable': bool(ready), 'next_action': manifest['next_action'], 'warnings': manifest['warnings']}
    try:
        store.get('datasets', dataset_id)
    except KeyError:
        store.create('datasets', record)
    store.update('jobs', job['id'], {'result': record})
