"""Page checkpoints make long native PDFs resumable without silent truncation."""
import io
import shutil
import subprocess
from pathlib import Path
from pypdf import PdfReader
from . import store, runtime


def extract_pdf(raw, cache, progress, check_cancel):
    if not raw.startswith(b'%PDF-'):
        raise ValueError('O arquivo não parece ser um PDF válido. Exporte-o novamente como PDF.')
    reader = PdfReader(io.BytesIO(raw))
    if reader.is_encrypted:
        raise ValueError('Este PDF está protegido. Salve uma cópia sem senha para preparar os dados.')
    cache.mkdir(parents=True, exist_ok=True)
    text = []
    source = cache/'source.pdf'
    for number, page in enumerate(reader.pages, 1):
        check_cancel()
        target = cache/f'{number}.txt'
        if target.exists():
            value = target.read_text(encoding='utf-8')
        else:
            value = page.extract_text() or ''
            if not value.strip():
                tools = runtime.ocr_tools()
                if not tools:
                    raise ValueError(f'A página {number} é uma imagem. Instale o recurso OCR do Studio e retome a preparação.')
                if not source.exists():
                    source.write_bytes(raw)
                image = cache/f'page-{number}'
                options = {'creationflags': subprocess.CREATE_NO_WINDOW} if __import__('os').name == 'nt' else {}
                subprocess.run([str(tools[0]), '-f', str(number), '-l', str(number), '-scale-to', '2400', '-singlefile', '-png', str(source), str(image)],
                               check=True, timeout=120, capture_output=True, **options)
                check_cancel()
                value = subprocess.run([str(tools[1]), str(image)+'.png', 'stdout', '--tessdata-dir', str(tools[1].parent.parent/'share/tessdata'), '-l', 'por+eng'],
                                       check=True, timeout=120, capture_output=True, text=True, encoding='utf-8', **options).stdout
            tmp = target.with_suffix('.tmp')
            tmp.write_text(value, encoding='utf-8')
            tmp.replace(target)
        text.append(value)
        progress(number, len(reader.pages))
    if not any(s.strip() for s in text):
        raise ValueError('Não encontramos texto legível neste PDF. Tente uma digitalização mais nítida.')
    return [{'text': '\n\n'.join(text)}], 'pdf'
