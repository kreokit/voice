//! `voice --self-test <fixtures/speech> [--models <dir>]`: a hidden mode for release CI
//! (milestone 6). It runs the app's own export pipeline inside the shipped binary (the
//! bundled FFmpeg, RNNoise and ONNX Runtime as built for release) and exits non-zero on any
//! failure. It runs before Tauri starts, so it needs no window or display.
//!
//! 1. FFmpeg must be the bundled one (next to the executable) in release builds.
//! 2. A noisy file: `voices.wav` plus seeded room noise at 5 dB SNR (`testsignals`).
//! 3. Clean it with Fast and with Best (downloaded from `models-v1` and checked, unless the
//!    CPU can't run it), exported as WAV without Polish: the noise in the pauses must drop
//!    and SI-SDR against the clean speech must rise, Best more than Fast.
//! 4. Export it with Polish (Social video, −14 LUFS) as WAV, FLAC, M4A and MP3: `export`'s
//!    own checks (±0.5 LU, true peak ≤ −1 dBTP, length) plus the written codec.
//! 5. Videos: `hello-late.mp4` as a video (picture bit-identical, sound from 0:00) and as
//!    MP3 (sound only, no picture); `hello.webm` as a video (Opus).

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use crate::clean::Model;
use crate::export::{self, Format, Job};
use crate::loudness::{PolishSettings, Target};
use crate::polish::Stages;
use crate::testsignals::{Noise, mix, noise, read_wav, write_wav};
use crate::{cpu, ffmpeg, metrics, models, probe};

/// The noise mixed in, and the least each model must do about it. Measured 2026-10-22 on
/// the dev Mac (Intel i5, our LGPL FFmpeg): Fast +6.1 dB SI-SDR, pauses −34.4 dB; Best
/// +10.8 dB, −44.9 dB. The limits are the quality tests' (`clean::tests`).
const SNR_DB: f64 = 5.0;
const MIN_SI_SDR_GAIN_DB: f64 = 2.3;
const MIN_PAUSE_DROP_DB: f64 = 25.0;
const MIN_PAUSE_DROP_BEST_DB: f64 = 35.0;

/// Runs the self-test when `args` (without the program name) start with `--self-test`;
/// returns `None` when the app should start normally.
pub fn run_if_requested(args: &[String]) -> Option<ExitCode> {
    if args.first().map(String::as_str) != Some("--self-test") {
        return None;
    }
    Some(match parse(&args[1..]).and_then(|options| run(&options)) {
        Ok(()) => {
            println!("self-test: ok");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("self-test: FAILED: {e}");
            ExitCode::FAILURE
        }
    })
}

#[derive(Debug, PartialEq)]
struct Options {
    fixtures: PathBuf,
    models_dir: PathBuf,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let usage = "usage: voice --self-test <fixtures/speech folder> [--models <dir>]";
    let mut fixtures = None;
    let mut models_dir = std::env::temp_dir().join("kreokit-voice-self-test-models");
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--models" => models_dir = PathBuf::from(args.next().ok_or(usage)?),
            flag if flag.starts_with("--") => return Err(format!("unknown flag {flag}; {usage}")),
            _ if fixtures.is_none() => fixtures = Some(PathBuf::from(arg)),
            _ => return Err(usage.into()),
        }
    }
    Ok(Options {
        fixtures: fixtures.ok_or(usage)?,
        models_dir,
    })
}

