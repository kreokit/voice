//! What a recording measures (milestone 4): EBU R128 loudness (integrated, loudness range,
//! true peak, via the `ebur128` crate, a port of libebur128), the noise floor and clipping.
//! Fed the same interleaved 16-bit samples as the preview cache while it is written, so
//! opening a file reads it only once.

use ebur128::{EbuR128, Mode};
use serde::{Deserialize, Serialize};

/// Levels for the noise floor: one RMS value per 50 ms.
const WINDOW_MS: usize = 50;
/// The noise floor is this quantile of the 50 ms levels: the quiet between words, not the
/// silence of a fade or a muted start (those are lower and rarer).
const FLOOR_QUANTILE: f64 = 0.10;
/// A sample this close to full scale counts as clipped when at least [`CLIP_RUN`] of them
/// follow each other in a channel (a flattened waveform top, not a single loud peak).
const CLIP_LEVEL: i16 = 32_700;
const CLIP_RUN: usize = 3;
/// Clipped runs closer than this (in samples) count as one place.
const CLIP_MERGE: usize = 480;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    /// Integrated loudness (LUFS), `None` for silence (everything below the gate).
    pub integrated_lufs: Option<f64>,
    /// Loudness range (LU), `None` when too short or silent.
    pub loudness_range_lu: Option<f64>,
    /// The highest true peak of any channel (dBTP, 4× oversampled; [`SILENT_DB`] for
    /// silence).
    pub true_peak_db: f64,
    /// The highest sample (dBFS).
    pub sample_peak_db: f64,
    /// The quiet between words (dBFS RMS), see [`FLOOR_QUANTILE`].
    pub noise_floor_db: f64,
    /// Places where the waveform is flattened at full scale.
    pub clipped_places: u32,
}

pub struct Analyzer {
    ebu: EbuR128,
    channels: usize,
    window: usize,
    window_sum: f64,
    window_count: usize,
    levels: Vec<f32>,
    /// Per channel: the current run of full-scale samples.
    runs: Vec<usize>,
    frame: u64,
    clipped_places: u32,
    last_clip: Option<u64>,
    samples: Vec<i16>,
}

/// Silence in dB (JSON has no -inf).
pub const SILENT_DB: f64 = -200.0;

fn db(power_or_amplitude: f64, power: bool) -> f64 {
    let scale = if power { 10.0 } else { 20.0 };
    if power_or_amplitude <= 0.0 {
        SILENT_DB
    } else {
        (scale * power_or_amplitude.log10()).max(SILENT_DB)
    }
}

impl Analyzer {
    pub fn new(channels: u16, sample_rate: u32) -> Result<Self, String> {
        let ebu = EbuR128::new(
            channels as u32,
            sample_rate,
            Mode::I | Mode::LRA | Mode::TRUE_PEAK | Mode::SAMPLE_PEAK,
        )
        .map_err(|e| format!("loudness meter: {e}"))?;
        Ok(Self {
            ebu,
            channels: channels as usize,
            window: sample_rate as usize * WINDOW_MS / 1000,
            window_sum: 0.0,
            window_count: 0,
            levels: Vec::new(),
            runs: vec![0; channels as usize],
            frame: 0,
            clipped_places: 0,
            last_clip: None,
            samples: Vec::new(),
        })
    }

    /// Feeds interleaved little-endian i16 bytes (whole frames).
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.samples.clear();
        self.samples.extend(
            bytes
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]])),
        );
        self.ebu
            .add_frames_i16(&self.samples)
            .map_err(|e| format!("loudness meter: {e}"))?;
        for frame in self.samples.chunks_exact(self.channels) {
            let mut power = 0.0f64;
            let mut clipped_now = false;
            for (ch, &s) in frame.iter().enumerate() {
                let v = s as f64 / 32768.0;
                power += v * v;
                if s >= CLIP_LEVEL || s <= -CLIP_LEVEL {
                    self.runs[ch] += 1;
                    if self.runs[ch] == CLIP_RUN {
                        clipped_now = true;
                    }
                } else {
                    self.runs[ch] = 0;
                }
            }
            if clipped_now {
                if self
                    .last_clip
                    .is_none_or(|last| self.frame - last > CLIP_MERGE as u64)
                {
                    self.clipped_places += 1;
                }
                self.last_clip = Some(self.frame);
            } else if self.runs.iter().any(|&r| r >= CLIP_RUN) {
                self.last_clip = Some(self.frame);
            }
            self.window_sum += power / self.channels as f64;
            self.window_count += 1;
            if self.window_count == self.window {
                self.levels
                    .push(db(self.window_sum / self.window as f64, true) as f32);
                self.window_sum = 0.0;
                self.window_count = 0;
            }
            self.frame += 1;
        }
        Ok(())
    }

    pub fn finish(self) -> Analysis {
        let peak = |f: &dyn Fn(u32) -> Result<f64, ebur128::Error>| {
            (0..self.channels as u32)
                .filter_map(|ch| f(ch).ok())
                .fold(0.0f64, f64::max)
        };
        let true_peak = peak(&|ch| self.ebu.true_peak(ch));
        let sample_peak = peak(&|ch| self.ebu.sample_peak(ch));
        let finite = |v: Result<f64, ebur128::Error>| v.ok().filter(|v| v.is_finite());
        Analysis {
            integrated_lufs: finite(self.ebu.loudness_global()),
            loudness_range_lu: finite(self.ebu.loudness_range()).filter(|_| self.frame > 0),
            true_peak_db: db(true_peak, false),
            sample_peak_db: db(sample_peak, false),
            noise_floor_db: noise_floor(&self.levels),
            clipped_places: self.clipped_places,
        }
    }
}

