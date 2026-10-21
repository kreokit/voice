# KreoKit Voice: build spec for Claude Code

Read this before every task. It is the source of truth for scope and architecture.
App #4 of the KreoKit suite. Apps #1–3 are published: KreoKit Captions (`kreokit/captions`), KreoKit Cutout (`kreokit/cutout`), KreoKit Clips (`kreokit/clips`). Reuse them wherever possible.

## What we are building

A cross-platform desktop app that makes voice recordings sound clean and professional, **fully offline**:
drop audio or video → remove background noise (and reduce room echo where possible) → polish (loudness, low rumble, harshness) → compare before/after → export, one file or a whole batch.

For video files, the picture is kept untouched (stream copy) and only the audio is replaced.

Target users: creators worldwide (podcasts, voice-overs, talking-head videos, TikToks recorded on a phone, livestream recordings). English-first UI.

## Hard rules (same as the other apps)

- **No proprietary code or assets.** Build only from public docs, open-source libraries and observed behaviour. Never copy another product's name, logo, icons or exact UI.
- **Offline first.** No servers, no accounts, no telemetry, no cloud AI APIs. The only network calls are one-time model downloads from our own mirror releases.
- **Licences must stay MIT/Apache-compatible, including model weights and their training data.** Only MIT / Apache-2.0 / BSD weights. Verify every model's licence from its official source **and check what its training data was and under which licences**, before adding it. Never use non-commercial or research-only weights. Record everything in `THIRD-PARTY-LICENSES.md`. Mirror model files in a `models-v1` release on `kreokit/voice` with licence + SOURCE.md + SHA256SUMS, **created as a pre-release** (no release in any KreoKit repo shows "Latest"). Apps download models only by tag, never `/releases/latest`.
- **FFmpeg: LGPL-only sidecars**, built and checked exactly as Captions/Clips do. Never use GPL-only filters (e.g. `boxblur`, `arnndn` if it is GPL in our build, anything needing `--enable-gpl`). Add a test that keeps GPL-only filters out of every FFmpeg command, like Clips.
- Every commit builds and passes `cargo test` and `pnpm test` on its own. Small topic-based commits. **Push after every step**, so a session limit never leaves work uncommitted.
- **Never modify the user's real files or projects for testing, not even temporarily.** Test on copies.
- Only screenshot the KreoKit window, never the full screen.
- Fixtures: only our own, synthetic (macOS `say` + generated noise), or free-licensed audio (CC0 preferred, e.g. Freesound CC0; licence + source URL per file in `fixtures/LICENSES.md`). No copyrighted podcasts, songs or videos in the repo or its history. Keep fixtures small.

## Lessons from Captions, Cutout and Clips (must follow)

- **Release builds use a newer macOS SDK than dev builds.** Weak imports can crash on older macOS. Run `check_macos_binary.py` (copy from Clips, with its documented allow-listed symbols) on every macOS release build.
- **ONNX Runtime:** `ort` 2.0.0-rc.10 + ONNX Runtime 1.22 static, pinned for Intel Mac / macOS 12 support, exactly as Cutout/Clips do.
- **Dev builds use the Mac's GPL FFmpeg; releases use our LGPL build.** Anything that works in dev can fail in the release. `verify_ffmpeg.py` must check every filter and encoder Voice uses, and the release `--self-test` must run the real export path.
- Release workflow: build → all checks (bundle, licences, macOS binary check, `--self-test`) → upload only if everything passed → **draft pre-release** → the user tests the Intel macOS build on macOS 12 → publish as a pre-release → update the `kreokit/.github` profile README row (link to `/releases`).
- Dev port: use **1450** (Captions 1420, Cutout 1430, Clips 1440).
- Measure, don't guess: when a result looks wrong, find the cause with a measurement before changing code.

## Reuse

Copy (don't reinvent):
- From **Captions / Clips**: FFmpeg sidecar scripts and `verify_ffmpeg.py`, probe, audio decode (incl. the late-audio padding rule from Clips), waveform, timeline/playback component, `awake.rs`, release workflow, `--self-test` pattern, "Report a problem".
- From **Cutout / Clips**: `ort` setup, model download/mirror/checksum code (`models.rs`), `memory.rs`, batch grid + "Export all" with conflicts first (Replace / Keep both / Cancel), Stop, summary, safe no-overwrite writing via a temporary file.
- From all: design tokens, UI kit, app shell, licence checks, issue templates, brand assets, CLAUDE.md "Release checklist".

## Stack

- Tauri 2 (Rust) + React + TypeScript + Vite, pnpm, Zustand
- Audio processing in Rust on f32 PCM (48 kHz mono/stereo internally); FFmpeg sidecar for decode, encode and remux
- Noise removal models (verify licences first; see "Models")
- No projects in v0.1: recent files list + per-file settings, like Cutout.

## Models (verify each before use; stop and ask if anything is unclear)

Candidates, in order of preference:
- **Fast:** RNNoise (Xiph, BSD-3), e.g. via the pure-Rust `nnnoiseless` port (check its licence and that its built-in weights are the BSD RNNoise weights). Tiny, real-time, no download needed if the weights are compiled in.
- **Best:** DeepFilterNet 3 (full-band 48 kHz). Code is MIT/Apache-2.0; **verify the weights' licence and the training data** (DNS Challenge and other sets; check each set's licence) before using it. Run it via its Rust implementation or ONNX through `ort`, whichever is pinned-version-safe on Intel macOS 12.
- **Echo / reverb reduction:** research permissively licensed options (model or DSP). If none is good and clean, ship v0.1 without a dedicated de-reverb and say so in the UI ("Noise removal; room echo reduction is coming"). Do not use non-commercial models.
- If a candidate fails the licence or training-data check, stop and present the alternatives with their licences.

