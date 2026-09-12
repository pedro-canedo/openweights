import json
import pytest
from app.data import parse, dedupe, split_rows


def test_conversation_normalization_and_group_isolation():
    rows = parse(json.dumps([{'instruction':f'p{i}', 'output':f'r{i}', 'group':f'g{i//2}'} for i in range(20)]).encode(), 'x.json')
    train, val = split_rows(rows, .2, 42)
    assert len(train)+len(val)==20
    assert not {r['group'] for r in train} & {r['group'] for r in val}
    assert split_rows(rows,.2,42)==(train,val)


def test_mixed_and_invalid_data_rejected():
    with pytest.raises(ValueError):
        dedupe([{'text':'texto'}, {'messages':[]}])
    with pytest.raises(ValueError):
        parse(b'[{"messages":[{"role":"assistant","content":"oi"}]}]', 'x.json')
    with pytest.raises(ValueError):
        split_rows([{'text':'same','group':'g'},{'text':'other','group':'g'}], .1,42)


def test_jsonl_text_and_deduplication():
    rows = parse(b'{"text":"one"}\n{"text":"two"}\n{"text":"one"}', 'x.jsonl')
    unique, n = dedupe(rows)
    assert len(unique)==2 and n==1
    assert len(parse(b'paragraph 1\n\nparagraph 2','x.txt'))==2
