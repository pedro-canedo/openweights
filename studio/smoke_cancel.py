"""Exercise cooperative cancellation and resume through the real durable queue."""
import argparse
import json
import os
import time
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--data', required=True)
    parser.add_argument('--runtime', required=True)
    args = parser.parse_args()
    root, runtime = Path(args.data).resolve(), Path(args.runtime).resolve()
    os.environ.update(LAB_DATA=str(root), HF_HOME=str(root.parent/'cache'),
                      OW_LLAMA_DIR=str(runtime/'llama.cpp'), OW_MODELS_DIR=str(root/'published'),
                      OW_STUDIO_RUNTIME=runtime.name, PYTHONIOENCODING='utf-8')
    from app import store
    from app.jobs import queue
    from app.v1 import Run, create_run, cancel, resume, detail
    queue.start()
    try:
        dataset = next(d for d in store.all_records('datasets') if d.get('trainable'))
        run = create_run(Run(dataset_id=dataset['id'], request_id=f'cancel-resume-{time.time_ns()}'))
        deadline = time.monotonic()+180
        while time.monotonic() < deadline:
            current = detail(run['id'])
            if current.get('stage') == 'training' and current['metrics']:
                cancel(run['id'])
                break
            if current['status'] == 'failed':
                raise RuntimeError(current.get('error'))
            time.sleep(.2)
        else:
            raise RuntimeError('Training did not reach a cancellation point')
        while time.monotonic() < deadline:
            current = detail(run['id'])
            if current['status'] == 'cancelled':
                assert current['checkpoints'], current
                print('CANCELLED', current['checkpoints'], flush=True)
                resume(run['id'])
                break
            time.sleep(.2)
        else:
            raise RuntimeError('Cancellation failed')
        deadline = time.monotonic()+180
        while time.monotonic() < deadline:
            current = detail(run['id'])
            if current['status'] in ('completed', 'failed'):
                print(json.dumps({k: current.get(k) for k in ('id', 'status', 'checkpoints', 'chat_model', 'error')}, ensure_ascii=False), flush=True)
                assert current['status'] == 'completed', current.get('error')
                return
            time.sleep(.5)
        raise RuntimeError('Resume timed out')
    finally:
        queue.stop()


if __name__ == '__main__':
    main()
