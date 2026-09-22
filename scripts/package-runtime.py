"""Package a pinned CUDA build of llama.cpp, including redistributable dependencies.

Usage:
  python scripts/package-runtime.py SOURCE BIN OUTPUT \
      --tag TAG --revision SHA --source-id ID --backend cuda-13.4 \
      --require-flag --decision-seqs [--binaries llama-server,llama-bench]

Writes `<OUTPUT>/openweights-<TAG>-<os>-x64-<backend sem hífen>.<zip|tar.gz>`
plus its `.sha256`, with a `runtime.json` manifest inside that the app
compares byte for byte with the identity it expects. Does not publish or
change any installed application runtime.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile


def package(source, binaries, output, *, tag, revision, source_id, backend, require_flag, names):
    head = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
    if head != revision:
        raise RuntimeError(f"Source revision {head} differs from the app's pin {revision}")
    windows = platform.system() == "Windows"
    os_name = "windows" if windows else "linux"
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="ow-runtime-package-") as temporary:
        staging = Path(temporary) / "runtime"
        staging.mkdir()
        for path in binaries.iterdir():
            if path.is_file() and (path.suffix in (".dll", ".exe") if windows else (".so" in path.name or path.name.startswith("llama-"))):
                shutil.copy2(path, staging / path.name, follow_symlinks=True)
        cuda = Path(os.environ.get("CUDA_PATH", "/usr/local/cuda"))
        if windows:
            libs = cuda / "bin"
            if (libs / "x64").is_dir():
                libs = libs / "x64"
            for pattern in ("cudart64*.dll", "cublas64*.dll", "cublasLt64*.dll"):
                matches = list(libs.glob(pattern))
                if not matches:
                    raise RuntimeError(f"Missing CUDA dependency {pattern}")
                for path in matches:
                    shutil.copy2(path, staging / path.name)
        else:
            # Flatten dependencies and give them an origin-relative search path.
            # Following every symlink would copy libcublas three times under
            # three names — hundreds of megabytes of the same file. The real
            # file is copied once; the other names stay symlinks, which the
            # loader needs and both tar and the app's extractor preserve.
            for pattern in ("libcudart.so*", "libcublas.so*", "libcublasLt.so*"):
                matches = list((cuda / "lib64").glob(pattern))
                if not matches:
                    raise RuntimeError(f"Missing CUDA dependency {pattern}")
                for path in matches:
                    if path.is_symlink():
                        (staging / path.name).symlink_to(os.readlink(path).split("/")[-1])
                    else:
                        shutil.copy2(path, staging / path.name)
        shutil.copy2(source / "LICENSE", staging / "LLAMA-LICENSE")
        cuda_license = cuda / "EULA.txt"
        if cuda_license.is_file():
            shutil.copy2(cuda_license, staging / "CUDA-EULA.txt")
        manifest = dict(source=source_id, revision=revision, backend=backend, platform=os_name)
        (staging / "runtime.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
        env = dict(os.environ)
        # Exercise the portable directory; do not accidentally resolve CUDA
        # from a developer's PATH. Linux still uses the installed GPU driver.
        env["PATH"] = str(staging) + os.pathsep + (str(Path(os.environ.get("SystemRoot", "C:/Windows")) / "System32") if windows else "/usr/bin:/bin")
        env["LD_LIBRARY_PATH"] = str(staging)
        for name in names:
            exe = staging / (name + (".exe" if windows else ""))
            if not exe.is_file():
                raise RuntimeError(f"Missing binary {exe.name}")
            result = subprocess.run([str(exe), "--help"], env=env, capture_output=True, timeout=60, text=True, encoding="utf-8", errors="replace")
            if result.returncode != 0:
                raise RuntimeError(f"Portable {name} failed: {result.stderr[-1000:]}")
            if name == "llama-server" and require_flag not in result.stdout + result.stderr:
                raise RuntimeError(f"Server does not expose the required capability {require_flag}")
        suffix = "zip" if windows else "tar.gz"
        archive = output / f"openweights-{tag}-{os_name}-x64-{backend.replace('-', '')}.{suffix}"
        if windows:
            with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as bundle:
                for path in staging.iterdir():
                    bundle.write(path, path.name)
        else:
            with tarfile.open(archive, "w:gz") as bundle:
                for path in staging.iterdir():
                    bundle.add(path, arcname=path.name)
        with archive.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        archive.with_name(archive.name + ".sha256").write_text(f"{digest}  {archive.name}\n", encoding="utf-8")
        print(archive)


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("source", type=Path)
    parser.add_argument("binaries", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--source-id", required=True, help="the `source` field of runtime.json (moeCache, decision)")
    parser.add_argument("--backend", required=True, help="e.g. cuda-13.4")
    parser.add_argument("--require-flag", required=True, help="a flag llama-server --help must list")
    parser.add_argument("--binaries", dest="names", default="llama-server", help="comma-separated executables to smoke test")
    args = parser.parse_args(argv)
    package(
        args.source.resolve(), args.binaries.resolve(), args.output.resolve(),
        tag=args.tag, revision=args.revision, source_id=args.source_id, backend=args.backend,
        require_flag=args.require_flag, names=[n.strip() for n in args.names.split(",") if n.strip()],
    )


if __name__ == "__main__":
    main(sys.argv[1:])
