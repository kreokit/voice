//! Playback of the preview cache through the system's audio output (cpal), decided
//! 2026-10-21: native playback reads the 16-bit cache straight from disk (memory-mapped),
//! so an hour-long recording never sits in the webview's memory, and M2's A/B switch can
//! change sources at the same sample.
//!
//! A player thread owns the output stream (cpal streams can't move between threads on
//! every platform); commands reach it through a channel. The audio callback shares
//! [`Shared`] with the commands: the current [`Pcm`], the play state and the position.
//! Seeks are handed to the callback through `pending_seek`, so the callback is the only
//! writer of `position` while it runs.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use serde::Serialize;

use crate::clean::{DEFAULT_STRENGTH, blend};
use crate::polish::{Chain, Params};
use crate::preview::SAMPLE_RATE;

/// No seek is waiting.
const NO_SEEK: u64 = u64::MAX;

/// Interleaved 16-bit samples of a preview cache.
pub struct Pcm {
    bytes: Box<dyn AsRef<[u8]> + Send + Sync>,
    channels: usize,
    frames: u64,
}

impl Pcm {
    /// Maps a cache's `audio.pcm`. The cache must not be rewritten while it is mapped:
    /// `lib.rs` unloads the player before preparing or removing a file's cache.
    pub fn open(path: &Path, channels: u16) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("cannot open the preview: {e}"))?;
        // SAFETY: the file is our own cache, never written while mapped (see above).
        let map = unsafe { memmap2::Mmap::map(&file) }
            .map_err(|e| format!("cannot read the preview: {e}"))?;
        Ok(Self::from_bytes(Box::new(map), channels))
    }

    fn from_bytes(bytes: Box<dyn AsRef<[u8]> + Send + Sync>, channels: u16) -> Self {
        let channels = channels.max(1) as usize;
        let frames = ((*bytes).as_ref().len() / (channels * 2)) as u64;
        Self {
            bytes,
            channels,
            frames,
        }
    }

    fn sample(&self, frame: u64, channel: usize) -> f32 {
        let i = (frame as usize * self.channels + channel.min(self.channels - 1)) * 2;
        let b = (*self.bytes).as_ref();
        i16::from_le_bytes([b[i], b[i + 1]]) as f32 / 32768.0
    }
}

/// Writes one source frame (mono or stereo) to an output frame: mono goes to every
/// output channel; stereo to the first two (a mono device gets their average).
fn write_frame(frame: &mut [f32], value: [f32; 2], source_channels: usize) {
    if frame.len() == 1 && source_channels == 2 {
        frame[0] = (value[0] + value[1]) * 0.5;
        return;
    }
    for (ch, out) in frame.iter_mut().enumerate() {
        *out = if source_channels == 1 {
            value[0]
        } else if ch < 2 {
            value[ch]
        } else {
            0.0
        };
    }
}

/// The original (A): fills `out` (interleaved, `out_channels` per frame) from `pcm`
/// starting at source frame `pos`, advancing `step` source frames per output frame (48 kHz
/// / device rate) with linear interpolation, at `gain` (the loudness match). Returns the
/// new position and whether the end was reached (the rest of `out` is then silence).
fn render(
    pcm: &Pcm,
    gain: f32,
    mut pos: f64,
    step: f64,
    out: &mut [f32],
    out_channels: usize,
) -> (f64, bool) {
    let last = pcm.frames.saturating_sub(1) as f64;
    for (n, frame) in out.chunks_exact_mut(out_channels).enumerate() {
        if pcm.frames == 0 || pos >= last {
            out[n * out_channels..].fill(0.0);
            return (pcm.frames as f64, true);
        }
        let i = pos as u64;
        let t = (pos - i as f64) as f32;
        let at = |ch: usize| {
            let a = pcm.sample(i, ch);
            (a + (pcm.sample(i + 1, ch) - a) * t) * gain
        };
        write_frame(frame, [at(0), at(1)], pcm.channels);
        pos += step;
    }
    (pos, false)
}

/// The processed sound (B), frame by frame: the original and the cleaned sound blended
/// at the Strength, then the Polish chain. It runs ahead of the playhead by the chain's
/// latency (the limiter's lookahead), so frame `i` comes out aligned with source frame `i`.
struct Processor {
    chain: Chain,
    channels: usize,
    /// The next source frame to feed into the chain.
    fed: u64,
    /// The source index of the newest processed frame (negative: none yet).
    newest: i64,
    /// The last two processed frames, by index parity.
    history: [[f32; 2]; 2],
}

