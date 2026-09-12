import pytest

from app.studio.clean import pack_sentences, quality_score, repair_layout, sentences, tag_role, words


BRAS = """Memórias Póstumas de Brás Cubas \n \n \n \nTexto-fonte: \nObra Completa, Machado de Assis, \nRio de Janeiro: Editora Nova Aguilar, 1994. \n \nPublicado originalmente em folhetins, a partir de março de 1880, na Revista Brasileira. \n \n \n \nAo verme \nque \nprimeiro roeu as frias carnes \ndo meu cadáver \ndedico \ncomo saudosa lembrança \nestas \nMemórias Póstumas \n \n \nPrólogo da terceira edição \n \nA primeira edição destas Memórias Póstumas de Brás Cubas foi feita \naos pedaços na Revista Brasileira, pelos anos de 1880."""


def test_repair_layout_collapses_pdf_newlines():
    text = repair_layout(BRAS)
    assert '\n \n' not in text
    assert 'Texto-fonte: Obra Completa, Machado de Assis, Rio de Janeiro: Editora Nova Aguilar, 1994.' in text
    assert 'Ao verme que primeiro roeu as frias carnes do meu cadáver' in text
    assert 'foi feita aos pedaços' in text


def test_dehyphenation_and_sentence_pack():
    text = repair_layout('A informa-\nção útil continua na linha seguinte. Outra frase completa fecha o trecho.')
    assert 'informação útil continua' in text
    packs = pack_sentences(sentences(text), max_words=8, overlap_words=2)
    assert packs
    assert all(len(words(part)) <= 8 for part in packs)
    if len(packs) > 1:
        assert words(packs[1])[:2] == words(packs[0])[-2:]


def test_long_sentence_is_split_by_words():
    sent = ' '.join(f'palavra{i}' for i in range(25)) + '.'
    packs = pack_sentences([sent], max_words=10)
    assert len(packs) == 3
    assert all(len(words(part)) <= 10 for part in packs)


def test_roles_keep_prose_and_flag_catalog():
    catalog = 'Texto-fonte: Obra Completa, Editora Nova Aguilar. Publicado originalmente em 1880.'
    prose = 'A primeira edição destas memórias foi feita aos pedaços na revista, pelos anos de 1880, e depois revista pelo autor.'
    assert tag_role(catalog) == 'metadata'
    assert tag_role(prose) == 'body'
    assert quality_score(prose) > quality_score('')
