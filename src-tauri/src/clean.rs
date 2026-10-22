//! Noise removal over whole recordings, with Fast (RNNoise) or Best (DPDFNet): one
//! streaming engine per channel, fed in order, so there are no chunk edges anywhere (the
//! result doesn't depend on how the audio arrives; `same_result_whatever_the_block_size`).
//! The engine's delay (20 ms Fast, 50 ms Best) is removed: output sample `i` lines up with
//! input sample `i`, and the output is exactly as long as the input, so it can be mixed with
//! the original (Strength) and put back under a video.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::preview::{self, Levels, SAMPLE_RATE};
use crate::{dpdfnet, models, rnnoise};

/// Both engines take 10 ms frames at 48 kHz.
const FRAME: usize = rnnoise::FRAME;
const _: () = assert!(dpdfnet::HOP == FRAME);

/// The noise remover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Model {
    /// RNNoise v0.2, compiled in: ~12× real time per channel on the dev Mac.
    #[default]
    Fast,
    /// DPDFNet-2 48 kHz, downloaded once (10.5 MB): ~3× real time per channel, much better
    /// (CLAUDE.md, "Best model").
    Best,
}

impl Model {
    /// The measured default Strength (CLAUDE.md "Fast clean" and "Best model"): Fast's
    /// SI-SDR peaks around 70–90% and clean speech suffers a little at 100%; Best keeps
    /// improving up to 95–100% and barely touches clean speech.
    pub fn default_strength(self) -> f32 {
        match self {
            Self::Fast => 0.85,
            Self::Best => 0.95,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Best => "best",
        }
    }

    /// Which engine and weights made a cleaned cache.
    fn id(self) -> String {
        match self {
            Self::Fast => format!(
                "rnnoise-{}",
                include_str!("../rnnoise/model_version").trim()
            ),
            Self::Best => "dpdfnet2_48khz_hr-7f0575a5".to_string(),
        }
    }
}

/// One channel's noise remover.
pub enum Engine {
    Fast(rnnoise::Denoiser),
    Best(Box<dpdfnet::Enhancer>),
}

impl Engine {
    /// A new engine for `model` (Best loads its downloaded file from `models_dir`).
    pub fn new(model: Model, models_dir: &Path) -> Result<Self, String> {
        Ok(match model {
            Model::Fast => Self::Fast(rnnoise::Denoiser::new()),
            Model::Best => {
                let cpu = crate::cpu::check();
                if !cpu.supported {
                    return Err(format!(
                        "this computer's processor lacks {} for the Best model",
                        cpu.missing.join(", ")
                    ));
                }
                if !models::best_downloaded(models_dir) {
                    return Err("the Best model isn't downloaded".into());
                }
                let session = dpdfnet::load(&models::best_path(models_dir))?;
                Self::Best(Box::new(dpdfnet::Enhancer::new(session)?))
            }
        })
    }

    fn delay(&self) -> usize {
        match self {
            Self::Fast(_) => rnnoise::DELAY,
            Self::Best(_) => dpdfnet::DELAY,
        }
    }

    fn process(&mut self, frame: &mut [f32; FRAME]) -> Result<(), String> {
        match self {
            Self::Fast(d) => {
                d.process(frame);
                Ok(())
            }
            Self::Best(e) => e.process(frame),
        }
    }
}

/// Cleans interleaved audio of 1 or 2 channels, block by block.
pub struct Cleaner {
    channels: usize,
    engines: Vec<Engine>,
    /// Each channel's samples waiting for a full frame.
    pending: Vec<Vec<f32>>,
    /// The engine's delay, and the output samples per channel still to drop for it.
    delay: usize,
    skip: usize,
    /// Input samples per channel so far, and output samples given out.
    taken: usize,
    given: usize,
}

impl Cleaner {
    /// A Fast cleaner (it can't fail).
    #[cfg(test)]
    pub fn new(channels: usize) -> Self {
        let engines = (0..channels)
            .map(|_| Engine::Fast(rnnoise::Denoiser::new()))
            .collect();
        Self::with(engines)
    }

    /// A cleaner with one engine per channel (all of the same model).
    pub fn with(engines: Vec<Engine>) -> Self {
        let channels = engines.len();
        assert!((1..=2).contains(&channels), "{channels} channels");
        let delay = engines[0].delay();
        Self {
            channels,
            engines,
            pending: vec![Vec::new(); channels],
            delay,
            skip: delay,
            taken: 0,
            given: 0,
        }
    }