/// The [`FLOOR_QUANTILE`] of the 50 ms levels (dBFS); [`SILENT_DB`] without any.
fn noise_floor(levels: &[f32]) -> f64 {
    let mut audible: Vec<f32> = levels
        .iter()
        .copied()
        .filter(|&l| l as f64 > SILENT_DB)
        .collect();
    if audible.is_empty() {
        return SILENT_DB;
    }
    audible.sort_by(f32::total_cmp);
    let index = ((audible.len() - 1) as f64 * FLOOR_QUANTILE).round() as usize;
    audible[index] as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testsignals::{Noise, mix, noise, wav};

    fn analyse(samples: &[f32], channels: u16) -> Analysis {
        let mut a = Analyzer::new(channels, 48_000).unwrap();
        let bytes: Vec<u8> = samples
            .iter()
            .flat_map(|s| ((s * 32768.0).round().clamp(-32768.0, 32767.0) as i16).to_le_bytes())
            .collect();
        // In odd pieces, as the decoder delivers them.
        let frame = channels as usize * 2;
        for piece in bytes.chunks(frame * 4_801) {
            a.push(piece).unwrap();
        }
        a.finish()
    }

    fn sine(db: f64, hz: f64, seconds: f64) -> Vec<f32> {
        let amp = 10f64.powf(db / 20.0);
        (0..(48_000.0 * seconds) as usize)
            .map(|i| (amp * (2.0 * std::f64::consts::PI * hz * i as f64 / 48_000.0).sin()) as f32)
            .collect()
    }

    #[test]
    fn a_1_khz_tone_reads_as_bs_1770_says() {
        // EBU Tech 3341: a 1 kHz sine at -23 dBFS on both channels of stereo is -23 LUFS;
        // on one (mono) channel 3 dB less.
        let tone = sine(-23.0, 1_000.0, 20.0);
        let stereo: Vec<f32> = tone.iter().flat_map(|&s| [s, s]).collect();
        let a = analyse(&stereo, 2);
        assert!((a.integrated_lufs.unwrap() + 23.0).abs() < 0.1, "{a:?}");
        assert!((a.true_peak_db + 23.0).abs() < 0.1, "{a:?}");
        assert!((a.sample_peak_db + 23.0).abs() < 0.1, "{a:?}");
        assert!(a.loudness_range_lu.unwrap() < 0.5, "a steady tone: {a:?}");
        assert_eq!(a.clipped_places, 0);
        let mono = analyse(&tone, 1);
        assert!(
            (mono.integrated_lufs.unwrap() + 26.0).abs() < 0.1,
            "{mono:?}"
        );
    }

    #[test]
    fn finds_the_noise_floor_between_words() {
        let speech = wav("voices.wav");
        let room = noise(Noise::Room, speech.len(), 1);
        // Room noise 30 dB under the speech: its level shows as the floor.
        let mixed = mix(&speech, &room, 30.0);
        let added: Vec<f32> = mixed.iter().zip(&speech).map(|(m, s)| m - s).collect();
        let noise_level = crate::metrics::rms_db(&added);
        let a = analyse(&mixed, 1);
        assert!(
            (a.noise_floor_db - noise_level).abs() < 3.0,
            "floor {} vs noise {noise_level}",
            a.noise_floor_db
        );
        assert!(a.integrated_lufs.is_some() && a.loudness_range_lu.is_some());
    }

    #[test]
    fn counts_clipped_places() {
        let mut x = sine(-6.0, 200.0, 2.0);
        // Two flattened tops 0.5 s apart, and a single full-scale sample (not clipping).
        for start in [24_000usize, 48_000] {
            for s in &mut x[start..start + 20] {
                *s = 1.0;
            }
        }
        x[80_000] = 1.0;
        let a = analyse(&x, 1);
        assert_eq!(a.clipped_places, 2, "{a:?}");
        assert!(a.sample_peak_db > -0.01);
    }

    #[test]
    fn silence_has_no_loudness() {
        let a = analyse(&vec![0.0; 48_000], 1);
        assert_eq!(a.integrated_lufs, None);
        assert_eq!(a.true_peak_db, SILENT_DB);
        assert_eq!(a.noise_floor_db, SILENT_DB);
        // It survives the preview's JSON.
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<Analysis>(&json).unwrap(), a);
    }
}

#[cfg(test)]
mod speed {
    use super::*;

    /// Seconds to analyse ten minutes of stereo:
    /// `cargo test --release analysis_speed -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn analysis_speed() {
        let frames = 48_000 * 600;
        let bytes: Vec<u8> = (0..frames * 2)
            .flat_map(|i| (((i as f32 * 0.013).sin() * 8_000.0) as i16).to_le_bytes())
            .collect();
        let started = std::time::Instant::now();
        let mut a = Analyzer::new(2, 48_000).unwrap();
        for piece in bytes.chunks(1 << 20) {
            a.push(piece).unwrap();
        }
        let _ = a.finish();
        let s = started.elapsed().as_secs_f64();
        eprintln!(
            "10 min stereo analysed in {s:.1} s ({:.0}× real time)",
            600.0 / s
        );
    }
}
