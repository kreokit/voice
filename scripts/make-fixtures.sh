#!/usr/bin/env bash
# Generates the synthetic speech fixtures in fixtures/speech/ (macOS only: uses `say`).
# Existing files are kept (re-encoding changes the bytes); pass --force to rebuild them.
#
#   hello.txt           the text spoken in every file
#   hello-mono.wav      48 kHz mono 16-bit PCM (Samantha)
#   hello-stereo.m4a    44.1 kHz stereo AAC, a different voice per channel (Samantha / Daniel):
#                       resampling, and that both channels are kept
#   hello-late.mp4      640x360 picture + mono AAC whose audio starts 0.5 s after the picture
#                       (like phone and screen recordings): decode pads the gap with silence
set -euo pipefail

cd "$(dirname "$0")/../fixtures/speech"

FORCE="${1:-}"
TEXT="Welcome to the voice test. The quick brown fox jumps over the lazy dog."

wanted() {
  if [[ -f "$1" && "$FORCE" != "--force" ]]; then
    echo "kept fixtures/speech/$1"
    return 1
  fi
}

encoder="h264_videotoolbox"
ffmpeg -hide_banner -encoders 2>/dev/null | grep -q " $encoder " || encoder="libx264"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
echo "$TEXT" > hello.txt
say -v Samantha -o "$tmp/samantha.aiff" "$TEXT"
say -v Daniel -o "$tmp/daniel.aiff" "$TEXT"

if wanted hello-mono.wav; then
  ffmpeg -nostdin -y -v error -i "$tmp/samantha.aiff" \
    -af "apad=pad_dur=0.5" -ac 1 -ar 48000 -c:a pcm_s16le hello-mono.wav
  echo "wrote fixtures/speech/hello-mono.wav"
fi

if wanted hello-stereo.m4a; then
  ffmpeg -nostdin -y -v error -i "$tmp/samantha.aiff" -i "$tmp/daniel.aiff" \
    -filter_complex "[0:a][1:a]join=inputs=2:channel_layout=stereo,apad=pad_dur=0.5[a]" \
    -map "[a]" -ar 44100 -c:a aac -b:a 96k hello-stereo.m4a
  echo "wrote fixtures/speech/hello-stereo.m4a"
fi

if wanted hello-late.mp4; then
  ffmpeg -nostdin -y -v error \
    -f lavfi -i "color=c=0x1e1e28:s=640x360:r=25:d=5" \
    -itsoffset 0.5 -i "$tmp/samantha.aiff" \
    -map 0:v -map 1:a \
    -c:v "$encoder" -b:v 100k -pix_fmt yuv420p \
    -c:a aac -b:a 64k -ac 1 -ar 48000 \
    hello-late.mp4
  echo "wrote fixtures/speech/hello-late.mp4 with $encoder"
fi