impl Processor {
    /// Starts at source frame `start` (after a load or a seek).
    fn new(params: Params, channels: usize, start: u64) -> Self {
        let chain = Chain::new(channels, params);
        let newest = start as i64 - 1 - chain.latency() as i64;
        Self {
            chain,
            channels,
            fed: start,
            newest,
            history: [[0.0; 2]; 2],
        }
    }

    /// Processes until frame `i` is out; `None` if `i` was already passed (a seek back).
    fn ensure(&mut self, pcm: &Pcm, cleaned: Option<(&Pcm, f32)>, i: u64) -> Option<()> {
        if (i as i64) < self.newest - 1 {
            return None;
        }
        while self.newest < i as i64 {
            let k = self.fed;
            let mut frame = [0.0f32; 2];
            if k < pcm.frames {
                for (ch, v) in frame.iter_mut().enumerate().take(self.channels) {
                    let o = pcm.sample(k, ch);
                    *v = match cleaned {
                        Some((c, strength)) => blend(o, c.sample(k, ch), strength),
                        None => o,
                    };
                }
            }
            self.chain.process(&mut frame[..self.channels]);
            self.fed += 1;
            self.newest += 1;
            if self.newest >= 0 {
                self.history[(self.newest % 2) as usize] = frame;
            }
        }
        Some(())
    }

    fn frame(&self, i: u64) -> [f32; 2] {
        self.history[(i % 2) as usize]
    }
}

