//! Quality measurements against a clean reference (the tests' oracle; the release
//! self-test uses them too): SI-SDR and the noise left in the pauses.

// Only the tests use these until the release self-test (milestone 6).
#![cfg_attr(not(test), allow(dead_code))]

/// Scale-invariant signal-to-distortion ratio in dB (Le Roux et al. 2019): how much of
/// `estimate` is the `reference` (at any gain) versus everything else. Both are made
/// zero-mean first; they must be time-aligned and the same length.
pub fn si_sdr(reference: &[f32], estimate: &[f32]) -> f64 {
    assert_eq!(reference.len(), estimate.len());
    let mean = |x: &[f32]| x.iter().map(|&v| v as f64).sum::<f64>() / x.len().max(1) as f64;
    let (mr, me) = (mean(reference), mean(estimate));
    let (mut dot, mut energy) = (0.0f64, 0.0f64);
    for (&r, &e) in reference.iter().zip(estimate) {
        let r = r as f64 - mr;
        dot += r * (e as f64 - me);
        energy += r * r;
    }
    let alpha = dot / energy.max(f64::MIN_POSITIVE);
    let (mut target, mut error) = (0.0f64, 0.0f64);
    for (&r, &e) in reference.iter().zip(estimate) {
        let t = alpha * (r as f64 - mr);
        target += t * t;
        let d = (e as f64 - me) - t;
        error += d * d;
    }
    10.0 * (target.max(1e-20) / error.max(1e-20)).log10()
}

/// RMS level in dBFS (-120 for silence).
pub fn rms_db(x: &[f32]) -> f64 {
    let power = x.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64;
    10.0 * power.max(1e-12).log10()
}

/// The pauses of a clean reference: the samples of every 10 ms window whose level is at
/// least 40 dB below the reference's loudest window.
pub fn pauses(reference: &[f32]) -> Vec<std::ops::Range<usize>> {
    const WINDOW: usize = 480;
    let levels: Vec<f64> = reference.chunks(WINDOW).map(rms_db).collect();
    let loudest = levels.iter().copied().fold(f64::MIN, f64::max);
    levels
        .iter()
        .enumerate()
        .filter(|(_, l)| **l < loudest - 40.0)
        .map(|(i, _)| i * WINDOW..((i + 1) * WINDOW).min(reference.len()))
        .collect()
}

/// RMS level in dBFS of `x` over the given sample ranges (the noise floor in the pauses).
pub fn level_in(x: &[f32], ranges: &[std::ops::Range<usize>]) -> f64 {
    let picked: Vec<f32> = ranges
        .iter()
        .flat_map(|r| x[r.clone()].iter().copied())
        .collect();
    rms_db(&picked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn si_sdr_ignores_gain_and_measures_noise() {
        let reference: Vec<f32> = (0..4_800).map(|i| (i as f32 * 0.05).sin()).collect();
        let louder: Vec<f32> = reference.iter().map(|s| s * 3.0).collect();
        assert!(si_sdr(&reference, &louder) > 100.0);
        // Adding a signal at a tenth of the amplitude (and uncorrelated): about 20 dB.
        let other: Vec<f32> = (0..4_800).map(|i| (i as f32 * 0.31).sin() * 0.1).collect();
        let noisy: Vec<f32> = reference.iter().zip(&other).map(|(a, b)| a + b).collect();
        assert!(
            (si_sdr(&reference, &noisy) - 20.0).abs() < 0.5,
            "{}",
            si_sdr(&reference, &noisy)
        );
    }

    #[test]
    fn levels_and_pauses() {
        assert!((rms_db(&[0.5; 100]) - (-6.02)).abs() < 0.01);
        assert_eq!(rms_db(&[0.0; 10]), -120.0);
        let mut x = vec![0.5f32; 960];
        x.extend(vec![0.001; 960]); // -60 dB: a pause
        assert_eq!(pauses(&x), vec![960..1440, 1440..1920]);
        assert!((level_in(&x, &pauses(&x)) - (-60.0)).abs() < 0.01);
    }
}
