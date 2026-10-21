# RNNoise (vendored)

Xiph.Org's RNNoise noise suppression, **BSD-3-Clause** (`COPYING`), the library sources only
(no training code), copied unchanged from <https://gitlab.xiph.org/xiph/rnnoise> (GitHub mirror
<https://github.com/xiph/rnnoise>) at commit `70f1d256acd4b34a572f999a05c87bf00b67730d`
(2025-02-22, after the v0.2 release).

The model weights are not in this folder: `build.rs` downloads
`https://media.xiph.org/rnnoise/models/rnnoise_data-<model_version>.tar.gz` (or, if Xiph's
server fails, the same file from our mirror, the `models-v1` pre-release on kreokit/voice),
checks it against the SHA-256 in `model_version` (Xiph's own scheme, see their
`download_model.sh`), and compiles its `src/rnnoise_data.c` into the app. Set
`KREOKIT_RNNOISE_MODEL=/path/to/that.tar.gz` to build offline. Training data of this model: see `docs/model-research.md` and
`THIRD-PARTY-LICENSES.md`.
