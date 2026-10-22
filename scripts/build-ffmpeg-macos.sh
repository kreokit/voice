#!/usr/bin/env bash
# Builds an LGPL FFmpeg (ffmpeg + ffprobe) for macOS from pinned source tarballs.
#
#   scripts/build-ffmpeg-macos.sh <x86_64|arm64> <output-dir>
#
# From KreoKit Clips' script, for what Voice needs (decided 2026-10-22): static LAME 3.100
# (MP3, LGPL-2.0) and libopus 1.5.2 (Opus in WebM, BSD-3-Clause) first, then FFmpeg with
# --disable-autodetect, AudioToolbox (Apple's AAC encoder) and the decoders, demuxers and
# muxers FFmpeg enables by default. No captions stack (Voice never touches the picture:
# it is copied), no --enable-gpl, no --enable-nonfree. Minimum macOS 12.0.
#
# Needs: Xcode command line tools, pkgconf, nasm (x86_64 only). iconv comes from the system's
# libiconv, linked explicitly (Clips got it through libass's static link flags).
# Versions and checksums are listed in src-tauri/binaries/README.md; keep them in sync.
set -euo pipefail

ARCH="${1:?arch: x86_64 or arm64}"
OUT="$(mkdir -p "${2:?output dir}" && cd "$2" && pwd)"
MIN_MACOS="12.0"

# name version url sha256
SOURCES=(
  "ffmpeg 9.0.2 https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz 8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e"
  "lame 3.100 https://downloads.sourceforge.net/project/lame/lame/3.100/lame-3.100.tar.gz ddfe36cab873794038ae2c1210557ad34857a4b6bdc515785d1da9e175b1da1e"
  "opus 1.5.2 https://downloads.xiph.org/releases/opus/opus-1.5.2.tar.gz 65c1d2f78b9f2fb20082c38cbe47c951ad5839345876e46941612ee87f9a7ce1"
)

WORK="$OUT/work"
PREFIX="$OUT/deps"
rm -rf "$WORK" "$PREFIX"
mkdir -p "$WORK" "$PREFIX"

export MACOSX_DEPLOYMENT_TARGET="$MIN_MACOS"
export CFLAGS="-arch $ARCH -mmacosx-version-min=$MIN_MACOS -O2"
export CXXFLAGS="$CFLAGS"
export LDFLAGS="-arch $ARCH -mmacosx-version-min=$MIN_MACOS"
export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig"
export PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig"

fetch() {
  local name="$1" version="$2" url="$3" sha="$4"
  local file="$WORK/$(basename "$url")"
  echo "== $name $version"
  curl -fsSL -o "$file" "$url"
  echo "$sha  $file" | shasum -a 256 -c -
  tar -xf "$file" -C "$WORK"
}

for source in "${SOURCES[@]}"; do
  # shellcheck disable=SC2086
  fetch $source
done

JOBS="$(sysctl -n hw.ncpu)"

# LAME: the library only (no command-line frontend, no decoder).
(cd "$WORK/lame-3.100" &&
  ./configure --prefix="$PREFIX" --disable-shared --enable-static --disable-frontend \
    --disable-decoder --disable-dependency-tracking &&
  make -j"$JOBS" && make install)

# libopus: the library only.
(cd "$WORK/opus-1.5.2" &&
  ./configure --prefix="$PREFIX" --disable-shared --enable-static --disable-doc \
    --disable-extra-programs --disable-dependency-tracking &&
  make -j"$JOBS" && make install)

ASM=()
[[ "$ARCH" == "arm64" ]] || ASM=(--x86asmexe=nasm)

(cd "$WORK/ffmpeg-9.0.2" &&
  ./configure --prefix="$OUT" --arch="$ARCH" --target-os=darwin \
    --cc="clang -arch $ARCH" --cxx="clang++ -arch $ARCH" \
    --extra-cflags="-mmacosx-version-min=$MIN_MACOS -I$PREFIX/include" \
    --extra-ldflags="-mmacosx-version-min=$MIN_MACOS -L$PREFIX/lib" \
    --extra-libs="-liconv" \
    --pkg-config-flags=--static ${ASM[@]+"${ASM[@]}"} \
    --disable-debug --disable-doc --disable-ffplay --disable-network \
    --disable-autodetect \
    --enable-audiotoolbox \
    --enable-libmp3lame --enable-libopus \
    --enable-zlib --enable-iconv &&
  make -j"$JOBS" &&
  make install)

# Licence summary plus the full LGPL 2.1 text, shipped with the app.
cat "$WORK/ffmpeg-9.0.2/LICENSE.md" "$WORK/ffmpeg-9.0.2/COPYING.LGPLv2.1" > "$OUT/LICENSE.md"

# The executables must only depend on system libraries and frameworks.
for tool in ffmpeg ffprobe; do
  if otool -L "$OUT/bin/$tool" | tail -n +2 | grep -vE "^\s+(/usr/lib/|/System/Library/)"; then
    echo "error: $tool links to non-system libraries" >&2
    exit 1
  fi
done
echo "built $OUT/bin/ffmpeg and ffprobe for $ARCH (macOS $MIN_MACOS+)"
