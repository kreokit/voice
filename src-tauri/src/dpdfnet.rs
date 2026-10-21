//! DPDFNet (Ceva, Apache-2.0 weights): the Best model candidate (milestone 3, measured
//! against RNNoise before it may ship). The pipeline follows the authors' `dpdfnet` 0.6.0
//! package: a 20 ms Vorbis-windowed STFT (hop 10 ms), one model call per frame carrying a
//! state vector initialised from the model's metadata, and overlap-add (the Vorbis window
//! meets the Princen-Bradley condition, so no extra normalisation). Streaming: each 10 ms
//! of input gives 10 ms of output, [`DELAY`] samples late.

use std::path::Path;
use std::sync::Arc;

use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::TensorRef;
use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

/// Samples per hop (10 ms at 48 kHz) and per window (20 ms).
pub const HOP: usize = 480;
const WINDOW: usize = 960;
const BINS: usize = WINDOW / 2 + 1;

/// How many samples the output lags the input: 50 ms, measured on speech in
/// `measures_the_delay` (correlation 0.997). The authors' offline code drops 1 920 samples
/// after librosa's centred STFT, which itself shifts by half a window (480).
pub const DELAY: usize = 2_400;

/// Loads a DPDFNet ONNX model for one thread.
pub fn load(path: &Path) -> Result<Session, String> {
    Session::builder()
        .and_then(|b| b.with_optimization_level(GraphOptimizationLevel::Level3))
        .and_then(|b| b.with_intra_threads(1))
        .and_then(|b| b.commit_from_file(path))
        .map_err(|e| format!("cannot load the model: {e}"))
}

/// The model's starting state, from its metadata (as `dpdfnet.onnx_backend` builds it).
fn initial_state(session: &Session) -> Result<Vec<f32>, String> {
    let meta = session.metadata().map_err(|e| e.to_string())?;
    let get = |key: &str| -> Result<String, String> {
        meta.custom(key)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("the model has no {key}"))
    };
    let number = |key: &str| -> Result<usize, String> {
        get(key)?.trim().parse().map_err(|e| format!("{key}: {e}"))
    };
    let list = |key: &str| -> Result<Vec<f32>, String> {
        get(key)?
            .split(',')
            .map(|v| v.trim().parse::<f32>().map_err(|e| format!("{key}: {e}")))
            .collect()
    };
    let (size, erb, spec) = (
        number("state_size")?,
        number("erb_norm_state_size")?,
        number("spec_norm_state_size")?,
    );
    let (erb_init, spec_init) = (list("erb_norm_init")?, list("spec_norm_init")?);
    if erb_init.len() != erb || spec_init.len() != spec || erb + spec > size {
        return Err("unexpected DPDFNet state metadata".into());
    }
    let mut state = vec![0.0; size];
    state[..erb].copy_from_slice(&erb_init);
    state[erb..erb + spec].copy_from_slice(&spec_init);
    Ok(state)
}

/// One channel's streaming enhancer.
pub struct Enhancer {
    session: Session,
    state: Vec<f32>,
    window: Vec<f32>,
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
    /// The previous hop of input, and the second half of the previous output frame.
    previous: Vec<f32>,
    overlap: Vec<f32>,
    frame: Vec<f32>,
    spectrum: Vec<Complex32>,
    model_in: Vec<f32>,
}

impl Enhancer {
    pub fn new(session: Session) -> Result<Self, String> {
        let state = initial_state(&session)?;
        let mut planner = RealFftPlanner::<f32>::new();
        // dpdfnet.audio.vorbis_window
        let half = WINDOW as f32 / 2.0;
        let window = (0..WINDOW)
            .map(|i| {
                let s = (0.5 * std::f32::consts::PI * (i as f32 + 0.5) / half).sin();
                (0.5 * std::f32::consts::PI * s * s).sin()
            })
            .collect();
        Ok(Self {
            session,
            state,
            window,
            forward: planner.plan_fft_forward(WINDOW),
            inverse: planner.plan_fft_inverse(WINDOW),
            previous: vec![0.0; HOP],
            overlap: vec![0.0; HOP],
            frame: vec![0.0; WINDOW],
            spectrum: vec![Complex32::new(0.0, 0.0); BINS],
            model_in: vec![0.0; BINS * 2],
        })
    }

    /// Enhances one hop of samples in place (the output is [`DELAY`] samples late).
    pub fn process(&mut self, hop: &mut [f32; HOP]) -> Result<(), String> {
        for i in 0..HOP {
            self.frame[i] = self.previous[i] * self.window[i];
            self.frame[HOP + i] = hop[i] * self.window[HOP + i];
        }
        self.previous.copy_from_slice(hop);
        self.forward
            .process(&mut self.frame, &mut self.spectrum)
            .map_err(|e| e.to_string())?;
        for (k, c) in self.spectrum.iter().enumerate() {
            self.model_in[2 * k] = c.re;
            self.model_in[2 * k + 1] = c.im;
        }
        let spec = TensorRef::from_array_view(([1usize, 1, BINS, 2], &self.model_in[..]))
            .map_err(|e| e.to_string())?;
        let state = TensorRef::from_array_view(([self.state.len()], &self.state[..]))
            .map_err(|e| e.to_string())?;
        let outputs = self
            .session
            .run(ort::inputs![spec, state])
            .map_err(|e| format!("the model failed: {e}"))?;
        let (_, enhanced) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        let (_, next_state) = outputs[1]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        for (k, c) in self.spectrum.iter_mut().enumerate() {
            *c = Complex32::new(enhanced[2 * k], enhanced[2 * k + 1]);
        }
        let next_state = next_state.to_vec();
        drop(outputs);
        self.state = next_state;
        // A real signal's DC and Nyquist bins have no imaginary part.
        self.spectrum[0].im = 0.0;
        self.spectrum[BINS - 1].im = 0.0;
        self.inverse
            .process(&mut self.spectrum, &mut self.frame)
            .map_err(|e| e.to_string())?;
        let scale = 1.0 / WINDOW as f32;
        for i in 0..HOP {
            hop[i] = self.overlap[i] + self.frame[i] * self.window[i] * scale;
            self.overlap[i] = self.frame[HOP + i] * self.window[HOP + i] * scale;
        }
        Ok(())
    }
}

