//! The preview cache: a file's sound decoded once to 48 kHz 16-bit PCM, for the waveform
//! and for playback (decided 2026-10-21: 16-bit keeps an hour of stereo at ~660 MB).
//!
//! `<cache dir>/preview/<id>/` holds `audio.pcm` (interleaved little-endian i16, mono or
//! stereo), `peaks.bin` (one level per 10 ms, see [`level`]) and `meta.json`, written last,
//! so a cache without it is unfinished and decoded again. Sample 0 is the file's 0:00:
//! audio that starts after the picture is padded with silence (the rule from KreoKit
//! Clips), so the sound stays in sync with the video. Files with more than two channels
//! are mixed down to stereo.
//!
//! This is only for listening and drawing; cleaning (M2) decodes the original again in
//! full 32-bit float.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::{disk, ffmpeg};

pub const SAMPLE_RATE: u32 = 48_000;
/// Waveform levels per second (one per 10 ms).
pub const PEAKS_PER_SECOND: u32 = 100;
/// Levels are drawn on a decibel scale from this floor up to 0 dBFS.
const FLOOR_DB: f32 = -48.0;
/// Bumped when the cache format changes; older caches are decoded again.
const VERSION: u32 = 1;
/// Free space to leave on the disk after the cache is written.
pub const DISK_MARGIN: u64 = 1 << 30;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub version: u32,
    /// The source file when it was decoded; any change decodes it again.
    pub source_size: u64,
    pub source_modified_ms: u64,
    pub channels: u16,
    /// Samples per channel.
    pub frames: u64,
}

/// A file's cache folder: a stable hash of its path (FNV-1a, 16 hex digits).
pub fn dir_for(root: &Path, source: &Path) -> PathBuf {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in source.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    root.join(format!("{hash:016x}"))
}

pub fn audio_path(dir: &Path) -> PathBuf {
    dir.join("audio.pcm")
}

fn peaks_path(dir: &Path) -> PathBuf {
    dir.join("peaks.bin")
}

fn meta_path(dir: &Path) -> PathBuf {
    dir.join("meta.json")
}

/// The source's size and modification time, which a cache must match.
fn fingerprint(source: &Path) -> Result<(u64, u64), String> {
    let meta =
        fs::metadata(source).map_err(|e| format!("cannot read {}: {e}", source.display()))?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64);
    Ok((meta.len(), modified))
}

/// The finished cache for `source`, if it is current.
pub fn cached(dir: &Path, source: &Path) -> Option<Meta> {
    let meta: Meta = serde_json::from_str(&fs::read_to_string(meta_path(dir)).ok()?).ok()?;
    let (size, modified) = fingerprint(source).ok()?;
    let complete = audio_path(dir)
        .metadata()
        .is_ok_and(|m| m.len() == meta.frames * meta.channels as u64 * 2);
    (meta.version == VERSION
        && meta.source_size == size
        && meta.source_modified_ms == modified
        && complete)
        .then_some(meta)
}

/// Mono stays mono; everything else becomes stereo.
pub fn output_channels(source_channels: u32) -> u16 {
    if source_channels == 1 { 1 } else { 2 }
}

/// Roughly how many bytes the cache takes for `duration_ms` of sound.
pub fn estimated_bytes(duration_ms: u64, channels: u16) -> u64 {
    let frames = duration_ms * SAMPLE_RATE as u64 / 1000;
    frames * channels as u64 * 2 + frames / (SAMPLE_RATE / PEAKS_PER_SECOND) as u64
}

/// The ffmpeg arguments that decode `source` to the cache's PCM on stdout.
fn decode_args(source: &Path, channels: u16) -> Vec<String> {
    let mut args: Vec<String> = ["-nostdin", "-v", "error", "-i"].map(String::from).to_vec();
    args.push(source.to_string_lossy().into_owned());
    args.extend(
        [
            "-map",
            "0:a:0",
            "-vn",
            "-sn",
            "-dn",
            "-ac",
            &channels.to_string(),
            "-ar",
            &SAMPLE_RATE.to_string(),
            // Silence before audio that starts after the picture, so sample 0 is the file's
            // 0:00; samples before 0:00 (encoder delay) are dropped.
            "-af",
            "aresample=async=1:first_pts=0",
            "-f",
            "s16le",
            "-acodec",
            "pcm_s16le",
            "pipe:1",
        ]
        .map(String::from),
    );
    args
}

