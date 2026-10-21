# Model research for milestones 2–4 (2026-10-21)

Licences of the code, the **weights** and the **training data** of each candidate, checked
at the official sources (links below). Nothing here is added to the app yet; the user
decides before M2/M3.

## Summary

| Candidate | Code | Weights | Training data | Verdict |
|---|---|---|---|---|
| nnnoiseless 0.5.2 (built-in weights) | BSD-3-Clause | BSD-3-Clause (RNNoise's 2017 v0.1 weights) | McGill TSP (BSD) + **NTT Multi-Lingual Speech DB** (commercial CD-ROM, no public licence) + unnamed noise | **Fails** the training-data check |
| RNNoise v0.2 (Xiph, 2024–25 models) | BSD-3-Clause | BSD-3-Clause | 26 OpenSLR TTS sets (**CC BY-SA 4.0**), Hi-Fi TTS (CC BY 4.0), DEMAND (CC BY 4.0), CC0 crowd noise, synthetic noise; `foreground_noise` + measured RIRs **undocumented** | Close; two items need an answer |
| DeepFilterNet 3 | MIT OR Apache-2.0 | **Not stated** (licence covers "all code"; `models/` has no licence; four open issues, no reply) | DNS4 (incl. AudioSet YouTube audio, VoxCeleb2, ODbL sets) | **Fails** (weights licence unclear) |
| DPDFNet (Ceva, 2025) | Apache-2.0 | **Apache-2.0** (stated on the HF model card) | DNS4 + MLS + MUSAN + **FSD50K** (contains CC BY-NC clips) | Weights clean, training data not |
| WPE de-reverb (DSP, e.g. nara_wpe) | MIT | none (no training) | none | Licence fine; **NTT patent family until ~2029** |
| Late-reverb spectral suppression (Lebart 2001 / Habets) | own Rust DSP | none | none | Clean; quality to be measured |

## RNNoise and nnnoiseless

- **Code licence:** BSD-3-Clause (`COPYING` in xiph/rnnoise and jneem/nnnoiseless; nnnoiseless
  adds "Copyright (c) 2020, Joe Neeman").
- **nnnoiseless weights:** `src/weights.rnn`, restored in commit 9a7bbbb (2022-12-14, "Restore
  the old weights"; CHANGELOG 0.5.1 "Reverts to the weights from RNNoise"). These are RNNoise's
  original 2017 model. nnnoiseless can't load the v0.2 model format.
- **v0.1 training data** (Valin, arXiv:1709.08243, §III-A "Training data"): "For speech data,
  we use the McGill TSP speech database (French and English) and the NTT Multi-Lingual Speech
  Database for Telephonometry (21 languages). Various sources of noise are used, including
  computer fans, office, crowd, airplane, car, train, construction."
  - McGill TSP: "covered by a permissive Simplified BSD licence"
    (<https://www.mmsp.ece.mcgill.ca/Documents/Data/>).
  - NTT Multi-Lingual Speech Database for Telephonometry (1994): sold on 4 CD-ROMs by NTT-AT
    (3GPP TS 26.077 annex); no public licence found. Noise sources not named.
- **v0.2 training data** (README "Training", `datasets.txt`, commits 41953aa…130913d on
  gitlab.xiph.org/xiph/rnnoise): "The models distributed with RNNoise are now trained using only
  the publicly available datasets listed below."
  - Speech: OpenSLR 30, 32, 37, 41–44, 61, 63–66, 69–80, 83, 86: each page says **CC BY-SA 4.0**;
    OpenSLR 109 (Hi-Fi TTS): **CC BY 4.0** (checked on every openslr.org page).
  - Noise: DEMAND (Zenodo 1227121: **CC BY 4.0**), the RNNoise crowd contributions
    (media.xiph.org/rnnoise/README.txt: "available under the Creative Commons CC0 license"),
    `synthetic_noise.sw`; later `background_noise(_v2).sw` = DEMAND + filtered contributions, and
    `foreground_noise(_v2).sw` ("e.g. keyboard sounds"): **source not documented**.
  - RIRs: `measured_rirs-v1..v3.tar.gz`: **source not documented**.
  - Models: media.xiph.org/rnnoise/models/rnnoise_data-<sha256>.tar.gz (latest 2025-01-30).
- **Rust route for v0.2:** compile Xiph's C (BSD) with `cc` in build.rs with the model data
  compiled in, or `rnnoise2-sys` (to check). Not `nnnoiseless`.
- **Open questions:** where `foreground_noise` and the measured RIRs come from (ask Jean-Marc
  Valin on the rnnoise tracker); whether CC BY-SA 4.0 speech is acceptable as training data.

## DeepFilterNet 3

- Repo licence (README "License"): "**All code** in this repository is dual-licensed under either
  MIT … or Apache-2.0". `models/` (DeepFilterNet3_onnx.tar.gz etc.) carries no licence file;
  GitHub reports `NOASSERTION`. Last push 2024-10-17.
- Open, unanswered issues asking exactly this: #697 (2026-07-15), #700 (2026-08-12), #709
  (2026-09-01), #712 (2026-09-21). One commenter emailed the author twice without reply.
- Training data (Schröter et al., arXiv:2305.08227): "the full multi-lingual DNS4 dataset …
  oversampling the high-quality PTDB and VCTK datasets by a factor of 10".
- DNS dataset licences (microsoft/DNS-Challenge README, "Dataset licenses"): LibriVox (public
  domain), PTDB-TUG (ODbL), Edinburgh 56 speakers, VocalSet (CC BY 4.0), CREMA-D (ODbL/DbCL),
  **VoxCeleb2** ("The copyright remains with the original owners of the video"), VCTK (ODC-By);
  noise: **AudioSet** (listed as CC BY 4.0, which covers Google's labels; the audio is from
  YouTube videos), Freesound (CC0 only), DEMAND; RIRs OpenSLR 26/28 (Apache-2.0).

## DPDFNet (ceva-ip/DPDFNet, Hugging Face Ceva-IP/DPDFNet)

- Code Apache-2.0; HF model card `license: apache-2.0` (explicit for the weights). 48 kHz
  models: dpdfnet2_48khz_hr (2.58 M params, ONNX 10 MB), dpdfnet8_48khz_hr (3.63 M, 14 MB).
- Training data (arXiv:2512.16420, §3.1): DNS4 downsampled to 16 kHz + 4,000 h Multilingual
  LibriSpeech (CC BY 4.0) + MUSAN + **FSD50K** noise; speech pre-cleaned with DPCRN. Reverb
  augmentation "enabling the model to suppress late reverberation". FSD50K's clips are each
  CC0, CC BY or **CC BY-NC**.

## Echo / reverb

- **WPE** (Nakatani, Yoshioka et al., IEEE TASLP 2010): DSP, no training data. nara_wpe
  (fgnt) is MIT. NTT patent family on delayed-linear-prediction dereverberation: CN102084667B
  (assignee NTT, inventors Nakatani, Yoshioka, Kinoshita, Miyoshi; priority 2008-03-03;
  anticipated expiry 2029-02-27 per Google Patents). US/JP counterparts not checked.
- **Late-reverberation spectral suppression** (Lebart, Boucher & Denbigh 2001; Habets 2005):
  estimates the late reverb from an exponential decay model and subtracts it in the STFT
  domain. Published > 20 years ago; own Rust DSP; no data. Gentle and limited.
- **ML models:** DPDFNet (above); DeepFilterNet (blocked). Non-commercial or LDC-data models
  (e.g. Facebook Denoiser CC BY-NC, WHAMR/WSJ0-trained SpeechBrain models) are excluded.
