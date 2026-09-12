"""Explicit real-GPU acceptance run in an isolated scratch data directory."""
import argparse
import json
import os
import subprocess
import sys
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--pdf', '--source', dest='source', required=True)
    parser.add_argument('--data', required=True)
    parser.add_argument('--runtime', required=True)
    args = parser.parse_args()
    root = Path(args.data).resolve()
    root.mkdir(parents=True, exist_ok=True)
    runtime = Path(args.runtime).resolve()
    os.environ.update(LAB_DATA=str(root), HF_HOME=str(root.parent/'cache'),
                      OW_LLAMA_DIR=str(runtime/'llama.cpp'), OW_MODELS_DIR=str(root/'published'),
                      OW_STUDIO_RUNTIME=runtime.name)
    from app import store
    from app.studio.storage import save_source
    from app.guided import prepare
    from app.v1 import Prepare, Run, prepare as enqueue, create_run
    input_path = Path(args.source)
    source = save_source(input_path.name, input_path.read_bytes())
    job = enqueue(Prepare(source_ids=[source['id']], name=input_path.stem))
    prepare(job, store.run_dir(job['id']), lambda: None)
    dataset = store.get('jobs', job['id'])['result']
    store.update('jobs', job['id'], {'status': 'completed'})
    print('PREPARED', json.dumps(dataset, ensure_ascii=False), flush=True)
    run = create_run(Run(dataset_id=dataset['id'], request_id='smoke-alienista-v1'))
    store.update('jobs', run['id'], {'status': 'running'})
    print('RUN', run['id'], flush=True)
    result = subprocess.run([sys.executable, '-u', '-m', 'app.worker', run['id']], cwd=runtime)
    store.update('jobs', run['id'], {'status': 'completed' if result.returncode == 0 else 'failed'})
    print('RESULT', json.dumps(store.get('jobs', run['id']), ensure_ascii=False), flush=True)
    raise SystemExit(result.returncode)


if __name__ == '__main__':
    main()
