//! Export (milestone 5): one file, exactly as the editor plays it, into a checked file.
//!
//! 1. The original is decoded in f32 (the preview's channel rule and late-audio padding),
//!    cleaned with the file's model at its Strength when it was cleaned, and written to a
//!    temporary f32 file in the cache (the exact loudness measurement reads it several
//!    times; Best never runs twice).
//! 2. `loudness::measure` (the exact one, never the ≈ estimate) on that file.
//! 3. The Polish chain over it, piped into FFmpeg, which writes a temporary file next to
//!    the destination ([`temp_name`]); a video's picture and other streams are copied.
//! 4. The written file is checked: loudness within ±0.5 LU of the target, true peak at most
//!    −1 dBTP (a lossy encoder's overshoot → once more with a lower limiter ceiling),
//!    duration; a video's picture bit-identical to the original's. Then it is renamed into
//!    place. Temporary files are removed on every error, cancel and stop.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::clean::{self, Cleaner, Engine, Model, blend};
use crate::loudness::{self, Measured, PolishSettings, Source, Target};
use crate::polish::{CEILING_DBTP, Chain};
use crate::preview::{self, SAMPLE_RATE};
use crate::probe::MediaInfo;
use crate::{disk, ffmpeg};

pub const CANCELLED: &str = "cancelled";
/// What "nothing would change" exports say (the user's rule, 2026-10-22).
pub const NOTHING_TO_DO: &str = "nothing would change";
/// Every temporary file Voice writes ends with this, so leftovers of a crash can be
/// recognised and removed (and nothing else).
pub const TEMP_SUFFIX: &str = ".kreokit-voice-tmp";
const UNREADABLE: &str = "The sound of this file couldn't be read. Check that it still plays in other apps; if it was moved or renamed, open it again from its new place.";
const UNREADABLE_OUTPUT: &str = "The written file couldn't be read back for the check, so it wasn't saved. Try again, or choose another folder.";
/// The promise of the loudness presets.
pub const LOUDNESS_TOLERANCE_LU: f64 = 0.5;
pub const MAX_TRUE_PEAK_DBTP: f64 = -1.0;

/// The output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// 24-bit PCM.
    Wav,
    /// 24-bit FLAC.
    Flac,
    /// AAC 256 kbps in M4A.
    M4a,
    /// MP3 320 kbps (LAME).
    Mp3,
    /// A video in its own container: the picture copied, the sound replaced.
    Video,
}

impl Format {
    /// The default for a file (the user's rule): its own kind, anything else WAV.
    pub fn default_for(path: &Path, info: &MediaInfo) -> Self {
        if info.video.is_some() {
            return Self::Video;
        }
        match extension(path).as_str() {
            "mp3" => Self::Mp3,
            "m4a" | "aac" => Self::M4a,
            "flac" => Self::Flac,
            _ => Self::Wav,
        }
    }

    /// The file extension (videos keep theirs).
    pub fn extension(self, source: &Path) -> String {
        match self {
            Self::Wav => "wav".into(),
            Self::Flac => "flac".into(),
            Self::M4a => "m4a".into(),
            Self::Mp3 => "mp3".into(),
            Self::Video => extension(source),
        }
    }

    /// Bytes per second of sound, for the size estimate (video: the audio part only).
    pub fn bytes_per_second(self, channels: u16) -> u64 {
        match self {
            Self::Wav => 48_000 * 3 * channels as u64,
            Self::Flac => 48_000 * 3 * channels as u64 * 6 / 10,
            Self::M4a | Self::Video => 32_000,
            Self::Mp3 => 40_000,
        }
    }
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// What to export and how (the file's settings as the editor plays them).
#[derive(Debug, Clone)]
pub struct Job {
    pub source: PathBuf,
    pub info: MediaInfo,
    /// The model the file was cleaned with, `None` when it wasn't (Polish only).
    pub cleaned: Option<Model>,
    pub strength: f32,
    pub polish: PolishSettings,
    pub format: Format,
    pub destination: PathBuf,
    /// Replace an existing destination (the user chose Replace).
    pub replace: bool,
    /// The file's preview folder, when it has one (for the dual-mono test, as when
    /// listening; files exported in a batch without being opened have none).
    pub preview_dir: Option<PathBuf>,
}

/// True when the export would only copy the sound (the user's rule: say so instead).
pub fn nothing_to_do(cleaned: Option<Model>, polish: &PolishSettings) -> bool {
    let s = polish.stages;
    cleaned.is_none() && !s.rumble && !s.de_ess && !s.compress && polish.target == Target::Off
}

/// The stages a progress report belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    Cleaning,
    Measuring,
    Writing,
    Checking,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Progress {
    pub stage: Stage,
    /// 0–1 over the whole file's export.
    pub fraction: f64,
}

/// Where each stage's share of the work lies (cleaning dominates with Best).
fn overall(stage: Stage, f: f64, cleaned: bool) -> f64 {
    let (start, share) = match (stage, cleaned) {
        (Stage::Cleaning, true) => (0.0, 0.55),
        (Stage::Cleaning, false) => (0.0, 0.10),
        (Stage::Measuring, true) => (0.55, 0.25),
        (Stage::Measuring, false) => (0.10, 0.55),
        (Stage::Writing, true) => (0.80, 0.12),
        (Stage::Writing, false) => (0.65, 0.20),
        (Stage::Checking, true) => (0.92, 0.08),
        (Stage::Checking, false) => (0.85, 0.15),
    };
    (start + share * f.clamp(0.0, 1.0)).min(1.0)
}

/// What the written file measured.
#[derive(Debug, Clone, Serialize)]
pub struct Done {
    pub path: PathBuf,
    pub lufs: Option<f64>,
    pub true_peak_db: f64,
    pub duration_ms: u64,
    pub bytes: u64,
    /// Seconds of work per second of sound for the whole export (for "time left").
    pub realtime_factor: f64,
}

/// A temporary name next to `path` (hidden on macOS and Linux) that only Voice writes.
pub fn temp_name(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}.{}-{n}{TEMP_SUFFIX}", std::process::id()))
}

/// Removes a file when dropped (temporary files on every way out).
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// FFmpeg is part of the app: when it can't start, the installation is broken.
fn not_started(e: std::io::Error) -> String {
    format!(
        "KreoKit's FFmpeg couldn't be started ({e}). Reinstall KreoKit Voice; if that doesn't help, please use Report a problem."
    )
}

/// Kills a running FFmpeg when dropped.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}

