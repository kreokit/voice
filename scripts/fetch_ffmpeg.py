#!/usr/bin/env python3
"""Puts LGPL ffmpeg + ffprobe for one target into src-tauri/binaries/ as Tauri sidecars.

    python3 scripts/fetch_ffmpeg.py <rust-target-triple> [--reuse]

--reuse (macOS): use an existing build in target-ffmpeg/<arch> (e.g. restored from the CI
cache) instead of building again.

Windows and Linux: downloads a pinned BtbN FFmpeg-Builds *LGPL* static build, checks its
SHA-256 and extracts the two executables. macOS: builds FFmpeg from source with
scripts/build-ffmpeg-macos.sh (no trustworthy LGPL macOS builds exist).

Every source, version and checksum is listed in src-tauri/binaries/README.md; keep the
two in sync. Run scripts/verify_ffmpeg.py afterwards to check the licence and features.
"""

import hashlib
import io
import platform
import shutil
import subprocess
import sys
import tarfile
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BINARIES = ROOT / "src-tauri" / "binaries"

# BtbN/FFmpeg-Builds monthly build (kept for two years), FFmpeg 9.0 release branch, LGPL.
BTBN_TAG = "autobuild-2026-09-30-13-08"
BTBN_NAME = "ffmpeg-n9.0.2-17-g2a571b6068-{platform}-lgpl-9.0"
BTBN = {
    "x86_64-pc-windows-msvc": (
        "win64",
        ".zip",
        "6b264b9e6019103f601d98c292bd332fd87acf1c5e941ddff4fb71760fe63432",
    ),
    "x86_64-unknown-linux-gnu": (
        "linux64",
        ".tar.xz",
        "2d41cbea0ca1a15029b638330740f78d6f5aeb062355bf425433fc938705350a",
    ),
}

MACOS = {
    "x86_64-apple-darwin": "x86_64",
    "aarch64-apple-darwin": "arm64",
}


def download(url: str, sha256: str) -> bytes:
    print(f"downloading {url}", flush=True)
    with urllib.request.urlopen(url) as response:
        data = response.read()
    digest = hashlib.sha256(data).hexdigest()
    if digest != sha256:
        sys.exit(f"checksum mismatch for {url}:\n  expected {sha256}\n  got      {digest}")
    return data


def install(name: str, data: bytes, triple: str, exe: str) -> None:
    target = BINARIES / f"{name}-{triple}{exe}"
    target.write_bytes(data)
    target.chmod(0o755)
    print(f"wrote {target.relative_to(ROOT)} ({len(data) // 1_000_000} MB)")


def fetch_btbn(triple: str) -> None:
    platform_name, ext, sha256 = BTBN[triple]
    folder = BTBN_NAME.format(platform=platform_name)
    url = f"https://github.com/BtbN/FFmpeg-Builds/releases/download/{BTBN_TAG}/{folder}{ext}"
    data = download(url, sha256)
    exe = ".exe" if "windows" in triple else ""
    wanted = {f"{folder}/bin/{tool}{exe}": tool for tool in ("ffmpeg", "ffprobe")}
    wanted[f"{folder}/LICENSE.txt"] = "LICENSE"
    found = {}
    if ext == ".zip":
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            for member, tool in wanted.items():
                found[tool] = archive.read(member)
    else:
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:xz") as archive:
            for member, tool in wanted.items():
                found[tool] = archive.extractfile(member).read()
    for tool in ("ffmpeg", "ffprobe"):
        install(tool, found[tool], triple, exe)
    (BINARIES / "FFMPEG-LICENSE.txt").write_bytes(found["LICENSE"])


def build_macos(triple: str, reuse: bool) -> None:
    arch = MACOS[triple]
    if platform.system() != "Darwin":
        sys.exit("macOS FFmpeg is built from source and needs a Mac")
    out = ROOT / "target-ffmpeg" / arch
    if reuse and (out / "bin" / "ffmpeg").is_file() and (out / "bin" / "ffprobe").is_file():
        print(f"reusing {out.relative_to(ROOT)}")
    else:
        subprocess.run([str(ROOT / "scripts" / "build-ffmpeg-macos.sh"), arch, str(out)], check=True)
    for tool in ("ffmpeg", "ffprobe"):
        install(tool, (out / "bin" / tool).read_bytes(), triple, "")
    shutil.copy(out / "LICENSE.md", BINARIES / "FFMPEG-LICENSE.txt")


def main() -> None:
    args = [a for a in sys.argv[1:] if a != "--reuse"]
    if len(args) != 1:
        sys.exit(__doc__)
    triple = args[0]
    BINARIES.mkdir(parents=True, exist_ok=True)
    if triple in BTBN:
        fetch_btbn(triple)
    elif triple in MACOS:
        build_macos(triple, "--reuse" in sys.argv)
    else:
        sys.exit(f"no FFmpeg source for {triple}; known: {', '.join([*BTBN, *MACOS])}")


if __name__ == "__main__":
    main()