fn run(options: &Options) -> Result<(), String> {
    let started = Instant::now();
    let ffmpeg = ffmpeg::locate("ffmpeg")?;
    let ffprobe = ffmpeg::locate("ffprobe")?;
    println!("ffmpeg:   {}", ffmpeg.display());
    println!("ffprobe:  {}", ffprobe.display());
    if !cfg!(debug_assertions) {
        bundled(&ffmpeg)?;
        bundled(&ffprobe)?;
    }
    let work = std::env::temp_dir().join(format!("kreokit-voice-self-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let result = steps(options, &work);
    let _ = std::fs::remove_dir_all(&work);
    result?;
    println!("total: {:.1} s", started.elapsed().as_secs_f64());
    Ok(())
}

fn steps(options: &Options, work: &Path) -> Result<(), String> {
    // 2. The noisy file.
    let speech = read_wav(&options.fixtures.join("voices.wav"))?;
    let room = noise(Noise::Room, speech.len(), 11);
    let noisy = mix(&speech, &room, SNR_DB);
    let source = work.join("noisy.wav");
    write_wav(&source, &noisy)?;
    let gaps = metrics::pauses(&speech);
    if gaps.is_empty() {
        return Err("the speech fixture has no pauses".into());
    }
    let (sdr_in, floor_in) = (
        metrics::si_sdr(&speech, &noisy),
        metrics::level_in(&noisy, &gaps),
    );
    println!("noisy:    SI-SDR {sdr_in:.1} dB, pauses {floor_in:.1} dBFS");

    // 3. Cleaning, without Polish.
    let mut models_run = vec![Model::Fast];
    let support = cpu::check();
    if support.supported {
        let t = Instant::now();
        models::download_best(&options.models_dir, &AtomicBool::new(false), |_| {})?;
        println!(
            "best:     model ready in {:.1} s",
            t.elapsed().as_secs_f64()
        );
        models_run.push(Model::Best);
    } else {
        println!(
            "best:     skipped, this CPU lacks {}",
            support.missing.join(", ")
        );
    }
    let raw = PolishSettings {
        stages: Stages {
            rumble: false,
            de_ess: false,
            compress: false,
        },
        target: Target::Off,
        ..Default::default()
    };
    let mut fast_gain = f64::NAN;
    for model in models_run {
        let dest = work.join(format!("clean-{model:?}.wav"));
        let done = export_one(
            &source,
            Some(model),
            raw,
            Format::Wav,
            &dest,
            &options.models_dir,
        )?;
        let out = decode_mono(&dest)?;
        if out.len() != speech.len() {
            return Err(format!(
                "{model:?}: {} samples out, {} in",
                out.len(),
                speech.len()
            ));
        }
        let (sdr, floor) = (
            metrics::si_sdr(&speech, &out),
            metrics::level_in(&out, &gaps),
        );
        let (gain, drop) = (sdr - sdr_in, floor_in - floor);
        println!(
            "{model:?}:{}SI-SDR {sdr:.1} dB ({gain:+.1}), pauses {floor:.1} dBFS (−{drop:.1} dB), RTF {:.2}",
            " ".repeat(10 - format!("{model:?}:").len()),
            done.realtime_factor
        );
        let min_drop = match model {
            Model::Fast => MIN_PAUSE_DROP_DB,
            Model::Best => MIN_PAUSE_DROP_BEST_DB,
        };
        if gain < MIN_SI_SDR_GAIN_DB || drop < min_drop {
            return Err(format!(
                "{model:?} cleaned too little: SI-SDR {gain:+.1} dB (needs +{MIN_SI_SDR_GAIN_DB}), pauses −{drop:.1} dB (needs −{min_drop})"
            ));
        }
        match model {
            Model::Fast => fast_gain = gain,
            Model::Best if gain <= fast_gain => {
                return Err(format!(
                    "Best ({gain:+.1} dB) isn't better than Fast ({fast_gain:+.1} dB)"
                ));
            }
            Model::Best => {}
        }
    }

    // 4. Every audio format with Polish.
    let polish = PolishSettings::default();
    for (format, codec) in [
        (Format::Wav, "pcm_s24le"),
        (Format::Flac, "flac"),
        (Format::M4a, "aac"),
        (Format::Mp3, "mp3"),
    ] {
        let dest = export::destination(work, &source, format);
        let done = export_one(
            &source,
            Some(Model::Fast),
            polish,
            format,
            &dest,
            &options.models_dir,
        )?;
        expect_audio(&dest, codec, false)?;
        report(&format!("{format:?}"), &done);
    }

    // 5. Videos.
    let late = options.fixtures.join("hello-late.mp4");
    let webm = options.fixtures.join("hello.webm");
    for (video, format, codec, picture) in [
        (&late, Format::Video, "aac", true),
        (&late, Format::Mp3, "mp3", false),
        (&webm, Format::Video, "opus", true),
    ] {
        let dest = export::destination(work, video, format);
        let done = export_one(
            video,
            Some(Model::Fast),
            polish,
            format,
            &dest,
            &options.models_dir,
        )?;
        expect_audio(&dest, codec, picture)?;
        let name = video.file_name().unwrap_or_default().to_string_lossy();
        report(&format!("{name} as {format:?}"), &done);
    }
    Ok(())
}

fn export_one(
    source: &Path,
    cleaned: Option<Model>,
    polish: PolishSettings,
    format: Format,
    destination: &Path,
    models_dir: &Path,
) -> Result<export::Done, String> {
    let info = probe::probe(source)?;
    let job = Job {
        source: source.to_path_buf(),
        info,
        cleaned,
        strength: 1.0,
        polish,
        format,
        destination: destination.to_path_buf(),
        replace: false,
        preview_dir: None,
    };
    let work = destination.parent().unwrap_or(Path::new(".")).join("work");
    export::export(
        &job,
        &work,
        models_dir,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .map_err(|e| {
        format!(
            "{} as {format:?}: {e}",
            source.file_name().unwrap_or_default().to_string_lossy()
        )
    })
}

fn report(what: &str, done: &export::Done) {
    println!(
        "export:   {what}: {} LUFS, {:.2} dBTP, {} ms, {} bytes, RTF {:.2}",
        done.lufs.map_or("-".into(), |l| format!("{l:.2}")),
        done.true_peak_db,
        done.duration_ms,
        done.bytes,
        done.realtime_factor
    );
}

/// The written file has `codec` as its only sound, and a picture exactly when `picture`.
fn expect_audio(path: &Path, codec: &str, picture: bool) -> Result<(), String> {
    let info = probe::probe(path)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    if info.audio.codec != codec {
        return Err(format!(
            "{name}: sound is {}, not {codec}",
            info.audio.codec
        ));
    }
    if info.audio.sample_rate != 48_000 {
        return Err(format!("{name}: {} Hz, not 48 kHz", info.audio.sample_rate));
    }
    if info.video.is_some() != picture {
        return Err(format!(
            "{name}: {}",
            if picture {
                "the picture is missing"
            } else {
                "has a picture"
            }
        ));
    }
    Ok(())
}

/// A file's first sound as 48 kHz mono f32, through the bundled FFmpeg.
fn decode_mono(path: &Path) -> Result<Vec<f32>, String> {
    let out = Command::new(ffmpeg::locate("ffmpeg")?)
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args([
            "-map", "0:a:0", "-ac", "1", "-ar", "48000", "-f", "f32le", "pipe:1",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("cannot decode {}", path.display()));
    }
    Ok(out
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

/// Release builds must run the FFmpeg they bundle, not one found on the system.
fn bundled(tool: &Path) -> Result<(), String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let overridden = ["KREOKIT_FFMPEG", "KREOKIT_FFPROBE"]
        .iter()
        .any(|var| std::env::var_os(var).is_some());
    if overridden || tool.parent() == exe_dir.as_deref() {
        Ok(())
    } else {
        Err(format!(
            "{} isn't the bundled one (next to the executable)",
            tool.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn only_with_the_flag() {
        assert!(run_if_requested(&[]).is_none());
        assert!(run_if_requested(&strings(&["--other"])).is_none());
    }

    #[test]
    fn parses_its_arguments() {
        let o = parse(&strings(&["fixtures/speech", "--models", "/m"])).unwrap();
        assert_eq!(o.fixtures, PathBuf::from("fixtures/speech"));
        assert_eq!(o.models_dir, PathBuf::from("/m"));
        assert!(parse(&[]).is_err());
        assert!(parse(&strings(&["a", "b"])).is_err());
        assert!(parse(&strings(&["a", "--nope"])).is_err());
    }

    /// The whole self-test in a debug build (Best is downloaded once):
    /// `cargo test self_test_passes -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn self_test_passes() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/speech");
        let options = Options {
            fixtures,
            models_dir: std::env::temp_dir().join("kreokit-voice-self-test-models"),
        };
        run(&options).unwrap();
    }
}