/// Exports `job`. `work_dir` holds the temporary f32 file; `models_dir` the Best model.
pub fn export(
    job: &Job,
    work_dir: &Path,
    models_dir: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<Done, String> {
    if nothing_to_do(job.cleaned, &job.polish) {
        return Err(NOTHING_TO_DO.into());
    }
    if job.destination.exists() && !job.replace {
        return Err(format!("{} already exists", job.destination.display()));
    }
    if job.destination == job.source {
        return Err(
            "The export would overwrite the original file. Choose another name or folder.".into(),
        );
    }
    let started = std::time::Instant::now();
    let cleaned = job.cleaned.is_some();
    let mut report = |stage, f| {
        progress(Progress {
            stage,
            fraction: overall(stage, f, cleaned),
        })
    };
    let channels = preview::output_channels(job.info.audio.channels) as usize;

    // 1. The sound as the editor plays it, before Polish, in a temporary f32 file.
    fs::create_dir_all(work_dir).map_err(|e| format!("KreoKit couldn't create its work folder ({e}). Check that the disk isn't full, then try again."))?;
    let seconds = job.info.duration_ms as f64 / 1000.0;
    let f32_bytes = (seconds * SAMPLE_RATE as f64) as u64 * channels as u64 * 4;
    disk::ensure_free(work_dir, f32_bytes, preview::DISK_MARGIN)?;
    let temp = TempFile(temp_name(&work_dir.join("sound.f32")));
    let frames = write_sound(job, channels, &temp.0, models_dir, cancel, &mut |f| {
        report(Stage::Cleaning, f)
    })?;

    // 2. The exact measurement.
    let source = F32File {
        path: temp.0.clone(),
        channels,
        frames,
    };
    let measured = loudness::measure(&source, &job.polish, cancel, &mut |f| {
        report(Stage::Measuring, f)
    })?;

    // 3 and 4. Write, check; a lossy overshoot gets one more go with a lower ceiling.
    let dest_dir = job
        .destination
        .parent()
        .ok_or("Choose a folder to save into.")?;
    disk::ensure_free(
        dest_dir,
        job.format.bytes_per_second(channels as u16) * seconds as u64 + video_bytes(job),
        preview::DISK_MARGIN,
    )?;
    let target = job.polish.target_lufs().or(measured.input_lufs);
    let mut ceiling = CEILING_DBTP;
    let mut last_error = String::new();
    for attempt in 0..2 {
        check(cancel)?;
        let out = TempFile(temp_name(&job.destination));
        write_output(job, &source, &measured, ceiling, &out.0, cancel, &mut |f| {
            report(Stage::Writing, f)
        })?;
        report(Stage::Checking, 0.0);
        let found = measure_file(&out.0, channels, cancel)?;
        if let Some(video) = job.info.video.as_ref().filter(|_| keeps_picture(job)) {
            check_video(&job.source, &out.0, video.fps)?;
        }
        report(Stage::Checking, 0.8);
        let duration_ok = found
            .duration_ms
            .abs_diff(frames * 1000 / SAMPLE_RATE as u64)
            <= 50;
        let loudness_ok = match (target, found.lufs) {
            (Some(t), Some(l)) => (t - l).abs() <= LOUDNESS_TOLERANCE_LU,
            (None, _) | (_, None) => true,
        };
        let peak_ok = found.true_peak_db <= MAX_TRUE_PEAK_DBTP;
        if duration_ok && loudness_ok && peak_ok {
            finish(&out.0, &job.destination, job.replace)?;
            std::mem::forget(out);
            report(Stage::Checking, 1.0);
            let bytes = fs::metadata(&job.destination).map_or(0, |m| m.len());
            return Ok(Done {
                path: job.destination.clone(),
                lufs: found.lufs,
                true_peak_db: found.true_peak_db,
                duration_ms: found.duration_ms,
                bytes,
                realtime_factor: started.elapsed().as_secs_f64() / seconds.max(1e-3),
            });
        }
        last_error = format!(
            "the written file measured {} LUFS (target {}), true peak {:.2} dBTP, {} ms long (sound {} ms)",
            found.lufs.map_or("no".into(), |l| format!("{l:.2}")),
            target.map_or("as recorded".into(), |t| format!("{t:.1}")),
            found.true_peak_db,
            found.duration_ms,
            frames * 1000 / SAMPLE_RATE as u64,
        );
        if !(duration_ok && loudness_ok) || attempt == 1 {
            break;
        }
        // The encoder pushed the peak over: lower the ceiling by the overshoot.
        ceiling -= (found.true_peak_db - MAX_TRUE_PEAK_DBTP) as f32 + 0.1;
    }
    Err(format!(
        "The written file didn't pass the check, so it wasn't saved ({last_error}). Try again; if it happens again, please use Report a problem."
    ))
}

/// True when the export keeps the picture (a video exported as a video). A video exported
/// as WAV, FLAC, M4A or MP3 is the cleaned sound only, without a picture to check.
pub(crate) fn keeps_picture(job: &Job) -> bool {
    job.format == Format::Video && job.info.video.is_some()
}

/// Video bytes to copy (for the free-space check).
fn video_bytes(job: &Job) -> u64 {
    if keeps_picture(job) {
        fs::metadata(&job.source).map_or(0, |m| m.len())
    } else {
        0
    }
}

/// Moves the checked temporary file into place.
fn finish(temp: &Path, destination: &Path, replace: bool) -> Result<(), String> {
    if destination.exists() {
        if !replace {
            return Err(format!(
                "{} was created by something else during the export. Export again and choose Replace or Keep both.",
                destination.display()
            ));
        }
        // Windows can't rename over a file.
        if cfg!(windows) {
            fs::remove_file(destination).map_err(|e| format!("The existing file couldn't be replaced ({e}). Close it in other apps, or choose Keep both."))?;
        }
    }
    fs::rename(temp, destination).map_err(|e| format!("The file couldn't be saved in that folder ({e}). Check that you can write there, or choose another folder."))
}

/// Decodes the original in f32 and writes it, cleaned and blended when the file was
/// cleaned, to `path`. Returns the samples per channel.
fn write_sound(
    job: &Job,
    channels: usize,
    path: &Path,
    models_dir: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f64),
) -> Result<u64, String> {
    let ffmpeg = ffmpeg::locate("ffmpeg")?;
    let mut child = Running(
        Command::new(&ffmpeg)
            .args(preview::decode_args_as(&job.source, channels as u16, true))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(not_started)?,
    );
    let mut input = BufReader::with_capacity(1 << 20, child.0.stdout.take().unwrap());
    let mut output = BufWriter::with_capacity(
        1 << 20,
        File::create(path).map_err(|e| format!("cannot write the work file: {e}"))?,
    );
    // Best on dual mono cleans the channels' average once (as for listening).
    let dual_mono = job.cleaned == Some(Model::Best)
        && channels == 2
        && match &job.preview_dir {
            Some(dir) if preview::audio_path(dir).is_file() => {
                clean::channel_difference_db(&preview::audio_path(dir))? <= clean::DUAL_MONO_DB
            }
            _ => false,
        };
    let mut cleaner = match job.cleaned {
        Some(model) => {
            let engines = (0..if dual_mono { 1 } else { channels })
                .map(|_| Engine::new(model, models_dir))
                .collect::<Result<Vec<_>, _>>()?;
            Some(Cleaner::with(engines))
        }
        None => None,
    };
    let expected = (job.info.duration_ms * SAMPLE_RATE as u64 / 1000).max(1);
    let block = SAMPLE_RATE as usize * channels * 4;
    let mut bytes = vec![0u8; block];
    let mut pending = 0usize;
    let mut original: Vec<f32> = Vec::new();
    let mut cleaned: Vec<f32> = Vec::new();
    let mut frames = 0u64;
    let mut written = 0usize;
    let frame_bytes = channels * 4;
    // Writes what is ready: the original blended with the cleaned sound (or as it is).
    let write = |original: &mut Vec<f32>,
                 cleaned: &mut Vec<f32>,
                 output: &mut BufWriter<File>,
                 written: &mut usize,
                 cleaner_on: bool|
     -> Result<(), String> {
        let ready = if cleaner_on {
            if dual_mono {
                cleaned.len() * 2
            } else {
                cleaned.len()
            }
        } else {
            original.len()
        };
        let ready = ready.min(original.len());
        let mut out = Vec::with_capacity(ready * 4);
        for i in 0..ready {
            let o = original[i];
            let v = if cleaner_on {
                let c = if dual_mono {
                    cleaned[i / 2]
                } else {
                    cleaned[i]
                };
                blend(o, c, job.strength)
            } else {
                o
            };
            out.extend_from_slice(&v.to_le_bytes());
        }
        original.drain(..ready);
        if cleaner_on {
            cleaned.drain(..if dual_mono { ready / 2 } else { ready });
        }
        *written += ready;
        output
            .write_all(&out)
            .map_err(|e| format!("cannot write the work file: {e}"))
    };
    loop {
        check(cancel)?;
        let n = match input.read(&mut bytes[pending..]) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(UNREADABLE.into()),
        };
        let filled = pending + n;
        let whole = filled / frame_bytes * frame_bytes;
        let samples: Vec<f32> = bytes[..whole]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        frames += (whole / frame_bytes) as u64;
        bytes.copy_within(whole..filled, 0);
        pending = filled - whole;
        if let Some(c) = cleaner.as_mut() {
            if dual_mono {
                let mid: Vec<f32> = samples
                    .chunks_exact(2)
                    .map(|f| (f[0] + f[1]) * 0.5)
                    .collect();
                c.push(&mid, &mut cleaned)?;
            } else {
                c.push(&samples, &mut cleaned)?;
            }
        }
        original.extend_from_slice(&samples);
        write(
            &mut original,
            &mut cleaned,
            &mut output,
            &mut written,
            cleaner.is_some(),
        )?;
        progress((frames as f64 / expected as f64).min(1.0));
    }
    let status = child.0.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(UNREADABLE.into());
    }
    if let Some(c) = cleaner.take() {
        c.finish(&mut cleaned)?;
    }
    write(
        &mut original,
        &mut cleaned,
        &mut output,
        &mut written,
        job.cleaned.is_some(),
    )?;
    output
        .flush()
        .map_err(|e| format!("cannot write the work file: {e}"))?;
    if frames == 0 {
        return Err("This file has no sound to export. Check it in another player; if it has sound there, please use Report a problem.".into());
    }
    if written as u64 != frames * channels as u64 {
        return Err("Something went wrong while cleaning (the cleaned sound has the wrong length). Try again; if it keeps happening, please use Report a problem.".into());
    }
    progress(1.0);
    Ok(frames)
}

