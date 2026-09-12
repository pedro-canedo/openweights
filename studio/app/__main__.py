"""Standalone localhost launcher, using the same private runtime as the desktop."""
import os
import secrets
from pathlib import Path


def main():
    import argparse
    import uvicorn
    parser = argparse.ArgumentParser()
    desktop = Path(os.environ.get('APPDATA', str(Path.home()/'.local/share')))/'dev.openweights.app'
    parser.add_argument('--data', default=str(desktop/'studio'))
    parser.add_argument('--port', type=int, default=7860)
    args = parser.parse_args()
    os.environ['LAB_DATA'] = str(Path(args.data).resolve())
    os.environ.setdefault('HF_HOME', str(Path(args.data).resolve()/'cache/huggingface'))
    os.environ.setdefault('OW_STUDIO_TOKEN', secrets.token_urlsafe(32))
    print(f"Abra http://127.0.0.1:{args.port}/studio.html#session={os.environ['OW_STUDIO_TOKEN']}", flush=True)
    uvicorn.run('app.main:app', host='127.0.0.1', port=args.port)


if __name__ == '__main__':
    main()
