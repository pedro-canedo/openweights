"""Layout repair, sentence packing and heuristic quality for free text."""
import re

ABBREVIATIONS = {
    'adm', 'al', 'approx', 'art', 'ave', 'cap', 'caps', 'cit', 'co', 'col', 'corp',
    'dept', 'dr', 'dra', 'e.g', 'ed', 'eds', 'eng', 'etc', 'ex', 'fig', 'figs',
    'gen', 'gov', 'i.e', 'id', 'ibid', 'inc', 'jr', 'ltd', 'max', 'min', 'mr',
    'mrs', 'ms', 'máx', 'no', 'nº', 'op', 'p', 'pa', 'pe', 'pp', 'prof', 'profa',
    'pte', 'sgt', 'sr', 'sra', 'srta', 'st', 'sta', 'sto', 'vol', 'vs',
}

WORD = re.compile(r'\w+', re.UNICODE)
SENT_END = re.compile(r'([.!?…]+)(\s+|$)')
METADATA_MARKERS = re.compile(
    r'texto-fonte|publicado originalmente|obra completa|\bisbn\b|editora\b|copyright|all rights reserved|'
    r'direitos reservados|catalogação|catalogacao',
    re.I,
)
FRONT_MATTER = re.compile(
    r'^\s*(prólogo|prologo|preface|prefácio|prefacio|sumário|sumario|índice|indice)\b',
    re.I,
)


def words(text):
    return WORD.findall(text or '')


def repair_layout(text):
    text = (text or '').replace('\r\n', '\n').replace('\r', '\n')
    previous = None
    while previous != text:
        previous = text
        text = re.sub(r'\n[ \t]*\n', '\n\n', text)
    text = re.sub(r'[ \t]+', ' ', text)
    paragraphs = []
    for para in re.split(r'\n\s*\n', text):
        lines = [line.strip() for line in para.split('\n') if line.strip()]
        if not lines:
            continue
        buf = lines[0]
        for nxt in lines[1:]:
            if buf.endswith('-') and nxt[:1].islower():
                buf = buf[:-1] + nxt
            else:
                buf = buf + ' ' + nxt
        paragraphs.append(buf)
    return '\n\n'.join(paragraphs).strip()


def _abbreviation(prefix):
    tokens = words(prefix.lower().rstrip())
    if not tokens:
        return False
    last = tokens[-1]
    return last in ABBREVIATIONS or len(last) == 1


def sentences(text):
    text = (text or '').strip()
    if not text:
        return []
    parts, start = [], 0
    for match in SENT_END.finditer(text):
        end = match.end()
        nxt = text[end:end + 1]
        if _abbreviation(text[start:match.start()]) or (nxt and nxt.islower()):
            continue
        chunk = text[start:match.end(1)].strip()
        if chunk:
            parts.append(chunk)
        start = end
    tail = text[start:].strip()
    if tail:
        parts.append(tail)
    return parts


def pack_sentences(parts, max_words, overlap_words=0):
    packs, buf, count = [], [], 0

    def flush():
        nonlocal buf, count
        if buf:
            packs.append(' '.join(buf))
        buf, count = [], 0

    def overlap():
        nonlocal buf, count
        if overlap_words and packs:
            tail = words(packs[-1])[-overlap_words:]
            if tail:
                buf, count = [' '.join(tail)], len(tail)

    for sent in parts:
        sw = words(sent)
        if not sw:
            continue
        if len(sw) > max_words:
            flush()
            for i in range(0, len(sw), max_words):
                packs.append(' '.join(sw[i:i + max_words]))
            overlap()
            continue
        if buf and count + len(sw) > max_words:
            flush()
            overlap()
            if buf and count + len(sw) > max_words:
                flush()
        buf.append(sent)
        count += len(sw)
    flush()
    return packs


def quality_score(text):
    tokens = words(text)
    if not tokens:
        return 0.0
    score = 1.0
    alpha = sum(ch.isalpha() for ch in text) / max(1, len(text))
    score *= min(1.0, alpha / 0.7)
    mean = sum(len(token) for token in tokens) / len(tokens)
    if mean < 2 or mean > 16:
        score *= 0.4
    if not re.search(r'[.!?…]', text):
        score *= 0.7
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    if lines and len(set(lines)) / len(lines) < 0.6:
        score *= 0.5
    return round(min(1.0, max(0.0, score)), 3)


def tag_role(text, index=0):
    tokens = words(text)
    if not tokens:
        return 'boilerplate'
    matches = METADATA_MARKERS.findall(text)
    if matches and (len(matches) >= 2 or len(tokens) < 40):
        return 'metadata'
    if len(tokens) < 8 and not re.search(r'[.!?…]', text):
        return 'boilerplate'
    if index == 0 and FRONT_MATTER.search(text) and len(tokens) < 60:
        return 'front_matter'
    return 'body'


def passes_quality(text, level, kind):
    if kind == 'code' or level == 'off':
        return True
    tokens = words(text)
    if not tokens:
        return False
    alpha = sum(ch.isalpha() for ch in text) / max(1, len(text))
    if alpha < 0.55:
        return False
    if level == 'light':
        return True
    mean = sum(len(token) for token in tokens) / len(tokens)
    if not 2.5 <= mean <= 14:
        return False
    symbols = sum(not (ch.isalnum() or ch.isspace()) for ch in text) / max(1, len(text))
    if symbols > 0.35:
        return False
    if len(tokens) > 20 and not re.search(r'[.!?…]', text):
        return False
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    if lines and len(set(lines)) / len(lines) < 0.6:
        return False
    return True
