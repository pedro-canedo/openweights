"""Deterministic preparation. Extraction checkpoints survive worker restarts."""
import hashlib
import json
import math
import os
import re
import time
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from .. import store
from .clean import pack_sentences, passes_quality, quality_score, repair_layout, sentences, tag_role, words
from .extractors import extract, ignored, zip_members
from .schemas import Recipe

VERSION = '2.2.1'


def digest(value):
    raw = value if isinstance(value, bytes) else json.dumps(value, ensure_ascii=False, sort_keys=True).encode()
    return hashlib.sha256(raw).hexdigest()


def body(row):
    return row.get('text') or '\n'.join(m['content'] for m in row['messages'])


def language(text):
    found = set(re.findall(r'\w+', text.lower()))
    scores = {key: len(found & set(v.split())) for key, v in {
        'pt': 'não uma são para com também você dos das pelo pela que',
        'en': 'the and with from this that are is for not you',
        'es': 'una los las para del con que por pero también está'}.items()}
    best = max(scores, key=scores.get)
    return best if scores[best] >= 2 else 'unknown'


def chunks(text, recipe, kind=None):
    mode = 'paragraph' if kind == 'code' and recipe.chunking == 'sentence' else recipe.chunking
    if mode == 'document':
        yield text
        return
    if mode == 'sentence':
        yield from pack_sentences(sentences(text), recipe.max_words, recipe.overlap_words)
        return
    start = 0
    while start < len(text):
        end = min(len(text), start + recipe.chunk_chars)
        if mode == 'paragraph' and end < len(text):
            point = text.rfind(recipe.separator, start + recipe.chunk_chars // 2, end)
            if point > start:
                end = point + len(recipe.separator)
        yield text[start:end]
        if end == len(text):
            break
        start = max(start + 1, end - recipe.overlap)


def keep_example(text, recipe, kind):
    if not recipe.min_chars <= len(text) <= recipe.max_chars:
        return False
    if kind != 'code' and len(words(text)) < recipe.min_words:
        return False
    return passes_quality(text, recipe.quality_filter, kind)


def annotate_example(item, text, kind, index):
    meta = item.setdefault('metadata', {})
    meta['words'] = len(words(text))
    if kind != 'code':
        meta['role'] = tag_role(text, index)
        meta['quality'] = quality_score(text)
    else:
        meta['role'] = 'body'
        meta['quality'] = 1.0


def sequence_cuts(n, recipe):
    """Hold out the tail of a document. Nearby sentences stay in the same split."""
    if n < 2:
        return 0, 0
    ntest = max(1, round(n * recipe.test)) if recipe.test else 0
    nval = max(1, round(n * recipe.validation))
    if ntest + nval >= n:
        ntest = 1 if recipe.test and n >= 3 else 0
        nval = 1 if n >= 2 + ntest else 0
    return nval, ntest


def assign_by_sequence(rows, recipe):
    by_doc, order = {}, []
    for row in rows:
        meta = row.get('metadata') or {}
        key = (meta.get('source_id', ''), meta.get('path', row.get('group')))
        if key not in by_doc:
            order.append(key)
            by_doc[key] = []
        by_doc[key].append(row)
    min_needed = 3 if recipe.test else 2
    if all(len(by_doc[key]) < min_needed for key in order) and len(rows) >= min_needed:
        order = [('_all', '')]
        by_doc = {('_all', ''): list(rows)}
    assigned = []
    for key in order:
        items = sorted(by_doc[key], key=lambda r: ((r.get('metadata') or {}).get('index', 0), (r.get('metadata') or {}).get('chunk', 0)))
        nval, ntest = sequence_cuts(len(items), recipe)
        n = len(items)
        for i, row in enumerate(items):
            if i >= n - ntest:
                split = 'test'
            elif i >= n - ntest - nval:
                split = 'validation'
            else:
                split = 'train'
            row['split'] = split
            row['group'] = digest([row.get('group'), split, key])
            assigned.append(row)
    return assigned


def prepare(job, folder, check_cancel=lambda: None):
    recipe = Recipe.model_validate(job['config'])
    if recipe.llm_classify:
        from .llm import api_key, completions_url
        from .network import public_target
        if not api_key():
            raise ValueError('Classificação externa exige LLM_API_KEY ou OPENROUTER_API_KEY no ambiente.')
        public_target(completions_url(recipe.llm_base_url))
    started = time.monotonic()
    cache = folder / 'extracted'
    cache.mkdir(exist_ok=True)
    entries, total_bytes = [], 0
    sources = [store.get('sources', ident) for ident in recipe.source_ids]
    for source in sources:
        check_cancel()
        raw = (store.ROOT / 'sources' / source['id'] / 'original').read_bytes()
        if digest(raw) != source['sha256']:
            raise ValueError('Fonte foi modificada após o upload.')
        members = zip_members(raw, recipe) if source['name'].lower().endswith('.zip') else [(source['name'], raw)]
        for name, content in members:
            total_bytes += len(content or b'')
            if total_bytes > recipe.expanded_mb * 1024**2 or len(entries) >= recipe.max_files:
                raise ValueError('Biblioteca expandida excede os limites da receita.')
            entries.append((source, name, content))
    store.update('jobs', job['id'], {'progress': {'stage': 'Extração', 'done': 0, 'total': len(entries)}})

    def extract_entry(entry):
        source, name, raw = entry
        key = digest([source['sha256'], name, recipe.model_dump(), VERSION])
        checkpoint = cache / f'{key}.json'
        if checkpoint.exists():
            return json.loads(checkpoint.read_text(encoding='utf-8'))
        report = {'source_id': source['id'], 'name': name, 'bytes': len(raw or b''), 'sha256': digest(raw or b'')}
        result = {'report': report, 'rows': []}
        if raw is None or ignored(name, recipe):
            report.update(status='ignored', reason='Excluído pelos filtros.')
        else:
            try:
                rows, kind, encoding = extract(raw, name, recipe)
                report.update(status='ready', kind=kind, encoding=encoding, extension=Path(name).suffix.lower())
                for i, row in enumerate(rows):
                    warning = row.pop('warning', None)
                    if warning:
                        report['warning'] = warning
                    original_group = row.get('group')
                    group = original_group or (source['sha256'] if recipe.grouping == 'source' else digest([source['sha256'], name]))
                    metadata = {'source_id': source['id'], 'path': name, 'index': i, 'kind': kind, 'sha256': report['sha256']}
                    if row.get('pages'):
                        metadata['pages'] = row['pages']
                    if row.get('pdf_pages'):
                        metadata['pdf_pages'] = row['pdf_pages']
                    item = {'group': group, 'metadata': metadata}
                    if 'messages' in row:
                        item['messages'] = row['messages']
                    else:
                        item['text'] = row['text']
                    result['rows'].append(item)
            except Exception as exc:
                # Do not expose document content, network credentials or parser dumps.
                report.update(status='error', reason=str(exc)[:200] if isinstance(exc, ValueError) else f'Falha de extração ({type(exc).__name__}).')
        store.write_json(checkpoint, result)
        return result

    extracted, reports = [], []
    with ThreadPoolExecutor(max_workers=recipe.workers) as pool:
        for i, result in enumerate(pool.map(extract_entry, entries), 1):
            check_cancel()
            if time.monotonic() - started > recipe.timeout_seconds:
                raise ValueError('Preparação excedeu o tempo configurado.')
            reports.append(result['report'])
            store.write_json(folder/'files.json', reports)
            if recipe.invalid == 'fail' and result['report']['status'] == 'error':
                raise ValueError(result['report']['reason'])
            extracted.extend(result['rows'])
            if len(extracted) > 100000:
                raise ValueError('Limite de 100 mil documentos por preparação.')
            store.update('jobs', job['id'], {'progress': {'stage': 'Extração', 'done': i, 'total': len(entries)}})
    del entries
    warnings, filtered, duplicates = [], 0, 0
    unique, seen, prompt_groups, parents = [], {}, {}, {}

    def root(group):
        parents.setdefault(group, group)
        while parents[group] != group:
            parents[group] = parents[parents[group]]
            group = parents[group]
        return group

    def join(a, b):
        parents[root(a)] = root(b)

    near_sets = []
    if recipe.near_dedupe and len(extracted) > 5000:
        raise ValueError('Deduplicação aproximada limitada a 5 mil documentos. Divida a biblioteca ou use a exata.')
    for row in extracted:
        check_cancel()
        if time.monotonic() - started > recipe.timeout_seconds:
            raise ValueError('Preparação excedeu o tempo configurado.')
        kind = row['metadata']['kind']
        text = body(row).replace('\r\n', '\n').replace('\r', '\n').strip()
        if kind != 'code' and 'text' in row and recipe.normalize_spaces:
            text = repair_layout(text)
        if recipe.normalize_markdown and Path(row['metadata']['path']).suffix.lower() in ('.md', '.markdown'):
            text = '\n'.join(line.rstrip() for line in text.splitlines())
        if not text:
            filtered += 1
            continue
        if 'text' in row:
            row['text'] = text
        if re.search(r'-----BEGIN .*PRIVATE KEY-----|\b(?:api_key|password|senha)\s*[:=]|[\w.+-]+@[\w.-]+\.[a-z]{2,}', text, re.I):
            row['metadata']['sensitive'] = True
        key = digest(row.get('text', row.get('messages')))
        if key in seen:
            join(row['group'], seen[key]['group'])
            if recipe.exact_dedupe:
                duplicates += 1
                continue
        if 'messages' in row:
            prompt = digest(row['messages'][:-1])
            if prompt in prompt_groups:
                join(row['group'], prompt_groups[prompt])
            prompt_groups[prompt] = row['group']
        if recipe.near_dedupe:
            tokens = re.findall(r'\w+', text.lower())
            shingles = set(tuple(tokens[i:i+3]) for i in range(max(1, len(tokens)-2)))
            match = next((old for old, previous in near_sets if len(shingles & previous) / max(1, len(shingles | previous)) >= recipe.similarity), None)
            if match is not None:
                join(row['group'], match['group'])
                duplicates += 1
                continue
            near_sets.append((row, shingles))
        seen[key] = row
        unique.append(row)
    if not unique:
        failed = [f"{r['name']}: {r.get('reason')}" for r in reports if r.get('status') == 'error']
        if failed:
            raise ValueError('Nenhum documento extraído. ' + ' '.join(failed[:8]))
        raise ValueError('Nenhum documento válido. Revise os filtros e os relatórios de extração.')
    kinds = {'chat' if 'messages' in row else 'text' for row in unique}
    if len(kinds) > 1:
        raise ValueError('A biblioteca contém texto e conversas. Prepare receitas separadas com fontes compatíveis.')
    kind = kinds.pop()
    if recipe.preset == 'conversations' and kind != 'chat':
        raise ValueError('Conversas exigem perguntas e respostas reais (messages ou instruction/output). Documentos não geram respostas automaticamente.')
    groups = sorted({root(r['group']) for r in unique}, key=lambda g: digest([recipe.seed, g]))
    required = 3 if recipe.test else 2
    sequence = recipe.grouping == 'sequence' or (kind == 'text' and len(groups) < required)
    if not sequence and len(groups) < required:
        raise ValueError(f'Adicione pelo menos {required} origens/grupos independentes após deduplicação. Um documento não deve aparecer em treino e validação.')
    if sequence and recipe.grouping != 'sequence':
        warnings.append('Há poucas origens independentes. O texto foi dividido em treino, validação e teste na ordem de leitura, sem misturar frases vizinhas. Adicione outras obras se quiser medir generalização entre documentos.')
    ntest = max(1, round(len(groups)*recipe.test)) if recipe.test else 0
    nval = max(1, min(len(groups)-ntest-1, round(len(groups)*recipe.validation))) if len(groups) >= required else 0
    assignment = {g: 'test' if i < ntest else 'validation' if i < ntest+nval else 'train' for i, g in enumerate(groups)}
    rows = []
    for row in unique:
        group = root(row['group'])
        source_kind = row['metadata']['kind']
        parts = [None] if kind == 'chat' else list(chunks(row['text'], recipe, source_kind))
        for i, part in enumerate(parts):
            if part is not None and len(part.strip()) < recipe.min_example_chars:
                continue
            item = {'messages': row['messages']} if kind == 'chat' else {'text': part}
            sample = body(item)
            if kind != 'chat' and not keep_example(sample, recipe, source_kind):
                filtered += 1
                continue
            if kind == 'chat' and not recipe.min_chars <= len(sample) <= recipe.max_chars:
                filtered += 1
                continue
            lang = language(sample) if recipe.detect_language else 'unknown'
            if recipe.language != 'any' and lang != recipe.language:
                filtered += 1
                continue
            item.update(group=group, split=assignment.get(group, 'train'), id=digest([row['metadata'], i, item])[:32])
            if recipe.metadata:
                item['metadata'] = dict(row['metadata'], chunk=i, language=lang)
                annotate_example(item, sample, source_kind, i)
                if recipe.drop_boilerplate and item['metadata'].get('role') in ('boilerplate', 'metadata'):
                    filtered += 1
                    continue
                if item['metadata'].get('quality', 1) < recipe.min_quality:
                    filtered += 1
                    continue
            elif recipe.drop_boilerplate and tag_role(sample, i) in ('boilerplate', 'metadata'):
                filtered += 1
                continue
            elif quality_score(sample) < recipe.min_quality:
                filtered += 1
                continue
            rows.append(item)
            if len(rows) > 100000:
                raise ValueError('Limite de 100 mil exemplos após divisão.')
    if recipe.llm_classify:
        from .llm import classify_rows
        store.update('jobs', job['id'], {'progress': {'stage': 'Classificação', 'done': 0, 'total': len(rows)}})
        rows, llm_warnings = classify_rows(rows, recipe, folder, check_cancel, started)
        warnings.extend(llm_warnings)
        kept = []
        for item in rows:
            meta = item.get('metadata') or {}
            role = meta.get('role') or tag_role(body(item), meta.get('chunk', 0))
            quality = meta.get('quality', quality_score(body(item)))
            if recipe.drop_boilerplate and role in ('boilerplate', 'metadata'):
                filtered += 1
                continue
            if quality < recipe.min_quality:
                filtered += 1
                continue
            if recipe.language != 'any' and recipe.detect_language and meta.get('language') not in (recipe.language, None):
                filtered += 1
                continue
            kept.append(item)
        rows = kept
        warnings.append('Classificação externa envia trechos à URL configurada. Não use com dados que não possam sair desta máquina.')
    exact_tokens = None
    tokenizer_info = None
    if recipe.tokenizer_model_id:
        os.environ['USE_TORCH'] = '0'
        from transformers import AutoTokenizer
        record = store.get('models', recipe.tokenizer_model_id)
        revision = record.get('resolved_revision')
        if record['source'] == 'hub' and not revision:
            raise ValueError('Baixe o modelo primeiro para fixar a revisão do tokenizer.')
        location = record['repo'] if record['source'] == 'hub' else str(Path('/models')/record['repo'])
        tok = AutoTokenizer.from_pretrained(location, revision=revision, local_files_only=True, trust_remote_code=False)
        tokenizer_info = {'model_id': record['id'], 'repo': record['repo'], 'revision': revision}
        accepted = []
        exact_tokens = 0
        for row in rows:
            check_cancel()
            ids = tok.encode(row['text'], add_special_tokens=False) if 'text' in row else tok.apply_chat_template(row['messages'], tokenize=True)
            if len(ids) > recipe.max_tokens:
                if recipe.long_examples == 'skip':
                    continue
                if recipe.long_examples == 'truncate_text' and 'text' in row:
                    row['text'] = tok.decode(ids[:recipe.max_tokens], skip_special_tokens=False)
                    ids = tok.encode(row['text'], add_special_tokens=False)
                else:
                    raise ValueError('Exemplo excede o limite do tokenizer. Reduza o bloco, aumente tokens ou escolha ignorar exemplos longos.')
            row['token_count'] = len(ids)
            exact_tokens += len(ids)
            accepted.append(row)
        rows = accepted
    # Chunking and truncation can create new duplicates. Link their source groups
    # before the final split even when the user chooses to retain duplicates.
    final_seen, final_rows = {}, []
    for row in rows:
        key = digest(row.get('text', row.get('messages')))
        if key in final_seen:
            join(row['group'], final_seen[key]['group'])
            if recipe.exact_dedupe:
                duplicates += 1
                continue
        final_seen[key] = row
        final_rows.append(row)
    rows = final_rows
    if not rows:
        raise ValueError('Nenhum exemplo restou após fatiar e filtrar. Afrouxe qualidade, idioma ou mínimo de palavras.')
    if sequence:
        rows = assign_by_sequence(rows, recipe)
    else:
        groups = sorted({root(r['group']) for r in rows}, key=lambda g: digest([recipe.seed, g]))
        if len(groups) < required:
            raise ValueError('Após dividir os textos, há trechos repetidos ligando suas origens. Adicione mais documentos independentes para evitar vazamento.')
        ntest = max(1, round(len(groups)*recipe.test)) if recipe.test else 0
        nval = max(1, min(len(groups)-ntest-1, round(len(groups)*recipe.validation)))
        assignment = {g: 'test' if i < ntest else 'validation' if i < ntest+nval else 'train' for i, g in enumerate(groups)}
        for row in rows:
            row['group'] = root(row['group'])
            row['split'] = assignment[row['group']]
    if exact_tokens is not None:
        exact_tokens = sum(r['token_count'] for r in rows)
    counts = dict(Counter(row['split'] for row in rows))
    if any(not counts.get(key) for key in ('train', 'validation') + (('test',) if recipe.test else ())):
        if sequence:
            raise ValueError('O texto é curto demais para montar treino, validação e teste na ordem de leitura. Adicione mais conteúdo ou desative o conjunto de teste.')
        raise ValueError('Filtros de exemplos deixaram um conjunto vazio. Reduza filtros ou adicione mais fontes.')
    if len(rows) < 100:
        warnings.append('Dataset pequeno: adequado para teste funcional; avalie com mais dados antes de uso real.')
    if counts.get('validation', 0) < 10:
        warnings.append('Poucos exemplos de validação; a loss terá alta variância.')
    sensitive = sum(bool(r['metadata'].get('sensitive')) for r in unique)
    if sensitive:
        warnings.append(f'{sensitive} documentos podem conter dados sensíveis. Revise antes de publicar.')
    langs = dict(Counter((r.get('metadata') or {}).get('language', 'unknown') for r in rows))
    if len(set(langs)-{'unknown'}) > 1:
        warnings.append('Há múltiplos idiomas no dataset.')
    if duplicates > len(extracted) / 4:
        warnings.append('Mais de 25% dos documentos eram duplicados.')
    if not recipe.metadata:
        for row in rows:
            row.pop('metadata', None)
    manifest = {'version': VERSION, 'job_id': job['id'], 'recipe': recipe.model_dump(), 'sources': sources,
                'sha256': digest(rows), 'created_at': store.now(), 'kind': kind, 'counts': counts,
                'documents': len(unique), 'examples': len(rows), 'filtered': filtered, 'duplicates': duplicates,
                'languages': langs, 'estimated_tokens': math.ceil(sum(len(body(r)) for r in rows)/4),
                'token_estimation': 'characters/4; não é contagem do tokenizer', 'warnings': warnings,
                'exact_tokens': exact_tokens, 'tokenizer': tokenizer_info,
                'files': reports, 'seconds': time.monotonic()-started}
    store.write_json(folder/'preview.json', rows)
    store.write_json(folder/'data-manifest.json', manifest)
    store.update('jobs', job['id'], {'result': {k: v for k, v in manifest.items() if k not in ('sources', 'files', 'recipe')},
                                   'progress': {'stage': 'Pronto para revisão', 'done': len(rows), 'total': len(rows)}})


def acquire(job, folder, check_cancel=lambda: None):
    from .network import fetch_page, repository_url, request
    from .storage import save_source
    result = []
    total = 0
    for url in job['config']['urls']:
        check_cancel()
        if job['config']['repository']:
            raw, mime, final = request(repository_url(url, job['config']['revision']))
            if not raw.startswith(b'PK'):
                raise ValueError('Repositório não retornou ZIP. Confira acesso público e revisão.')
            name = 'repository.zip'
        else:
            raw, name = fetch_page(url)
        total += len(raw)
        if total > 256 * 1024**2:
            raise ValueError('Importação remota excede 256 MiB no total.')
        source = save_source(name, raw, origin=url, license=job['config'].get('license', ''))
        result.append(source['id'])
        store.update('jobs', job['id'], {'result': {'source_ids': result}})