/// The temporary f32 file as a loudness source.
struct F32File {
    path: PathBuf,
    channels: usize,
    frames: u64,
}

impl Source for F32File {
    fn channels(&self) -> usize {
        self.channels
    }

    fn frames(&self) -> u64 {
        self.frames
    }

    fn for_each_block(
        &self,
        each: &mut dyn FnMut(&[f32]) -> Result<(), String>,
    ) -> Result<(), String> {
        self.for_each_block_in(&[(0, self.frames)], each)
    }

    fn for_each_block_in(
        &self,
        ranges: &[(u64, u64)],
        each: &mut dyn FnMut(&[f32]) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut file = BufReader::with_capacity(
            1 << 20,
            File::open(&self.path).map_err(|e| format!("cannot read the work file: {e}"))?,
        );
        let frame_bytes = self.channels * 4;
        let mut bytes = vec![0u8; SAMPLE_RATE as usize * frame_bytes];
        let mut samples = Vec::with_capacity(bytes.len() / 4);
        for &(start, end) in ranges {
            file.seek(SeekFrom::Start(start * frame_bytes as u64))
                .map_err(|e| format!("cannot read the work file: {e}"))?;
            let mut left = (end.min(self.frames).saturating_sub(start) as usize) * frame_bytes;
            while left > 0 {
                let want = left.min(bytes.len());
                file.read_exact(&mut bytes[..want])
                    .map_err(|e| format!("cannot read the work file: {e}"))?;
                left -= want;
                samples.clear();
                samples.extend(
                    bytes[..want]
                        .chunks_exact(4)
                        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                );
                each(&samples)?;
            }
        }
        Ok(())
    }
}

/// The encoders this FFmpeg has (asked once).
fn encoders() -> &'static str {
    static ENCODERS: OnceLock<String> = OnceLock::new();
    ENCODERS.get_or_init(|| {
        ffmpeg::locate("ffmpeg")
            .ok()
            .and_then(|f| {
                Command::new(f)
                    .args(["-hide_banner", "-encoders"])
                    .output()
                    .ok()
            })
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    })
}

/// Whether MP3 can be written (LAME in this FFmpeg).
pub fn mp3_available() -> bool {
    has_encoder("libmp3lame")
}

