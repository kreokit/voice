# FFmpeg sidecar binaries

Release builds bundle `ffmpeg` and `ffprobe` as Tauri sidecars, one pair per target, named
`ffmpeg-<target-triple>[.exe]` / `ffprobe-<target-triple>[.exe]`. The sidecar list lives in
`../tauri.release.conf.json` (Tauri refuses to start, even in `tauri dev`, when a listed
sidecar is missing). Development uses the FFmpeg on your system instead (`PATH`, or
`KREOKIT_FFMPEG` / `KREOKIT_FFPROBE`; see `src/ffmpeg.rs`).

The binaries are not committed. CI puts them here with

```bash
python3 scripts/fetch_ffmpeg.py <target-triple>   # download or build, checksum-verified
python3 scripts/verify_ffmpeg.py <target-triple>  # licence + filters + a real 9:16 export
```

## Licence

The same Windows and Linux builds as KreoKit Captions and Clips; Voice's own macOS build. All bundled FFmpeg builds are **LGPL**: built without `--enable-gpl` and
`--enable-nonfree`, so without libx264, libx265 or any other GPL-only library.
`scripts/verify_ffmpeg.py` fails the release if that ever changes: it checks the build
configuration, that `ffmpeg -L` shows the GNU Lesser General Public License, that every
encoder and muxer Voice uses is there (WAV 24-bit, FLAC, AAC (and `aac_at` on macOS), MP3 via
LAME, Opus for WebM), and that a sound exports to each audio format and a video remuxes
with its picture unchanged.

FFmpeg runs as a separate program; KreoKit doesn't link to it. Its licence text and this
source information ship with the app (THIRD-PARTY-LICENSES.md).

## Exact sources

FFmpeg **9.0.2** on every platform.

### Windows x64 and Linux x64: BtbN FFmpeg-Builds, LGPL static variant

- Release: <https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-09-30-13-08>
  (a month-end build; BtbN keeps these for two years)
- FFmpeg revision: `n9.0.2-17-g2a571b6068` (the 9.0.2 release plus 17 fixes from the
  `release/9.0` branch)
- Licence: **LGPL-3.0-or-later** (configured with `--enable-version3`); includes libass,
  freetype, fribidi, harfbuzz, fontconfig and libopenh264; `--disable-libx264
  --disable-libx265`
- Build scripts and full dependency list: <https://github.com/BtbN/FFmpeg-Builds>

| Target | File | SHA-256 |
|---|---|---|
| `x86_64-pc-windows-msvc` | `ffmpeg-n9.0.2-17-g2a571b6068-win64-lgpl-9.0.zip` | `6b264b9e6019103f601d98c292bd332fd87acf1c5e941ddff4fb71760fe63432` |
| `x86_64-unknown-linux-gnu` | `ffmpeg-n9.0.2-17-g2a571b6068-linux64-lgpl-9.0.tar.xz` | `2d41cbea0ca1a15029b638330740f78d6f5aeb062355bf425433fc938705350a` |

### macOS (Apple Silicon and Intel): built from source in CI

No trustworthy LGPL macOS builds exist, so `scripts/build-ffmpeg-macos.sh` builds one
from these official tarballs (static libraries, macOS 12.0 minimum, `--disable-autodetect`,
AudioToolbox, LAME and libopus; no captions stack, as Voice never touches the picture; no
GPL or nonfree parts). Licence: **LGPL-2.1-or-later**. Built and checked with
`verify_ffmpeg.py` on the dev Mac (Intel) on 2026-10-22: ffmpeg 27.1 MB, ffprobe 26.9 MB,
only system libraries.

| Component | Version | Source | SHA-256 | Licence |
|---|---|---|---|---|
| FFmpeg | 9.0.2 | <https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz> | `8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e` | LGPL-2.1-or-later |
| LAME | 3.100 | <https://downloads.sourceforge.net/project/lame/lame/3.100/lame-3.100.tar.gz> | `ddfe36cab873794038ae2c1210557ad34857a4b6bdc515785d1da9e175b1da1e` (the same as Homebrew's 3.100 formula) | LGPL-2.0-or-later |
| Opus | 1.5.2 | <https://downloads.xiph.org/releases/opus/opus-1.5.2.tar.gz> | `65c1d2f78b9f2fb20082c38cbe47c951ad5839345876e46941612ee87f9a7ce1` (Xiph's SHA256SUMS.txt) | BSD-3-Clause |

## Updating

Change the version, URL and SHA-256 in `scripts/fetch_ffmpeg.py` (BtbN) or
`scripts/build-ffmpeg-macos.sh` (macOS), update this file and THIRD-PARTY-LICENSES.md, and
let CI run `verify_ffmpeg.py`.

## TODO

- [ ] The BtbN executables are large (~130 MB each, statically linked with every
      dependency). A minimal custom build with only what KreoKit uses would make the
      Windows and Linux installers much smaller.
