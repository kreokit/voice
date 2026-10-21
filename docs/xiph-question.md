# Draft: question to Xiph about the RNNoise v0.2 training data

**Status:** posted 2026-10-21 with the user's OK, from their account, as
https://github.com/xiph/rnnoise/issues/287 (where media.xiph.org/rnnoise/README.txt says to
ask questions about the data). Waiting for an answer.

---

**Title:** Sources and licences of foreground_noise and measured_rirs (training data for the v0.2 models)

Hi, and thank you for RNNoise and for documenting the training data so openly.

We're building a free, open-source (MIT/Apache-2.0) offline voice-cleanup app, and we'd like
to use the RNNoise v0.2 models. We list the training data and its licences for every model we
ship. Most of the v0.2 data is clearly documented: the OpenSLR speech sets in `datasets.txt`,
DEMAND, the CC0 noise contributions (media.xiph.org/rnnoise/README.txt) and the synthetic noise.

We couldn't find where two of the files come from:

1. `foreground_noise.sw` / `foreground_noise_v2.sw` (media.xiph.org/rnnoise/data/), which the
   README describes as "e.g. keyboard sounds"
2. `measured_rirs-v1/v2/v3.tar.gz` (the room impulse responses)

Could you tell us their sources, and the licence they're under (or that they're your own
recordings)? A one-line answer would be plenty, and we'd credit them correctly.

Thanks again!