fn has_encoder(name: &str) -> bool {
    encoders()
        .lines()
        .any(|l| l.split_whitespace().nth(1) == Some(name))
}

/// The audio codec arguments for a format (the first output audio stream).
pub(crate) fn audio_codec_args(format: Format, video_extension: &str, aac_at: bool) -> Vec<String> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let aac = if aac_at { "aac_at" } else { "aac" };
    match format {
        Format::Wav => s(&["-c:a:0", "pcm_s24le"]),
        Format::Flac => s(&[
            "-c:a:0",
            "flac",
            "-sample_fmt:a:0",
            "s32",
            "-bits_per_raw_sample:a:0",
            "24",
        ]),
        Format::M4a => s(&["-c:a:0", aac, "-b:a:0", "256k"]),
        Format::Mp3 => s(&["-c:a:0", "libmp3lame", "-b:a:0", "320k"]),
        Format::Video if video_extension == "webm" => s(&["-c:a:0", "libopus", "-b:a:0", "192k"]),
        Format::Video => s(&["-c:a:0", aac, "-b:a:0", "256k"]),
    }
}

/// The muxer (`-f`) for a format: explicit, as the temporary name hides the extension.
pub(crate) fn muxer(format: Format, video_extension: &str) -> &'static str {
    match format {
        Format::Wav => "wav",
        Format::Flac => "flac",
        Format::M4a => "ipod",
        Format::Mp3 => "mp3",
        Format::Video => match video_extension {
            "mov" => "mov",
            "mkv" => "matroska",
            "webm" => "webm",
            "m4v" => "ipod",
            _ => "mp4",
        },
    }
}

/// The FFmpeg arguments that encode f32 samples from stdin (input 0, or input 1 after the
/// original for a video) into `out`.
pub(crate) fn encode_args(job: &Job, channels: usize, out: &Path, aac_at: bool) -> Vec<String> {
    let video_ext = extension(&job.source);
    let mut a: Vec<String> = ["-nostdin", "-y", "-v", "error"].map(String::from).to_vec();
    let raw = [
        "-f",
        "f32le",
        "-ar",
        "48000",
        "-ac",
        if channels == 1 { "1" } else { "2" },
        "-i",
        "pipe:0",
    ]
    .map(String::from);
    if keeps_picture(job) {
        a.push("-i".into());
        a.push(job.source.to_string_lossy().into_owned());
        a.extend(raw);
        // Picture first, then the new sound as the first audio track, then the original's
        // other audio tracks, subtitles and attachments, all copied.
        a.extend(["-map", "0:v?", "-map", "1:a:0"].map(String::from));
        for i in 1..job.info.audio_streams {
            a.push("-map".into());
            a.push(format!("0:a:{i}?"));
        }
        a.extend(["-map", "0:s?", "-map", "0:t?"].map(String::from));
        a.extend(["-c", "copy", "-map_metadata", "0", "-map_chapters", "0"].map(String::from));
        a.extend(["-disposition:a:0", "default", "-ar:a:0", "48000"].map(String::from));
    } else {
        a.extend(raw);
        a.extend(["-map", "0:a:0", "-map_metadata", "-1", "-ar", "48000"].map(String::from));
    }
    a.extend(audio_codec_args(job.format, &video_ext, aac_at));
    let mux = muxer(job.format, &video_ext);
    if matches!(mux, "mp4" | "ipod" | "mov") {
        a.extend(["-movflags", "+faststart"].map(String::from));
    }
    a.extend(["-f", mux].map(String::from));
    a.push(out.to_string_lossy().into_owned());
    a
}

/// The chain over the work file, piped into FFmpeg writing `out`.
fn write_output(
    job: &Job,
    source: &F32File,
    measured: &Measured,
    ceiling: f32,
    out: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f64),
) -> Result<(), String> {
    match job.format {
        Format::Mp3 if !has_encoder("libmp3lame") => {
            return Err(
                "MP3 isn't available in this build. Choose WAV, FLAC or M4A instead.".into(),
            );
        }
        Format::Video if extension(&job.source) == "webm" && !has_encoder("libopus") => {
            return Err("This build can't write WebM sound (Opus). Export the sound as WAV, FLAC, M4A or MP3 instead.".into());
        }
        _ => {}
    }
    let args = encode_args(job, source.channels, out, has_encoder("aac_at"));
    let ffmpeg = ffmpeg::locate("ffmpeg")?;
    let mut child = Running(
        Command::new(&ffmpeg)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(not_started)?,
    );
    let mut stderr = child.0.stderr.take().unwrap();
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let mut stdin = BufWriter::with_capacity(1 << 20, child.0.stdin.take().unwrap());
    let mut chain = Chain::new(source.channels, measured.params);
    chain.set_ceiling_db(ceiling);
    let latency = chain.latency() * source.channels;
    let mut skip = latency;
    let mut done = 0u64;
    let mut buffer = Vec::new();
    let mut send = |samples: &[f32], skip: &mut usize| -> Result<(), String> {
        let from = (*skip).min(samples.len());
        *skip -= from;
        let bytes: Vec<u8> = samples[from..]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        stdin
            .write_all(&bytes)
            .map_err(|_| "ffmpeg stopped while writing".to_string())
    };
    source.for_each_block(&mut |block| {
        check(cancel)?;
        buffer.clear();
        buffer.extend_from_slice(block);
        chain.process(&mut buffer);
        send(&buffer, &mut skip)?;
        done += (block.len() / source.channels) as u64;
        progress(done as f64 / source.frames.max(1) as f64);
        Ok(())
    })?;
    let mut tail = Vec::new();
    chain.flush(&mut tail);
    send(&tail, &mut skip)?;
    drop(send);
    stdin
        .into_inner()
        .map_err(|_| "ffmpeg stopped while writing".to_string())?;
    let status = child.0.wait().map_err(|e| e.to_string())?;
    let errors = errors.join().unwrap_or_default();
    if !status.success() {
        return Err(format!(
            "The file couldn't be written ({}). Check that there's free space in that folder, or try another format.",
            errors.trim()
        ));
    }
    Ok(())
}

/// The written file's sound, measured independently: loudness, true peak, length.
struct Found {
    lufs: Option<f64>,
    true_peak_db: f64,
    duration_ms: u64,
}

