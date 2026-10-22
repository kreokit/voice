//! Room echo (late reverberation) suppression, milestone 4 experiment: Lebart, Boucher &
//! Denbigh (2001), "A new method based on spectral subtraction for speech dereverberation".
//! The late reverb's power in each STFT bin is predicted as the power `delay` earlier,
//! decayed as a room with reverberation time `rt60` would decay it, and subtracted (a gain
//! with a floor). No training data; the method is from 2001.

use std::f32::consts::PI;

use realfft::RealFftPlanner;
use realfft::num_complex::Complex32;

const FRAME: usize = 1024;
const HOP: usize = 256;
const RATE: f32 = 48_000.0;

#[derive(Debug, Clone, Copy)]
pub struct Settings {
    /// The assumed reverberation time of the room (s).
    pub rt60: f32,
    /// Where "late" starts (s): reflections before it are kept (they help intelligibility).
    pub late_after: f32,
    /// The lowest gain (linear): how much a bin may be turned down.
    pub floor: f32,
    /// Smoothing of the power estimate over frames (0–1).
    pub smoothing: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            rt60: 0.6,
            late_after: 0.05,
            floor: 0.3,
            smoothing: 0.6,
        }
    }
}

/// Suppresses late reverberation in a mono buffer (offline; the output lines up with the
/// input and is as long).
pub fn process(input: &[f32], s: Settings) -> Vec<f32> {
    let mut planner = RealFftPlanner::<f32>::new();
    let forward = planner.plan_fft_forward(FRAME);
    let inverse = planner.plan_fft_inverse(FRAME);
    // √Hann analysis and synthesis: their product, Hann, sums to 2 at 75% overlap.
    let window: Vec<f32> = (0..FRAME)
        .map(|i| (0.5 - 0.5 * (2.0 * PI * i as f32 / FRAME as f32).cos()).sqrt())
        .collect();
    let bins = FRAME / 2 + 1;
    let delay_frames = ((s.late_after * RATE) / HOP as f32).round().max(1.0) as usize;
    let delta = 3.0 * 10f32.ln() / s.rt60.max(0.05);
    let decay = (-2.0 * delta * delay_frames as f32 * HOP as f32 / RATE).exp();

    let padded_len = input.len() + 2 * FRAME;
    let mut padded = vec![0.0f32; padded_len];
    padded[FRAME..FRAME + input.len()].copy_from_slice(input);
    let mut out = vec![0.0f32; padded_len];
    let mut power_history: Vec<Vec<f32>> = Vec::new();
    let mut smoothed = vec![0.0f32; bins];
    let mut frame = vec![0.0f32; FRAME];
    let mut spectrum = vec![Complex32::new(0.0, 0.0); bins];
    let mut start = 0;
    while start + FRAME <= padded_len {
        for i in 0..FRAME {
            frame[i] = padded[start + i] * window[i];
        }
        forward.process(&mut frame, &mut spectrum).unwrap();
        for (k, c) in spectrum.iter().enumerate() {
            let p = c.norm_sqr();
            smoothed[k] = s.smoothing * smoothed[k] + (1.0 - s.smoothing) * p;
        }
        power_history.push(smoothed.clone());
        let n = power_history.len();
        if n > delay_frames {
            let past = &power_history[n - 1 - delay_frames];
            for (k, c) in spectrum.iter_mut().enumerate() {
                let late = decay * past[k];
                let gain = (1.0 - late / smoothed[k].max(1e-12)).max(s.floor);
                *c *= gain;
            }
        }
        // Keep only what the delay needs.
        if power_history.len() > delay_frames + 1 {
            power_history.remove(0);
        }
        spectrum[0].im = 0.0;
        spectrum[bins - 1].im = 0.0;
        inverse.process(&mut spectrum, &mut frame).unwrap();
        for i in 0..FRAME {
            out[start + i] += frame[i] * window[i] / FRAME as f32 * 0.5;
        }
        start += HOP;
    }
    out[FRAME..FRAME + input.len()].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::si_sdr;
    use crate::testsignals::wav;

    /// A synthetic room: the direct sound, then a decaying noise tail from 2.5 ms with
    /// reverberation time `rt60`, its energy `drr_db` below the direct sound's.
    fn room(rt60: f32, drr_db: f32, seed: u64) -> Vec<f32> {
        let len = (RATE * rt60 * 1.5) as usize;
        let mut x = seed.max(1);
        let mut h: Vec<f32> = (0..len)
            .map(|i| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let n = (x >> 40) as f32 / (1u64 << 23) as f32 - 1.0;
                let t = i as f32 / RATE;
                if t < 0.0025 {
                    0.0
                } else {
                    n * (-3.0 * 10f32.ln() * t / rt60).exp()
                }
            })
            .collect();
        let tail: f32 = h.iter().map(|v| v * v).sum();
        let scale = (10f32.powf(-drr_db / 10.0) / tail).sqrt();
        for v in &mut h {
            *v *= scale;
        }
        h[0] = 1.0;
        h
    }

    pub(super) fn room_for_tuning(rt60: f32) -> Vec<f32> {
        room(rt60, 0.0, 9)
    }

    pub(super) fn convolve_for_tuning(x: &[f32], h: &[f32]) -> Vec<f32> {
        convolve(x, h)
    }

    fn convolve(x: &[f32], h: &[f32]) -> Vec<f32> {
        // Through the FFT, block by block (overlap-add).
        let n = (h.len() + 8192).next_power_of_two();
        let block = n - h.len() + 1;
        let mut planner = RealFftPlanner::<f32>::new();
        let (f, i) = (planner.plan_fft_forward(n), planner.plan_fft_inverse(n));
        let mut hb = h.to_vec();
        hb.resize(n, 0.0);
        let mut hs = f.make_output_vec();
        f.process(&mut hb, &mut hs).unwrap();
        let mut out = vec![0.0f32; x.len() + h.len()];
        for (b, chunk) in x.chunks(block).enumerate() {
            let mut xb = chunk.to_vec();
            xb.resize(n, 0.0);
            let mut xs = f.make_output_vec();
            f.process(&mut xb, &mut xs).unwrap();
            for (a, c) in xs.iter_mut().zip(&hs) {
                *a *= c;
            }
            let mut y = i.make_output_vec();
            i.process(&mut xs, &mut y).unwrap();
            for (k, v) in y.iter().enumerate() {
                if let Some(o) = out.get_mut(b * block + k) {
                    *o += v / n as f32;
                }
            }
        }
        out.truncate(x.len());
        out
    }

    /// The experiment's table: `cargo test --release dereverb_table -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn dereverb_table() {
        let dry = wav("voices.wav");
        eprintln!("room RT60  assumed  SI-SDR vs direct+early: reverberant → processed (gain)");
        for rt60 in [0.3f32, 0.6, 0.9] {
            let h = room(rt60, 0.0, 9);
            let wet = convolve(&dry, &h);
            let early = convolve(&dry, &h[..(0.05 * RATE) as usize]);
            for assumed in [0.3f32, 0.6, 0.9] {
                let out = process(
                    &wet,
                    Settings {
                        rt60: assumed,
                        ..Default::default()
                    },
                );
                let (a, b) = (si_sdr(&early, &wet), si_sdr(&early, &out));
                eprintln!(
                    "  {rt60:.1} s     {assumed:.1} s    {a:6.1} → {b:6.1} ({:+.1} dB)",
                    b - a
                );
            }
        }
        for assumed in [0.3f32, 0.6, 0.9] {
            let out = process(
                &dry,
                Settings {
                    rt60: assumed,
                    ..Default::default()
                },
            );
            eprintln!(
                "dry speech, assumed {assumed:.1} s: SI-SDR {:.1} dB (harm)",
                si_sdr(&dry, &out)
            );
        }
        // Does Best already remove room echo?
        let dir = crate::models::tests::models_dir_for_tests();
        for rt60 in [0.3f32, 0.6, 0.9] {
            let h = room(rt60, 0.0, 9);
            let wet = convolve(&dry, &h);
            let early = convolve(&dry, &h[..(0.05 * RATE) as usize]);
            let mut e = crate::dpdfnet::Enhancer::new(
                crate::dpdfnet::load(&crate::models::best_path(&dir)).unwrap(),
            )
            .unwrap();
            let out = crate::dpdfnet::clean(&mut e, &wet).unwrap();
            let (a, b) = (si_sdr(&early, &wet), si_sdr(&early, &out));
            eprintln!(
                "Best alone, room {rt60:.1} s: {a:6.1} → {b:6.1} ({:+.1} dB)",
                b - a
            );
            let both = process(
                &out,
                Settings {
                    rt60,
                    ..Default::default()
                },
            );
            let c = si_sdr(&early, &both);
            eprintln!("Best + dereverb ({rt60:.1} s): {c:6.1} ({:+.1} dB)", c - a);
        }
    }
}

