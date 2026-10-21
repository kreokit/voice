# Third-party licences

KreoKit Voice is free software, dual-licensed under MIT or Apache-2.0. It includes or
uses the third-party components below, each under its own licence.

## AI models

Only models whose weights carry an explicit MIT, Apache-2.0 or BSD licence from their
publisher are used, with no non-commercial terms, and their training data is listed here.

### RNNoise v0.2 (the Fast noise remover, compiled into the app)

- **Code and weights:** Xiph.Org's RNNoise, © 2017 Mozilla, © 2007–2017 Jean-Marc Valin,
  © 2005–2017 Xiph.Org Foundation, © 2003–2004 Mark Borgerding, **BSD-3-Clause**
  (`src-tauri/rnnoise/COPYING`). Library sources from <https://gitlab.xiph.org/xiph/rnnoise>
  at commit `70f1d256acd4b34a572f999a05c87bf00b67730d`; model
  `rnnoise_data-0a8755f8e2d834eff6a54714ecc7d75f9932e845df35f8b59bc52a7cfe6e8b37.tar.gz`
  from <https://media.xiph.org/rnnoise/models/> (2025-01-30), checked against that SHA-256.
- **Training data** (Xiph: "trained using only the publicly available datasets listed"; the
  repository's `datasets.txt` and README):
  - Speech: Google's crowd-sourced TTS corpora on OpenSLR (resources 30, 32, 37, 41–44,
    61, 63–66, 69–80, 83, 86), **CC BY-SA 4.0**; Hi-Fi Multi-Speaker English TTS (OpenSLR
    109, from LibriVox), **CC BY 4.0**.
  - Noise: DEMAND (Thiemann, Ito, Vincent 2013, <https://zenodo.org/records/1227121>),
    **CC BY 4.0**; the RNNoise noise contributions (recordings donated through Xiph's demo
    page), **CC0**; synthetic noise made by Xiph; `foreground_noise` ("e.g. keyboard
    sounds"), source not documented yet.
  - Room impulse responses: Xiph's `measured_rirs`, source not documented yet.
  - The two undocumented sets were asked about in
    <https://github.com/xiph/rnnoise/issues/287>.
- The original 2017 RNNoise weights (trained partly on the commercial NTT speech database)
  are **not** used.

## Audio output

Playback uses the `cpal` crate (Apache-2.0) and the operating system's audio API:
CoreAudio on macOS, WASAPI on Windows, and on Linux the system's ALSA library
(`libasound`, **LGPL-2.1-or-later**, © the ALSA project, <https://www.alsa-project.org>),
which the app links dynamically and does not modify.

## Fonts (bundled)

Inter (interface font), © The Inter Project Authors, **SIL Open Font License 1.1**; the
licence ships with the app as `licenses/Inter-OFL.txt`.

## Icons

Lucide icons, © Lucide contributors, **ISC** licence: <https://lucide.dev>.