/// The processed sound (B) into `out`, like [`render`]: the processor is (re)started when
/// missing or when `pos` jumped back.
fn render_processed(
    pcm: &Pcm,
    cleaned: Option<(&Pcm, f32)>,
    params: Params,
    processor: &mut Option<Processor>,
    mut pos: f64,
    step: f64,
    out: &mut [f32],
    out_channels: usize,
) -> (f64, bool) {
    let last = pcm.frames.saturating_sub(1) as f64;
    for (n, frame) in out.chunks_exact_mut(out_channels).enumerate() {
        if pcm.frames == 0 || pos >= last {
            out[n * out_channels..].fill(0.0);
            return (pcm.frames as f64, true);
        }
        let i = pos as u64;
        let t = (pos - i as f64) as f32;
        let p = processor.get_or_insert_with(|| Processor::new(params, pcm.channels, i));
        if p.ensure(pcm, cleaned, i + 1).is_none() || (i as i64) < p.newest - 1 {
            *p = Processor::new(params, pcm.channels, i);
            p.ensure(pcm, cleaned, i + 1);
        }
        let (a, b) = (p.frame(i), p.frame(i + 1));
        write_frame(
            frame,
            [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
            pcm.channels,
        );
        pos += step;
    }
    (pos, false)
}

/// The callback's own state: the processor and the settings version it was made for.
#[derive(Default)]
struct Live {
    processor: Option<Processor>,
    version: u64,
}

/// State shared between the commands and the audio callback.
#[derive(Default)]
struct Shared {
    pcm: Mutex<Option<Arc<Pcm>>>,
    /// The cleaned sound of the same file (same length), when there is one.
    cleaned: Mutex<Option<Arc<Pcm>>>,
    /// Listening to the cleaned sound (B) rather than the original (A).
    listen_cleaned: AtomicBool,
    /// Strength (0–1) as f32 bits.
    strength: AtomicU32,
    /// The Polish chain's settings for the processed sound (B), and a version that changes
    /// whenever they, the cleaned sound or the playhead jump (the callback then catches up).
    polish: Mutex<Params>,
    version: AtomicU64,
    /// The original's (A) gain to match the processed sound's loudness, as f32 bits.
    match_gain: AtomicU32,
    playing: AtomicBool,
    /// Source frames (48 kHz) as f64 bits.
    position: AtomicU64,
    /// A seek for the callback to apply, in source frames, or [`NO_SEEK`].
    pending_seek: AtomicU64,
    /// The last output-stream error, shown to the user.
    error: Mutex<Option<String>>,
}

impl Shared {
    fn position(&self) -> f64 {
        match self.pending_seek.load(Ordering::SeqCst) {
            NO_SEEK => f64::from_bits(self.position.load(Ordering::SeqCst)),
            frame => frame as f64,
        }
    }

    fn set_position(&self, frames: f64) {
        self.position.store(frames.to_bits(), Ordering::SeqCst);
    }

    /// The audio callback: one buffer of output.
    fn fill(&self, out: &mut [f32], out_channels: usize, step: f64, live: &mut Live) {
        let seek = self.pending_seek.swap(NO_SEEK, Ordering::SeqCst);
        if seek != NO_SEEK {
            self.set_position(seek as f64);
            live.processor = None;
        }
        // try_lock: a load in progress is never waited for in the audio thread.
        let pcm = self.pcm.try_lock().ok().and_then(|p| p.clone());
        let Some(pcm) = pcm.filter(|_| self.playing.load(Ordering::SeqCst)) else {
            out.fill(0.0);
            return;
        };
        let pos = self.position();
        let (pos, ended) = if self.listen_cleaned.load(Ordering::SeqCst) {
            let Ok(params) = self.polish.try_lock().map(|p| *p) else {
                out.fill(0.0);
                return;
            };
            let version = self.version.load(Ordering::SeqCst);
            if version != live.version {
                live.version = version;
                match &mut live.processor {
                    // New gains: no restart (no click), unless the latency changes.
                    Some(p) if p.chain.latency() == Chain::new(1, params).latency() => {
                        p.chain.set_params(params)
                    }
                    _ => live.processor = None,
                }
            }
            let cleaned = self.cleaned.try_lock().ok().and_then(|c| c.clone());
            let strength = f32::from_bits(self.strength.load(Ordering::SeqCst));
            let mix = cleaned
                .as_deref()
                .filter(|c| c.frames == pcm.frames && c.channels == pcm.channels)
                .map(|c| (c, strength));
            render_processed(
                &pcm,
                mix,
                params,
                &mut live.processor,
                pos,
                step,
                out,
                out_channels,
            )
        } else {
            live.processor = None;
            let gain = f32::from_bits(self.match_gain.load(Ordering::SeqCst));
            render(&pcm, gain, pos, step, out, out_channels)
        };
        self.set_position(pos);
        if ended {
            self.playing.store(false, Ordering::SeqCst);
        }
    }
}

enum Command {
    Play,
    Pause,
}

/// What the interface shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlayerState {
    pub playing: bool,
    pub position_ms: f64,
    pub duration_ms: f64,
    /// A cleaned sound is loaded.
    pub has_cleaned: bool,
    /// Listening to the cleaned sound (B) rather than the original (A).
    pub listen_cleaned: bool,
    pub strength: f32,
    /// The output failed (e.g. no audio device); playback is stopped.
    pub error: Option<String>,
}

/// The app's player (managed Tauri state).
pub struct Player {
    shared: Arc<Shared>,
    /// The player thread's inbox; `None` until the first play, and again after the
    /// output failed to open (so the next play tries again).
    commands: Arc<Mutex<Option<Sender<Command>>>>,
    /// The cache folder that is mapped, which must not be removed meanwhile.
    loaded: Mutex<Option<PathBuf>>,
}

impl Default for Player {
    fn default() -> Self {
        let shared = Arc::new(Shared::default());
        shared.pending_seek.store(NO_SEEK, Ordering::SeqCst);
        shared
            .strength
            .store(DEFAULT_STRENGTH.to_bits(), Ordering::SeqCst);
        shared.match_gain.store(1.0f32.to_bits(), Ordering::SeqCst);
        *shared.polish.lock().unwrap() = Params::bypass();
        Self {
            shared,
            commands: Arc::default(),
            loaded: Mutex::new(None),
        }
    }
}

impl Player {
    /// Plays `pcm` (from the cache folder `dir`) from now on, stopped at the start.
    pub fn load(&self, pcm: Pcm, dir: PathBuf) {
        *self.loaded.lock().unwrap() = Some(dir);
        self.shared.playing.store(false, Ordering::SeqCst);
        *self.shared.cleaned.lock().unwrap() = None;
        self.shared.listen_cleaned.store(false, Ordering::SeqCst);
        *self.shared.pcm.lock().unwrap() = Some(Arc::new(pcm));
        self.shared.pending_seek.store(NO_SEEK, Ordering::SeqCst);
        self.shared.set_position(0.0);
        *self.shared.error.lock().unwrap() = None;
    }

