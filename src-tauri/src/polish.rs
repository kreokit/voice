//! Polish (milestone 4): the chain after noise removal, in the order a voice engineer
//! would use it: rumble filter → de-esser → light compressor → loudness gain → true-peak
//! limiter. Own Rust DSP on 48 kHz f32, streaming (any block size, same result), so the
//! player runs it live and export (M5) runs it over the whole file.
//!
//! The dynamics stages have fixed thresholds because the chain first brings the voice to a
//! working level of [`WORK_LUFS`] (`pre_gain_db`, measured per file in `loudness.rs`); the
//! final `gain_db` then sets the target loudness, and the limiter keeps true peaks under
//! [`CEILING_DBTP`].

use serde::{Deserialize, Serialize};
use std::f32::consts::PI;

const RATE: f32 = 48_000.0;

/// The level the dynamics stages are tuned for.
pub const WORK_LUFS: f64 = -20.0;
/// The limiter's ceiling: 0.2 dB under the −1 dBTP the presets promise, for the
/// oversampling's estimate error.
pub const CEILING_DBTP: f32 = -1.2;

/// Which stages run (all on by default; the user can switch each off).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Stages {
    /// High-pass at 80 Hz against rumble (traffic, handling, air conditioning).
    pub rumble: bool,
    /// Gentle de-esser against harsh "s" sounds.
    pub de_ess: bool,
    /// Light compression to even out the level.
    pub compress: bool,
}

impl Default for Stages {
    fn default() -> Self {
        Self {
            rumble: true,
            de_ess: true,
            compress: true,
        }
    }
}

/// What the chain needs to run: the stages and the two measured gains.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Params {
    pub stages: Stages,
    /// Brings the input to [`WORK_LUFS`] before the dynamics.
    pub pre_gain_db: f32,
    /// After the dynamics: reaches the target loudness.
    pub gain_db: f32,
    /// The true-peak limiter (on whenever the loudness is targeted).
    pub limit: bool,
}

impl Default for Params {
    fn default() -> Self {
        Self::bypass()
    }
}

impl Params {
    /// Nothing changes the sound.
    pub fn bypass() -> Self {
        Self {
            stages: Stages {
                rumble: false,
                de_ess: false,
                compress: false,
            },
            pre_gain_db: 0.0,
            gain_db: 0.0,
            limit: false,
        }
    }
}

fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn gain_to_db(g: f32) -> f32 {
    20.0 * g.max(1e-9).log10()
}

/// A one-pole smoothing coefficient for a time constant.
fn coefficient(ms: f32) -> f32 {
    (-1.0 / (ms * 0.001 * RATE)).exp()
}