/// Decodes `source` into `dir` unless a current cache is there. `progress` gets the
/// fraction done (0–1) about every 100 ms of work; `cancel` stops it (the unfinished
/// folder is removed).
pub fn prepare(
    dir: &Path,
    source: &Path,
    source_channels: u32,
    duration_ms: u64,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f64),
) -> Result<Meta, String> {
    if let Some(meta) = cached(dir, source) {
        return Ok(meta);
    }
    let channels = output_channels(source_channels);
    // Each job decodes into its own folder and moves it into place when finished, so a
    // cancelled job never touches another one's files (opening the same file twice in a
    // row, e.g. React's StrictMode in dev, cancels the first while the second runs).
    let part = part_dir(dir);
    let _ = fs::remove_dir_all(&part);
    fs::create_dir_all(&part).map_err(|e| format!("cannot create the preview folder: {e}"))?;
    let result = disk::ensure_free(&part, estimated_bytes(duration_ms, channels), DISK_MARGIN)
        .and_then(|()| decode(&part, source, channels, duration_ms, cancel, &mut progress))
        .and_then(|meta| {
            let _ = fs::remove_dir_all(dir);
            fs::rename(&part, dir)
                .map_err(|e| format!("cannot finish the preview: {e}"))
                .map(|()| meta)
        });
    if result.is_err() {
        let _ = fs::remove_dir_all(&part);
    }
    result
}

/// A job's own working folder next to `dir`: `<id>.part-<n>`.
fn part_dir(dir: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let name = dir.file_name().unwrap_or_default().to_string_lossy();
    dir.with_file_name(format!("{name}.part-{n}"))
}

fn decode(
    dir: &Path,
    source: &Path,
    channels: u16,
    duration_ms: u64,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(f64),
) -> Result<Meta, String> {
    let (source_size, source_modified_ms) = fingerprint(source)?;
    let ffmpeg = ffmpeg::locate("ffmpeg")?;
    let mut child = Command::new(&ffmpeg)
        .args(decode_args(source, channels))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run {}: {e}", ffmpeg.display()))?;
    // Read stderr on the side so a chatty ffmpeg can never block on a full pipe.
    let mut stderr = child.stderr.take().unwrap();
    let stderr = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let mut stdout = BufReader::with_capacity(1 << 20, child.stdout.take().unwrap());

    let mut audio = BufWriter::with_capacity(
        1 << 20,
        File::create(audio_path(dir)).map_err(|e| e.to_string())?,
    );
    let mut levels = Levels::new(channels);
    let expected_frames = (duration_ms * SAMPLE_RATE as u64 / 1000).max(1);
    let mut frames = 0u64;
    let mut buffer = vec![0u8; 1 << 16];
    let mut pending = 0usize;
    let mut reported = 0u64;
    let frame_bytes = channels as usize * 2;
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("cancelled".into());
        }
        let n = match stdout.read(&mut buffer[pending..]) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("cannot read the sound: {e}")),
        };
        let filled = pending + n;
        let whole = filled / frame_bytes * frame_bytes;
        audio
            .write_all(&buffer[..whole])
            .map_err(|e| format!("cannot write the preview: {e}"))?;
        levels.push(&buffer[..whole]);
        frames += (whole / frame_bytes) as u64;
        buffer.copy_within(whole..filled, 0);
        pending = filled - whole;
        if frames - reported >= SAMPLE_RATE as u64 * 10 {
            reported = frames;
            progress((frames as f64 / expected_frames as f64).min(1.0));
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let stderr = stderr.join().unwrap_or_default();
    if !status.success() {
        return Err(format!(
            "ffmpeg could not read the sound: {}",
            stderr.trim()
        ));
    }
    if frames == 0 {
        return Err("this file's sound is empty".into());
    }
    audio
        .flush()
        .map_err(|e| format!("cannot write the preview: {e}"))?;
    fs::write(peaks_path(dir), levels.finish()).map_err(|e| e.to_string())?;
    let meta = Meta {
        version: VERSION,
        source_size,
        source_modified_ms,
        channels,
        frames,
    };
    let json = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    fs::write(meta_path(dir), json).map_err(|e| e.to_string())?;
    progress(1.0);
    Ok(meta)
}

/// The waveform of a finished cache: one level per 10 ms (an hour is 360 KB).
pub fn peaks(dir: &Path) -> Result<Vec<u8>, String> {
    fs::read(peaks_path(dir)).map_err(|e| format!("cannot read the waveform: {e}"))
}

/// A peak sample value (0–1) as one byte on a dB scale: [`FLOOR_DB`] and quieter is 0,
/// full scale is 255. A dB scale keeps quiet speech visible next to loud parts.
pub fn level(peak: f32) -> u8 {
    let db = 20.0 * peak.max(1e-6).log10();
    ((db - FLOOR_DB) / -FLOOR_DB)
        .clamp(0.0, 1.0)
        .mul_add(255.0, 0.5) as u8
}

/// Waveform levels from interleaved i16 bytes: the loudest sample of any channel per
/// 10 ms.
pub(crate) struct Levels {
    channels: usize,
    per_level: usize,
    count: usize,
    peak: i32,
    out: Vec<u8>,
}

