//! Xiph's RNNoise v0.2 (vendored C in `rnnoise/`, compiled by build.rs with its model):
//! the Fast noise remover. One [`Denoiser`] per channel; it is a stream, fed 10 ms frames
//! of 48 kHz audio in order, so a whole recording is cleaned without any chunk edges.

use std::os::raw::{c_int, c_void};

/// Samples per frame (10 ms at 48 kHz).
pub const FRAME: usize = 480;

/// RNNoise works on samples at 16-bit scale.
const SCALE: f32 = 32768.0;

#[repr(C)]
struct DenoiseState {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn rnnoise_get_frame_size() -> c_int;
    fn rnnoise_create(model: *mut c_void) -> *mut DenoiseState;
    fn rnnoise_destroy(st: *mut DenoiseState);
    fn rnnoise_process_frame(st: *mut DenoiseState, out: *mut f32, input: *const f32) -> f32;
}

/// One channel's denoiser, with the model compiled into the app.
pub struct Denoiser {
    state: *mut DenoiseState,
    buffer: [f32; FRAME],
}

// SAFETY: the state is only used through `&mut self`, by one thread at a time.
unsafe impl Send for Denoiser {}

impl Denoiser {
    pub fn new() -> Self {
        // SAFETY: plain calls into the library; a null model means the built-in one.
        let state = unsafe {
            assert_eq!(rnnoise_get_frame_size() as usize, FRAME);
            rnnoise_create(std::ptr::null_mut())
        };
        assert!(!state.is_null(), "RNNoise could not start");
        Self {
            state,
            buffer: [0.0; FRAME],
        }
    }

    /// Cleans one frame of samples in -1..1, in place. Returns RNNoise's voice
    /// probability (0–1) for it. The output lags the input by [`DELAY`] samples.
    pub fn process(&mut self, frame: &mut [f32; FRAME]) -> f32 {
        for (b, s) in self.buffer.iter_mut().zip(frame.iter()) {
            *b = s * SCALE;
        }
        // SAFETY: both buffers hold FRAME samples; the state is ours.
        let vad =
            unsafe { rnnoise_process_frame(self.state, frame.as_mut_ptr(), self.buffer.as_ptr()) };
        for s in frame.iter_mut() {
            *s /= SCALE;
        }
        vad
    }
}

impl Default for Denoiser {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Denoiser {
    fn drop(&mut self) {
        // SAFETY: created by rnnoise_create, destroyed once.
        unsafe { rnnoise_destroy(self.state) }
    }
}

/// How many samples the output lags the input: 20 ms (measured on speech in
/// `measures_the_delay`; two frames: RNNoise's analysis window plus its look-ahead).
pub const DELAY: usize = 960;

#[cfg(test)]
mod tests {
    use super::*;

    /// A reproducible noise source (xorshift), uniform in -1..1.
    fn noise(n: usize, seed: u64) -> Vec<f32> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 40) as f32 / (1u64 << 23) as f32 - 1.0
            })
            .collect()
    }

    fn run(input: &[f32]) -> Vec<f32> {
        let mut d = Denoiser::new();
        let mut out = Vec::with_capacity(input.len());
        for chunk in input.chunks_exact(FRAME) {
            let mut frame: [f32; FRAME] = chunk.try_into().unwrap();
            d.process(&mut frame);
            out.extend_from_slice(&frame);
        }
        out
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn removes_steady_noise() {
        let input: Vec<f32> = noise(48_000 * 3, 7).iter().map(|s| s * 0.05).collect();
        let out = run(&input);
        // The last second, once the model has settled: at least 20 dB quieter.
        let before = rms(&input[96_000..]);
        let after = rms(&out[96_000..]);
        assert!(after < before / 10.0, "{before} -> {after}");
    }

    /// The fixture's samples: 16-bit 48 kHz mono PCM after the WAV header's "data" tag.
    pub(crate) fn speech() -> Vec<f32> {
        let bytes = std::fs::read(crate::probe::fixture("hello-mono.wav")).unwrap();
        let data = bytes.windows(4).position(|w| w == b"data").unwrap() + 8;
        bytes[data..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / SCALE)
            .collect()
    }

    #[test]
    fn measures_the_delay() {
        // Clean speech passes almost unchanged: the lag where input and output match best
        // (normalized correlation) is the delay.
        let input = speech();
        let out = run(&input);
        // One second of speech, from 1.0 s on.
        let corr = |lag: usize| -> f32 {
            let (a, b) = (&input[48_000 - lag..96_000 - lag], &out[48_000..96_000]);
            let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
            dot / (rms(a) * rms(b) * a.len() as f32)
        };
        let best = (0..1_920)
            .max_by(|&a, &b| corr(a).total_cmp(&corr(b)))
            .unwrap();
        assert_eq!(best, DELAY, "correlation {}", corr(best));
        assert!(corr(DELAY) > 0.9, "{}", corr(DELAY));
    }

    #[test]
    fn independent_streams_give_identical_results() {
        let input: Vec<f32> = noise(48_000, 3).iter().map(|s| s * 0.1).collect();
        assert_eq!(run(&input), run(&input));
    }
}
