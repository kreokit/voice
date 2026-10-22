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
#   voices.wav          the clean reference for the quality tests: ~13 s, 48 kHz mono 16-bit,
#                       two voices (Samantha, Daniel) with pauses between sentences, peaks near
#                       -4 dBFS. The tests add noise to it at known SNRs (noise made in Rust,
#                       src-tauri/src/testsignals.rs) and measure what cleaning gives back.
#   babble.wav          crowd noise for those tests: four other voices (Fred, Karen, Moira,
#                       Tessa) reading different texts at once, ~11 s, 48 kHz mono 16-bit
#   hello.webm          hello-late.mp4 as WebM (VP8 picture, Opus sound) for the release
#                       self-test: Voice's own FFmpeg can't encode a picture to make one
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

if wanted voices.wav; then
  say -v Samantha -o "$tmp/v1.aiff" "Thanks for listening to this episode. [[slnc 700]] Today we talk about recording at home."
  say -v Daniel -o "$tmp/v2.aiff" "Good sound makes people stay. [[slnc 700]] A quiet room matters more than an expensive microphone."
  ffmpeg -nostdin -y -v error -i "$tmp/v1.aiff" -i "$tmp/v2.aiff" \
    -filter_complex "[0:a]aresample=48000,apad=pad_dur=0.8[a];[1:a]aresample=48000[b];[a][b]concat=n=2:v=0:a=1,adelay=500:all=1,apad=pad_dur=0.5,loudnorm=I=-20:TP=-3:LRA=11:linear=true,aresample=48000" \
    -ac 1 -ar 48000 -c:a pcm_s16le voices.wav
  echo "wrote fixtures/speech/voices.wav"
fi

if wanted babble.wav; then
  i=0
  for v in Fred Karen Moira Tessa; do
    i=$((i + 1))
    say -v "$v" -o "$tmp/b$i.aiff" "The weather was fine and the market was busy all morning. People were talking about the new bridge, the price of bread and the football match on Sunday evening. Somebody laughed near the door."
  done
  ffmpeg -nostdin -y -v error -i "$tmp/b1.aiff" -i "$tmp/b2.aiff" -i "$tmp/b3.aiff" -i "$tmp/b4.aiff" \
    -filter_complex "[1:a]adelay=700[b];[2:a]adelay=1500[c];[3:a]adelay=2300[d];[0:a][b][c][d]amix=inputs=4:normalize=0,atrim=duration=11.5,aresample=48000,loudnorm=I=-26:TP=-6:linear=true,aresample=48000" \
    -ac 1 -ar 48000 -c:a pcm_s16le babble.wav
  echo "wrote fixtures/speech/babble.wav"
fi

if wanted hello.webm; then
  ffmpeg -nostdin -y -v error -i hello-late.mp4 -map 0:v -map 0:a \
    -c:v libvpx -b:v 150k -c:a libopus -b:a 48k hello.webm
  echo "wrote fixtures/speech/hello.webm"
fi
