//! Noise removal over whole recordings: one streaming [`Denoiser`] per channel, fed in
//! order, so there are no chunk edges anywhere (the result doesn't depend on how the
//! audio arrives; `same_result_whatever_the_block_size`). The denoiser's 20 ms delay is
//! removed: output sample `i` lines up with input sample `i`, and the output is exactly as
//! long as the input, so it can be mixed with the original (Strength) and put back under a
//! video.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::preview::{self, Levels, SAMPLE_RATE};
use crate::rnnoise::{DELAY, Denoiser, FRAME};

/// Cleans interleaved audio of 1 or 2 channels, block by block.
pub struct Cleaner {
    channels: usize,
    denoisers: Vec<Denoiser>,
    /// Each channel's samples waiting for a full frame.
    pending: Vec<Vec<f32>>,
    /// Output samples per channel still to drop (the denoiser's delay).
    skip: usize,
    /// Input samples per channel so far, and output samples given out.
    taken: usize,
    given: usize,
}

impl Cleaner {
    pub fn new(channels: usize) -> Self {
        assert!((1..=2).contains(&channels), "{channels} channels");
        Self {
            channels,
            denoisers: (0..channels).map(|_| Denoiser::new()).collect(),
            pending: vec![Vec::new(); channels],
            skip: DELAY,
            taken: 0,
            given: 0,
        }
    }

    /// Feeds interleaved samples (whole frames of all channels) and appends the cleaned
    /// samples that are ready to `out`, interleaved.
    pub fn push(&mut self, interleaved: &[f32], out: &mut Vec<f32>) {
        for frame in interleaved.chunks_exact(self.channels) {
            for (ch, s) in frame.iter().enumerate() {
                self.pending[ch].push(*s);
            }
        }
        self.taken += interleaved.len() / self.channels;
        self.run(out);
    }

    /// Ends the stream: the last samples come out (the denoiser is fed silence to flush
    /// its delay), and the output is cut to the input's length.
    pub fn finish(mut self, out: &mut Vec<f32>) {
        let target = self.taken;
        let flush = DELAY + (FRAME - self.pending[0].len() % FRAME) % FRAME;
        for p in &mut self.pending {
            p.extend(std::iter::repeat_n(0.0, flush));
        }
        let before = out.len();
        self.run(out);
        let extra = (self.given - target) * self.channels;
        out.truncate(out.len().max(before + extra) - extra);
    }

    /// Cleans all full frames of every channel (channels in parallel) and interleaves.
    fn run(&mut self, out: &mut Vec<f32>) {
        let frames = self.pending[0].len() / FRAME;
        if frames == 0 {
            return;
        }
        let used = frames * FRAME;
        let mut cleaned: Vec<Vec<f32>> = Vec::with_capacity(self.channels);
        std::thread::scope(|scope| {
            let jobs: Vec<_> = self
                .denoisers
                .iter_mut()
                .zip(&self.pending)
                .map(|(denoiser, pending)| {
                    let input = &pending[..used];
                    let mut work = move || {
                        let mut out = input.to_vec();
                        for chunk in out.chunks_exact_mut(FRAME) {
                            denoiser.process(chunk.try_into().unwrap());
                        }
                        out
                    };
                    if self.channels > 1 && frames >= 10 {
                        Ok(scope.spawn(work))
                    } else {
                        Err(work())
                    }
                })
                .collect();
            for job in jobs {
                cleaned.push(match job {
                    Ok(handle) => handle.join().unwrap(),
                    Err(done) => done,
                });
            }
        });
        for p in &mut self.pending {
            p.drain(..used);
        }
        let skip = self.skip.min(used);
        self.skip -= skip;
        out.reserve((used - skip) * self.channels);
        for i in skip..used {
            for ch in &cleaned {
                out.push(ch[i]);
            }
        }
        self.given += used - skip;
    }
}

/// Cleans a whole mono or interleaved buffer at once.
#[cfg(test)]
pub fn clean(interleaved: &[f32], channels: usize) -> Vec<f32> {
    let mut cleaner = Cleaner::new(channels);
    let mut out = Vec::with_capacity(interleaved.len());
    cleaner.push(interleaved, &mut out);
    cleaner.finish(&mut out);
    out
}

