"""Split a verified runtime ZIP into GitHub release assets under 2 GiB."""
import hashlib
import json
import sys
from pathlib import Path


def split(catalog_path):
    path = Path(catalog_path)
    catalog = json.loads(path.read_text())
    archive = path.parent/catalog['url'].rsplit('/', 1)[-1]
    parts = []
    with archive.open('rb') as source:
        number = 0
        while chunk := source.read(1024**3):
            number += 1
            target = archive.with_name(archive.name+f'.{number:03}')
            target.write_bytes(chunk)
            parts.append({'url': catalog['url']+f'.{number:03}', 'size': len(chunk), 'sha256': hashlib.sha256(chunk).hexdigest()})
    assert sum(part['size'] for part in parts) == catalog['size']
    catalog['parts'] = parts
    path.write_text(json.dumps(catalog, indent=2))
    print(json.dumps({'size': catalog['size'], 'parts': len(parts)}))


if __name__ == '__main__': split(sys.argv[1])