#[cfg(test)]
mod tuning {
    use super::*;
    use crate::metrics::si_sdr;
    use crate::testsignals::wav;

    /// `cargo test --release dereverb_tuning -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn dereverb_tuning() {
        let dry = wav("voices.wav");
        let rooms: Vec<(f32, Vec<f32>, Vec<f32>)> = [0.3f32, 0.6, 0.9]
            .iter()
            .map(|&rt| {
                let h = super::tests::room_for_tuning(rt);
                (
                    rt,
                    super::tests::convolve_for_tuning(&dry, &h),
                    super::tests::convolve_for_tuning(&dry, &h[..2_400]),
                )
            })
            .collect();
        for floor in [0.3f32, 0.5, 0.7] {
            for smoothing in [0.6f32, 0.9] {
                let s = Settings {
                    rt60: 0.5,
                    floor,
                    smoothing,
                    ..Default::default()
                };
                let gains: Vec<String> = rooms
                    .iter()
                    .map(|(_, wet, early)| {
                        format!(
                            "{:+.1}",
                            si_sdr(early, &process(wet, s)) - si_sdr(early, wet)
                        )
                    })
                    .collect();
                let harm = si_sdr(&dry, &process(&dry, s));
                eprintln!(
                    "floor {floor} smoothing {smoothing}: rooms 0.3/0.6/0.9 s {} dB; dry speech {harm:.1} dB",
                    gains.join(" / ")
                );
            }
        }
    }
}
