"""Kernel lock compatible with Rust File::try_lock, released on process death."""
import os
from pathlib import Path
from contextlib import contextmanager
from . import store


@contextmanager
def lease():
    path = Path(os.environ.get('OW_GPU_LOCK', str(store.ROOT.parent/'gpu.lock')))
    with exclusive_file(path, 'O motor ou outro treinamento está usando a GPU. Pare-o antes de iniciar este treino.'):
        yield


@contextmanager
def exclusive_file(path, message):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('a+b') as stream:
        try:
            stream.seek(0)
            if os.name == 'nt':
                import msvcrt
                msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as exc:
            raise ValueError(message) from exc
        try:
            yield
        finally:
            stream.seek(0)
            if os.name == 'nt':
                msvcrt.locking(stream.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(stream, fcntl.LOCK_UN)
