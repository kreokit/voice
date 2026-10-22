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
  from <https://media.xiph.org/rnnoise/models/> (2025-01-30; mirrored unchanged in this
  project's `models-v1` release for the build), checked against that SHA-256.
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
- The licence ships with the app as `licenses/RNNOISE-COPYING.txt`.

### DPDFNet-2, 48 kHz (the Best noise remover, downloaded on first use, not bundled)

- **Weights and code:** © Ceva Technologies, Ltd., **Apache-2.0**, stated for the weights on
  the official model card (<https://huggingface.co/Ceva-IP/DPDFNet>, `license: apache-2.0`)
  and in <https://github.com/ceva-ip/DPDFNet> (`LICENSE`, shipped with the app as
  `licenses/MODEL-DPDFNET-LICENSE.txt`). Paper: Rika, Sapir, Gus et al., "DPDFNet: Boosting
  DeepFilterNet2 via Dual-Path RNN", arXiv:2512.16420.
- **File:** `onnx/dpdfnet2_48khz_hr.onnx` from Hugging Face revision
  `dd6818d00f50c836fed43a6243ebe49116de5964`, mirrored unchanged in this project's
  `models-v1` release (with SOURCE.md and SHA256SUMS); the app downloads it only from there
  and checks its SHA-256 (`7f0575a5…`).
- **Training data** (paper, section 3.1): the Deep Noise Suppression Challenge 4 dataset
  (Microsoft; its parts and licences are listed at
  <https://github.com/microsoft/DNS-Challenge#dataset-licenses>: LibriVox (public domain),
  PTDB-TUG (ODbL), the Edinburgh 56-speaker set, VocalSet (CC BY 4.0), CREMA-D (ODbL/DbCL),
  VoxCeleb2 (CC BY 4.0, "the copyright remains with the original owners of the video"), VCTK
  (ODC-By); noise from AudioSet (labels CC BY 4.0; YouTube audio), Freesound (CC0 only) and
  DEMAND; room responses OpenSLR 26/28 (Apache-2.0)); Multilingual LibriSpeech English
  (OpenSLR 94, CC BY 4.0); MUSAN (OpenSLR 17, CC BY 4.0); FSD50K (CC BY as a whole; each clip
  keeps its Freesound licence, and about 11% of the clips are CC BY-NC).
- Used under KreoKit's model licence rule: an explicit permissive licence for the weights
  from their publisher, no non-commercial terms in the model's own licence or card, and the
  training data documented here. DeepFilterNet 3 is not used (no licence stated for its
  weights).


## ONNX Runtime (compiled into the app)

The Best model runs with **ONNX Runtime 1.22.0**, © Microsoft Corporation, **MIT** licence
(<https://github.com/microsoft/onnxruntime>), statically linked through the `ort` crate
(2.0.0-rc.10, MIT OR Apache-2.0) from pyke's prebuilt libraries. ONNX Runtime contains
third-party components (for example protobuf, Abseil, Eigen, XNNPACK), each under its own
licence; their notices ship with the app as `licenses/ONNXRUNTIME-ThirdPartyNotices.txt`,
next to `licenses/ONNXRUNTIME-LICENSE.txt`.

On Windows, ONNX Runtime is linked against `DirectML.dll`. KreoKit Voice does **not** ship
that file: the copy that comes with Windows 10 (version 1903 or later) and Windows 11 is
used, and the model runs on the CPU only.

## FFmpeg (bundled program)

KreoKit Voice bundles `ffmpeg` and `ffprobe` from **FFmpeg 9.0.2** as separate programs
(sidecars); the app runs them, it does not link to them.

- Licence: **GNU Lesser General Public License** (LGPL-2.1-or-later for the macOS builds,
  LGPL-3.0-or-later for the Windows and Linux builds). No GPL or non-free components are
  enabled. Full text: `licenses/FFMPEG-LICENSE.txt` in the app, and
  <https://www.gnu.org/licenses/lgpl-3.0.html>.
- Source code: FFmpeg <https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz>. The exact builds,
  their sources and SHA-256 checksums are listed in
  [src-tauri/binaries/README.md](src-tauri/binaries/README.md):
  - Windows / Linux: BtbN FFmpeg-Builds, LGPL variant, release `autobuild-2026-09-30-13-08`
    (revision `n9.0.2-17-g2a571b6068`), <https://github.com/BtbN/FFmpeg-Builds>
  - macOS: built from source by `scripts/build-ffmpeg-macos.sh` in this repository
- You may replace the bundled FFmpeg with your own build: put it next to the app's
  executable under the same name, or point `KREOKIT_FFMPEG` / `KREOKIT_FFPROBE` at it.

Libraries built into the bundled FFmpeg that Voice uses:

| Library | Use | Licence | Source |
|---|---|---|---|
| LAME 3.101 (macOS build); Windows/Linux: BtbN's LAME SVN trunk r6835 (4.1 alpha) | MP3 export | LGPL-2.0-or-later | <https://lame.sourceforge.io> |
| libopus 1.5.2 | Opus sound in WebM videos | BSD-3-Clause | <https://opus-codec.org> |

The Windows and Linux builds contain further LGPL-compatible libraries enabled by BtbN's
LGPL variant; the complete list is in `ffmpeg -buildconf` and in BtbN's build scripts. On
macOS, AAC is encoded by Apple's AudioToolbox (part of macOS).

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