    /// Feeds interleaved samples (whole frames of all channels) and appends the cleaned
    /// samples that are ready to `out`, interleaved.
    pub fn push(&mut self, interleaved: &[f32], out: &mut Vec<f32>) -> Result<(), String> {
        for frame in interleaved.chunks_exact(self.channels) {
            for (ch, s) in frame.iter().enumerate() {
                self.pending[ch].push(*s);
            }
        }
        self.taken += interleaved.len() / self.channels;
        self.run(out)
    }

    /// Ends the stream: the last samples come out (the engine is fed silence to flush its
    /// delay), and the output is cut to the input's length.
    pub fn finish(mut self, out: &mut Vec<f32>) -> Result<(), String> {
        let target = self.taken;
        let flush = self.delay + (FRAME - self.pending[0].len() % FRAME) % FRAME;
        for p in &mut self.pending {
            p.extend(std::iter::repeat_n(0.0, flush));
        }
        let before = out.len();
        self.run(out)?;
        let extra = (self.given - target) * self.channels;
        out.truncate(out.len().max(before + extra) - extra);
        Ok(())
    }

    /// Cleans all full frames of every channel (channels in parallel) and interleaves.
    fn run(&mut self, out: &mut Vec<f32>) -> Result<(), String> {
        let frames = self.pending[0].len() / FRAME;
        if frames == 0 {
            return Ok(());
        }
        let used = frames * FRAME;
        let parallel = self.channels > 1 && frames >= 10;
        let mut cleaned: Vec<Result<Vec<f32>, String>> = Vec::with_capacity(self.channels);
        std::thread::scope(|scope| {
            let jobs: Vec<_> = self
                .engines
                .iter_mut()
                .zip(&self.pending)
                .map(|(engine, pending)| {
                    let input = &pending[..used];
                    let mut work = move || -> Result<Vec<f32>, String> {
                        let mut out = input.to_vec();
                        for chunk in out.chunks_exact_mut(FRAME) {
                            engine.process(chunk.try_into().unwrap())?;
                        }
                        Ok(out)
                    };
                    if parallel {
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
        let cleaned = cleaned.into_iter().collect::<Result<Vec<_>, _>>()?;
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
        Ok(())
    }
}

/// Cleans a whole mono or interleaved buffer at once with Fast.
#[cfg(test)]
pub fn clean(interleaved: &[f32], channels: usize) -> Vec<f32> {
    let mut cleaner = Cleaner::new(channels);
    let mut out = Vec::with_capacity(interleaved.len());
    cleaner.push(interleaved, &mut out).unwrap();
    cleaner.finish(&mut out).unwrap();
    out
}

/// Fast's default Strength (measured 2026-10-21, CLAUDE.md "Fast clean"): the noise left
/// in the pauses is 20·log10(1 − s) dB, about −16.5 dB here, while clean speech keeps
/// 17.8 dB SI-SDR instead of 16.4 at full strength, and steady noise is within 0.3 dB of
/// the best.
pub const DEFAULT_STRENGTH: f32 = 0.85;

/// Strength: the cleaned sound mixed with the original (0 = original, 1 = fully cleaned).
pub fn blend(original: f32, cleaned: f32, strength: f32) -> f32 {
    original + (cleaned - original) * strength
}

/// Bumped when the cleaned cache's format or processing changes; older ones are redone.
const VERSION: u32 = 1;

/// A finished cleaned preview (`clean-<model>.*` next to the preview cache).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CleanMeta {
    pub version: u32,
    pub model: String,
    /// Samples per channel, the same as the preview's.
    pub frames: u64,
    /// Seconds of work per second of sound (cleaning time / duration).
    pub realtime_factor: f64,
    /// A stereo file whose channels are the same sound (dual mono): Best cleaned their
    /// average once and wrote it to both ([`DUAL_MONO_DB`]).
    #[serde(default)]
    pub dual_mono: bool,
}

/// Stereo counts as dual mono when the channels' difference (L − R) is at least this far
/// below their average ((L + R) / 2): the same microphone on both sides, give or take
/// encoding noise. Real stereo is far above it.
pub const DUAL_MONO_DB: f64 = -45.0;

/// How far the channels' difference is below their average, in dB, over the whole
/// interleaved 16-bit stereo preview (−inf when identical).
pub fn channel_difference_db(preview_pcm: &Path) -> Result<f64, String> {
    let mut input = BufReader::with_capacity(
        1 << 20,
        File::open(preview_pcm).map_err(|e| format!("cannot read the preview: {e}"))?,
    );
    let mut bytes = vec![0u8; 1 << 20];
    let (mut diff, mut mid) = (0f64, 0f64);
    loop {
        let n = read_full(&mut input, &mut bytes)?;
        if n == 0 {
            break;
        }
        for f in bytes[..n].chunks_exact(4) {
            let l = i16::from_le_bytes([f[0], f[1]]) as f64;
            let r = i16::from_le_bytes([f[2], f[3]]) as f64;
            diff += (l - r) * (l - r);
            mid += (l + r) * (l + r) / 4.0;
        }
    }
    if diff == 0.0 {
        return Ok(f64::NEG_INFINITY);
    }
    Ok(10.0 * (diff / mid.max(f64::MIN_POSITIVE)).log10())
}

pub fn audio_path(dir: &Path, model: Model) -> PathBuf {
    dir.join(format!("clean-{}.pcm", model.name()))
}

fn peaks_path(dir: &Path, model: Model) -> PathBuf {
    dir.join(format!("clean-{}.peaks", model.name()))
}

fn meta_path(dir: &Path, model: Model) -> PathBuf {
    dir.join(format!("clean-{}.json", model.name()))
}

/// `model`'s cleaned preview in `dir`, if it is complete and matches `preview` and the
/// model's weights.
pub fn cached(dir: &Path, preview: &preview::Meta, model: Model) -> Option<CleanMeta> {
    let meta: CleanMeta =
        serde_json::from_str(&fs::read_to_string(meta_path(dir, model)).ok()?).ok()?;
    let complete = audio_path(dir, model)
        .metadata()
        .is_ok_and(|m| m.len() == meta.frames * preview.channels as u64 * 2);
    (meta.version == VERSION
        && meta.model == model.id()
        && meta.frames == preview.frames
        && complete)
        .then_some(meta)
}

/// The cleaned waveform: one level per 10 ms, like the preview's.
pub fn peaks(dir: &Path, model: Model) -> Result<Vec<u8>, String> {
    fs::read(peaks_path(dir, model)).map_err(|e| format!("cannot read the cleaned waveform: {e}"))
}

/// Cleans the preview in `dir` (described by `preview`) with `model` into
/// `clean-<model>.*`, fully (Strength is applied when listening). `progress` gets the
/// fraction done about every second of sound; `cancel` stops it and leaves nothing behind.
pub fn prepare(
    dir: &Path,
    preview: &preview::Meta,
    model: Model,
    models_dir: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f64),
) -> Result<CleanMeta, String> {
    if let Some(meta) = cached(dir, preview, model) {
        return Ok(meta);
    }
    let part = dir.join(format!("clean-{}.pcm.part", model.name()));
    // Best on dual mono: one channel's work instead of two (the user's request, 2026-10-22).
    let dual_mono = model == Model::Best
        && preview.channels == 2
        && channel_difference_db(&preview::audio_path(dir))? <= DUAL_MONO_DB;
    let result = clean_preview(
        dir,
        &part,
        preview,
        model,
        models_dir,
        dual_mono,
        cancel,
        &mut progress,
    );
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

fn clean_preview(
    dir: &Path,
    part: &Path,
    preview: &preview::Meta,
    model: Model,
    models_dir: &Path,
    dual_mono: bool,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(f64),
) -> Result<CleanMeta, String> {
    let started = std::time::Instant::now();
    let channels = preview.channels as usize;
    // Dual mono: one engine on the channels' average; its output goes to both.
    let engines = (0..if dual_mono { 1 } else { channels })
        .map(|_| Engine::new(model, models_dir))
        .collect::<Result<Vec<_>, _>>()?;
    let mut cleaner = Cleaner::with(engines);
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
    // One second of sound per step.
    let block = SAMPLE_RATE as usize * channels * 2;
    let mut bytes = vec![0u8; block];
    let mut samples = Vec::with_capacity(block / 2);
    let mut cleaned = Vec::with_capacity(block / 2);
    let mut done = 0u64;
    let copies = if dual_mono { 2 } else { 1 };
    let mut write = |cleaned: &mut Vec<f32>, levels: &mut Levels| -> Result<(), String> {
        let out: Vec<u8> = cleaned
            .iter()
            .flat_map(|s| {
                let v = ((s * 32768.0).round().clamp(-32768.0, 32767.0) as i16).to_le_bytes();
                std::iter::repeat_n(v, copies).flatten()
            })
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
        let value = |b: &[u8]| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0;
        if dual_mono {
            samples.extend(
                bytes[..n]
                    .chunks_exact(4)
                    .map(|f| (value(&f[..2]) + value(&f[2..])) * 0.5),
            );
        } else {
            samples.extend(bytes[..n].chunks_exact(2).map(value));
        }
        cleaner.push(&samples, &mut cleaned)?;
        write(&mut cleaned, &mut levels)?;
        done += (n / (channels * 2)) as u64;
        progress((done as f64 / preview.frames.max(1) as f64).min(1.0));
    }
    cleaner.finish(&mut cleaned)?;
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
    fs::write(peaks_path(dir, model), levels.finish()).map_err(|e| e.to_string())?;
    fs::rename(part, audio_path(dir, model))
        .map_err(|e| format!("cannot finish the cleaned sound: {e}"))?;
    let seconds = preview.frames as f64 / SAMPLE_RATE as f64;
    let meta = CleanMeta {
        version: VERSION,
        model: model.id(),
        frames: preview.frames,
        // Rounded: JSON doesn't give every f64 back exactly, and 4 decimals is plenty.
        realtime_factor: (started.elapsed().as_secs_f64() / seconds.max(1e-3) * 1e4).round() / 1e4,
        dual_mono,
    };
    let json = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    fs::write(meta_path(dir, model), json).map_err(|e| e.to_string())?;
    progress(1.0);
    Ok(meta)
}

/// The sound Polish works on: the preview, blended at the Strength with its cleaned sound
/// when there is one (`model`), read in one-second blocks.
pub struct BlendedPreview {
    pub dir: PathBuf,
    pub preview: preview::Meta,
    pub cleaned: Option<Model>,
    pub strength: f32,
}

impl crate::loudness::Source for BlendedPreview {
    fn channels(&self) -> usize {
        self.preview.channels as usize
    }

    fn frames(&self) -> u64 {
        self.preview.frames
    }

    fn for_each_block(
        &self,
        each: &mut dyn FnMut(&[f32]) -> Result<(), String>,
    ) -> Result<(), String> {
        self.for_each_block_in(&[(0, self.preview.frames)], each)
    }

    fn for_each_block_in(
        &self,
        ranges: &[(u64, u64)],
        each: &mut dyn FnMut(&[f32]) -> Result<(), String>,
    ) -> Result<(), String> {
        use std::io::{Seek, SeekFrom};
        let open = |path: PathBuf| -> Result<BufReader<File>, String> {
            File::open(&path)
                .map(|f| BufReader::with_capacity(1 << 20, f))
                .map_err(|e| format!("cannot read {}: {e}", path.display()))
        };
        let mut original = open(preview::audio_path(&self.dir))?;
        let mut cleaned = match self.cleaned {
            Some(model) => Some(open(audio_path(&self.dir, model))?),
            None => None,
        };
        let frame_bytes = self.channels() as u64 * 2;
        let block = SAMPLE_RATE as usize * frame_bytes as usize;
        let (mut a, mut b) = (vec![0u8; block], vec![0u8; block]);
        let mut samples = Vec::with_capacity(block / 2);
        let value = |b: &[u8]| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0;
        for &(start, end) in ranges {
            let seek = |r: &mut BufReader<File>| {
                r.seek(SeekFrom::Start(start * frame_bytes))
                    .map_err(|e| format!("cannot read the preview: {e}"))
            };
            seek(&mut original)?;
            if let Some(c) = cleaned.as_mut() {
                seek(c)?;
            }
            let mut left =
                (end.min(self.preview.frames).saturating_sub(start) * frame_bytes) as usize;
            while left > 0 {
                let want = left.min(block);
                let n = read_full(&mut original, &mut a[..want])?;
                if n == 0 {
                    break;
                }
                left -= n;
                samples.clear();
                match cleaned.as_mut() {
                    Some(c) => {
                        let m = read_full(c, &mut b[..n])?;
                        if m != n {
                            return Err("the cleaned sound is shorter than the preview".into());
                        }
                        samples.extend(
                            a[..n]
                                .chunks_exact(2)
                                .zip(b[..n].chunks_exact(2))
                                .map(|(o, c)| blend(value(o), value(c), self.strength)),
                        );
                    }
                    None => samples.extend(a[..n].chunks_exact(2).map(value)),
                }
                each(&samples)?;
            }
        }
        Ok(())
    }
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
                cleaner.push(piece, &mut out).unwrap();
            }
            cleaner.finish(&mut out).unwrap();
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
        let models = Path::new("");
        let meta = prepare(
            &dir,
            &preview,
            Model::Fast,
            models,
            &AtomicBool::new(false),
            |f| reports.push(f),
        )
        .unwrap();
        assert_eq!(meta.frames, preview.frames);
        assert_eq!(reports.last(), Some(&1.0));
        assert_eq!(
            fs::metadata(audio_path(&dir, Model::Fast)).unwrap().len(),
            preview.frames * 4
        );
        assert_eq!(
            peaks(&dir, Model::Fast).unwrap().len() as u64,
            preview.frames.div_ceil(480)
        );
        eprintln!("realtime factor {:.3}", meta.realtime_factor);
        // Found again without work (cancel is set: real work would fail).
        assert_eq!(
            prepare(
                &dir,
                &preview,
                Model::Fast,
                models,
                &AtomicBool::new(true),
                |_| {}
            )
            .unwrap(),
            meta
        );
        // Cancelling leaves nothing behind.
        fs::remove_file(meta_path(&dir, Model::Fast)).unwrap();
        assert!(
            prepare(
                &dir,
                &preview,
                Model::Fast,
                models,
                &AtomicBool::new(true),
                |_| {}
            )
            .is_err()
        );
        assert!(!dir.join("clean-fast.pcm.part").exists());
        // Best without its model explains why.
        let err = prepare(
            &dir,
            &preview,
            Model::Best,
            &dir,
            &AtomicBool::new(false),
            |_| {},
        );
        assert!(err.is_err());
    }

    fn write_pcm(path: &Path, frames: &[(i16, i16)]) {
        let bytes: Vec<u8> = frames
            .iter()
            .flat_map(|(l, r)| [l.to_le_bytes(), r.to_le_bytes()].concat())
            .collect();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn tells_dual_mono_from_stereo() {
        let dir = std::env::temp_dir().join("kreokit-voice-dual-mono");
        fs::create_dir_all(&dir).unwrap();
        let pcm = dir.join("audio.pcm");
        let voice: Vec<i16> = wav("hello-mono.wav")
            .iter()
            .map(|s| (s * 32767.0) as i16)
            .collect();
        // The same channel twice.
        write_pcm(&pcm, &voice.iter().map(|&v| (v, v)).collect::<Vec<_>>());
        assert_eq!(channel_difference_db(&pcm).unwrap(), f64::NEG_INFINITY);
        // Encoding noise on one side (±1 LSB): still dual mono.
        let jittered: Vec<_> = voice
            .iter()
            .enumerate()
            .map(|(i, &v)| (v, v.saturating_add((i % 3) as i16 - 1)))
            .collect();
        write_pcm(&pcm, &jittered);
        assert!(channel_difference_db(&pcm).unwrap() <= DUAL_MONO_DB);
        // Two different voices (the stereo fixture's idea): far above.
        let other: Vec<i16> = wav("voices.wav")
            .iter()
            .map(|s| (s * 32767.0) as i16)
            .collect();
        let stereo: Vec<_> = voice.iter().zip(&other).map(|(&l, &r)| (l, r)).collect();
        write_pcm(&pcm, &stereo);
        assert!(channel_difference_db(&pcm).unwrap() > -10.0);
        // A little width (the right channel 10% quieter): not dual mono either.
        let wide: Vec<_> = voice
            .iter()
            .map(|&v| (v, (v as f32 * 0.9) as i16))
            .collect();
        write_pcm(&pcm, &wide);
        assert!(channel_difference_db(&pcm).unwrap() > DUAL_MONO_DB);
    }

    /// Best on a 60 s dual-mono noisy file, both ways: the time saved and that the result
    /// is the same. `cargo test --release dual_mono_saves -- --ignored --nocapture`
    #[test]
    #[ignore]
    #[cfg(unix)]
    fn dual_mono_saves_time_with_best() {
        let models = crate::models::tests::models_dir_for_tests();
        let dir = std::env::temp_dir().join("kreokit-voice-dual-mono-best");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // 60 s: the voices fixture looped, with fan noise, the same on both channels.
        let speech = wav("voices.wav");
        let long: Vec<f32> = speech.iter().copied().cycle().take(48_000 * 60).collect();
        let noisy = mix(&long, &noise(Noise::Fan, long.len(), 3), 5.0);
        let frames: Vec<(i16, i16)> = noisy
            .iter()
            .map(|s| {
                let v = (s * 32767.0).clamp(-32768.0, 32767.0) as i16;
                (v, v)
            })
            .collect();
        write_pcm(&preview::audio_path(&dir), &frames);
        let meta = preview::Meta {
            version: 1,
            source_size: 0,
            source_modified_ms: 0,
            channels: 2,
            frames: frames.len() as u64,
            analysis: crate::analysis::Analysis {
                integrated_lufs: None,
                loudness_range_lu: None,
                true_peak_db: 0.0,
                sample_peak_db: 0.0,
                noise_floor_db: 0.0,
                clipped_places: 0,
            },
        };
        // Processor time of this process (user + system), in seconds.
        let cpu = || {
            let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
            // SAFETY: a valid out pointer.
            unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
            let secs = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
            secs(usage.ru_utime) + secs(usage.ru_stime)
        };
        let run = |dual: bool| {
            let part = dir.join("clean.part");
            let cpu_before = cpu();
            let started = std::time::Instant::now();
            let m = clean_preview(
                &dir,
                &part,
                &meta,
                Model::Best,
                &models,
                dual,
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap();
            let seconds = started.elapsed().as_secs_f64();
            let cpu_seconds = cpu() - cpu_before;
            let out = fs::read(audio_path(&dir, Model::Best)).unwrap();
            (m, seconds, cpu_seconds, out)
        };
        let (both, t_both, cpu_both, out_both) = run(false);
        let (one, t_one, cpu_one, out_one) = run(true);
        eprintln!(
            "processor time: both channels {cpu_both:.1} s, one channel copied {cpu_one:.1} s: {:.0}% less",
            100.0 * (1.0 - cpu_one / cpu_both)
        );
        assert!(!both.dual_mono && one.dual_mono);
        eprintln!(
            "60 s dual-mono stereo with Best: both channels {t_both:.1} s (RTF {:.3}), one channel copied {t_one:.1} s (RTF {:.3}): {:.0}% less time",
            both.realtime_factor,
            one.realtime_factor,
            100.0 * (1.0 - t_one / t_both)
        );
        // Identical channels: processing one gives the same sound, sample for sample.
        assert_eq!(out_both.len(), out_one.len());
        assert!(out_both == out_one, "the shortcut changed the result");
    }

    /// The whole path for listening: a stereo file's preview, cleaned with Fast, blended at
    /// 85% and measured through Polish (Social): the result checked independently.
    /// Needs ffmpeg: `cargo test polish_on_a_cleaned_file -- --ignored`.
    #[test]
    #[ignore]
    fn polish_on_a_cleaned_file() {
        use crate::loudness::{PolishSettings, Source, measure};
        let source = crate::probe::fixture("hello-stereo.m4a");
        let dir = std::env::temp_dir().join("kreokit-voice-polish-path");
        let _ = fs::remove_dir_all(&dir);
        let preview =
            preview::prepare(&dir, &source, 2, 5_000, &AtomicBool::new(false), |_| {}).unwrap();
        let none = AtomicBool::new(false);
        prepare(&dir, &preview, Model::Fast, Path::new(""), &none, |_| {}).unwrap();
        let blended = BlendedPreview {
            dir: dir.clone(),
            preview: preview.clone(),
            cleaned: Some(Model::Fast),
            strength: 0.85,
        };
        let settings = PolishSettings::default();
        let m = measure(&blended, &settings, &none, &mut |_| {}).unwrap();
        // Independently: the blended samples through the chain, metered.
        let mut samples = Vec::new();
        blended
            .for_each_block(&mut |b| {
                samples.extend_from_slice(b);
                Ok(())
            })
            .unwrap();
        assert_eq!(samples.len() as u64, preview.frames * 2);
        let out = crate::polish::run(&mut crate::polish::Chain::new(2, m.params), &samples);
        let mut meter =
            ebur128::EbuR128::new(2, 48_000, ebur128::Mode::I | ebur128::Mode::TRUE_PEAK).unwrap();
        meter.add_frames_f32(&out).unwrap();
        let lufs = meter.loudness_global().unwrap();
        let tp = (0..2)
            .map(|c| meter.true_peak(c).unwrap())
            .fold(0.0, f64::max);
        assert!((lufs + 14.0).abs() <= 0.5, "{lufs}");
        assert!(20.0 * tp.log10() <= -1.0, "{tp}");
    }
}
