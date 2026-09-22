"""Package the pinned MoE-cache runtime (thin wrapper over package-runtime.py).

Usage: python scripts/package-moe-runtime.py SOURCE BIN OUTPUT
Does not publish or change any installed application runtime.
"""
import runpy
import sys
from pathlib import Path

REVISION = "b46f7f7a436f990932d3da3ec53380e2b9effc89"
TAG = "moe-runtime-b46f7f7a436f-v1"

if __name__ == "__main__":
    sys.argv = [
        str(Path(__file__).with_name("package-runtime.py")),
        *sys.argv[1:4],
        "--tag", TAG, "--revision", REVISION, "--source-id", "moeCache", "--backend", "cuda-13.3",
        # `--require-flag=` com o igual: o valor começa com dois traços e o
        # argparse o tomaria por outra opção se viesse separado.
        "--require-flag=--moe-expert-cache-size", "--binaries", "llama-server,llama-bench,llama-fit-params",
    ]
    runpy.run_path(sys.argv[0], run_name="__main__")
