from .. import store
from .extractors import safe_name
from .pipeline import digest


def save_source(name, raw, origin='upload', license=''):
    name = safe_name(name)
    if len(raw) > 32 * 1024**2 or not raw:
        raise ValueError('Arquivo vazio ou maior que 32 MiB.')
    source = store.create('sources', {'name': name, 'size': len(raw), 'sha256': digest(raw), 'origin': origin,
                                      'license': license or 'Não informada', 'status': 'uploading'})
    folder = store.ROOT / 'sources' / source['id']
    folder.mkdir(parents=True)
    temporary = folder / 'original.tmp'
    temporary.write_bytes(raw)
    temporary.replace(folder / 'original')
    return store.update('sources', source['id'], {'status': 'ready'})