## Core pipeline

1. **Probe + decode** (ffmpeg) to 48 kHz f32, keeping the original channel layout and the video stream info.
2. **Analyse:** loudness (EBU R128 integrated, LRA, true peak), noise floor estimate, clipping detection.
3. **Clean:** noise removal with the chosen model, processed in chunks with overlap (no clicks at chunk edges), progress + cancel. **Strength** = dry/wet mix or the model's attenuation limit (e.g. 0–100%), so voices never sound robotic at default.
4. **Polish** (all optional, sensible defaults, LGPL FFmpeg filters or own Rust DSP):
   - high-pass (rumble) ~80 Hz
   - de-ess / harshness reduction (gentle)
   - light compression
   - loudness normalisation with presets: **Social video −14 LUFS** (TikTok/Reels/Shorts/YouTube, default), **Podcast −16 LUFS**, **Custom**; true peak ≤ −1 dBTP
5. **Compare:** instant A/B (Original / Cleaned) at the same playhead, loudness-matched so "louder" doesn't fool the ear.
6. **Export:**
   - audio: WAV (24-bit), MP3 (if an LGPL-compatible encoder is in our FFmpeg build; otherwise AAC/M4A only, say so), M4A/AAC, FLAC
   - video: same container, **video stream copied untouched**, audio re-encoded (AAC 48 kHz), duration and sync verified with ffprobe
   - one file or "Export all", naming `<name>-clean.<ext>`, no silent overwrites

## Screens (v0.1)

1. **Home:** drop zone (audio or video, one or many), recent files.
2. **Editor:** waveform (original and cleaned overlaid or stacked), play / A-B toggle (key: B), loop a region, Strength slider, Fast/Best model choice, Polish panel with the loudness preset, before/after loudness readout (LUFS, true peak), Export.
3. **Batch:** list/grid of files with status, same settings for all, "Export all".

## Milestones

1. **Scaffold:** repo from Clips/Cutout foundations, dev port 1450, Home, probe + decode, waveform + playback.
2. **Fast clean:** licence check, RNNoise (or verified alternative), chunked processing with overlap, Strength, A/B compare, progress + cancel.
3. **Best model:** licence + training-data check, conversion/mirror in `models-v1` (pre-release) if needed, Fast/Best choice, memory check, speed measurement on the dev machine.
4. **Polish + loudness:** analysis (R128), high-pass, de-ess, compression, loudness presets, loudness-matched A/B, echo reduction only if a clean option exists.
5. **Export + batch + video:** audio formats, video remux with untouched video stream, batch grid, Export all, safe writing, ffprobe checks.
6. **Release:** reuse the Clips release workflow, `--self-test` (decode a short noisy fixture, clean with each model, check the noise drops by a minimum amount, normalise, export audio + a video remux, verify with ffprobe), draft pre-release v0.1.0-alpha.

## Tests (our oracle)

- `fixtures/`: synthetic speech (`say`, several voices) mixed with CC0 noise at known SNRs (fan/AC hum, street, keyboard, crowd, wind, room tone), plus the clean originals as references. Mixing script in `scripts/`, results reproducible.
- Quality per model: SI-SDR improvement and noise-floor drop vs the clean reference, logged per fixture; a minimum threshold enforced once the baseline is known. Optionally STOI (only with a permissively licensed implementation). Never PESQ (restrictive licence).
- No damage: on a clean speech fixture, cleaning must not reduce SI-SDR noticeably.
- Chunk edges: no clicks (check sample discontinuities at chunk boundaries).
- Loudness: exported files measure the preset target ±0.5 LU and true peak ≤ −1 dBTP.
- Video: exported video stream is bit-identical (stream copy), duration within one frame, audio in sync.
- Batch: N inputs → N outputs, names preserved, nothing overwritten without confirmation.
- Performance: log real-time factor per model on the dev machine.

## Dev environment

- macOS 12 Monterey, Intel Core i5 (CPU only), 16 GB RAM, limited free disk (~19 GB). Keep the Fast model real-time here; show progress for anything over 1 s.
- Windows 10/11, Linux (AppImage + .deb), macOS Apple Silicon.

## Release checklist

- After publishing, update the kreokit/.github profile README row (link to `/releases`).

## Backlog (not v0.1)

- Dedicated de-reverb / echo removal (if no clean option in v0.1)
- Filler-word and long-pause removal (with Whisper from Captions)
- Multi-track podcast (one file per speaker), auto-leveling between speakers
- Real-time mic monitoring
- A shared KreoKit models folder across apps