    /// Stops and releases the mapped cache (before it is rewritten or removed).
    pub fn unload(&self) {
        self.shared.playing.store(false, Ordering::SeqCst);
        self.send(Command::Pause);
        *self.shared.pcm.lock().unwrap() = None;
        *self.shared.cleaned.lock().unwrap() = None;
        self.shared.set_position(0.0);
        *self.loaded.lock().unwrap() = None;
    }

    /// The cleaned sound of the loaded file (or `None` to release it before it is
    /// rewritten). Playback goes on at the same position.
    pub fn set_cleaned(&self, cleaned: Option<Pcm>) {
        *self.shared.cleaned.lock().unwrap() = cleaned.map(Arc::new);
        self.shared.version.fetch_add(1, Ordering::SeqCst);
    }

    /// The Polish chain for the processed sound (B); `Params::bypass()` for none.
    pub fn set_polish(&self, params: Params) {
        *self.shared.polish.lock().unwrap() = params;
        self.shared.version.fetch_add(1, Ordering::SeqCst);
    }

    /// The original's (A) gain so it plays as loud as the processed sound.
    pub fn set_match_gain_db(&self, db: f32) {
        let g = if db.is_finite() {
            10f32.powf(db.clamp(-40.0, 40.0) / 20.0)
        } else {
            1.0
        };
        self.shared.match_gain.store(g.to_bits(), Ordering::SeqCst);
    }

    /// A/B: listen to the cleaned sound (true) or the original.
    pub fn listen_cleaned(&self, cleaned: bool) {
        self.shared.listen_cleaned.store(cleaned, Ordering::SeqCst);
    }

    pub fn set_strength(&self, strength: f32) {
        let s = if strength.is_finite() {
            strength.clamp(0.0, 1.0)
        } else {
            DEFAULT_STRENGTH
        };
        self.shared.strength.store(s.to_bits(), Ordering::SeqCst);
    }

    /// The cache folder in use, if any.
    pub fn loaded(&self) -> Option<PathBuf> {
        self.loaded.lock().unwrap().clone()
    }

    pub fn play(&self) -> Result<(), String> {
        let frames = match self.shared.pcm.lock().unwrap().as_ref() {
            Some(pcm) => pcm.frames,
            None => return Err("nothing to play".into()),
        };
        // From the start again after the end.
        if self.shared.position() >= frames.saturating_sub(1) as f64 {
            self.seek_frames(0);
        }
        *self.shared.error.lock().unwrap() = None;
        self.shared.playing.store(true, Ordering::SeqCst);
        self.ensure_thread();
        self.send(Command::Play);
        Ok(())
    }

    pub fn pause(&self) {
        self.shared.playing.store(false, Ordering::SeqCst);
        self.send(Command::Pause);
    }

    pub fn seek_ms(&self, ms: f64) {
        let frames = (ms.max(0.0) * SAMPLE_RATE as f64 / 1000.0).round() as u64;
        self.seek_frames(frames);
    }

    fn seek_frames(&self, frame: u64) {
        let frames = self
            .shared
            .pcm
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, |p| p.frames);
        let frame = frame.min(frames);
        if self.shared.playing.load(Ordering::SeqCst) {
            self.shared.pending_seek.store(frame, Ordering::SeqCst);
        } else {
            // No callback runs while paused: move the position directly.
            self.shared.pending_seek.store(NO_SEEK, Ordering::SeqCst);
            self.shared.set_position(frame as f64);
        }
    }

    pub fn state(&self) -> PlayerState {
        let frames = self
            .shared
            .pcm
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, |p| p.frames);
        let to_ms = |f: f64| f * 1000.0 / SAMPLE_RATE as f64;
        PlayerState {
            playing: self.shared.playing.load(Ordering::SeqCst),
            position_ms: to_ms(self.shared.position().min(frames as f64)),
            duration_ms: to_ms(frames as f64),
            has_cleaned: self.shared.cleaned.lock().unwrap().is_some(),
            listen_cleaned: self.shared.listen_cleaned.load(Ordering::SeqCst),
            strength: f32::from_bits(self.shared.strength.load(Ordering::SeqCst)),
            error: self.shared.error.lock().unwrap().clone(),
        }
    }

    fn send(&self, command: Command) {
        if let Some(sender) = self.commands.lock().unwrap().as_ref() {
            let _ = sender.send(command);
        }
    }

    /// Starts the player thread (and opens the output) on first play.
    fn ensure_thread(&self) {
        let mut commands = self.commands.lock().unwrap();
        if commands.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let shared = self.shared.clone();
        let inbox = self.commands.clone();
        std::thread::Builder::new()
            .name("kreokit-player".into())
            .spawn(move || run(shared, receiver, inbox))
            .expect("cannot start the player thread");
        *commands = Some(sender);
    }
}