impl Levels {
    pub(crate) fn new(channels: u16) -> Self {
        Self {
            channels: channels as usize,
            per_level: (SAMPLE_RATE / PEAKS_PER_SECOND) as usize,
            count: 0,
            peak: 0,
            out: Vec::new(),
        }
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        for frame in bytes.chunks_exact(self.channels * 2) {
            for sample in frame.chunks_exact(2) {
                let v = i16::from_le_bytes([sample[0], sample[1]]) as i32;
                self.peak = self.peak.max(v.abs());
            }
            self.count += 1;
            if self.count == self.per_level {
                self.emit();
            }
        }
    }

    fn emit(&mut self) {
        self.out.push(level(self.peak as f32 / 32768.0));
        self.count = 0;
        self.peak = 0;
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        if self.count > 0 {
            self.emit();
        }
        self.out
    }
}

/// Removes cache folders under `root` that aren't in `keep` (files that left the
/// recent list), so the cache never grows past what Home shows.
pub fn prune(root: &Path, keep: &[PathBuf]) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // A job's working folder is left alone while it may still be running (an hour),
        // and removed later if a crash left it behind.
        let working = path.to_string_lossy().contains(".part-")
            && entry
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|t| t.elapsed().is_ok_and(|age| age.as_secs() < 3600));
        if path.is_dir() && !keep.contains(&path) && !working {
            let _ = fs::remove_dir_all(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{self, fixture};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("kreokit-voice-preview-tests")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn le_bytes(samples: &[i16]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    #[test]
    fn levels_on_a_db_scale() {
        assert_eq!(level(0.0), 0);
        assert_eq!(level(1.0), 255);
        assert!(level(0.0039) <= 1, "about -48 dB");
        assert!(
            (126..=130).contains(&level(0.0631)),
            "about -24 dB: halfway"
        );
    }

    #[test]
    fn one_level_per_10_ms_from_the_loudest_channel() {
        let mut levels = Levels::new(2);
        // 10 ms of stereo: silent left, full-scale right.
        let mut frames = Vec::new();
        for _ in 0..480 {
            frames.extend([0, i16::MIN]);
        }
        // Then 10 ms of silence and a short last bucket at half scale.
        frames.extend(vec![0; 480 * 2]);
        frames.extend(vec![16_384; 100 * 2]);
        // Pushed in odd-sized pieces, as the pipe delivers them.
        let bytes = le_bytes(&frames);
        for piece in bytes.chunks(4 * 333) {
            levels.push(piece);
        }
        let out = levels.finish();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], 255);
        assert_eq!(out[1], 0);
        assert!((222..=224).contains(&out[2]), "-6 dB: {}", out[2]);
    }

    #[test]
    fn channels_and_size() {
        assert_eq!(output_channels(1), 1);
        assert_eq!(output_channels(2), 2);
        assert_eq!(output_channels(6), 2);
        // An hour of stereo: 48000 × 3600 × 2 channels × 2 bytes, plus 360 KB of levels.
        assert_eq!(estimated_bytes(3_600_000, 2), 691_200_000 + 360_000);
    }

    #[test]
    fn stable_folder_per_path() {
        let root = Path::new("/cache");
        let a = dir_for(root, Path::new("/Users/me/podcast.wav"));
        assert_eq!(a, dir_for(root, Path::new("/Users/me/podcast.wav")));
        assert_ne!(a, dir_for(root, Path::new("/Users/me/podcast2.wav")));
        assert_eq!(a.file_name().unwrap().len(), 16);
    }

    #[test]
    fn prunes_folders_not_kept() {
        let root = temp_dir("prune");
        let keep = root.join("aaaa");
        let gone = root.join("bbbb");
        fs::create_dir_all(&keep).unwrap();
        fs::create_dir_all(&gone).unwrap();
        prune(&root, std::slice::from_ref(&keep));
        assert!(keep.is_dir());
        assert!(!gone.exists());
    }

    #[test]
    fn the_decode_command_is_lgpl() {
        let args = decode_args(Path::new("/in.mp4"), 2);
        assert_eq!(ffmpeg::gpl_only(&args), None);
        assert!(args.windows(2).any(|w| w == ["-ac", "2"]));
    }

    fn prepare_fixture(name: &str, test: &str) -> (PathBuf, probe::MediaInfo, Meta) {
        let source = fixture(name);
        let info = probe::probe(&source).unwrap();
        let dir = temp_dir(test);
        let mut reports = Vec::new();
        let meta = prepare(
            &dir,
            &source,
            info.audio.channels,
            info.duration_ms,
            &AtomicBool::new(false),
            |f| reports.push(f),
        )
        .unwrap();
        assert_eq!(reports.last(), Some(&1.0));
        (dir, info, meta)
    }

    fn ms(frames: u64) -> u64 {
        frames * 1000 / SAMPLE_RATE as u64
    }

    fn read_pcm(dir: &Path) -> Vec<i16> {
        let bytes = fs::read(audio_path(dir)).unwrap();
        bytes
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect()
    }

    /// Needs ffmpeg: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn decodes_mono_and_reuses_the_cache() {
        let (dir, info, meta) = prepare_fixture("hello-mono.wav", "mono");
        assert_eq!(meta.channels, 1);
        assert!(ms(meta.frames).abs_diff(info.duration_ms) <= 5, "{meta:?}");
        let pcm = read_pcm(&dir);
        assert_eq!(pcm.len() as u64, meta.frames);
        let peaks = peaks(&dir).unwrap();
        assert_eq!(peaks.len() as u64, meta.frames.div_ceil(480));
        assert!(peaks.iter().any(|&l| l > 150), "speech is there");

        // A second prepare finds the cache without decoding (the cancel flag is set, so a
        // decode would fail).
        let again = prepare(
            &dir,
            &fixture("hello-mono.wav"),
            1,
            info.duration_ms,
            &AtomicBool::new(true),
            |_| {},
        );
        assert_eq!(again.unwrap(), meta);
    }

    /// Needs ffmpeg: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn keeps_both_stereo_channels() {
        let (dir, _, meta) = prepare_fixture("hello-stereo.m4a", "stereo");
        assert_eq!(meta.channels, 2);
        let pcm = read_pcm(&dir);
        // Two different voices: the channels differ.
        let differing = pcm.chunks_exact(2).filter(|f| f[0] != f[1]).count();
        assert!(differing > pcm.len() / 4, "{differing}");
    }

    /// Needs ffmpeg: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn pads_audio_that_starts_after_the_picture() {
        let (dir, info, meta) = prepare_fixture("hello-late.mp4", "late");
        // Sample 0 is the video's 0:00: the first ~0.48 s are silence, then speech. The
        // sound ends where it ends (4.87 s), a little before the 5 s picture.
        let length = ms(meta.frames);
        assert!(
            (4_800..=info.duration_ms).contains(&length),
            "{meta:?} {info:?}"
        );
        let pcm = read_pcm(&dir);
        let first_sound = pcm.iter().position(|s| s.abs() > 300).unwrap();
        let start_ms = first_sound as u64 * 1000 / 48_000;
        assert!(start_ms >= info.audio_start_ms, "{start_ms} ms");
        assert!(start_ms < info.audio_start_ms + 400, "{start_ms} ms");
    }

    /// Needs ffmpeg: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn mixes_surround_down_to_stereo() {
        let dir = temp_dir("surround");
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("surround.wav");
        let status = Command::new(ffmpeg::locate("ffmpeg").unwrap())
            .args(["-y", "-v", "error", "-f", "lavfi"])
            .args(["-i", "sine=frequency=440:duration=1:sample_rate=44100"])
            .args(["-af", "pan=5.1|c0=c0|c1=c0|c2=c0|c3=c0|c4=c0|c5=c0"])
            .arg(&source)
            .status()
            .unwrap();
        assert!(status.success());
        let info = probe::probe(&source).unwrap();
        assert_eq!(info.audio.channels, 6);
        let cache = dir.join("cache");
        let meta = prepare(
            &cache,
            &source,
            6,
            info.duration_ms,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!((meta.channels, meta.frames), (2, 48_000));
    }

    #[test]
    fn prune_keeps_a_running_job_folder() {
        let root = temp_dir("prune-part");
        let part = root.join("aaaa.part-3");
        fs::create_dir_all(&part).unwrap();
        prune(&root, &[]);
        assert!(part.is_dir());
    }

    /// Needs ffmpeg: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn a_cancelled_job_leaves_another_jobs_files_alone() {
        // Another job's finished files in the folder (here: any file).
        let dir = temp_dir("two-jobs");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("audio.pcm"), b"other job").unwrap();
        let err = prepare(
            &dir,
            &fixture("hello-mono.wav"),
            1,
            5_000,
            &AtomicBool::new(true),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(err, "cancelled");
        assert_eq!(fs::read(dir.join("audio.pcm")).unwrap(), b"other job");
        // And its own working folder is gone.
        let leftovers = fs::read_dir(dir.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("two-jobs.part-")
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    /// Needs ffmpeg: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn cancelling_removes_the_unfinished_cache() {
        let dir = temp_dir("cancel");
        let err = prepare(
            &dir,
            &fixture("hello-mono.wav"),
            1,
            5_000,
            &AtomicBool::new(true),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(err, "cancelled");
        assert!(!dir.exists());
    }
}
