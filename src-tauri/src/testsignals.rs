//! Test signals for the quality tests: the speech fixtures and reproducible noises made
//! here (seeded), so results don't depend on the FFmpeg version or the machine.

use std::f32::consts::PI;

const RATE: f32 = 48_000.0;

/// A fixture's samples: 16-bit 48 kHz mono PCM after the WAV header's "data" tag.
pub fn wav(name: &str) -> Vec<f32> {
    let bytes = std::fs::read(crate::probe::fixture(name)).unwrap();
    let data = bytes.windows(4).position(|w| w == b"data").unwrap() + 8;
    bytes[data..]
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect()
}

/// Uniform noise in -1..1 (xorshift).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }
}

/// The kinds of noise the quality tests use (CLAUDE.md, "Tests").
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Noise {
    /// Room tone / air conditioning: pink noise.
    Room,
    /// A fan: brown (red) noise, mostly low frequencies.
    Fan,
    /// Mains hum: 50 Hz and harmonics.
    Hum,
    /// Typing: short bright clicks at irregular intervals.
    Keyboard,
    /// Wind: low rumble swelling and fading in gusts.
    Wind,
    /// A crowd: four voices talking at once (`fixtures/speech/babble.wav`).
    Babble,
}

pub const ALL_NOISES: [Noise; 6] = [
    Noise::Room,
    Noise::Fan,
    Noise::Hum,
    Noise::Keyboard,
    Noise::Wind,
    Noise::Babble,
];

/// `n` samples of the noise (any level; [`mix`] sets it).
pub fn noise(kind: Noise, n: usize, seed: u64) -> Vec<f32> {
    let mut rng = Rng(seed.max(1));
    match kind {
        Noise::Room => {
            // Paul Kellet's pink filter.
            let mut b = [0.0f32; 7];
            (0..n)
                .map(|_| {
                    let w = rng.next();
                    b[0] = 0.99886 * b[0] + w * 0.0555179;
                    b[1] = 0.99332 * b[1] + w * 0.0750759;
                    b[2] = 0.96900 * b[2] + w * 0.1538520;
                    b[3] = 0.86650 * b[3] + w * 0.3104856;
                    b[4] = 0.55000 * b[4] + w * 0.5329522;
                    b[5] = -0.7616 * b[5] - w * 0.0168980;
                    let out = b.iter().sum::<f32>() + w * 0.5362;
                    b[6] = w * 0.115926;
                    out
                })
                .collect()
        }
        Noise::Fan => brown(&mut rng, n),
        Noise::Hum => (0..n)
            .map(|i| {
                let t = i as f32 / RATE;
                [(1.0, 1.0), (2.0, 0.5), (3.0, 0.35), (5.0, 0.2)]
                    .iter()
                    .map(|(h, a)| a * (2.0 * PI * 50.0 * h * t).sin())
                    .sum()
            })
            .collect(),
        Noise::Keyboard => {
            let mut out = vec![0.0f32; n];
            let mut at = 0usize;
            while at < n {
                // A 3 ms click with a fast decay, every 80–250 ms.
                for k in 0..144.min(n - at) {
                    out[at + k] = rng.next() * (-(k as f32) / 30.0).exp();
                }
                at += 3_840 + ((rng.next() + 1.0) * 4_080.0) as usize;
            }
            out
        }
        Noise::Wind => {
            let rumble = brown(&mut rng, n);
            rumble
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let t = i as f32 / RATE;
                    let gust = 0.55
                        + 0.45 * (2.0 * PI * 0.23 * t).sin() * (2.0 * PI * 0.07 * t + 1.0).sin();
                    s * gust
                })
                .collect()
        }
        Noise::Babble => wav("babble.wav").into_iter().cycle().take(n).collect(),
    }
}

fn brown(rng: &mut Rng, n: usize) -> Vec<f32> {
    let mut y = 0.0f32;
    (0..n)
        .map(|_| {
            y = (y + rng.next() * 0.02) * 0.998;
            y
        })
        .collect()
}

/// `speech` plus `noise` scaled so the speech is `snr_db` louder (RMS over the whole clip).
pub fn mix(speech: &[f32], noise: &[f32], snr_db: f64) -> Vec<f32> {
    let gain = 10f64
        .powf((crate::metrics::rms_db(speech) - snr_db - crate::metrics::rms_db(noise)) / 20.0)
        as f32;
    speech
        .iter()
        .zip(noise)
        .map(|(s, n)| s + n * gain)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::rms_db;

    #[test]
    fn noises_are_reproducible_and_mix_at_the_snr() {
        let speech = wav("voices.wav");
        for kind in ALL_NOISES {
            let a = noise(kind, speech.len(), 5);
            assert_eq!(a, noise(kind, speech.len(), 5), "{kind:?}");
            assert!(rms_db(&a) > -60.0, "{kind:?} is not silent");
            let mixed = mix(&speech, &a, 5.0);
            let added: Vec<f32> = mixed.iter().zip(&speech).map(|(m, s)| m - s).collect();
            let snr = rms_db(&speech) - rms_db(&added);
            assert!((snr - 5.0).abs() < 0.01, "{kind:?}: {snr}");
        }
    }
}