/// Cleans a whole mono buffer: the output lines up with the input and is as long.
pub fn clean(enhancer: &mut Enhancer, input: &[f32]) -> Result<Vec<f32>, String> {
    let mut padded = input.to_vec();
    padded.resize((input.len() + DELAY).div_ceil(HOP) * HOP, 0.0);
    for hop in padded.chunks_exact_mut(HOP) {
        enhancer.process(hop.try_into().unwrap())?;
    }
    Ok(padded[DELAY..DELAY + input.len()].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{level_in, pauses, si_sdr};
    use crate::testsignals::{ALL_NOISES, Noise, mix, noise, wav};

    /// The models to measure: `KREOKIT_DPDFNET_MODELS=/dir` with the 48 kHz ONNX files.
    fn models() -> Vec<std::path::PathBuf> {
        let Ok(dir) = std::env::var("KREOKIT_DPDFNET_MODELS") else {
            return Vec::new();
        };
        ["dpdfnet2_48khz_hr.onnx", "dpdfnet8_48khz_hr.onnx"]
            .iter()
            .map(|f| Path::new(&dir).join(f))
            .filter(|p| p.is_file())
            .collect()
    }

    fn enhancer(path: &Path) -> Enhancer {
        Enhancer::new(load(path).unwrap()).unwrap()
    }

    #[test]
    #[ignore]
    fn measures_the_delay() {
        for path in models() {
            // Clean speech passes almost unchanged: the best-matching lag is the delay.
            let input = wav("voices.wav");
            let mut e = enhancer(&path);
            let mut out = input.clone();
            out.resize(input.len().div_ceil(HOP) * HOP, 0.0);
            for hop in out.chunks_exact_mut(HOP) {
                e.process(hop.try_into().unwrap()).unwrap();
            }
            let corr = |lag: usize| -> f64 {
                let (a, b) = (&input[96_000 - lag..192_000 - lag], &out[96_000..192_000]);
                let dot: f64 = a
                    .iter()
                    .zip(b)
                    .map(|(x, y)| (*x as f64) * (*y as f64))
                    .sum();
                let n = |x: &[f32]| x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>().sqrt();
                dot / (n(a) * n(b))
            };
            let best = (0..4_800)
                .max_by(|&a, &b| corr(a).total_cmp(&corr(b)))
                .unwrap();
            eprintln!(
                "{}: delay {best}, correlation {:.3}",
                path.display(),
                corr(best)
            );
            assert_eq!(best, DELAY);
        }
    }

    /// The same oracle as RNNoise's (`clean::tests::quality_on_every_noise`), plus the
    /// speed: `KREOKIT_DPDFNET_MODELS=/dir cargo test --release dpdfnet::tests::quality -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn quality_on_every_noise() {
        let speech = wav("voices.wav");
        let gaps = pauses(&speech);
        let seconds = speech.len() as f64 / 48_000.0;
        for path in models() {
            eprintln!("== {}", path.file_name().unwrap().to_string_lossy());
            eprintln!("noise      SNR  SI-SDR in → out (gain)   pauses in → out (drop)");
            let mut work = 0.0;
            for kind in ALL_NOISES {
                let n = noise(kind, speech.len(), 11);
                for snr in [0.0, 5.0, 10.0] {
                    let mixed = mix(&speech, &n, snr);
                    let started = std::time::Instant::now();
                    let out = clean(&mut enhancer(&path), &mixed).unwrap();
                    work += started.elapsed().as_secs_f64();
                    let (before, after) = (si_sdr(&speech, &mixed), si_sdr(&speech, &out));
                    let (floor_in, floor_out) = (level_in(&mixed, &gaps), level_in(&out, &gaps));
                    eprintln!(
                        "{:<9} {snr:>4}  {before:>6.1} → {after:>5.1} ({:+5.1} dB)   {floor_in:>6.1} → {floor_out:>6.1} ({:+5.1} dB)",
                        format!("{kind:?}"),
                        after - before,
                        floor_out - floor_in
                    );
                }
            }
            let out = clean(&mut enhancer(&path), &speech).unwrap();
            eprintln!(
                "clean speech in: SI-SDR of the output {:.1} dB",
                si_sdr(&speech, &out)
            );
            eprintln!(
                "realtime factor {:.3} (one thread)",
                work / (seconds * 18.0)
            );
            let _ = Noise::Room;
        }
    }
}
