"""Real OCR acceptance against rasterized pages of a supplied public PDF.

Development-only fixture dependencies: reportlab and Pillow. No mocks or GPU.
"""
import argparse
import json
import os
import re
import subprocess
import tempfile
from difflib import SequenceMatcher
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--pdf', required=True)
    parser.add_argument('--ocr', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    from pypdf import PdfReader, PdfWriter
    from reportlab.pdfgen import canvas
    from PIL import Image, ImageFilter
    root, ocr = Path(args.output).resolve(), Path(args.ocr).resolve()
    root.mkdir(parents=True, exist_ok=True)
    components = root/'runtimes/studio-ocr'
    components.mkdir(parents=True, exist_ok=True)
    # Test an extracted copy at the actual private runtime layout.
    import shutil
    shutil.copytree(ocr, components/'1.0.0', dirs_exist_ok=True)
    (components/'active.txt').write_text('1.0.0')
    os.environ.update(LAB_DATA=str(root/'data'), OW_OCR_COMPONENTS=str(components))
    from app.pdf_text import extract_pdf
    reader = PdfReader(args.pdf)
    page_numbers = [1, 2, 3]
    reference = '\n'.join(reader.pages[n-1].extract_text() or '' for n in page_numbers)
    scan = root/'scan.pdf'
    generated = canvas.Canvas(str(scan), pagesize=(595, 842))
    for number in page_numbers:
        image = root/f'page-{number}'
        subprocess.run([str(ocr/'bin/pdftoppm.exe'), '-f', str(number), '-l', str(number), '-singlefile', '-r', '180', '-png', args.pdf, str(image)], check=True, capture_output=True)
        # Mild scan degradation; fixture is derived from real book pages.
        with Image.open(str(image)+'.png') as picture:
            picture.convert('L').filter(ImageFilter.GaussianBlur(.3)).save(str(image)+'.png')
        generated.drawImage(str(image)+'.png', 0, 0, width=595, height=842)
        generated.showPage()
    generated.save()
    assert not any(page.extract_text() for page in PdfReader(scan).pages)
    normalized = lambda s: re.sub(r'\W+', ' ', s.casefold()).strip()
    progress = []
    rows, _ = extract_pdf(scan.read_bytes(), root/'scan-cache', lambda n, total: progress.append([n, total]), lambda: None)
    similarity = SequenceMatcher(None, normalized(reference), normalized(rows[0]['text']), autojunk=False).ratio()
    assert similarity > .85, similarity
    assert progress[-1] == [3, 3]
    writer = PdfWriter()
    writer.add_page(reader.pages[0])
    writer.add_page(PdfReader(scan).pages[1])
    mixed = root/'mixed.pdf'
    with mixed.open('wb') as stream: writer.write(stream)
    calls = []
    extract_pdf(mixed.read_bytes(), root/'mixed-cache', lambda n, total: calls.append(n), lambda: None)
    assert not (root/'mixed-cache/page-1.png').exists()
    assert (root/'mixed-cache/page-2.png').exists()
    assert calls == [1, 2]
    # Resume must use the saved text without needing an installed OCR engine.
    (components/'active.txt').unlink()
    resumed, _ = extract_pdf(scan.read_bytes(), root/'scan-cache', lambda *_: None, lambda: None)
    assert resumed == rows
    result = {'scan_pages': 3, 'similarity': similarity, 'mixed_native_pages_skipped': True,
              'resume_without_ocr': True, 'ocr_runtime': json.loads((ocr/'runtime.json').read_text())['version']}
    (root/'result.json').write_text(json.dumps(result, indent=2))
    print(json.dumps(result))


if __name__ == '__main__': main()
