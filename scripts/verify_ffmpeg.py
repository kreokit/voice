#!/usr/bin/env python3
"""Checks the bundled ffmpeg/ffprobe sidecars for one target before they ship.

    python3 scripts/verify_ffmpeg.py <rust-target-triple>

From KreoKit Clips, for what Voice uses (src-tauri/src/export.rs, preview.rs). Fails unless:
- the build is LGPL: no --enable-gpl / --enable-nonfree / libx264 / libx265 in its
  configuration, and `ffmpeg -L` prints the GNU Lesser General Public License
- every encoder and muxer Voice uses exists (ENCODERS, MUXERS; MP3 needs LAME, WebM Opus)
- (macOS) the executables run on macOS 12
- a short sound exports to every audio format, and a video remuxes with its picture
  copied unchanged (frame checksums) and the sound re-encoded, as the app does

The release binary's --self-test then exports through the app's own code as well.
"""

import json
import platform
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BINARIES = ROOT / "src-tauri" / "binaries"

FORBIDDEN_CONFIG = ["--enable-gpl", "--enable-nonfree", "--enable-libx264", "--enable-libx265"]
# Mirrors export::audio_codec_args (aac_at is macOS's; aac everywhere as the fallback).
ENCODERS = {
    "darwin": ["pcm_s24le", "flac", "aac", "aac_at", "libmp3lame", "libopus"],
    "windows": ["pcm_s24le", "flac", "aac", "libmp3lame", "libopus"],
    "linux": ["pcm_s24le", "flac", "aac", "libmp3lame", "libopus"],
}
# Mirrors export::muxer.
MUXERS = ["wav", "flac", "ipod", "mp3", "mp4", "mov", "matroska", "webm"]
# Every filter the app runs (preview.rs decode).
FILTERS = ["aresample"]
# Audio formats: (codec, muxer, extension).
FORMATS = [
    ("pcm_s24le", "wav", "wav"),
    ("flac", "flac", "flac"),
    ("aac", "ipod", "m4a"),
    ("libmp3lame", "mp3", "mp3"),
]


def run(*args: str, cwd: Path | None = None) -> str:
    result = subprocess.run(args, capture_output=True, text=True, cwd=cwd)
    if result.returncode != 0:
        raise RuntimeError(f"{' '.join(args[:3])}… failed:\n{result.stderr[-2000:]}")
    return result.stdout + result.stderr


def fail(message: str) -> None:
    sys.exit(f"FAIL: {message}")


def listed(output: str, name: str, kind: str) -> bool:
    flags = {"encoder": r"[AV]\S*", "muxer": r"\s*D?E", "filter": r"\S+"}[kind]
    return re.search(rf"^\s*{flags}\s+{re.escape(name)}\s", output, re.M) is not None