/// The default Strength (measured 2026-10-21, CLAUDE.md "Fast clean"): the noise left in
/// the pauses is 20·log10(1 − s) dB, about −16.5 dB here, while clean speech keeps 17.8 dB
/// SI-SDR instead of 16.4 at full strength, and steady noise is within 0.3 dB of the best.
pub const DEFAULT_STRENGTH: f32 = 0.85;

/// Strength: the cleaned sound mixed with the original (0 = original, 1 = fully cleaned).
pub fn blend(original: f32, cleaned: f32, strength: f32) -> f32 {
    original + (cleaned - original) * strength
}

/// Bumped when the cleaned cache's format or processing changes; older ones are redone.
const VERSION: u32 = 1;

/// The model that made a cleaned cache: RNNoise and its weights' hash.
fn model_id() -> String {
    format!(
        "rnnoise-{}",
        include_str!("../rnnoise/model_version").trim()
    )
}

/// A finished cleaned preview (`clean-fast.*` next to the preview cache).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CleanMeta {
    pub version: u32,
    pub model: String,
    /// Samples per channel, the same as the preview's.
    pub frames: u64,
    /// Seconds of work per second of sound (cleaning time / duration).
    pub realtime_factor: f64,
}

pub fn audio_path(dir: &Path) -> PathBuf {
    dir.join("clean-fast.pcm")
}

fn peaks_path(dir: &Path) -> PathBuf {
    dir.join("clean-fast.peaks")
}

fn meta_path(dir: &Path) -> PathBuf {
    dir.join("clean-fast.json")
}

/// The cleaned preview in `dir`, if it is complete and matches `preview` and this model.
pub fn cached(dir: &Path, preview: &preview::Meta) -> Option<CleanMeta> {
    let meta: CleanMeta = serde_json::from_str(&fs::read_to_string(meta_path(dir)).ok()?).ok()?;
    let complete = audio_path(dir)
        .metadata()
        .is_ok_and(|m| m.len() == meta.frames * preview.channels as u64 * 2);
    (meta.version == VERSION
        && meta.model == model_id()
        && meta.frames == preview.frames
        && complete)
        .then_some(meta)
}

/// The cleaned waveform: one level per 10 ms, like the preview's.
pub fn peaks(dir: &Path) -> Result<Vec<u8>, String> {
    fs::read(peaks_path(dir)).map_err(|e| format!("cannot read the cleaned waveform: {e}"))
}