fn measure_file(path: &Path, channels: usize, cancel: &AtomicBool) -> Result<Found, String> {
    let ffmpeg = ffmpeg::locate("ffmpeg")?;
    let mut args: Vec<String> = ["-nostdin", "-v", "error", "-i"].map(String::from).to_vec();
    args.push(path.to_string_lossy().into_owned());
    args.extend(
        [
            "-map",
            "0:a:0",
            "-ac",
            &channels.to_string(),
            "-ar",
            "48000",
            "-f",
            "f32le",
            "pipe:1",
        ]
        .map(String::from),
    );
    let mut child = Running(
        Command::new(&ffmpeg)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(not_started)?,
    );
    let mut input = BufReader::with_capacity(1 << 20, child.0.stdout.take().unwrap());
    let mut meter = ebur128::EbuR128::new(
        channels as u32,
        48_000,
        ebur128::Mode::I | ebur128::Mode::TRUE_PEAK,
    )
    .map_err(|e| e.to_string())?;
    let mut bytes = vec![0u8; 1 << 20];
    let mut pending = 0;
    let mut frames = 0u64;
    let frame_bytes = channels * 4;
    loop {
        check(cancel)?;
        let n = match input.read(&mut bytes[pending..]) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(UNREADABLE_OUTPUT.into()),
        };
        let filled = pending + n;
        let whole = filled / frame_bytes * frame_bytes;
        let samples: Vec<f32> = bytes[..whole]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        meter.add_frames_f32(&samples).map_err(|e| e.to_string())?;
        frames += (whole / frame_bytes) as u64;
        bytes.copy_within(whole..filled, 0);
        pending = filled - whole;
    }
    if !child.0.wait().map_err(|e| e.to_string())?.success() {
        return Err(UNREADABLE_OUTPUT.into());
    }
    let peak = (0..channels as u32)
        .filter_map(|c| meter.true_peak(c).ok())
        .fold(0.0f64, f64::max);
    Ok(Found {
        lufs: meter.loudness_global().ok().filter(|l| l.is_finite()),
        true_peak_db: if peak > 0.0 {
            20.0 * peak.log10()
        } else {
            crate::analysis::SILENT_DB
        },
        duration_ms: frames * 1000 / 48_000,
    })
}

/// The video in `output` must be the original's, packet for packet (frame checksums of
/// the copied stream; the same frames in the same number means the same length), and the
/// new sound must start with the picture (within one frame). The container's duration may
/// grow by the audio encoder's padding (MKV keeps AAC's last frame whole), so it isn't
/// compared.
fn check_video(original: &Path, output: &Path, fps: f64) -> Result<(), String> {
    let sums = |path: &Path| -> Result<Vec<String>, String> {
        let ffmpeg = ffmpeg::locate("ffmpeg")?;
        let out = Command::new(&ffmpeg)
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(path)
            .args(["-map", "0:v:0", "-c", "copy", "-f", "framemd5", "-"])
            .output()
            .map_err(not_started)?;
        if !out.status.success() {
            return Err(
                "The picture couldn't be read for the check, so nothing was saved. Try again; to keep only the sound, choose WAV, FLAC, M4A or MP3.".into());
        }
        // Keep the checksum column (the last), not the timestamps (a remux may shift them).
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.rsplit(',').next().map(|s| s.trim().to_string()))
            .collect())
    };
    let (a, b) = (sums(original)?, sums(output)?);
    if a != b {
        return Err(format!(
            "The picture would have changed ({} frames in the original, {} in the export), so nothing was saved. Please use Report a problem; meanwhile you can export the sound only (WAV, FLAC, M4A or MP3).",
            a.len(),
            b.len()
        ));
    }
    let frame_ms = (1000.0 / fps.max(1.0)).ceil() as u64;
    let start = crate::probe::probe(output)?.audio_start_ms;
    if start > frame_ms {
        return Err(format!(
            "The new sound would start {start} ms after the picture, so nothing was saved. Please use Report a problem; meanwhile you can export the sound only (WAV, FLAC, M4A or MP3)."
        ));
    }
    Ok(())
}

/// Removes leftovers of a crashed export in `folder`: only files whose names end with
/// [`TEMP_SUFFIX`]. Returns how many were removed.
pub fn remove_leftovers(folder: &Path) -> usize {
    let Ok(entries) = fs::read_dir(folder) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(TEMP_SUFFIX))
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| fs::remove_file(e.path()).is_ok())
        .count()
}

/// `<name>-clean.<ext>` in `folder` for `source`, with Clips' safe file names.
pub fn destination(folder: &Path, source: &Path, format: Format) -> PathBuf {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "recording".into());
    folder.join(format!(
        "{}-clean.{}",
        safe_stem(&stem),
        format.extension(source)
    ))
}

/// The longest file name stem kept, in characters (as in KreoKit Clips).
pub const MAX_STEM_CHARS: usize = 60;
const FORBIDDEN: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
const RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// `name` as the start of a file name that works on Windows and macOS (from KreoKit Clips):
/// forbidden and control characters become spaces, runs of spaces collapse, no leading dot
/// or trailing dot or space, at most [`MAX_STEM_CHARS`] characters, "recording" if nothing
/// is left, and Windows' device names get an underscore.
pub fn safe_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if FORBIDDEN.contains(&c) || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let mut s = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    s = s.trim_start_matches('.').to_string();
    if s.chars().count() > MAX_STEM_CHARS {
        let cut: String = s.chars().take(MAX_STEM_CHARS).collect();
        s = match cut.rfind(' ') {
            Some(i) if i > MAX_STEM_CHARS * 2 / 3 => cut[..i].to_string(),
            _ => cut,
        };
    }
    s = s.trim_end_matches(['.', ' ']).to_string();
    if s.is_empty() {
        return "recording".into();
    }
    if RESERVED.contains(&s.to_lowercase().as_str()) {
        s.push('_');
    }
    s
}

