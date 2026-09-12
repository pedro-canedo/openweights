"""One subprocess at a time. No Docker socket and no model loading in the web process."""
import os
import signal
import subprocess
import sys
import threading
import time
from . import store

class Queue:
    def __init__(self):
        self.stopping = threading.Event()
        self.thread = None
        self.process = None

    def start(self):
        for job in store.all_records('jobs'):
            if job['status'] in ('queued', 'running', 'cancelling', 'preparing'):
                store.update('jobs', job['id'], {'status': 'interrupted', 'error': 'Aplicação reiniciada. Retome de um checkpoint disponível.'})
        self.thread = threading.Thread(target=self.loop, daemon=True)
        self.thread.start()

    def loop(self):
        while not self.stopping.is_set():
            pending = [r for r in reversed(store.all_records('jobs')) if r['status'] == 'queued']
            if not pending:
                self.stopping.wait(1)
                continue
            job = pending[0]
            ident = job['id']
            folder = store.run_dir(ident)
            store.update('jobs', ident, {'status': 'running', 'started_at': store.now(), 'error': None})
            try:
                with open(folder / 'worker.log', 'a', encoding='utf-8') as log:
                    self.process = subprocess.Popen([sys.executable, '-u', '-m', 'app.worker', ident],
                        stdout=log, stderr=subprocess.STDOUT, start_new_session=(os.name != 'nt'),
                        **({'creationflags': subprocess.CREATE_NO_WINDOW} if os.name == 'nt' else {}))
                    signalled_at = None
                    process_started = time.monotonic()
                    while self.process.poll() is None:
                        current = store.get('jobs', ident)
                        if job['kind'] in ('prepare', 'acquire') and time.monotonic() - process_started > job['config'].get('timeout_seconds', 900):
                            store.update('jobs', ident, {'error': 'Preparação excedeu o limite de tempo. Reduza o lote ou aumente o limite.'})
                            if os.name != 'nt':
                                os.killpg(self.process.pid, signal.SIGKILL)
                            else:
                                self.process.kill()
                            break
                        if current.get('cancel_requested') or self.stopping.is_set():
                            if signalled_at is None:
                                store.update('jobs', ident, {'cancel_requested': True})
                                signalled_at = time.monotonic()
                            elif time.monotonic() - signalled_at > 60:
                                if job['kind'] in ('prepare', 'acquire') and os.name != 'nt':
                                    os.killpg(self.process.pid, signal.SIGKILL)
                                else:
                                    import psutil
                                    for child in psutil.Process(self.process.pid).children(recursive=True):
                                        try: child.kill()
                                        except psutil.NoSuchProcess: pass
                                    self.process.kill()
                        time.sleep(0.5)
                    self.process.wait()
                    current = store.get('jobs', ident)
                    cancelled = current.get('cancel_requested') or self.stopping.is_set()
                    if cancelled:
                        status, error = 'cancelled', 'Interrompido pelo usuário ou desligamento. Checkpoints existentes foram preservados.'
                    elif self.process.returncode == 0:
                        status, error = 'completed', None
                    else:
                        status, error = 'failed', current.get('error') or 'Falha no processo. Consulte os logs.'
                    store.update('jobs', ident, {'status': status, 'finished_at': store.now(), 'error': error})
            except Exception as exc:
                store.update('jobs', ident, {'status': 'failed', 'error': str(exc)})
            finally:
                self.process = None

    def stop(self):
        self.stopping.set()
        if self.thread:
            self.thread.join(timeout=70)


queue = Queue()
