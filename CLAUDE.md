# KreoKit Voice: build spec for Claude Code

Read this before every task. It is the source of truth for scope and architecture.
**Status (2026-10-22):** milestones 1 (Scaffold), 2 (Fast clean) and 3 (Best model) built and checked in the app on the dev Mac; waiting for the user's test. Best = DPDFNet-2 48 kHz, mirrored in the `models-v1` pre-release. The question to Xiph was posted as xiph/rnnoise#287 (waiting for an answer).
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
- **Model licence rule (decided by the user, 2026-10-21):** a model's weights need an **explicit permissive licence (MIT / Apache-2.0 / BSD) from the publisher** (a licence file covering the weights, or the official model card); a code licence that says "all code" does not count. **No non-commercial or research-only terms** in the model's own licence or card. **Training data checked and documented** in `THIRD-PARTY-LICENSES.md` (each dataset and its licence; open-licence data such as CC BY / CC BY-SA is acceptable). When two options work, **prefer the cleaner one**.
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

## Models (decided by the user, 2026-10-21; research in `docs/model-research.md`)

- **Fast: RNNoise v0.2** (Xiph's 2024–25 model), **compiled from Xiph's BSD-3 C code** (`cc` in build.rs, model data compiled in, pinned by SHA-256), not `nnnoiseless` (its built-in weights are the 2017 v0.1 ones, trained partly on the commercial NTT speech database, and it can't load v0.2 models). The v0.2 training data (OpenSLR TTS sets CC BY-SA 4.0, Hi-Fi TTS CC BY 4.0, DEMAND CC BY 4.0, Xiph crowd noise CC0) is **accepted**. The sources of `foreground_noise` and the measured RIRs are undocumented: asked in https://github.com/xiph/rnnoise/issues/287 (posted 2026-10-21 with the user's OK; `docs/xiph-question.md`). Record the answer in THIRD-PARTY.
- **Best: DPDFNet** (Ceva, 48 kHz, weights explicitly Apache-2.0 on the Hugging Face model card) is **allowed under the practical rule**; its training data (DNS4, MLS, MUSAN, FSD50K) is documented in THIRD-PARTY. In M3, **measure it against RNNoise** (SI-SDR improvement, reverberant fixtures, real-time factor on the dev i5) and ship it as "Best" **only if it is clearly better and fast enough**; otherwise v0.1 ships Fast only. Runs through `ort` 2.0.0-rc.10 (ONNX), mirrored in `models-v1` (pre-release).
- **DeepFilterNet 3: rejected** (no licence for the weights; four open issues unanswered).
- **Echo / reverb: own spectral late-reverb suppression** (Lebart 2001 / Habets style, Rust DSP, no training data) in M4, **shipped only if measurements show it helps**; otherwise the UI says "room echo reduction is coming". **No WPE** (NTT patent family, ~2029).

## Scaffold (milestone 1, built 2026-10-21)

- **Copied:** from Clips the tokens, base styles, UI kit (`src/ui/`, with `logic.test.ts`), top bar, macOS overlay title bar, `ffmpeg.rs`, `probe.rs`, timeline maths (`src/lib/timeline.ts`) and styles, CI; from Cutout the drop zone, `recent.rs`, `relativeTime`. Plus `deny.toml`, `third_party.py`, `check_macos_binary.py`, issue templates.
- **Probe** (`probe.rs`): duration, container, the first audio stream (codec, rate, channels, layout), `audio_start_ms`, and the picture of videos (cover art isn't one). Files without sound are refused.
- **GPL guard** (`ffmpeg::gpl_only`): FFmpeg 9's GPL-only filters and GPL/non-free encoders; every module's FFmpeg command builder has a test that runs its arguments through it.
- **Preview cache** (`preview.rs`, decided by the user: 16-bit): `<app cache>/preview/<FNV hash of the path>/` holds `audio.pcm` (48 kHz interleaved i16; mono stays mono, more channels become stereo), `peaks.bin` (one dB-scaled level per 10 ms, computed while decoding) and `meta.json` (written last; source size + mtime, so a changed file is decoded again). Sample 0 is the file's 0:00 (`aresample=async=1:first_pts=0`, Clips' late-audio rule); the sound ends where it ends (the late fixture's audio stops 0.13 s before its picture). Each decode works in its own `<id>.part-N` folder and is renamed into place when finished: React's StrictMode opens a file twice in dev, and a cancelled decode used to delete the running one's folder. Free space is checked first (1 GB spare). Caches of files that leave Recent are pruned (not the loaded one, not running `.part-N` folders younger than an hour). Cleaning (M2) decodes the original again in f32.
- **Playback** (`player.rs`, decided by the user: native, cpal 0.18): a player thread owns the output stream; the cache is memory-mapped (`memmap2`, pinned `=0.9.11`); the callback renders with linear interpolation when the device isn't at 48 kHz (mono on every speaker, stereo on the first two, a mono device gets the average); seeks are handed to the callback through an atomic. The player is unloaded before its cache is rewritten. The UI polls `player_state` every 100 ms and draws the playhead per frame in between. M2's A/B adds a second source to `Shared`. `KREOKIT_AUDIO_TEST=1 cargo test plays_through -- --ignored` plays through the real output (not in CI: no sound device).
- **Recent files** (`recent.rs`): `<app data>/recent.json`, 20 entries with length and audio/video, atomic writes; missing files show "Locate…" (`relocate_recent` keeps the position).
- **UI:** Home (drop zone, recent list, several files at once: all go to Recent, the first opens, refused ones get a message); Editor (facts, "the picture is kept as it is" for videos, "Reading the sound" progress with Cancel, timeline with waveform, Play/Pause, Space, ←/→ 5 s). ⌘O opens files anywhere. `KREOKIT_DEV_OPEN=/path pnpm tauri dev` opens a file at startup (debug builds only).
- **Fixtures:** `scripts/make-fixtures.sh` (macOS `say`): `fixtures/speech/hello-mono.wav`, `hello-stereo.m4a` (a different voice per channel, 44.1 kHz), `hello-late.mp4` (audio starts 0.48 s after the picture).
- **Testing the app on the dev Mac:** synthetic clicks (Swift CGEvent helper in the scratchpad) only reach the window when it is frontmost: `osascript -e 'tell application "System Events" to set frontmost of process "voice" to true'` before each click; window points = screenshot pixels / 2.
- **Watch at release (M6):** cpal pulls in objc2 CoreAudio/AVFAudio bindings: check weak imports with `check_macos_binary.py` on the release build for macOS 12. Linux links the system `libasound` (LGPL, dynamic; noted in THIRD-PARTY): check whether the AppImage bundles it and keep the notice. CI installs `libasound2-dev`.

## Fast clean (milestone 2, built 2026-10-21)

- **RNNoise v0.2** (`src-tauri/rnnoise/`, `rnnoise.rs`): Xiph's library sources (BSD-3) vendored at commit `70f1d25`; `build.rs` downloads `rnnoise_data-<model_version>.tar.gz` from media.xiph.org once, checks its SHA-256 (Xiph's scheme), unpacks `src/rnnoise_data.c` (78 MB of C) into OUT_DIR and compiles it in (22 s, 1.5 GB RAM on the dev Mac; ~15 MB in the binary), so the app downloads nothing for Fast. `KREOKIT_RNNOISE_MODEL=/path/tarball` builds offline. x86: Xiph's runtime CPU detection (`RNN_ENABLE_X86_RTCD`, SSE4.1/AVX2 files with their own flags; MSVC needs `OPUS_X86_MAY_HAVE_SSE4_1` etc. for `__SSE4_1__`); identical output and speed to an AVX2-only build. ARM: NEON. The C is compiled at -O2 even in dev. **Mirrored** (2026-10-22, user): the same tarball is in `models-v1`; `build.rs` falls back to it when Xiph's server fails (tested with `KREOKIT_RNNOISE_XIPH_URL` pointing at a dead URL).
- **Delay measured: 960 samples (20 ms)** on speech (`measures_the_delay`, correlation 0.99), not one frame. `clean::Cleaner` drops it, so output sample i lines up with input sample i and the output is exactly as long as the input. One streaming denoiser per channel (stereo in parallel threads), no chunk edges (`same_result_whatever_the_block_size`).
- **Cleaning job** (`clean::prepare`): cleans the 16-bit preview (not a new decode: same length and alignment as what it's compared with; 16-bit rounding is far below what RNNoise reacts to) into `clean-fast.pcm` + `clean-fast.peaks` + `clean-fast.json` (model id, frames, realtime factor) in the preview folder, via a `.part` file. Stereo on the dev Mac: ~11× faster than real time (RTF 0.09). **Export (M5) must clean the original in f32**, not the preview.
- **Strength = dry/wet blend, applied in the player** (instant, never re-runs the model; `clean::blend`). Noise left in pauses = 20·log10(1 − s) dB. **Default 85%**, measured: −16.5 dB in pauses, clean speech keeps 17.8 dB SI-SDR (16.4 at 100%), steady noise within 0.3 dB of the best SI-SDR (peak at 70–90%). Saved per file in `recent.json` (`settings.strength`, kept on reopen and Locate).
- **A/B:** the player mixes the cleaned sound at the same sample (`listen_cleaned`); Original/Cleaned control under the transport, key B. Loudness matching comes in M4. The waveform draws the original dim and the cleaned sound at the current Strength in front (`blendLevel`).
- **Quality oracle** (`clean::tests::quality_on_every_noise`, `metrics.rs`, `testsignals.rs`): `fixtures/speech/voices.wav` (two `say` voices, 13 s, pauses) mixed with seeded Rust-made noises (room = pink, fan = brown, 50 Hz hum + harmonics, keyboard clicks, gusty wind) and `babble.wav` (four `say` voices), at 0/5/10 dB SNR. **Baseline 2026-10-21 (full strength):** steady noises SI-SDR +3.3…+13.8 dB, noise in pauses −29…−46 dB; **babble: SI-SDR −2…−6 dB** (RNNoise can't separate other voices; pauses still −5…−10 dB); clean speech in → 16.4 dB SI-SDR. Asserted: steady > +2.3 dB and pauses < −25 dB; babble pauses < −3 dB; clean speech > 15 dB (full) and > 16.5 dB (default). These are the numbers DPDFNet must clearly beat in M3.

## Best model (milestone 3, built 2026-10-22)

- **Measured before shipping** (branch `m3-dpdfnet`, merged): the authors' `dpdfnet` 0.6.0 pipeline reimplemented in Rust (`dpdfnet.rs`) on `ort` =2.0.0-rc.10 (ONNX Runtime 1.22 static; no Python onnxruntime wheels exist for Intel Macs): 960-point Vorbis-window STFT, hop 480, one model call per frame with a 56 436-value state from the model's metadata (`erb_norm_init`, `spec_norm_init`), overlap-add (Vorbis is Princen-Bradley: plain 1/N). **Delay measured: 2400 samples (50 ms)**, correlation 0.997 (the Python code drops 1920 after librosa's centred STFT, which shifts by 480).
- **Results on the M2 oracle** (dev i5, one thread): `dpdfnet2_48khz_hr` steady noises **+9.1…+20.4 dB** SI-SDR (RNNoise +3.3…+13.8), pauses −38…−50 dB, **babble +3.2…−0.3 dB** (RNNoise −2…−6), clean speech **28.0 dB** (RNNoise 16.4), realtime factor **0.33**; `dpdfnet8_48khz_hr` no better (keyboard pauses only −29 dB) at 0.73. → **Best = dpdfnet2_48khz_hr** (the user's rule: clearly better and fast enough). Asserted: steady > +8 dB and pauses < −35 dB, babble > −1.5 dB, clean speech > 26 dB (`dpdfnet::tests::quality_on_every_noise`, ignored; Linux CI runs it with the model from the mirror).
- **Memory:** ~62 MB peak RSS for the whole measuring test binary; no memory gate needed (stereo = two ~10 MB sessions).
- **Mirror:** `models-v1` on kreokit/voice, **pre-release** (created 2026-10-22; `/releases/latest` is 404 → nothing is Latest): `dpdfnet2_48khz_hr.onnx` (HF `Ceva-IP/DPDFNet` revision `dd6818d`, unchanged, SHA-256 `7f0575a5…`), `LICENSE-DPDFNET.txt` (Apache-2.0), `SOURCE.md` (origin, licence, training data), `SHA256SUMS`. `models.rs` downloads only `…/releases/download/models-v1/<file>` and checks size + SHA-256; `<app data>/models/`.
- **CPU:** ONNX Runtime needs x86-64-v2 (`cpu.rs` from Cutout); Best is disabled with the reason on older CPUs; Fast always works (RNNoise picks its kernels at run time). No startup dialog.
- **App:** `clean::Model` (Fast/Best) and `Engine` per channel; caches `clean-fast.*` / `clean-best.*` side by side (switching back is instant). Stereo channels run in parallel: Best cleaned a 39 s stereo file at RTF 0.43 (2× real time) on the dev Mac. **Default Strength per model** (measured): Fast 85%, Best 95% (Best's SI-SDR keeps rising to 95–100%; clean speech > 28 dB at any strength). Per-file settings: `model`, `fast_strength`, `best_strength` (M2's `strength` is read as `fast_strength`).
- **UI:** Model (Fast "Quick, built in" / Best "Clearest · 10.5 MB download", then "Clearest, slower"); Remove noise with Best downloads first ("Downloading the Best model… 3.2 of 10.5 MB", Cancel); the speed line shows "N× faster than real time".
- **Training-data note (user-accepted practical rule):** DNS4 includes AudioSet (YouTube audio) and VoxCeleb2; FSD50K is ~11% CC BY-NC clips. All listed in THIRD-PARTY and the mirror's SOURCE.md.

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

1. 🔶 **Scaffold** (built 2026-10-21, waiting for the user's test): repo from Clips/Cutout foundations, dev port 1450, Home, probe + decode, waveform + playback.
2. 🔶 **Fast clean** (built 2026-10-21, waiting for the user's test): licence check, RNNoise (or verified alternative), chunked processing with overlap, Strength, A/B compare, progress + cancel.
3. 🔶 **Best model** (built 2026-10-22, waiting for the user's test): licence + training-data check, conversion/mirror in `models-v1` (pre-release) if needed, Fast/Best choice, memory check, speed measurement on the dev machine.
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