/// A free name for `path` when it is taken: `name-2.ext`, `name-3.ext`… (Keep both).
pub fn keep_both(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (2..)
        .map(|n| path.with_file_name(format!("{stem}-{n}{ext}")))
        .find(|p| !p.exists())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loudness::Target;
    use crate::polish::Stages;
    use crate::probe::{self, fixture};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("kreokit-voice-export-tests")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn default_formats_and_names() {
        let audio = |ext: &str| {
            let info = probe::parse_ffprobe_json(
                r#"{"streams":[{"codec_type":"audio","sample_rate":"48000","channels":2}],"format":{"duration":"1"}}"#,
            )
            .unwrap();
            Format::default_for(Path::new(&format!("/a/x.{ext}")), &info)
        };
        assert_eq!(audio("mp3"), Format::Mp3);
        assert_eq!(audio("M4A"), Format::M4a);
        assert_eq!(audio("flac"), Format::Flac);
        assert_eq!(audio("wav"), Format::Wav);
        assert_eq!(audio("ogg"), Format::Wav, "anything else: WAV");
        let folder = Path::new("/out");
        assert_eq!(
            destination(folder, Path::new("/in/Episode 12: final?.wav"), Format::Mp3),
            PathBuf::from("/out/Episode 12 final-clean.mp3")
        );
        assert_eq!(
            destination(folder, Path::new("/in/clip.MOV"), Format::Video),
            PathBuf::from("/out/clip-clean.mov")
        );
        assert_eq!(safe_stem("..."), "recording");
        assert_eq!(safe_stem("con"), "con_");
        assert!(safe_stem(&"word ".repeat(30)).chars().count() <= MAX_STEM_CHARS);
    }

    #[test]
    fn keep_both_finds_a_free_name() {
        let dir = temp_dir("keep-both");
        let taken = dir.join("a-clean.wav");
        fs::write(&taken, b"x").unwrap();
        fs::write(dir.join("a-clean-2.wav"), b"x").unwrap();
        assert_eq!(keep_both(&taken), dir.join("a-clean-3.wav"));
    }

    #[test]
    fn nothing_to_do_only_when_nothing_changes() {
        let off = PolishSettings {
            stages: Stages {
                rumble: false,
                de_ess: false,
                compress: false,
            },
            target: Target::Off,
            ..Default::default()
        };
        assert!(nothing_to_do(None, &off));
        assert!(!nothing_to_do(Some(Model::Fast), &off), "cleaned");
        assert!(!nothing_to_do(
            None,
            &PolishSettings {
                target: Target::Social,
                ..off
            }
        ));
        let rumble = PolishSettings {
            stages: Stages {
                rumble: true,
                ..off.stages
            },
            ..off
        };
        assert!(!nothing_to_do(None, &rumble));
    }

    #[test]
    fn leftovers_are_ours_only() {
        let dir = temp_dir("leftovers");
        let ours = temp_name(&dir.join("talk-clean.wav"));
        fs::write(&ours, b"x").unwrap();
        fs::write(dir.join("talk.wav"), b"user's").unwrap();
        fs::write(dir.join(".hidden-user-file"), b"user's").unwrap();
        assert!(ours.file_name().unwrap().to_string_lossy().starts_with('.'));
        assert_eq!(remove_leftovers(&dir), 1);
        assert!(!ours.exists());
        assert!(dir.join("talk.wav").exists() && dir.join(".hidden-user-file").exists());
    }

    fn job(
        source: &Path,
        info: &MediaInfo,
        format: Format,
        dest: &Path,
        preview_dir: &Path,
    ) -> Job {
        Job {
            source: source.to_path_buf(),
            info: info.clone(),
            cleaned: None,
            strength: 0.85,
            polish: PolishSettings::default(),
            format,
            destination: dest.to_path_buf(),
            replace: false,
            preview_dir: Some(preview_dir.to_path_buf()),
        }
    }

    #[test]
    fn every_ffmpeg_command_is_lgpl() {
        let info = probe::parse_ffprobe_json(
            r#"{"streams":[{"codec_type":"video","width":640,"height":360,"avg_frame_rate":"25/1"},
                           {"codec_type":"audio","sample_rate":"48000","channels":2},
                           {"codec_type":"audio","sample_rate":"48000","channels":2}],
                "format":{"duration":"1"}}"#,
        )
        .unwrap();
        for format in [
            Format::Wav,
            Format::Flac,
            Format::M4a,
            Format::Mp3,
            Format::Video,
        ] {
            for source in ["/a/x.mp4", "/a/x.webm", "/a/x.mkv"] {
                let j = job(
                    Path::new(source),
                    &info,
                    format,
                    Path::new("/o/y"),
                    Path::new("/p"),
                );
                for aac_at in [false, true] {
                    let args = encode_args(&j, 2, Path::new("/o/tmp"), aac_at);
                    assert_eq!(ffmpeg::gpl_only(&args), None, "{args:?}");
                }
            }
        }
        // The second audio track is kept.
        let j = job(
            Path::new("/a/x.mp4"),
            &info,
            Format::Video,
            Path::new("/o/y"),
            Path::new("/p"),
        );
        let args = encode_args(&j, 2, Path::new("/o/tmp"), false);
        assert!(args.windows(2).any(|w| w == ["-map", "0:a:1?"]), "{args:?}");
        assert!(args.windows(2).any(|w| w == ["-c", "copy"]));
        assert_eq!(muxer(Format::Video, "webm"), "webm");
        assert!(audio_codec_args(Format::Video, "webm", false).contains(&"libopus".to_string()));
    }

    /// A video exported as an audio format is the sound only: the original isn't an input
    /// (no picture, subtitles or other tracks), and there's no picture to check.
    #[test]
    fn audio_formats_from_a_video_drop_the_picture() {
        let info = probe::parse_ffprobe_json(
            r#"{"streams":[{"codec_type":"video","width":1280,"height":720,"avg_frame_rate":"30/1"},
                           {"codec_type":"audio","sample_rate":"44100","channels":2}],
                "format":{"duration":"357"}}"#,
        )
        .unwrap();
        for source in ["/a/x.mp4", "/a/x.mov", "/a/x.mkv", "/a/x.webm"] {
            for format in [Format::Wav, Format::Flac, Format::M4a, Format::Mp3] {
                let j = job(
                    Path::new(source),
                    &info,
                    format,
                    Path::new("/o/y"),
                    Path::new("/p"),
                );
                assert!(!keeps_picture(&j));
                let args = encode_args(&j, 2, Path::new("/o/tmp"), false);
                assert!(!args.iter().any(|a| a == source), "{args:?}");
                assert!(
                    !args.iter().any(|a| a.starts_with("0:v") || a == "copy"),
                    "{args:?}"
                );
                assert_eq!(args.iter().filter(|a| *a == "-i").count(), 1, "{args:?}");
                assert_eq!(muxer(format, "mp4"), muxer(format, "webm"));
            }
            let j = job(
                Path::new(source),
                &info,
                Format::Video,
                Path::new("/o/y"),
                Path::new("/p"),
            );
            assert!(keeps_picture(&j));
        }
    }

    /// Prepares a copy of a fixture (preview, and Fast cleaning when asked) in `dir`.
    fn prepared(name: &str, dir: &Path, clean_it: bool) -> (PathBuf, MediaInfo, PathBuf) {
        let source = dir.join(name);
        fs::copy(fixture(name), &source).unwrap();
        let info = probe::probe(&source).unwrap();
        let preview_dir = dir.join("preview");
        let none = AtomicBool::new(false);
        let p = preview::prepare(
            &preview_dir,
            &source,
            info.audio.channels,
            info.duration_ms,
            &none,
            |_| {},
        )
        .unwrap();
        if clean_it {
            clean::prepare(&preview_dir, &p, Model::Fast, Path::new(""), &none, |_| {}).unwrap();
        }
        (source, info, preview_dir)
    }

    fn ffprobe_audio(path: &Path) -> serde_json::Value {
        let out = Command::new(ffmpeg::locate("ffprobe").unwrap())
            .args(["-v", "error", "-select_streams", "a:0", "-show_entries"])
            .args([
                "stream=codec_name,sample_rate,channels,bits_per_raw_sample,start_time",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .unwrap();
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["streams"][0].clone()
    }

    fn leftovers(d: &Path) -> usize {
        fs::read_dir(d)
            .map(|r| {
                r.flatten()
                    .filter(|e| e.file_name().to_string_lossy().ends_with(TEMP_SUFFIX))
                    .count()
            })
            .unwrap_or(0)
    }

    /// Every audio format from a cleaned stereo copy: the checks pass (±0.5 LU, ≤ −1 dBTP,
    /// length), the codecs are right, and nothing temporary is left.
    /// Needs ffmpeg with libmp3lame: `cargo test exports_every_audio_format -- --ignored`.
    #[test]
    #[ignore]
    fn exports_every_audio_format() {
        let dir = temp_dir("audio-formats");
        let (source, info, preview_dir) = prepared("hello-stereo.m4a", &dir, true);
        let work = dir.join("work");
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        for (format, codec) in [
            (Format::Wav, "pcm_s24le"),
            (Format::Flac, "flac"),
            (Format::M4a, "aac"),
            (Format::Mp3, "mp3"),
        ] {
            let dest = destination(&out, &source, format);
            let mut j = job(&source, &info, format, &dest, &preview_dir);
            j.cleaned = Some(Model::Fast);
            let mut stages = Vec::new();
            let done = export(
                &j,
                &work,
                Path::new(""),
                &AtomicBool::new(false),
                &mut |p| stages.push(p.stage),
            )
            .unwrap_or_else(|e| panic!("{format:?}: {e}"));
            eprintln!(
                "{format:?}: {:?} LUFS, {:.2} dBTP, {} ms, {} bytes, RTF {:.3}",
                done.lufs, done.true_peak_db, done.duration_ms, done.bytes, done.realtime_factor
            );
            assert!((done.lufs.unwrap() + 14.0).abs() <= 0.5);
            assert!(done.true_peak_db <= -1.0);
            assert!(stages.contains(&Stage::Measuring) && stages.contains(&Stage::Checking));
            let a = ffprobe_audio(&dest);
            assert!(a["codec_name"].as_str().unwrap().starts_with(codec), "{a}");
            assert_eq!(a["sample_rate"], "48000");
            assert_eq!(a["channels"], 2);
            if format == Format::Flac {
                assert_eq!(a["bits_per_raw_sample"], "24");
            }
            // Exporting again without Replace refuses; with it, works.
            assert!(
                export(
                    &j,
                    &work,
                    Path::new(""),
                    &AtomicBool::new(false),
                    &mut |_| {}
                )
                .is_err()
            );
            j.replace = true;
            export(
                &j,
                &work,
                Path::new(""),
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap();
        }
        assert_eq!(leftovers(&out), 0);
        assert_eq!(leftovers(&work), 0);
    }

    /// Polish only (never cleaned), Podcast, mono; and "nothing would change".
    #[test]
    #[ignore]
    fn polish_only_and_nothing_to_do() {
        let dir = temp_dir("polish-only");
        let (source, info, preview_dir) = prepared("hello-mono.wav", &dir, false);
        let dest = dir.join("hello-mono-clean.wav");
        let mut j = job(&source, &info, Format::Wav, &dest, &preview_dir);
        j.polish.target = Target::Podcast;
        let done = export(
            &j,
            &dir.join("work"),
            Path::new(""),
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        assert!((done.lufs.unwrap() + 16.0).abs() <= 0.5, "{:?}", done.lufs);
        assert_eq!(ffprobe_audio(&dest)["channels"], 1);
        j.polish = PolishSettings {
            stages: Stages {
                rumble: false,
                de_ess: false,
                compress: false,
            },
            target: Target::Off,
            ..Default::default()
        };
        j.destination = dir.join("other.wav");
        let err = export(
            &j,
            &dir.join("work"),
            Path::new(""),
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(err, NOTHING_TO_DO);
        assert!(!j.destination.exists());
    }

    /// The late-audio video: the picture copied bit for bit (checked by export), the new
    /// sound AAC 48 kHz from 0:00.
    #[test]
    #[ignore]
    fn remuxes_a_video_with_late_audio() {
        let dir = temp_dir("video");
        let (source, info, preview_dir) = prepared("hello-late.mp4", &dir, true);
        let dest = destination(&dir, &source, Format::Video);
        let mut j = job(&source, &info, Format::Video, &dest, &preview_dir);
        j.cleaned = Some(Model::Fast);
        export(
            &j,
            &dir.join("work"),
            Path::new(""),
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        let out = probe::probe(&dest).unwrap();
        assert_eq!(out.video.as_ref().unwrap().width, 640);
        let a = ffprobe_audio(&dest);
        assert_eq!(a["codec_name"], "aac");
        assert_eq!(a["sample_rate"], "48000");
        // The sound starts at the picture's 0:00 now (the gap was padded with silence).
        assert!(out.audio_start_ms <= 40, "{}", out.audio_start_ms);
    }

    /// A sample video in `dir`: a test picture and a 300 Hz tone (AAC 44.1 kHz stereo unless
    /// `extra` says otherwise). Made by the system's FFmpeg (on PATH): it needs video
    /// encoders, which Voice's own build doesn't have (it never encodes a picture).
    fn sample_video(dir: &Path, name: &str, size: &str, extra: &[&str]) -> PathBuf {
        let path = dir.join(name);
        let status = Command::new("ffmpeg")
            .args(["-nostdin", "-y", "-v", "error", "-f", "lavfi"])
            .arg("-i")
            .arg(format!("testsrc=size={size}:rate=25:duration=3"))
            .args([
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=300:sample_rate=44100:duration=3",
            ])
            .args(extra)
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success(), "{name}");
        path
    }

    /// Every audio format from MP4, MOV, MKV and WebM sources: the sound only, no picture,
    /// checked like any audio export (the bug: MP3 from an MP4 failed on the picture check).
    #[test]
    #[ignore]
    fn audio_formats_from_videos() {
        let dir = temp_dir("audio-from-video");
        let stereo = ["-ac", "2"];
        let videos = [
            ("talk.mp4", "1280x720", ["-c:v", "mpeg4", "-c:a", "aac"]),
            ("phone.mov", "320x240", ["-c:v", "mpeg4", "-c:a", "aac"]),
            ("stream.mkv", "320x240", ["-c:v", "mpeg4", "-c:a", "aac"]),
            ("web.webm", "320x240", ["-c:v", "libvpx", "-c:a", "libopus"]),
        ];
        for (name, size, codecs) in videos {
            let extra: Vec<&str> = ["-map", "0:v", "-map", "1:a"]
                .into_iter()
                .chain(codecs)
                .chain(stereo)
                .collect();
            let source = sample_video(&dir, name, size, &extra);
            let info = probe::probe(&source).unwrap();
            assert!(info.video.is_some(), "{name}");
            let preview_dir = dir.join(format!("preview-{name}"));
            preview::prepare(
                &preview_dir,
                &source,
                info.audio.channels,
                info.duration_ms,
                &AtomicBool::new(false),
                |_| {},
            )
            .unwrap();
            for (format, codec) in [
                (Format::Wav, "pcm_s24le"),
                (Format::Flac, "flac"),
                (Format::M4a, "aac"),
                (Format::Mp3, "mp3"),
            ] {
                if format == Format::Mp3 && !mp3_available() {
                    panic!("MP3 isn't available in this FFmpeg");
                }
                let dest = destination(&dir, &source, format);
                let mut j = job(&source, &info, format, &dest, &preview_dir);
                j.cleaned = Some(Model::Fast);
                let done = export(
                    &j,
                    &dir.join("work"),
                    Path::new(""),
                    &AtomicBool::new(false),
                    &mut |_| {},
                )
                .unwrap_or_else(|e| panic!("{name} as {format:?}: {e}"));
                assert!(
                    (done.lufs.unwrap() + 14.0).abs() <= 0.5,
                    "{name} {format:?}"
                );
                assert!(done.true_peak_db <= -1.0, "{name} {format:?}");
                let out = probe::probe(&dest).unwrap();
                assert!(out.video.is_none(), "no picture: {name} as {format:?}");
                assert_eq!(out.audio_streams, 1, "{name} {format:?}");
                let a = ffprobe_audio(&dest);
                assert_eq!(a["codec_name"], codec, "{name} {format:?}");
                assert_eq!(a["sample_rate"], "48000");
                assert_eq!(a["channels"], 2);
                assert!(
                    out.duration_ms.abs_diff(info.duration_ms) <= 100,
                    "{name} {format:?}: {} vs {} ms",
                    out.duration_ms,
                    info.duration_ms
                );
                fs::remove_file(&dest).unwrap();
            }
        }
        assert_eq!(leftovers(&dir) + leftovers(&dir.join("work")), 0);
    }

    /// Generated videos in other containers: MKV with two audio tracks, MOV rotated, WebM.
    #[test]
    #[ignore]
    fn remuxes_mkv_mov_and_webm() {
        let dir = temp_dir("containers");
        let make = |name: &str, extra: &[&str]| sample_video(&dir, name, "320x240", extra);
        let videos = [
            make(
                "two-tracks.mkv",
                &[
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=500:duration=3",
                    "-map",
                    "0:v",
                    "-map",
                    "1:a",
                    "-map",
                    "2:a",
                    "-c:v",
                    "mpeg4",
                    "-c:a",
                    "aac",
                ],
            ),
            make(
                "phone.mov",
                &[
                    "-map",
                    "0:v",
                    "-map",
                    "1:a",
                    "-c:v",
                    "mpeg4",
                    "-c:a",
                    "aac",
                    "-metadata:s:v:0",
                    "rotate=90",
                ],
            ),
            make(
                "web.webm",
                &[
                    "-map", "0:v", "-map", "1:a", "-c:v", "libvpx", "-b:v", "200k", "-c:a",
                    "libopus",
                ],
            ),
        ];
        for source in videos {
            let info = probe::probe(&source).unwrap();
            let name = source.file_name().unwrap().to_string_lossy().into_owned();
            let preview_dir = dir.join(format!("preview-{name}"));
            preview::prepare(
                &preview_dir,
                &source,
                info.audio.channels,
                info.duration_ms,
                &AtomicBool::new(false),
                |_| {},
            )
            .unwrap();
            let dest = destination(&dir, &source, Format::Video);
            let j = job(&source, &info, Format::Video, &dest, &preview_dir);
            export(
                &j,
                &dir.join("work"),
                Path::new(""),
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            let out = probe::probe(&dest).unwrap();
            assert_eq!(out.audio_streams, info.audio_streams, "{name}");
            assert_eq!(
                out.video.as_ref().unwrap().rotation,
                info.video.as_ref().unwrap().rotation,
                "rotation kept: {name}"
            );
            let codec = ffprobe_audio(&dest)["codec_name"]
                .as_str()
                .unwrap()
                .to_string();
            let want = if name.ends_with("webm") {
                "opus"
            } else {
                "aac"
            };
            assert_eq!(codec, want, "{name}");
        }
    }

    /// Cancelling removes the temporary files.
    #[test]
    #[ignore]
    fn cancel_leaves_nothing() {
        let dir = temp_dir("cancel");
        let (source, info, preview_dir) = prepared("hello-mono.wav", &dir, false);
        let dest = dir.join("out.wav");
        let j = job(&source, &info, Format::Wav, &dest, &preview_dir);
        let cancel = AtomicBool::new(false);
        let err = export(&j, &dir.join("work"), Path::new(""), &cancel, &mut |p| {
            if p.stage == Stage::Measuring {
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .unwrap_err();
        assert_eq!(err, CANCELLED);
        assert!(!dest.exists());
        assert_eq!(
            leftovers(&dir.join("work")) + leftovers(&dir),
            0,
            "nothing left"
        );
    }
}