/// Cleans the preview in `dir` (described by `preview`) into `clean-fast.*`, fully
/// (Strength is applied when listening). `progress` gets the fraction done about every
/// second of sound; `cancel` stops it and leaves nothing behind.
pub fn prepare(
    dir: &Path,
    preview: &preview::Meta,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f64),
) -> Result<CleanMeta, String> {
    if let Some(meta) = cached(dir, preview) {
        return Ok(meta);
    }
    let part = dir.join("clean-fast.pcm.part");
    let result = clean_preview(dir, &part, preview, cancel, &mut progress);
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

fn clean_preview(
    dir: &Path,
    part: &Path,
    preview: &preview::Meta,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(f64),
) -> Result<CleanMeta, String> {
    let started = std::time::Instant::now();
    let channels = preview.channels as usize;
    let mut input = BufReader::with_capacity(
        1 << 20,
        File::open(preview::audio_path(dir))
            .map_err(|e| format!("cannot read the preview: {e}"))?,
    );
    let mut output = BufWriter::with_capacity(
        1 << 20,
        File::create(part).map_err(|e| format!("cannot write the cleaned sound: {e}"))?,
    );
    let mut levels = Levels::new(preview.channels);
    let mut cleaner = Cleaner::new(channels);
    // One second of sound per step.
    let block = SAMPLE_RATE as usize * channels * 2;
    let mut bytes = vec![0u8; block];
    let mut samples = Vec::with_capacity(block / 2);
    let mut cleaned = Vec::with_capacity(block / 2);
    let mut done = 0u64;
    let mut write = |cleaned: &mut Vec<f32>, levels: &mut Levels| -> Result<(), String> {
        let out: Vec<u8> = cleaned
            .iter()
            .flat_map(|s| ((s * 32768.0).round().clamp(-32768.0, 32767.0) as i16).to_le_bytes())
            .collect();
        levels.push(&out);
        cleaned.clear();
        output
            .write_all(&out)
            .map_err(|e| format!("cannot write the cleaned sound: {e}"))
    };
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let n = read_full(&mut input, &mut bytes)?;
        if n == 0 {
            break;
        }
        samples.clear();
        samples.extend(
            bytes[..n]
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0),
        );
        cleaner.push(&samples, &mut cleaned);
        write(&mut cleaned, &mut levels)?;
        done += (n / (channels * 2)) as u64;
        progress((done as f64 / preview.frames.max(1) as f64).min(1.0));
    }
    cleaner.finish(&mut cleaned);
    write(&mut cleaned, &mut levels)?;
    drop(write);
    output
        .into_inner()
        .map_err(|e| format!("cannot write the cleaned sound: {e}"))?
        .sync_all()
        .map_err(|e| format!("cannot write the cleaned sound: {e}"))?;
    if done != preview.frames {
        return Err(format!(
            "the preview changed while cleaning ({done} of {} samples)",
            preview.frames
        ));
    }
    fs::write(peaks_path(dir), levels.finish()).map_err(|e| e.to_string())?;
    fs::rename(part, audio_path(dir))
        .map_err(|e| format!("cannot finish the cleaned sound: {e}"))?;
    let seconds = preview.frames as f64 / SAMPLE_RATE as f64;
    let meta = CleanMeta {
        version: VERSION,
        model: model_id(),
        frames: preview.frames,
        realtime_factor: started.elapsed().as_secs_f64() / seconds.max(1e-3),
    };
    let json = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    fs::write(meta_path(dir), json).map_err(|e| e.to_string())?;
    progress(1.0);
    Ok(meta)
}

