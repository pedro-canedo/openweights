"""Package a pinned CUDA runtime, including redistributable dependencies.

Usage: python scripts/package-moe-runtime.py SOURCE BIN OUTPUT
Does not publish or change any installed application runtime.
"""
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

REVISION = "b46f7f7a436f990932d3da3ec53380e2b9effc89"
TAG = "moe-runtime-b46f7f7a436f-v1"


def package(source, binaries, output):
    revision = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
    if revision != REVISION:
        raise RuntimeError("Source revision differs from the app's pin")
    windows = platform.system() == "Windows"
    os_name = "windows" if windows else "linux"
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="ow-moe-package-") as temporary:
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
        manifest = dict(source="moeCache", revision=REVISION, backend="cuda-13.3", platform=os_name)
        (staging / "runtime.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
        env = dict(os.environ)
        # Exercise the portable directory; do not accidentally resolve CUDA
        # from a developer's PATH. Linux still uses the installed GPU driver.
        env["PATH"] = str(staging) + os.pathsep + (str(Path(os.environ.get("SystemRoot", "C:/Windows")) / "System32") if windows else "/usr/bin:/bin")
        env["LD_LIBRARY_PATH"] = str(staging)
        for name in ("llama-server", "llama-bench", "llama-fit-params"):
            exe = staging / (name + (".exe" if windows else ""))
            result = subprocess.run([str(exe), "--help"], env=env, capture_output=True, timeout=60, text=True, encoding="utf-8", errors="replace")
            if result.returncode != 0:
                raise RuntimeError(f"Portable {name} failed: {result.stderr[-1000:]}")
            if name == "llama-server" and "--moe-expert-cache-size" not in result.stdout + result.stderr:
                raise RuntimeError("Server does not expose the required cache capability")
        suffix = "zip" if windows else "tar.gz"
        archive = output / f"openweights-{TAG}-{os_name}-x64-cuda13.3.{suffix}"
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


if __name__ == "__main__":
    package(*(Path(arg).resolve() for arg in sys.argv[1:]))