/// The player thread: opens the default output and plays or pauses it on command.
fn run(
    shared: Arc<Shared>,
    commands: Receiver<Command>,
    inbox: Arc<Mutex<Option<Sender<Command>>>>,
) {
    let stream = match open_stream(shared.clone()) {
        Ok(stream) => stream,
        Err(e) => {
            shared.playing.store(false, Ordering::SeqCst);
            *shared.error.lock().unwrap() = Some(e);
            // The next play starts a new thread and tries to open the output again.
            *inbox.lock().unwrap() = None;
            return;
        }
    };
    for command in commands {
        let result = match command {
            Command::Play => stream.play(),
            Command::Pause => stream.pause(),
        };
        if let Err(e) = result {
            shared.playing.store(false, Ordering::SeqCst);
            *shared.error.lock().unwrap() = Some(format!("audio output: {e}"));
        }
    }
}

fn open_stream(shared: Arc<Shared>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("No audio output found. Connect speakers or headphones and try again.")?;
    let config = device
        .default_output_config()
        .map_err(|e| format!("audio output: {e}"))?;
    let format = config.sample_format();
    let config: cpal::StreamConfig = config.into();
    match format {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config, shared),
        cpal::SampleFormat::I16 => build::<i16>(&device, &config, shared),
        cpal::SampleFormat::U16 => build::<u16>(&device, &config, shared),
        cpal::SampleFormat::I32 => build::<i32>(&device, &config, shared),
        other => Err(format!("audio output: unsupported sample format {other}")),
    }
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String> {
    let channels = config.channels as usize;
    let step = SAMPLE_RATE as f64 / config.sample_rate as f64;
    let errors = shared.clone();
    let mut buffer: Vec<f32> = Vec::new();
    let mut live = Live::default();
    let stream = device
        .build_output_stream(
            config.clone(),
            move |out: &mut [T], _| {
                buffer.resize(out.len(), 0.0);
                shared.fill(&mut buffer, channels, step, &mut live);
                for (o, s) in out.iter_mut().zip(&buffer) {
                    *o = T::from_sample(*s);
                }
            },
            move |e| {
                errors.playing.store(false, Ordering::SeqCst);
                *errors.error.lock().unwrap() = Some(format!("audio output: {e}"));
            },
            None,
        )
        .map_err(|e| format!("audio output: {e}"))?;
    stream.pause().map_err(|e| format!("audio output: {e}"))?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(samples: &[i16], channels: u16) -> Pcm {
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        Pcm::from_bytes(Box::new(bytes), channels)
    }

    #[test]
    fn plays_mono_on_both_speakers() {
        let p = pcm(&[0, 16_384, -16_384, 0], 1);
        let mut out = [9.0; 4];
        let (pos, ended) = render(&p, 1.0, 0.0, 1.0, &mut out, 2);
        assert_eq!(out, [0.0, 0.0, 0.5, 0.5]);
        assert_eq!((pos, ended), (2.0, false));
    }

    #[test]
    fn stereo_to_stereo_surround_and_mono_devices() {
        let p = pcm(&[16_384, -16_384, 16_384, -16_384, 0, 0], 2);
        let mut stereo = [9.0; 2];
        render(&p, 1.0, 0.0, 1.0, &mut stereo, 2);
        assert_eq!(stereo, [0.5, -0.5]);
        let mut surround = [9.0; 6];
        render(&p, 1.0, 0.0, 1.0, &mut surround, 6);
        assert_eq!(surround, [0.5, -0.5, 0.0, 0.0, 0.0, 0.0]);
        let mut mono = [9.0; 1];
        render(&p, 1.0, 0.0, 1.0, &mut mono, 1);
        assert_eq!(mono, [0.0]);
    }

    #[test]
    fn interpolates_for_a_44_1_khz_device() {
        // A ramp: interpolated values stay on it.
        let ramp: Vec<i16> = (0..100).map(|i| i * 100).collect();
        let p = pcm(&ramp, 1);
        let step = 48_000.0 / 44_100.0;
        let mut out = [0.0f32; 50];
        let (pos, _) = render(&p, 1.0, 10.0, step, &mut out, 1);
        for (n, v) in out.iter().enumerate() {
            let expected = (10.0 + n as f64 * step) * 100.0 / 32768.0;
            assert!(
                (*v as f64 - expected).abs() < 1e-6,
                "{n}: {v} vs {expected}"
            );
        }
        assert!((pos - (10.0 + 50.0 * step)).abs() < 1e-9);
    }

    #[test]
    fn ends_with_silence() {
        let p = pcm(&[16_384, 16_384, 16_384], 1);
        let mut out = [9.0f32; 6];
        let (pos, ended) = render(&p, 1.0, 1.0, 1.0, &mut out, 1);
        assert!(ended);
        assert_eq!(pos, 3.0);
        assert_eq!(out, [0.5, 0.0, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn a_b_and_strength() {
        let original = pcm(&[16_384, 16_384, 16_384, 16_384], 1);
        let cleaned = pcm(&[0, 0, 0, 0], 1);
        let mut out = [9.0f32; 2];
        let bypass = Params::bypass();
        let mut p = None;
        render_processed(
            &original,
            Some((&cleaned, 1.0)),
            bypass,
            &mut p,
            0.0,
            1.0,
            &mut out,
            1,
        );
        assert_eq!(out, [0.0, 0.0], "fully cleaned");
        let mut p = None;
        render_processed(
            &original,
            Some((&cleaned, 0.75)),
            bypass,
            &mut p,
            0.0,
            1.0,
            &mut out,
            1,
        );
        assert_eq!(out, [0.125, 0.125], "a quarter of the original left");
        // The original at a matching gain (+6 dB).
        render(&original, 2.0, 0.0, 1.0, &mut out, 1);
        assert_eq!(out, [1.0, 1.0]);

        // Through the player: A/B switches at the same position.
        let player = Player::default();
        player.load(original_pcm(), PathBuf::from("/cache/a"));
        assert!(!player.state().has_cleaned);
        player.set_cleaned(Some(pcm(&vec![0; 48_000], 1)));
        player.listen_cleaned(true);
        player.set_strength(0.5);
        player.seek_ms(500.0);
        let state = player.state();
        assert!(state.has_cleaned && state.listen_cleaned);
        assert_eq!((state.strength, state.position_ms), (0.5, 500.0));
        player.set_strength(f32::NAN);
        assert_eq!(player.state().strength, DEFAULT_STRENGTH);
        // A new file starts on the original, without a cleaned sound.
        player.load(original_pcm(), PathBuf::from("/cache/b"));
        assert!(!player.state().has_cleaned && !player.state().listen_cleaned);
    }

    /// Live playback of the processed sound, block by block, equals the whole chain run
    /// offline (`polish::run`), sample for sample, despite the limiter's lookahead.
    #[test]
    fn live_polish_lines_up_with_the_offline_chain() {
        let samples: Vec<i16> = (0..20_000)
            .map(|i| ((i as f32 * 0.07).sin() * 30_000.0 * ((i / 3_000) % 2) as f32) as i16)
            .collect();
        let original = pcm(&samples, 1);
        let params = Params {
            stages: crate::polish::Stages::default(),
            pre_gain_db: 2.0,
            gain_db: 6.0,
            limit: true,
        };
        let input: Vec<f32> = samples.iter().map(|&s| s as f32 / 32768.0).collect();
        let offline = crate::polish::run(&mut Chain::new(1, params), &input);
        let mut processor = None;
        let mut live = Vec::new();
        let mut pos = 0.0;
        while live.len() < 15_000 {
            let mut block = [0.0f32; 512];
            pos = render_processed(
                &original,
                None,
                params,
                &mut processor,
                pos,
                1.0,
                &mut block,
                1,
            )
            .0;
            live.extend_from_slice(&block);
        }
        let worst = live[..15_000]
            .iter()
            .zip(&offline)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-6, "{worst}");
    }

    #[test]
    fn a_seek_restarts_the_chain_where_it_lands() {
        let original = pcm(&vec![8_000; 48_000], 1);
        let params = Params {
            limit: true,
            ..Params::bypass()
        };
        let mut processor = None;
        let mut out = [0.0f32; 64];
        render_processed(
            &original,
            None,
            params,
            &mut processor,
            30_000.0,
            1.0,
            &mut out,
            1,
        );
        assert!(out.iter().all(|&v| (v - 8_000.0 / 32768.0).abs() < 1e-6));
        // Back to the start: restarted, still the right samples.
        render_processed(
            &original,
            None,
            params,
            &mut processor,
            0.0,
            1.0,
            &mut out,
            1,
        );
        assert!(out.iter().all(|&v| (v - 8_000.0 / 32768.0).abs() < 1e-6));
    }

    fn original_pcm() -> Pcm {
        pcm(&vec![1_000; 48_000], 1)
    }

    #[test]
    fn seeks_while_paused_and_reports_state() {
        let player = Player::default();
        assert!(player.play().is_err(), "nothing loaded");
        player.load(pcm(&vec![0; 96_000], 1), PathBuf::from("/cache/a"));
        assert_eq!(player.loaded(), Some(PathBuf::from("/cache/a")));
        player.seek_ms(1_500.0);
        let state = player.state();
        assert_eq!(state.position_ms, 1_500.0);
        assert_eq!(state.duration_ms, 2_000.0);
        assert!(!state.playing);
        // Past the end clamps to the end.
        player.seek_ms(9_999.0);
        assert_eq!(player.state().position_ms, 2_000.0);
        player.unload();
        assert_eq!(player.state().duration_ms, 0.0);
        assert_eq!(player.loaded(), None);
    }

    #[test]
    fn the_callback_applies_seeks_and_stops_at_the_end() {
        let shared = Shared::default();
        shared.pending_seek.store(NO_SEEK, Ordering::SeqCst);
        shared.strength.store(1.0f32.to_bits(), Ordering::SeqCst);
        shared.match_gain.store(1.0f32.to_bits(), Ordering::SeqCst);
        *shared.polish.lock().unwrap() = Params::bypass();
        let mut live = Live::default();
        *shared.pcm.lock().unwrap() = Some(Arc::new(pcm(&[0, 1_000, 2_000, 3_000, 4_000], 1)));
        let mut out = [0.0f32; 2];
        // Paused: silence, position unchanged.
        shared.fill(&mut out, 1, 1.0, &mut live);
        assert_eq!((out, shared.position()), ([0.0, 0.0], 0.0));
        shared.playing.store(true, Ordering::SeqCst);
        shared.pending_seek.store(2, Ordering::SeqCst);
        shared.fill(&mut out, 1, 1.0, &mut live);
        assert_eq!(out, [2_000.0 / 32768.0, 3_000.0 / 32768.0]);
        shared.fill(&mut out, 1, 1.0, &mut live);
        assert!(!shared.playing.load(Ordering::SeqCst), "stopped at the end");
    }

    /// Plays the mono fixture's cache through the real default output for half a second.
    /// Needs a sound device, so it runs only with `KREOKIT_AUDIO_TEST=1` (CI has none):
    /// `KREOKIT_AUDIO_TEST=1 cargo test plays_through -- --ignored`.
    #[test]
    #[ignore]
    fn plays_through_the_default_output() {
        if std::env::var("KREOKIT_AUDIO_TEST").as_deref() != Ok("1") {
            return;
        }
        let source = crate::probe::fixture("hello-mono.wav");
        let dir = std::env::temp_dir().join("kreokit-voice-player-test");
        let meta =
            crate::preview::prepare(&dir, &source, 1, 5_000, &AtomicBool::new(false), |_| {})
                .unwrap();
        let player = Player::default();
        let pcm = Pcm::open(&crate::preview::audio_path(&dir), meta.channels).unwrap();
        player.load(pcm, dir);
        player.seek_ms(1_000.0);
        player.play().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let state = player.state();
        player.pause();
        assert_eq!(state.error, None);
        assert!(state.playing, "{state:?}");
        // About 1.5 s, give or take the output's buffering.
        assert!((1_300.0..1_700.0).contains(&state.position_ms), "{state:?}");
    }
}
