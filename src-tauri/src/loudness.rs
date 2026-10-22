//! Loudness targets (milestone 4): the presets, and the measurement that turns a file's
//! sound plus its Polish settings into the chain's two gains (`polish::Params`).
//!
//! Passes over the whole sound (the preview, cleaned and blended at the Strength):
//! 1. its loudness → `pre_gain_db`, bringing it to the chain's working level;
//! 2. the chain without the limiter → `gain_db` to the target;
//! 3. the full chain, measured (loudness and true peak); while it misses the target by
//!    more than [`TOLERANCE_LU`] (the limiter takes a little loudness off very dynamic
//!    sound), the gain is corrected and the pass repeated, at most twice more.

// Used by the player and the commands from the next commit on.
#![cfg_attr(not(test), allow(dead_code))]

use std::sync::atomic::{AtomicBool, Ordering};

use ebur128::{EbuR128, Mode};
use serde::{Deserialize, Serialize};

use crate::polish::{Chain, Params, Stages, WORK_LUFS};

/// How close to the target the result must measure (the presets promise ±0.5 LU).
pub const TOLERANCE_LU: f64 = 0.3;
/// Gains are kept within this many dB (a near-silent file isn't boosted without end).
const MAX_GAIN_DB: f64 = 30.0;

/// The loudness preset.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Target {
    /// TikTok, Reels, Shorts, YouTube: −14 LUFS (the default).
    Social,
    /// −16 LUFS.
    Podcast,
    /// `custom_lufs`.
    Custom,
    /// Keep the original loudness (peaks still under −1 dBTP).
    Off,
}

/// The Polish settings of a file.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PolishSettings {
    #[serde(default)]
    pub stages: Stages,
    #[serde(default = "social")]
    pub target: Target,
    #[serde(default = "custom_default")]
    pub custom_lufs: f64,
}

fn social() -> Target {
    Target::Social
}

fn custom_default() -> f64 {
    -14.0
}

impl Default for PolishSettings {
    fn default() -> Self {
        Self {
            stages: Stages::default(),
            target: Target::Social,
            custom_lufs: custom_default(),
        }
    }
}

impl PolishSettings {
    /// The loudness to reach, `None` for "keep the original".
    pub fn target_lufs(&self) -> Option<f64> {
        match self.target {
            Target::Social => Some(-14.0),
            Target::Podcast => Some(-16.0),
            Target::Custom => Some(self.custom_lufs.clamp(-30.0, -9.0)),
            Target::Off => None,
        }
    }
}

/// What the measurement found: the chain's parameters and the result's readings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Measured {
    pub params: Params,
    /// The sound before Polish (cleaned at the Strength), for the loudness-matched A/B.
    pub input_lufs: Option<f64>,
    pub output_lufs: Option<f64>,
    pub output_true_peak_db: f64,
}

/// A source of blocks of interleaved samples, from the start each time it is called.
pub trait Source {
    fn channels(&self) -> usize;
    /// Calls `each` with consecutive blocks of the whole sound.
    fn for_each_block(
        &self,
        each: &mut dyn FnMut(&[f32]) -> Result<(), String>,
    ) -> Result<(), String>;
    /// Samples per channel in the whole sound (for progress).
    fn frames(&self) -> u64;
}

struct Meter {
    ebu: EbuR128,
}

impl Meter {
    fn new(channels: usize, true_peak: bool) -> Result<Self, String> {
        let mode = if true_peak {
            Mode::I | Mode::TRUE_PEAK
        } else {
            Mode::I
        };
        Ok(Self {
            ebu: EbuR128::new(channels as u32, 48_000, mode).map_err(|e| e.to_string())?,
        })
    }

    fn add(&mut self, block: &[f32]) -> Result<(), String> {
        self.ebu.add_frames_f32(block).map_err(|e| e.to_string())
    }

    fn lufs(&self) -> Option<f64> {
        self.ebu.loudness_global().ok().filter(|l| l.is_finite())
    }

    fn true_peak_db(&self, channels: usize) -> f64 {
        let peak = (0..channels as u32)
            .filter_map(|c| self.ebu.true_peak(c).ok())
            .fold(0.0f64, f64::max);
        if peak > 0.0 {
            20.0 * peak.log10()
        } else {
            crate::analysis::SILENT_DB
        }
    }
}

