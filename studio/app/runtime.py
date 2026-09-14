"""Paths belong to the private runtime, never to the user's Python installation."""
import os
import sys
from pathlib import Path


def llama_dir():
    return Path(os.environ.get('OW_LLAMA_DIR', str(Path(sys.executable).parent / 'llama.cpp'))).resolve()


def converter():
    return llama_dir() / 'convert_hf_to_gguf.py'


def quantizer():
    return llama_dir() / ('llama-quantize.exe' if os.name == 'nt' else 'llama-quantize')


def models_dir():
    from . import store
    return Path(os.environ.get('OW_TRAIN_MODELS', str(store.ROOT / 'models'))).resolve()


def runtime_id():
    return os.environ.get('OW_STUDIO_RUNTIME', 'development') + '+studio2'


def ocr_tools():
    from . import store
    root = Path(os.environ.get('OW_OCR_COMPONENTS', str(store.ROOT.parent/'runtimes/studio-ocr')))
    try:
        version = (root/'active.txt').read_text(encoding='utf-8').strip()
        if not version or any(not (c.isalnum() or c in '.-') for c in version):
            return None
        path = root/version
        poppler, tesseract = path/'bin/pdftoppm.exe', path/'bin/tesseract.exe'
        if poppler.is_file() and tesseract.is_file():
            return poppler, tesseract
    except OSError:
        pass
    return None
