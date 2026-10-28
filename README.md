<p align="center">
  <img src="assets/brand/kreokit-logo-wide.png" alt="KreoKit" width="420">
</p>

# KreoKit Voice

**Free, offline voice cleanup for your recordings.** Drop in a podcast, voice-over or talking-head video, remove background noise, polish the sound to the right loudness for TikTok, Reels, Shorts, YouTube or podcasts, and export, with no account and no upload.

Part of [KreoKit](https://github.com/kreokit), free open-source creator tools.

🌐 **Website: [kreokit.vercel.app/voice](https://kreokit.vercel.app/voice)**: download for your computer and see how to install it.

> **Status:** alpha. Download it from [Releases](https://github.com/kreokit/voice/releases) (macOS, Windows, Linux).

## What it does

- **Remove background noise** offline: **Fast** (RNNoise, built in) or **Best** (DPDFNet, a one-time 10.5 MB download), with a Strength slider.
- **Polish:** low rumble, harsh S sounds, an even level, and loudness for **Social video (−14 LUFS)**, **Podcast (−16 LUFS)** or your own target, peaks under −1 dBTP.
- **Compare** Original and Cleaned at the same moment, loudness-matched (key B).
- **Export** WAV, FLAC, M4A or MP3, each file checked after writing; videos keep their picture untouched, only the sound is replaced.
- **Batch:** the same settings for many files, Export all with time left, nothing overwritten without asking.

Unsigned alpha: see the release notes for how to open it the first time.

## Development

Needs Node 22+, pnpm 10, Rust (stable) and the [Tauri 2 prerequisites](https://tauri.app/start/prerequisites/).

```bash
pnpm install
pnpm tauri dev       # the app, with the interface on http://localhost:1450
pnpm test            # interface tests
cd src-tauri && cargo test
```

## Licence

KreoKit Voice is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. Third-party components and their licences are listed in [THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).