/// A biquad (RBJ cookbook), transposed direct form II.
#[derive(Debug, Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn new(b: [f32; 3], a: [f32; 3]) -> Self {
        Self {
            b0: b[0] / a[0],
            b1: b[1] / a[0],
            b2: b[2] / a[0],
            a1: a[1] / a[0],
            a2: a[2] / a[0],
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// A 2nd-order Butterworth high-pass (Q = 1/√2).
    fn high_pass(hz: f32) -> Self {
        let w = 2.0 * PI * hz / RATE;
        let (cos, alpha) = (w.cos(), w.sin() / (2.0 * std::f32::consts::FRAC_1_SQRT_2));
        Self::new(
            [(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0],
            [1.0 + alpha, -2.0 * cos, 1.0 - alpha],
        )
    }

    /// A 2nd-order Butterworth low-pass (Q = 1/√2).
    fn low_pass(hz: f32) -> Self {
        let w = 2.0 * PI * hz / RATE;
        let (cos, alpha) = (w.cos(), w.sin() / (2.0 * std::f32::consts::FRAC_1_SQRT_2));
        Self::new(
            [(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0],
            [1.0 + alpha, -2.0 * cos, 1.0 - alpha],
        )
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Splits at 5 kHz with a 4th-order Linkwitz-Riley crossover: low + high is flat in level
/// (an all-pass), so the de-esser changes nothing until it acts.
#[derive(Debug, Clone, Copy)]
struct Crossover {
    low: [Biquad; 2],
    high: [Biquad; 2],
}

const SPLIT_HZ: f32 = 5_000.0;

impl Crossover {
    fn new() -> Self {
        Self {
            low: [Biquad::low_pass(SPLIT_HZ); 2],
            high: [Biquad::high_pass(SPLIT_HZ); 2],
        }
    }

    fn split(&mut self, x: f32) -> (f32, f32) {
        let low = self.low[0].process(x);
        let low = self.low[1].process(low);
        let high = self.high[0].process(x);
        let high = self.high[1].process(high);
        // LR4's high band is in phase with the low band at the crossover, so the sum is flat.
        (low, high)
    }
}

/// De-esser settings: the high band may come within [`DE_ESS_RELATIVE_DB`] of the full
/// signal; beyond that it is turned down 2:1, at most [`DE_ESS_MAX_DB`], and only when the
/// high band is above [`DE_ESS_FLOOR_DB`] (not on hiss in the pauses).
const DE_ESS_RELATIVE_DB: f32 = -8.0;
const DE_ESS_MAX_DB: f32 = 8.0;
const DE_ESS_FLOOR_DB: f32 = -50.0;

/// Compressor settings (at the working level): soft knee around −26 dBFS RMS, 2:1.
const COMP_THRESHOLD_DB: f32 = -26.0;
const COMP_RATIO: f32 = 2.0;
const COMP_KNEE_DB: f32 = 6.0;

/// The de-esser's and compressor's gain targets are computed every this many frames
/// (3 kHz, well inside their 1–10 ms time constants); the gains glide per sample.
const CONTROL_EVERY: u32 = 16;

/// Lookahead of the limiter (2 ms): the output is this many samples late.
pub const LIMITER_DELAY: usize = 96;
/// The limiter's gain is a moving average over this many samples: short enough that a need
/// recorded up to OVERSAMPLE_DELAY (+2) samples late still covers its sample.
const AVERAGE: usize = LIMITER_DELAY - OVERSAMPLE_DELAY - 2;

/// 4× oversampling for true-peak estimates: windowed-sinc interpolation at ¼, ½ and ¾
/// between two samples, 12 taps each (like the filter in ITU-R BS.1770 annex 2). The
/// interpolated points lie [`OVERSAMPLE_DELAY`] samples behind the newest one.
struct Oversampler {
    phases: [[f32; 12]; 3],
    /// The last 12 samples, written twice (at `pos` and `pos + 12`) so the newest 12 are
    /// always one contiguous slice.
    history: [f32; 24],
    pos: usize,
}

/// How far behind the newest sample the interpolated points are.
const OVERSAMPLE_DELAY: usize = 6;

impl Oversampler {
    fn new() -> Self {
        let mut phases = [[0.0f32; 12]; 3];
        for (p, phase) in phases.iter_mut().enumerate() {
            // The point between the 6th and 7th of the last 12 samples.
            let at = 5.0 + (p as f32 + 1.0) / 4.0;
            for (k, tap) in phase.iter_mut().enumerate() {
                let t = k as f32 - at;
                let sinc = if t.abs() < 1e-6 {
                    1.0
                } else {
                    (PI * t).sin() / (PI * t)
                };
                *tap = sinc * (0.5 + 0.5 * (PI * t / 6.5).cos());
            }
            let sum: f32 = phase.iter().sum();
            for tap in phase.iter_mut() {
                *tap /= sum;
            }
        }
        Self {
            phases,
            history: [0.0; 24],
            pos: 0,
        }
    }

    /// The highest of the newest sample and the interpolated points behind it.
    fn peak(&mut self, x: f32) -> f32 {
        self.history[self.pos] = x;
        self.history[self.pos + 12] = x;
        self.pos = (self.pos + 1) % 12;
        // Oldest to newest.
        let window = &self.history[self.pos..self.pos + 12];
        let mut peak = x.abs().max(window[5].abs());
        for phase in &self.phases {
            let v: f32 = phase.iter().zip(window).map(|(t, h)| t * h).sum();
            peak = peak.max(v.abs());
        }
        peak
    }
}

/// A fixed-size delay line.
struct Ring {
    data: Vec<f32>,
    pos: usize,
}

impl Ring {
    fn new(len: usize) -> Self {
        Self {
            data: vec![0.0; len.max(1)],
            pos: 0,
        }
    }

    /// Stores `x` and returns the value stored `len` calls ago (0 at first).
    fn swap(&mut self, x: f32) -> f32 {
        let old = std::mem::replace(&mut self.data[self.pos], x);
        self.pos = (self.pos + 1) % self.data.len();
        old
    }
}

/// The chain for interleaved audio of 1 or 2 channels; dynamics are linked across
/// channels so the stereo image stays put.
pub struct Chain {
    params: Params,
    channels: usize,
    rumble: Vec<Biquad>,
    crossovers: Vec<Crossover>,
    ess_high: f32,
    ess_full: f32,
    ess_gain: f32,
    ess_target: f32,
    comp_power: f32,
    comp_gain: f32,
    comp_target: f32,
    tick: u32,
    oversamplers: Vec<Oversampler>,
    /// Limiter: delayed frames (one line per channel), the required gains, and the
    /// smoothed gain.
    delay: Vec<Ring>,
    /// Monotonic queue (index, required gain) for the sliding minimum.
    required: std::collections::VecDeque<(u64, f32)>,
    index: u64,
    held: Ring,
    held_sum: f32,
    held_count: usize,
    limiter_gain: f32,
    pre_gain: f32,
    gain: f32,
    ceiling: f32,
    ess_attack: f32,
    ess_release: f32,
    glide: f32,
    comp_smooth: f32,
    comp_attack: f32,
    comp_release: f32,
    limiter_release: f32,
}

impl Chain {
    pub fn new(channels: usize, params: Params) -> Self {
        let mut chain = Self {
            params,
            channels,
            rumble: vec![Biquad::high_pass(80.0); channels],
            crossovers: vec![Crossover::new(); channels],
            ess_high: 0.0,
            ess_full: 0.0,
            ess_gain: 1.0,
            ess_target: 1.0,
            comp_power: 0.0,
            comp_gain: 1.0,
            comp_target: 1.0,
            tick: 0,
            oversamplers: (0..channels).map(|_| Oversampler::new()).collect(),
            delay: (0..channels).map(|_| Ring::new(LIMITER_DELAY)).collect(),
            required: Default::default(),
            index: 0,
            held: Ring::new(AVERAGE),
            held_sum: 0.0,
            held_count: 0,
            limiter_gain: 1.0,
            pre_gain: 1.0,
            gain: 1.0,
            ceiling: db_to_gain(CEILING_DBTP),
            ess_attack: coefficient(1.0),
            ess_release: coefficient(60.0),
            glide: coefficient(0.3),
            comp_smooth: coefficient(10.0),
            comp_attack: coefficient(10.0),
            comp_release: coefficient(150.0),
            limiter_release: coefficient(80.0),
        };
        chain.set_params(params);
        chain
    }

    /// New settings or gains; the filters keep their state (no click).
    pub fn set_params(&mut self, params: Params) {
        self.params = params;
        self.pre_gain = db_to_gain(params.pre_gain_db);
        self.gain = db_to_gain(params.gain_db);
    }

    /// A lower limiter ceiling (dBTP) than [`CEILING_DBTP`]: export uses it when a lossy
    /// encoder pushed the true peak over −1 dBTP.
    pub fn set_ceiling_db(&mut self, db: f32) {
        self.ceiling = db_to_gain(db.min(CEILING_DBTP));
    }

    /// Samples the output lags the input (the limiter's lookahead, when on).
    pub fn latency(&self) -> usize {
        if self.params.limit { LIMITER_DELAY } else { 0 }
    }

    /// Processes interleaved frames in place. With the limiter on, the output is
    /// [`LIMITER_DELAY`] frames late (the first ones are silence).
    pub fn process(&mut self, interleaved: &mut [f32]) {
        let ch = self.channels;
        let mut frame = [0.0f32; 2];
        for out in interleaved.chunks_exact_mut(ch) {
            frame[..ch].copy_from_slice(out);
            self.stages(&mut frame[..ch]);
            if self.params.limit {
                self.limit(&mut frame[..ch]);
            }
            out.copy_from_slice(&frame[..ch]);
        }
    }

    /// Everything before the limiter, for one frame.
    fn stages(&mut self, frame: &mut [f32]) {
        let s = self.params.stages;
        for x in frame.iter_mut() {
            *x *= self.pre_gain;
        }
        if s.rumble {
            for (x, f) in frame.iter_mut().zip(&mut self.rumble) {
                *x = f.process(*x);
            }
        }
        if s.de_ess {
            let mut bands = [(0.0f32, 0.0f32); 2];
            let (mut high_peak, mut full_peak) = (0.0f32, 0.0f32);
            for (i, (x, c)) in frame.iter().zip(&mut self.crossovers).enumerate() {
                bands[i] = c.split(*x);
                high_peak = high_peak.max(bands[i].1.abs());
                full_peak = full_peak.max(x.abs());
            }
            let follow = |env: &mut f32, v: f32, attack: f32, release: f32| {
                let k = if v > *env { attack } else { release };
                *env = v + (*env - v) * k;
            };
            follow(
                &mut self.ess_high,
                high_peak,
                self.ess_attack,
                self.ess_release,
            );
            follow(
                &mut self.ess_full,
                full_peak,
                self.ess_attack,
                self.ess_release,
            );
            if self.tick.is_multiple_of(CONTROL_EVERY) {
                let (high_db, full_db) = (gain_to_db(self.ess_high), gain_to_db(self.ess_full));
                let excess = high_db - full_db - DE_ESS_RELATIVE_DB;
                let cut = if high_db > DE_ESS_FLOOR_DB && excess > 0.0 {
                    (excess * (1.0 - 1.0 / 2.0)).min(DE_ESS_MAX_DB)
                } else {
                    0.0
                };
                self.ess_target = db_to_gain(-cut);
            }
            self.ess_gain = self.ess_target + (self.ess_gain - self.ess_target) * self.glide;
            let g = self.ess_gain;
            for (x, (low, high)) in frame.iter_mut().zip(&bands) {
                *x = low + high * g;
            }
        }
        if s.compress {
            let power = frame.iter().map(|x| x * x).sum::<f32>() / frame.len() as f32;
            self.comp_power = power + (self.comp_power - power) * self.comp_smooth;
            if self.tick.is_multiple_of(CONTROL_EVERY) {
                let level = 10.0 * self.comp_power.max(1e-12).log10();
                let over = level - COMP_THRESHOLD_DB;
                let half = COMP_KNEE_DB / 2.0;
                let target_db = if over <= -half {
                    0.0
                } else if over >= half {
                    -over * (1.0 - 1.0 / COMP_RATIO)
                } else {
                    -(1.0 - 1.0 / COMP_RATIO) * (over + half).powi(2) / (2.0 * COMP_KNEE_DB)
                };
                self.comp_target = db_to_gain(target_db);
            }
            let k = if self.comp_target < self.comp_gain {
                self.comp_attack
            } else {
                self.comp_release
            };
            self.comp_gain = self.comp_target + (self.comp_gain - self.comp_target) * k;
            let g = self.comp_gain;
            for x in frame.iter_mut() {
                *x *= g;
            }
        }
        for x in frame.iter_mut() {
            *x *= self.gain;
        }
        self.tick = self.tick.wrapping_add(1);
    }

    /// The true-peak limiter for one frame: the gain stays at or below what every sample
    /// in the lookahead needs (sliding minimum, then a moving average over the same
    /// window), so the ceiling holds; it recovers over ~80 ms.
    fn limit(&mut self, frame: &mut [f32]) {
        let mut peak = 0.0f32;
        for (x, o) in frame.iter().zip(&mut self.oversamplers) {
            peak = peak.max(o.peak(*x));
        }
        let need = if peak > self.ceiling {
            self.ceiling / peak
        } else {
            1.0
        };
        // Sliding minimum over the last LIMITER_DELAY + 1 required gains (a monotonic queue:
        // constant time per sample).
        let i = self.index;
        self.index += 1;
        while self.required.back().is_some_and(|&(_, g)| g >= need) {
            self.required.pop_back();
        }
        self.required.push_back((i, need));
        while self
            .required
            .front()
            .is_some_and(|&(j, _)| j + (LIMITER_DELAY as u64) < i)
        {
            self.required.pop_front();
        }
        let min = self.required.front().map_or(1.0, |&(_, g)| g);
        // Moving average of that minimum, over a window short enough that a need recorded
        // up to OVERSAMPLE_DELAY (+2) samples late still covers its sample when it leaves
        // the delay line.
        // (Until the window is full the missing values count as 1: no limiting needed.)
        let dropped = if self.held_count < AVERAGE {
            self.held_count += 1;
            self.held.swap(min);
            0.0
        } else {
            self.held.swap(min)
        };
        self.held_sum += min - dropped;
        let average =
            ((self.held_sum + (AVERAGE - self.held_count) as f32) / AVERAGE as f32).min(1.0);
        self.limiter_gain = if average < self.limiter_gain {
            average
        } else {
            average + (self.limiter_gain - average) * self.limiter_release
        };
        // Delay the audio by LIMITER_DELAY frames to meet its gain (silence at first).
        for (x, line) in frame.iter_mut().zip(&mut self.delay) {
            *x = line.swap(*x) * self.limiter_gain;
        }
    }

    /// Flushes the limiter's delay at the end of a stream: appends the last
    /// [`Chain::latency`] frames to `out`.
    pub fn flush(&mut self, out: &mut Vec<f32>) {
        let mut silence = vec![0.0f32; self.channels];
        for _ in 0..self.latency() {
            silence.fill(0.0);
            // Silence through the stages keeps the filters' tails; through the limiter
            // it pushes the delayed audio out.
            self.stages(&mut silence);
            self.limit(&mut silence);
            out.extend_from_slice(&silence);
        }
    }
}

/// Runs `chain` over a whole interleaved buffer: the output lines up with the input and
/// is as long.
#[cfg(test)]
pub fn run(chain: &mut Chain, interleaved: &[f32]) -> Vec<f32> {
    let latency = chain.latency() * chain.channels;
    let mut out = interleaved.to_vec();
    chain.process(&mut out);
    chain.flush(&mut out);
    out.drain(..latency);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::rms_db;

    fn sine(db: f32, hz: f32, seconds: f32) -> Vec<f32> {
        let amp = db_to_gain(db);
        (0..(RATE * seconds) as usize)
            .map(|i| amp * (2.0 * PI * hz * i as f32 / RATE).sin())
            .collect()
    }

    fn with(stages: Stages, limit: bool) -> Params {
        Params {
            stages,
            pre_gain_db: 0.0,
            gain_db: 0.0,
            limit,
        }
    }

    fn only(rumble: bool, de_ess: bool, compress: bool) -> Stages {
        Stages {
            rumble,
            de_ess,
            compress,
        }
    }

    /// RMS of the second half (after the filters settle).
    fn settled_db(x: &[f32]) -> f64 {
        rms_db(&x[x.len() / 2..])
    }

    #[test]
    fn bypass_changes_nothing() {
        let x = sine(-10.0, 440.0, 0.5);
        assert_eq!(run(&mut Chain::new(1, Params::bypass()), &x), x);
    }

    #[test]
    fn the_rumble_filter_cuts_lows_and_keeps_the_voice() {
        let p = with(only(true, false, false), false);
        let at = |hz| {
            let x = sine(-10.0, hz, 1.0);
            settled_db(&run(&mut Chain::new(1, p), &x)) - settled_db(&x)
        };
        assert!(at(30.0) < -16.0, "30 Hz: {}", at(30.0));
        assert!(
            (at(80.0) + 3.0).abs() < 0.3,
            "80 Hz is the corner: {}",
            at(80.0)
        );
        assert!(at(300.0) > -0.4, "300 Hz: {}", at(300.0));
        assert!(at(1_000.0) > -0.1);
    }

    #[test]
    fn the_crossover_is_transparent_until_the_de_esser_acts() {
        // A quiet mix of a low and a high tone: the high band stays far below the
        // full signal, so nothing is turned down and the level is unchanged.
        let low = sine(-20.0, 300.0, 1.0);
        let high = sine(-40.0, 7_000.0, 1.0);
        let x: Vec<f32> = low.iter().zip(&high).map(|(a, b)| a + b).collect();
        let y = run(
            &mut Chain::new(1, with(only(false, true, false), false)),
            &x,
        );
        assert!((settled_db(&y) - settled_db(&x)).abs() < 0.1);
    }

    #[test]
    fn the_de_esser_turns_down_sibilance_only() {
        let p = with(only(false, true, false), false);
        // A loud 7 kHz "s" (the high band is most of the signal): turned down, at most 8 dB.
        let s = sine(-12.0, 7_000.0, 1.0);
        let cut = settled_db(&run(&mut Chain::new(1, p), &s)) - settled_db(&s);
        assert!(cut < -2.0 && cut > -8.5, "{cut}");
        // A vowel-like low tone at the same level: untouched.
        let vowel = sine(-12.0, 400.0, 1.0);
        let change = settled_db(&run(&mut Chain::new(1, p), &vowel)) - settled_db(&vowel);
        assert!(change.abs() < 0.1, "{change}");
    }

    #[test]
    fn the_compressor_evens_out_loud_and_quiet() {
        let p = with(only(false, false, true), false);
        let loud = sine(-6.0, 300.0, 1.0);
        let quiet = sine(-36.0, 300.0, 1.0);
        let out_loud = settled_db(&run(&mut Chain::new(1, p), &loud));
        let out_quiet = settled_db(&run(&mut Chain::new(1, p), &quiet));
        // 30 dB apart in, about 2:1 above the threshold out; quiet passes untouched.
        assert!((out_quiet - settled_db(&quiet)).abs() < 0.2);
        assert!(out_loud - out_quiet < 22.0, "{}", out_loud - out_quiet);
        assert!(out_loud - out_quiet > 15.0, "light, not squashed");
    }

    #[test]
    fn the_limiter_keeps_true_peaks_under_the_ceiling() {
        // Speech-like bursts pushed 10 dB over full scale.
        let mut x = sine(4.0, 3_123.0, 1.0);
        for (i, s) in x.iter_mut().enumerate() {
            *s *= ((i as f32 / 4_800.0).sin().abs() * 3.0).min(1.0);
        }
        let mut p = with(only(false, false, false), true);
        p.gain_db = 6.0;
        let y = run(&mut Chain::new(1, p), &x);
        assert_eq!(y.len(), x.len());
        let mut meter = ebur128::EbuR128::new(1, 48_000, ebur128::Mode::TRUE_PEAK).unwrap();
        meter.add_frames_f32(&y).unwrap();
        let tp = 20.0 * meter.true_peak(0).unwrap().log10();
        assert!(tp <= -1.0, "true peak {tp} dBTP");
        assert!(tp > -2.0, "not over-limited: {tp}");
    }

    #[test]
    fn the_limiter_leaves_quiet_audio_alone_and_lines_up() {
        let x = sine(-20.0, 500.0, 0.5);
        let y = run(
            &mut Chain::new(1, with(only(false, false, false), true)),
            &x,
        );
        assert_eq!(y.len(), x.len());
        let worst = x
            .iter()
            .zip(&y)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(worst < 1e-6, "{worst}");
    }

    #[test]
    fn same_result_whatever_the_block_size() {
        let left = sine(0.0, 700.0, 0.5);
        let right = sine(-30.0, 300.0, 0.5);
        let x: Vec<f32> = left
            .iter()
            .zip(&right)
            .flat_map(|(a, b)| [*a, *b])
            .collect();
        let p = Params {
            stages: Stages::default(),
            pre_gain_db: 3.0,
            gain_db: 2.0,
            limit: true,
        };
        let whole = run(&mut Chain::new(2, p), &x);
        for block in [2usize, 96, 1_000, 48_000] {
            let mut chain = Chain::new(2, p);
            let mut out = Vec::new();
            for piece in x.chunks(block) {
                let mut piece = piece.to_vec();
                chain.process(&mut piece);
                out.extend(piece);
            }
            chain.flush(&mut out);
            out.drain(..LIMITER_DELAY * 2);
            assert!(out == whole, "block {block}");
        }
    }
}

#[cfg(test)]
mod speed {
    use super::*;

    /// Seconds per stage for one minute of stereo:
    /// `cargo test --release stage_speed -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn stage_speed() {
        let x: Vec<f32> = (0..48_000 * 60 * 2)
            .map(|i| (i as f32 * 0.0131).sin() * 0.3)
            .collect();
        let none = Stages {
            rumble: false,
            de_ess: false,
            compress: false,
        };
        for (name, stages, limit) in [
            ("gain only", none, false),
            (
                "rumble",
                Stages {
                    rumble: true,
                    ..none
                },
                false,
            ),
            (
                "de-ess",
                Stages {
                    de_ess: true,
                    ..none
                },
                false,
            ),
            (
                "compress",
                Stages {
                    compress: true,
                    ..none
                },
                false,
            ),
            ("limiter", none, true),
        ] {
            let p = Params {
                stages,
                pre_gain_db: 0.0,
                gain_db: 0.0,
                limit,
            };
            let started = std::time::Instant::now();
            let mut buf = x.clone();
            Chain::new(2, p).process(&mut buf);
            eprintln!(
                "{name:<10} {:.3} s per minute",
                started.elapsed().as_secs_f64()
            );
        }
        let started = std::time::Instant::now();
        let mut m = ebur128::EbuR128::new(2, 48_000, ebur128::Mode::I).unwrap();
        m.add_frames_f32(&x).unwrap();
        eprintln!(
            "meter I    {:.3} s per minute",
            started.elapsed().as_secs_f64()
        );
        let started = std::time::Instant::now();
        let mut m =
            ebur128::EbuR128::new(2, 48_000, ebur128::Mode::I | ebur128::Mode::TRUE_PEAK).unwrap();
        m.add_frames_f32(&x).unwrap();
        eprintln!(
            "meter I+TP {:.3} s per minute",
            started.elapsed().as_secs_f64()
        );
    }
}