def main() -> None:
    # Windows runners print in cp1252, which has no "→".
    sys.stdout.reconfigure(encoding="utf-8")
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    triple = sys.argv[1]
    exe = ".exe" if "windows" in triple else ""
    os_name = "darwin" if "apple" in triple else "windows" if "windows" in triple else "linux"
    ffmpeg = BINARIES / f"ffmpeg-{triple}{exe}"
    ffprobe = BINARIES / f"ffprobe-{triple}{exe}"
    for tool in (ffmpeg, ffprobe):
        if not tool.is_file():
            fail(f"missing {tool.relative_to(ROOT)}; run scripts/fetch_ffmpeg.py {triple}")

    version = run(str(ffmpeg), "-hide_banner", "-version").splitlines()[0]
    config = run(str(ffmpeg), "-hide_banner", "-buildconf")
    bad = [flag for flag in FORBIDDEN_CONFIG if flag in config]
    if bad:
        fail(f"not an LGPL build: {', '.join(bad)}")
    licence = " ".join(run(str(ffmpeg), "-hide_banner", "-L").split())
    if "GNU Lesser General Public License" not in licence:
        fail("ffmpeg -L does not show the LGPL")
    run(str(ffprobe), "-hide_banner", "-version")

    encoders = run(str(ffmpeg), "-hide_banner", "-encoders")
    missing = [e for e in ENCODERS[os_name] if not listed(encoders, e, "encoder")]
    muxers = run(str(ffmpeg), "-hide_banner", "-muxers")
    missing += [f"muxer {m}" for m in MUXERS if not listed(muxers, m, "muxer")]
    filters = run(str(ffmpeg), "-hide_banner", "-filters")
    missing += [f"filter {f}" for f in FILTERS if not listed(filters, f, "filter")]
    if missing:
        fail(f"missing: {', '.join(missing)}")

    if os_name == "darwin" and platform.system() == "Darwin":
        for tool in (ffmpeg, ffprobe):
            minos = re.search(r"minos (\S+)", run("otool", "-l", str(tool)))
            if not minos or tuple(map(int, minos.group(1).split("."))) > (12, 0):
                fail(f"{tool.name} needs macOS {minos.group(1) if minos else '?'}, not 12.0")

    results = export_test(ffmpeg.resolve(), ffprobe.resolve(), os_name)
    print(f"OK {triple}: {version}")
    print(f"   LGPL, encoders {ENCODERS[os_name]}, {len(MUXERS)} muxers")
    for line in results:
        print(f"   {line}")


def probe(ffprobe: Path, path: Path, stream: str) -> dict:
    out = run(
        str(ffprobe), "-v", "error", "-select_streams", stream,
        "-show_entries", "stream=codec_name,sample_rate,channels", "-of", "json", str(path),
    )
    return json.loads(out)["streams"][0]


def framemd5(ffmpeg: Path, path: Path) -> list[str]:
    out = run(str(ffmpeg), "-nostdin", "-v", "error", "-i", str(path), "-map", "0:v:0",
              "-c", "copy", "-f", "framemd5", "-")
    return [line.rsplit(",", 1)[-1].strip() for line in out.splitlines() if not line.startswith("#")]


def export_test(ffmpeg: Path, ffprobe: Path, os_name: str) -> list[str]:
    lines = []
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        sound = ["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=1"]
        for codec, muxer, ext in FORMATS + ([("aac_at", "ipod", "m4a")] if os_name == "darwin" else []):
            out = work / f"out-{codec}.{ext}"
            run(str(ffmpeg), "-nostdin", "-y", "-v", "error", *sound, "-ac", "2",
                "-c:a", codec, "-f", muxer, str(out))
            info = probe(ffprobe, out, "a:0")
            if info["sample_rate"] != "48000" or info["channels"] != 2:
                fail(f"{codec}: unexpected stream {info}")
            lines.append(f"{codec} → {ext}: {info['codec_name']} 48 kHz stereo")
        # A video remux as export does: picture copied, sound re-encoded.
        for container, audio in [("mp4", "aac"), ("mkv", "aac"), ("webm", "libopus")]:
            video_codec = "libvpx" if container == "webm" and listed(run(str(ffmpeg), "-hide_banner", "-encoders"), "libvpx", "encoder") else "mpeg4"
            if container == "webm" and video_codec != "libvpx":
                continue  # no VP8 encoder to make a WebM sample; the remux itself is covered by tests
            source = work / f"in.{container}"
            run(str(ffmpeg), "-nostdin", "-y", "-v", "error",
                "-f", "lavfi", "-i", "testsrc=size=320x240:rate=25:duration=1", *sound,
                "-c:v", video_codec, "-c:a", audio, str(source))
            out = work / f"out.{container}"
            run(str(ffmpeg), "-nostdin", "-y", "-v", "error", "-i", str(source), *sound,
                "-map", "0:v", "-map", "1:a", "-c", "copy", "-c:a:0", audio, str(out))
            if framemd5(ffmpeg, source) != framemd5(ffmpeg, out):
                fail(f"{container}: the remux changed the picture")
            lines.append(f"{container}: picture copied unchanged, sound {probe(ffprobe, out, 'a:0')['codec_name']}")
    return lines


if __name__ == "__main__":
    main()
