"""Run with bundled Python after extraction on a clean Windows runner."""
import argparse
import json
import os
import sys
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--ocr-components', required=True)
    parser.add_argument('--fixtures', required=True)
    parser.add_argument('--data', required=True)
    args = parser.parse_args()
    root = Path(sys.executable).resolve().parent.parent
    assert (root/'runtime.json').is_file(), 'Use the bundled Python'
    os.environ.update(LAB_DATA=str(Path(args.data).resolve()), OW_OCR_COMPONENTS=str(Path(args.ocr_components).resolve()),
                      OW_LLAMA_DIR=str(root/'llama.cpp'), PYTHONNOUSERSITE='1')
    # Dependencies must work without Conda, a system Python or a CUDA Toolkit on PATH.
    os.environ['PATH'] = str(Path(os.environ['SystemRoot'])/'System32')
    import torch, peft, trl, bitsandbytes
    from app.pdf_text import extract_pdf
    fixtures = Path(args.fixtures)
    data = Path(args.data)
    for name, pages in [('scan', 3), ('mixed', 2)]:
        progress = []
        rows, _ = extract_pdf((fixtures/f'{name}.pdf').read_bytes(), data/name, lambda n, total: progress.append((n, total)), lambda: None)
        assert progress[-1] == (pages, pages)
        assert 'simão' in rows[0]['text'].casefold(), 'Portuguese OCR did not recover the expected book content'
    assert not (data/'mixed/page-1.png').exists()
    assert (data/'mixed/page-2.png').exists()
    print(json.dumps({'private_python': sys.version, 'torch': torch.__version__, 'ocr': 'passed', 'gpu_available': torch.cuda.is_available()}))


if __name__ == '__main__': main()