/// Finds the chain's gains for `source` with `settings`. `progress` gets 0–1 over all the
/// passes; `cancel` stops it.
pub fn measure(
    source: &dyn Source,
    settings: &PolishSettings,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f64),
) -> Result<Measured, String> {
    let channels = source.channels();
    // Passes: input, chain, then 1–3 checks. Progress assumes 3 and finishes at 1.
    let total = source.frames().max(1) as f64 * 3.0;
    let mut done = 0.0f64;
    let mut step = |frames: usize, progress: &mut dyn FnMut(f64)| {
        done += frames as f64;
        progress((done / total).min(0.99));
    };

    let check = |cancel: &AtomicBool| {
        if cancel.load(Ordering::Relaxed) {
            Err("cancelled".to_string())
        } else {
            Ok(())
        }
    };

    // 1. The input's loudness.
    let mut meter = Meter::new(channels, false)?;
    source.for_each_block(&mut |block| {
        check(cancel)?;
        meter.add(block)?;
        step(block.len() / channels, progress);
        Ok(())
    })?;
    let input_lufs = meter.lufs();
    let clamp = |db: f64| db.clamp(-MAX_GAIN_DB, MAX_GAIN_DB);
    let pre_gain_db = input_lufs.map_or(0.0, |l| clamp(WORK_LUFS - l));

    // Runs the chain over the whole sound and measures the result.
    let mut run = |params: Params,
                   true_peak: bool,
                   progress: &mut dyn FnMut(f64)|
     -> Result<(Option<f64>, f64), String> {
        let mut chain = Chain::new(channels, params);
        let mut meter = Meter::new(channels, true_peak)?;
        let mut buffer = Vec::new();
        source.for_each_block(&mut |block| {
            check(cancel)?;
            buffer.clear();
            buffer.extend_from_slice(block);
            chain.process(&mut buffer);
            meter.add(&buffer)?;
            step(block.len() / channels, progress);
            Ok(())
        })?;
        let mut tail = Vec::new();
        chain.flush(&mut tail);
        meter.add(&tail)?;
        Ok((meter.lufs(), meter.true_peak_db(channels)))
    };

    // 2. The chain without the limiter → the gain to the target.
    let mut params = Params {
        stages: settings.stages,
        pre_gain_db: pre_gain_db as f32,
        gain_db: 0.0,
        limit: false,
    };
    let (chained, _) = run(params, false, progress)?;
    let target = settings.target_lufs().or(input_lufs);
    params.gain_db = match (target, chained) {
        (Some(t), Some(l)) => clamp(t - l) as f32,
        _ => -pre_gain_db as f32,
    };
    params.limit = true;

    // 3. The full chain, checked and corrected.
    let mut result = run(params, true, progress)?;
    for _ in 0..2 {
        let (Some(t), Some(l)) = (target, result.0) else {
            break;
        };
        if (t - l).abs() <= TOLERANCE_LU {
            break;
        }
        params.gain_db = clamp(params.gain_db as f64 + t - l) as f32;
        result = run(params, true, progress)?;
    }
    progress(1.0);
    Ok(Measured {
        params,
        input_lufs,
        output_lufs: result.0,
        output_true_peak_db: result.1,
    })
}

/// A whole sound in memory.
pub struct Buffer<'a> {
    pub samples: &'a [f32],
    pub channels: usize,
}

