#!/usr/bin/env python3
"""Checks that macOS executables really run on the oldest macOS we support.

    python3 scripts/check_macos_binary.py <file-or-.app> [...] [--min-macos 12.0]

For every Mach-O file given (for an .app: everything in Contents/MacOS and
Contents/Frameworks), fails unless:
- its minimum macOS (LC_BUILD_VERSION / LC_VERSION_MIN_MACOSX `minos`) is at most --min-macos
- it has no weak-imported symbols (except libc++'s operator new/delete and std exception
  typeinfo, and libSystem's _dispatch_once_f and __availability_version_check; see
  ALLOWED_WEAK) and no weakly linked libraries

Why: release builds use a newer SDK than dev builds. A system function that is newer than
the minimum macOS gets weak-imported, and on an older macOS it resolves to NULL; calling it
crashes with a jump to address 0 (KreoKit Captions 0.1.0/0.1.1 crashed on macOS 12 this way, through
whisper.cpp's `cblas_sgemm$NEWLAPACK$ILP64` from Accelerate, which needs macOS 13.3).
The release runners have a new macOS, so running the app there cannot catch this.
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path

# libc++'s replaceable operator new/delete and the std exception typeinfo can show up as weak
# imports (C++ lets programs replace them). Every macOS 12 libc++ has them.
# _dispatch_once_f is in every macOS since 10.6, so it never resolves to NULL on macOS 12.
# The linker adds it as a weak import to KreoKit Cutout's binary: no crate, C library, Rust std or
# ONNX Runtime archive references it (checked 2026-10-15, also before libwebp). Only this
# exact symbol from libSystem is allowed, nothing else from libdispatch.
# _availability_version_check is what compiler-generated OS-version checks
# (__builtin_available / @available) call through compiler-rt, which imports it weakly on
# purpose and checks it for NULL before calling. It exists since macOS 11; verified present
# at runtime on macOS 12.7.6 (dlsym finds it, in libxpc re-exported by libSystem). The
# runners' newer SDK adds it to both macOS builds (2026-10-15). Only this exact symbol.
ALLOWED_WEAK = re.compile(
    r"__Z(nw|na|dl|da)\S* \(from libc\+\+\)|__ZTISt\S* \(from libc\+\+\)"
    r"|_dispatch_once_f \(from libSystem\)"
    r"|__availability_version_check \(from libSystem\)"
)

MACHO_MAGICS = {
    bytes.fromhex(m)
    for m in ("cffaedfe", "cefaedfe", "feedfacf", "feedface", "cafebabe", "bebafeca")
}


def run(*args: str) -> str:
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def version(text: str) -> tuple[int, ...]:
    return tuple(int(part) for part in text.split("."))


def macho_files(path: Path) -> list[Path]:
    if path.suffix == ".app":
        dirs = [path / "Contents" / "MacOS", path / "Contents" / "Frameworks"]
        candidates = [f for d in dirs if d.is_dir() for f in sorted(d.rglob("*"))]
    else:
        candidates = [path]
    found = []
    for f in candidates:
        if f.is_file() and not f.is_symlink():
            with f.open("rb") as handle:
                if handle.read(4) in MACHO_MAGICS:
                    found.append(f)
    return found


def problems(binary: Path, min_macos: str) -> list[str]:
    found = []
    load_commands = run("otool", "-l", str(binary))
    commands = load_commands.split("Load command ")
    minos = [
        m.group(1)
        for c in commands
        if (m := re.search(r"cmd LC_BUILD_VERSION\n(?:.*\n)*?\s*minos (\S+)", c)
            or re.search(r"cmd LC_VERSION_MIN_MACOSX\n(?:.*\n)*?\s*version (\S+)", c))
    ]
    if not minos:
        found.append("no minimum macOS version in the load commands")
    for v in minos:
        if version(v) > version(min_macos):
            found.append(f"needs macOS {v}, newer than {min_macos}")

    # A weakly linked library or symbol that is missing on an older macOS is NULL at run
    # time. Code that checks for that first is fine, but nothing we ship needs it.
    for lib in re.findall(r"cmd LC_LOAD_WEAK_DYLIB\n.*\n\s*name (\S+)", load_commands):
        found.append(f"weakly linked library {lib}")
    for line in run("nm", "-m", str(binary)).splitlines():
        if "(undefined) weak external" in line:
            symbol = line.split("weak external", 1)[1].strip()
            if not ALLOWED_WEAK.fullmatch(symbol):
                found.append("weak import " + symbol)
    return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("paths", nargs="+", type=Path)
    parser.add_argument("--min-macos", default="12.0")
    args = parser.parse_args()

    failed = False
    for path in args.paths:
        binaries = macho_files(path)
        if not binaries:
            print(f"FAIL {path}: no Mach-O files found")
            failed = True
        for binary in binaries:
            found = problems(binary, args.min_macos)
            failed |= bool(found)
            print(f"{'FAIL' if found else 'ok  '} {binary}")
            for problem in found:
                print(f"       {problem}")
    if failed:
        print(f"Some binaries may crash or not start on macOS {args.min_macos}.")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
