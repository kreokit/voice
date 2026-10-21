<p align="center">
  <img src="assets/brand/kreokit-logo-wide.png" alt="KreoKit" width="420">
</p>

# KreoKit Voice

**Free, offline voice cleanup for your recordings.** Drop in a podcast, voice-over or talking-head video, remove background noise, polish the sound to the right loudness for TikTok, Reels, Shorts, YouTube or podcasts, and export, with no account and no upload.

Part of [KreoKit](https://github.com/kreokit), free open-source creator tools.

> **Status:** in development. Not ready to download yet.

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