impl Source for Buffer<'_> {
    fn channels(&self) -> usize {
        self.channels
    }

    fn for_each_block(
        &self,
        each: &mut dyn FnMut(&[f32]) -> Result<(), String>,
    ) -> Result<(), String> {
        for block in self.samples.chunks(48_000 * self.channels) {
            each(block)?;
        }
        Ok(())
    }

    fn frames(&self) -> u64 {
        (self.samples.len() / self.channels) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clean::clean;
    use crate::testsignals::{Noise, mix, noise, wav};

    fn measured(samples: &[f32], channels: usize, settings: PolishSettings) -> Measured {
        measure(
            &Buffer { samples, channels },
            &settings,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap()
    }

    /// The result of the chain with the measured params, measured independently.
    fn check(samples: &[f32], channels: usize, m: &Measured) -> (f64, f64) {
        let out = crate::polish::run(&mut Chain::new(channels, m.params), samples);
        let mut meter = EbuR128::new(channels as u32, 48_000, Mode::I | Mode::TRUE_PEAK).unwrap();
        meter.add_frames_f32(&out).unwrap();
        let tp = (0..channels as u32)
            .map(|c| meter.true_peak(c).unwrap())
            .fold(0.0, f64::max);
        (meter.loudness_global().unwrap(), 20.0 * tp.log10())
    }

    #[test]
    fn presets() {
        let mut s = PolishSettings::default();
        assert_eq!(s.target_lufs(), Some(-14.0));
        s.target = Target::Podcast;
        assert_eq!(s.target_lufs(), Some(-16.0));
        s.target = Target::Custom;
        s.custom_lufs = -5.0;
        assert_eq!(s.target_lufs(), Some(-9.0), "clamped");
        s.target = Target::Off;
        assert_eq!(s.target_lufs(), None);
        // Settings saved before a field existed read its default.
        let old: PolishSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(old, PolishSettings::default());
    }

    /// The presets' promise (CLAUDE.md "Tests"): exports measure the target ±0.5 LU and true
    /// peak ≤ −1 dBTP, here on cleaned speech for every preset, quiet and loud recordings,
    /// mono and stereo.
    #[test]
    fn reaches_every_target_within_half_a_lu_under_minus_1_dbtp() {
        let speech = wav("voices.wav");
        let noisy = mix(&speech, &noise(Noise::Fan, speech.len(), 2), 15.0);
        let cleaned = clean(&noisy, 1);
        for level_db in [-18.0f32, 0.0] {
            let gain = 10f32.powf(level_db / 20.0);
            let mono: Vec<f32> = cleaned.iter().map(|s| s * gain).collect();
            let stereo: Vec<f32> = mono.iter().flat_map(|&s| [s, s * 0.8]).collect();
            for (samples, channels) in [(&mono, 1usize), (&stereo, 2)] {
                for target in [Target::Social, Target::Podcast, Target::Custom] {
                    let settings = PolishSettings {
                        target,
                        custom_lufs: -20.0,
                        ..Default::default()
                    };
                    let m = measured(samples, channels, settings);
                    let want = settings.target_lufs().unwrap();
                    let (lufs, tp) = check(samples, channels, &m);
                    eprintln!(
                        "{target:?} input {level_db:>5} dB {channels} ch: {lufs:.2} LUFS (target {want}), {tp:.2} dBTP"
                    );
                    assert!(
                        (lufs - want).abs() <= 0.5,
                        "{target:?} {level_db} dB {channels} ch: {lufs} LUFS"
                    );
                    assert!(
                        tp <= -1.0,
                        "{target:?} {level_db} dB {channels} ch: {tp} dBTP"
                    );
                    assert!((m.output_lufs.unwrap() - lufs).abs() < 0.05);
                }
            }
        }
    }

    #[test]
    fn off_keeps_the_loudness_and_the_stages_can_be_switched_off() {
        let speech = wav("voices.wav");
        let settings = PolishSettings {
            target: Target::Off,
            ..Default::default()
        };
        let m = measured(&speech, 1, settings);
        let (lufs, tp) = check(&speech, 1, &m);
        assert!(
            (lufs - m.input_lufs.unwrap()).abs() <= 0.5,
            "{lufs} vs {:?}",
            m.input_lufs
        );
        assert!(tp <= -1.0);
        let none = PolishSettings {
            stages: Stages {
                rumble: false,
                de_ess: false,
                compress: false,
            },
            ..Default::default()
        };
        assert_eq!(measured(&speech, 1, none).params.stages, none.stages);
    }

    #[test]
    fn silence_and_cancel() {
        let m = measured(&vec![0.0; 48_000], 1, PolishSettings::default());
        assert_eq!(m.input_lufs, None);
        assert_eq!(m.params.pre_gain_db, 0.0);
        let speech = wav("hello-mono.wav");
        let err = measure(
            &Buffer {
                samples: &speech,
                channels: 1,
            },
            &PolishSettings::default(),
            &AtomicBool::new(true),
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(err, "cancelled");
    }
}

#[cfg(test)]
mod speed {
    use super::*;

    /// Ten minutes of stereo speech-like sound through the whole measurement:
    /// `cargo test --release measurement_speed -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn measurement_speed() {
        let speech = crate::testsignals::wav("voices.wav");
        let samples: Vec<f32> = speech
            .iter()
            .cycle()
            .take(48_000 * 600)
            .flat_map(|&s| [s, s * 0.9])
            .collect();
        let started = std::time::Instant::now();
        let mut passes = 0;
        let m = measure(
            &Buffer {
                samples: &samples,
                channels: 2,
            },
            &PolishSettings::default(),
            &AtomicBool::new(false),
            &mut |f| {
                if f >= 1.0 {
                    passes += 1;
                }
            },
        )
        .unwrap();
        let s = started.elapsed().as_secs_f64();
        eprintln!(
            "10 min stereo measured in {s:.1} s ({:.0}x real time): {:?} LUFS, {:.2} dBTP",
            600.0 / s,
            m.output_lufs,
            m.output_true_peak_db
        );
        let _ = passes;
    }
}