/// Reads until `buf` is full or the file ends; returns the bytes read.
fn read_full(input: &mut impl Read, buf: &mut [u8]) -> Result<usize, String> {
    let mut filled = 0;
    while filled < buf.len() {
        match input.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(format!("cannot read the preview: {e}")),
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{level_in, pauses, si_sdr};
    use crate::testsignals::{ALL_NOISES, Noise, mix, noise, wav};

    #[test]
    fn same_length_and_aligned_with_the_input() {
        let speech = wav("hello-mono.wav");
        let out = clean(&speech, 1);
        assert_eq!(out.len(), speech.len());
        // Aligned: the cleaned clean speech matches the input closely at lag 0.
        assert!(si_sdr(&speech, &out) > 15.0, "{}", si_sdr(&speech, &out));
    }

    #[test]
    fn same_result_whatever_the_block_size() {
        let speech = wav("hello-mono.wav");
        let whole = clean(&speech, 1);
        for block in [1usize, 333, 480, 4_801, 48_000] {
            let mut cleaner = Cleaner::new(1);
            let mut out = Vec::new();
            for piece in speech.chunks(block) {
                cleaner.push(piece, &mut out);
            }
            cleaner.finish(&mut out);
            assert!(out == whole, "block {block}");
        }
    }

    #[test]
    fn stereo_channels_are_cleaned_separately() {
        let speech = wav("hello-mono.wav");
        let hum = noise(Noise::Hum, speech.len(), 1);
        let noisy = mix(&speech, &hum, 0.0);
        // Left: speech; right: the same speech with hum.
        let interleaved: Vec<f32> = speech
            .iter()
            .zip(&noisy)
            .flat_map(|(a, b)| [*a, *b])
            .collect();
        let out = clean(&interleaved, 2);
        assert_eq!(out.len(), interleaved.len());
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        assert_eq!(left, clean(&speech, 1));
    }

    #[test]
    fn strength_blends() {
        assert_eq!(blend(1.0, 0.0, 0.0), 1.0);
        assert_eq!(blend(1.0, 0.0, 1.0), 0.0);
        assert_eq!(blend(1.0, 0.0, 0.75), 0.25);
    }

    /// The quality oracle (CLAUDE.md, "Tests"): every noise at 0, 5 and 10 dB SNR, SI-SDR
    /// and the noise left in the pauses, before and after. Prints the table; the asserted
    /// minimums sit a little below the baseline measured on 2026-10-21 (see CLAUDE.md).
    #[test]
    fn quality_on_every_noise() {
        let speech = wav("voices.wav");
        let gaps = pauses(&speech);
        assert!(gaps.len() > 100, "the fixture has pauses");
        eprintln!("noise      SNR  SI-SDR in → out (gain)   pauses in → out (drop)");
        for kind in ALL_NOISES {
            let n = noise(kind, speech.len(), 11);
            for snr in [0.0, 5.0, 10.0] {
                let mixed = mix(&speech, &n, snr);
                let out = clean(&mixed, 1);
                let (before, after) = (si_sdr(&speech, &mixed), si_sdr(&speech, &out));
                let (floor_in, floor_out) = (level_in(&mixed, &gaps), level_in(&out, &gaps));
                eprintln!(
                    "{:<9} {snr:>4}  {before:>6.1} → {after:>5.1} ({:+5.1} dB)   {floor_in:>6.1} → {floor_out:>6.1} ({:+5.1} dB)",
                    format!("{kind:?}"),
                    after - before,
                    floor_out - floor_in
                );
                // Baseline 2026-10-21: steady noises gained 3.3–13.8 dB of SI-SDR and their
                // pauses dropped 29–46 dB. Babble (other voices) is beyond RNNoise: it loses
                // 2–6 dB of SI-SDR, though its pauses still drop 5–10 dB.
                if kind == Noise::Babble {
                    assert!(floor_out < floor_in - 3.0, "{kind:?} {snr}");
                } else {
                    assert!(after - before > 2.3, "{kind:?} {snr}: {before} → {after}");
                    assert!(floor_out < floor_in - 25.0, "{kind:?} {snr}");
                }
            }
        }
        // No damage: clean speech in keeps 16.4 dB of SI-SDR at full strength, 17.8 at the
        // default.
        let out = clean(&speech, 1);
        let full = si_sdr(&speech, &out);
        let blended: Vec<f32> = speech
            .iter()
            .zip(&out)
            .map(|(o, c)| blend(*o, *c, DEFAULT_STRENGTH))
            .collect();
        let default = si_sdr(&speech, &blended);
        eprintln!(
            "clean speech in: SI-SDR {full:.1} dB (full), {default:.1} dB (default strength)"
        );
        assert!(full > 15.0 && default > 16.5, "{full} {default}");
    }

    /// Needs ffmpeg: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn cleans_a_prepared_file_and_reuses_the_result() {
        let source = crate::probe::fixture("hello-stereo.m4a");
        let dir = std::env::temp_dir().join("kreokit-voice-clean-job");
        let _ = fs::remove_dir_all(&dir);
        let preview =
            preview::prepare(&dir, &source, 2, 5_000, &AtomicBool::new(false), |_| {}).unwrap();
        let mut reports = Vec::new();
        let meta = prepare(&dir, &preview, &AtomicBool::new(false), |f| reports.push(f)).unwrap();
        assert_eq!(meta.frames, preview.frames);
        assert_eq!(reports.last(), Some(&1.0));
        assert_eq!(
            fs::metadata(audio_path(&dir)).unwrap().len(),
            preview.frames * 4
        );
        assert_eq!(
            peaks(&dir).unwrap().len() as u64,
            preview.frames.div_ceil(480)
        );
        eprintln!("realtime factor {:.3}", meta.realtime_factor);
        // Found again without work (cancel is set: real work would fail).
        assert_eq!(
            prepare(&dir, &preview, &AtomicBool::new(true), |_| {}).unwrap(),
            meta
        );
        // Cancelling leaves nothing behind.
        fs::remove_file(meta_path(&dir)).unwrap();
        assert!(prepare(&dir, &preview, &AtomicBool::new(true), |_| {}).is_err());
        assert!(!dir.join("clean-fast.pcm.part").exists());
    }
}
