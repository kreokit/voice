//! `probe`: what an audio or video file contains, read with ffprobe (from KreoKit Clips,
//! for audio files too). Voice needs the sound; a video's picture is only described, since
//! it is copied untouched on export.

use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::ffmpeg;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub duration_ms: u64,
    /// ffprobe's container name, e.g. "wav" or "mov,mp4,m4a,3gp,3g2,mj2".
    pub container: String,
    /// The first audio stream, the one Voice cleans.
    pub audio: AudioInfo,
    /// How many audio streams the file has (export keeps the others as they are).
    #[serde(default = "one")]
    pub audio_streams: u32,
    /// How long after the file's 0:00 the audio starts (0 when it starts first). Decoding
    /// pads this with silence, so the cleaned sound stays in sync with the picture.
    pub audio_start_ms: u64,
    /// The picture, for video files; cover art in audio files doesn't count.
    pub video: Option<VideoInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioInfo {
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u32,
    /// e.g. "mono", "stereo", "5.1(side)"; absent in some files.
    pub channel_layout: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoInfo {
    pub codec: String,
    /// Size as displayed: rotated phone videos (90° / 270°) have width and height swapped
    /// relative to the encoded frames.
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// Clockwise rotation applied on display (0, 90, 180 or 270).
    pub rotation: u32,
}

fn one() -> u32 {
    1
}

pub fn probe(path: &Path) -> Result<MediaInfo, String> {
    if !path.is_file() {
        return Err(format!("{} doesn't exist", path.display()));
    }
    let ffprobe = ffmpeg::locate("ffprobe")?;
    let output = Command::new(&ffprobe)
        .args(probe_args())
        .arg(path)
        .output()
        .map_err(|e| format!("failed to run {}: {e}", ffprobe.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "this file can't be read as audio or video: {}",
            stderr.trim()
        ));
    }
    parse_ffprobe_json(&String::from_utf8_lossy(&output.stdout))
}

/// Whole sections rather than -show_entries: section names differ between FFmpeg versions
/// (4.4 has no `stream_side_data`), and unknown fields are ignored.
fn probe_args() -> [&'static str; 6] {
    [
        "-v",
        "error",
        "-show_streams",
        "-show_format",
        "-of",
        "json",
    ]
}

#[derive(Deserialize)]
struct FfprobeOutput {
    #[serde(default)]
    streams: Vec<FfprobeStream>,
    format: Option<FfprobeFormat>,
}

#[derive(Deserialize)]
struct FfprobeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    start_time: Option<String>,
    duration: Option<String>,
    // Audio
    sample_rate: Option<String>,
    channels: Option<u32>,
    channel_layout: Option<String>,
    // Video
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    disposition: Option<Disposition>,
    #[serde(default)]
    side_data_list: Vec<SideData>,
    #[serde(default)]
    tags: Tags,
}

#[derive(Deserialize, Default)]
struct Tags {
    /// Clockwise rotation in degrees, as FFmpeg 4.x and older also report it.
    rotate: Option<String>,
}

#[derive(Deserialize)]
struct Disposition {
    #[serde(default)]
    attached_pic: u8,
}

#[derive(Deserialize)]
struct SideData {
    rotation: Option<f64>,
}

#[derive(Deserialize)]
struct FfprobeFormat {
    format_name: Option<String>,
    duration: Option<String>,
    start_time: Option<String>,
}

fn seconds(value: &Option<String>) -> Option<f64> {
    value
        .as_deref()?
        .parse::<f64>()
        .ok()
        .filter(|s| s.is_finite())
}

/// Parses the JSON printed by the ffprobe invocation in [`probe`].
pub fn parse_ffprobe_json(json: &str) -> Result<MediaInfo, String> {
    let parsed: FfprobeOutput =
        serde_json::from_str(json).map_err(|e| format!("unexpected ffprobe output: {e}"))?;

    let audio = parsed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"))
        .ok_or("this file has no sound")?;
    let sample_rate = audio
        .sample_rate
        .as_deref()
        .and_then(|r| r.parse::<u32>().ok())
        .filter(|&r| r > 0)
        .ok_or("the sound has no sample rate")?;
    let channels = audio
        .channels
        .filter(|&c| c > 0)
        .ok_or("the sound has no channels")?;

    // The first real video stream; cover art (MP3/M4A artwork) counts as a video stream
    // in ffprobe but is a still image.
    let video = parsed
        .streams
        .iter()
        .find(|s| {
            s.codec_type.as_deref() == Some("video")
                && s.disposition.as_ref().is_none_or(|d| d.attached_pic == 0)
        })
        .map(parse_video)
        .transpose()?;

    // ffmpeg puts the earliest stream at 0:00 (the container's start time), like players.
    let start = parsed.format.as_ref().and_then(|f| seconds(&f.start_time));
    let audio_start_ms = match (seconds(&audio.start_time), start) {
        (Some(a), Some(start)) => ((a - start) * 1000.0).round().max(0.0) as u64,
        _ => 0,
    };

    // The container duration covers all streams; fall back to the audio stream's own.
    let format = parsed.format.as_ref();
    let duration = format
        .and_then(|f| seconds(&f.duration))
        .or_else(|| seconds(&audio.duration))
        .filter(|s| *s > 0.0)
        .ok_or("the file has no duration")?;

    Ok(MediaInfo {
        duration_ms: (duration * 1000.0).round() as u64,
        container: format
            .and_then(|f| f.format_name.clone())
            .unwrap_or_default(),
        audio: AudioInfo {
            codec: audio.codec_name.clone().unwrap_or_default(),
            sample_rate,
            channels,
            channel_layout: audio.channel_layout.clone().filter(|l| !l.is_empty()),
        },
        audio_start_ms,
        audio_streams: parsed
            .streams
            .iter()
            .filter(|s| s.codec_type.as_deref() == Some("audio"))
            .count() as u32,
        video,
    })
}

fn parse_video(stream: &FfprobeStream) -> Result<VideoInfo, String> {
    let (coded_width, coded_height) = stream
        .width
        .zip(stream.height)
        .ok_or("the video stream has no resolution")?;

    // avg_frame_rate is "0/0" for some containers; fall back to r_frame_rate.
    let fps = [&stream.avg_frame_rate, &stream.r_frame_rate]
        .into_iter()
        .flatten()
        .find_map(|rate| parse_rate(rate))
        .ok_or("the video stream has no frame rate")?;

    let rotation = stream
        .side_data_list
        .iter()
        .find_map(|d| d.rotation)
        .map(normalize_rotation)
        .or_else(|| {
            let clockwise = stream.tags.rotate.as_deref()?.trim().parse::<f64>().ok()?;
            Some(normalize_rotation(-clockwise))
        })
        .unwrap_or(0);
    let (width, height) = if rotation % 180 == 90 {
        (coded_height, coded_width)
    } else {
        (coded_width, coded_height)
    };
    Ok(VideoInfo {
        codec: stream.codec_name.clone().unwrap_or_default(),
        width,
        height,
        fps,
        rotation,
    })
}

/// ffprobe reports the display matrix rotation counter-clockwise (-90 for a phone held
/// upright). Returns the clockwise rotation snapped to 0, 90, 180 or 270.
fn normalize_rotation(degrees: f64) -> u32 {
    let quarter = (-degrees / 90.0).round() as i64;
    (quarter.rem_euclid(4) * 90) as u32
}

/// Parses an ffprobe rational like "30000/1001". Returns `None` for "0/0" and other
/// unusable values.
fn parse_rate(rate: &str) -> Option<f64> {
    let (num, den) = rate.split_once('/')?;
    let (num, den) = (
        num.trim().parse::<f64>().ok()?,
        den.trim().parse::<f64>().ok()?,
    );
    let fps = num / den;
    (fps.is_finite() && fps > 0.0).then_some(fps)
}

#[cfg(test)]
pub(crate) fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/speech")
        .join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ffprobe `-of json` output for a 1080p 29.97 fps clip with stereo sound.
    const VIDEO: &str = r#"{
        "streams": [
            {
                "codec_type": "video", "codec_name": "h264",
                "width": 1920, "height": 1080,
                "r_frame_rate": "30000/1001", "avg_frame_rate": "30000/1001",
                "duration": "12.312300",
                "disposition": { "default": 1, "attached_pic": 0 }
            },
            { "codec_type": "audio", "codec_name": "aac", "sample_rate": "48000",
              "channels": 2, "channel_layout": "stereo" }
        ],
        "format": { "format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "12.345000" }
    }"#;

    #[test]
    fn parses_a_video() {
        let info = parse_ffprobe_json(VIDEO).unwrap();
        assert_eq!(info.duration_ms, 12_345);
        assert_eq!(info.container, "mov,mp4,m4a,3gp,3g2,mj2");
        assert_eq!(
            info.audio,
            AudioInfo {
                codec: "aac".into(),
                sample_rate: 48_000,
                channels: 2,
                channel_layout: Some("stereo".into()),
            }
        );
        let video = info.video.unwrap();
        assert_eq!((video.width, video.height, video.rotation), (1920, 1080, 0));
        assert!((video.fps - 29.97).abs() < 0.001, "fps was {}", video.fps);
        assert_eq!(video.codec, "h264");
    }

    #[test]
    fn an_audio_file_with_cover_art_has_no_video() {
        let json = r#"{
            "streams": [{ "codec_type": "audio", "codec_name": "mp3", "sample_rate": "44100",
                          "channels": 1, "channel_layout": "mono" },
                        { "codec_type": "video", "width": 600, "height": 600,
                          "avg_frame_rate": "0/0", "r_frame_rate": "90000/1",
                          "disposition": { "attached_pic": 1 } }],
            "format": { "format_name": "mp3", "duration": "1800.0" }
        }"#;
        let info = parse_ffprobe_json(json).unwrap();
        assert_eq!(info.video, None);
        assert_eq!(info.duration_ms, 1_800_000);
        assert_eq!(info.audio.channels, 1);
    }

    #[test]
    fn audio_starting_after_the_picture() {
        let json = |audio: &str, format: &str| {
            format!(
                r#"{{"streams": [{{ "codec_type": "video", "width": 640, "height": 360,
                                   "avg_frame_rate": "25/1", "start_time": "{format}" }},
                                 {{ "codec_type": "audio", "sample_rate": "48000", "channels": 1,
                                    "start_time": "{audio}" }}],
                    "format": {{ "duration": "10.0", "start_time": "{format}" }}}}"#
            )
        };
        let start = |audio, format| {
            parse_ffprobe_json(&json(audio, format))
                .unwrap()
                .audio_start_ms
        };
        assert_eq!(start("0.676780", "0.000000"), 677);
        // Measured from the container's start, as ffmpeg does.
        assert_eq!(start("1.900000", "1.400000"), 500);
        // Audio first (encoder delay): nothing to pad.
        assert_eq!(start("-0.021333", "-0.021333"), 0);
    }

    #[test]
    fn falls_back_to_the_audio_duration() {
        let json = r#"{
            "streams": [{ "codec_type": "audio", "codec_name": "pcm_s16le",
                          "sample_rate": "48000", "channels": 6, "duration": "3.0004" }],
            "format": { "format_name": "wav" }
        }"#;
        let info = parse_ffprobe_json(json).unwrap();
        assert_eq!(info.duration_ms, 3_000);
        assert_eq!(info.audio.channels, 6);
        assert_eq!(info.audio.channel_layout, None);
    }

    #[test]
    fn swaps_size_of_rotated_phone_video() {
        let json = r#"{
            "streams": [{ "codec_type": "video", "width": 1920, "height": 1080,
                          "avg_frame_rate": "30/1",
                          "side_data_list": [{ "rotation": -90 }] },
                        { "codec_type": "audio", "sample_rate": "44100", "channels": 2 }],
            "format": { "duration": "5.0" }
        }"#;
        let video = parse_ffprobe_json(json).unwrap().video.unwrap();
        assert_eq!(
            (video.width, video.height, video.rotation),
            (1080, 1920, 90)
        );
        let older = json.replace(
            r#""side_data_list": [{ "rotation": -90 }]"#,
            r#""tags": { "rotate": "90" }"#,
        );
        let video = parse_ffprobe_json(&older).unwrap().video.unwrap();
        assert_eq!(
            (video.width, video.height, video.rotation),
            (1080, 1920, 90)
        );
    }

    #[test]
    fn normalizes_rotations() {
        assert_eq!(normalize_rotation(0.0), 0);
        assert_eq!(normalize_rotation(-90.0), 90);
        assert_eq!(normalize_rotation(90.0), 270);
        assert_eq!(normalize_rotation(180.0), 180);
        assert_eq!(normalize_rotation(-180.0), 180);
        assert_eq!(normalize_rotation(270.0), 90);
    }

    #[test]
    fn refuses_a_file_without_sound() {
        let json = r#"{ "streams": [{ "codec_type": "video", "width": 640, "height": 360,
                                       "avg_frame_rate": "25/1" }],
                         "format": { "duration": "60.0" } }"#;
        assert_eq!(
            parse_ffprobe_json(json).unwrap_err(),
            "this file has no sound"
        );
    }

    #[test]
    fn rejects_missing_duration_and_bad_json() {
        let json =
            r#"{ "streams": [{ "codec_type": "audio", "sample_rate": "48000", "channels": 1 }] }"#;
        assert_eq!(
            parse_ffprobe_json(json).unwrap_err(),
            "the file has no duration"
        );
        assert!(parse_ffprobe_json("not json").is_err());
    }

    #[test]
    fn rejects_missing_file() {
        let err = probe(Path::new("/no/such/file.wav")).unwrap_err();
        assert!(err.contains("doesn't exist"), "{err}");
    }

    #[test]
    fn parses_rates() {
        assert_eq!(parse_rate("60/1"), Some(60.0));
        assert_eq!(parse_rate("0/0"), None);
        assert_eq!(parse_rate("30/0"), None);
        assert_eq!(parse_rate("abc"), None);
    }

    #[test]
    fn the_probe_command_is_lgpl() {
        assert_eq!(ffmpeg::gpl_only(&probe_args()), None);
    }

    /// Needs ffprobe: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn probes_the_fixtures() {
        let wav = probe(&fixture("hello-mono.wav")).unwrap();
        assert_eq!((wav.audio.sample_rate, wav.audio.channels), (48_000, 1));
        assert_eq!(wav.audio.codec, "pcm_s16le");
        assert_eq!(wav.video, None);
        assert!((4_500..5_500).contains(&wav.duration_ms), "{wav:?}");

        let m4a = probe(&fixture("hello-stereo.m4a")).unwrap();
        assert_eq!((m4a.audio.sample_rate, m4a.audio.channels), (44_100, 2));

        let mp4 = probe(&fixture("hello-late.mp4")).unwrap();
        let video = mp4.video.as_ref().unwrap();
        assert_eq!((video.width, video.height, video.fps), (640, 360, 25.0));
        assert_eq!(mp4.duration_ms, 5_000);
        // The audio starts ~0.5 s after the picture (AAC priming makes it 0.48 s).
        assert!((450..=520).contains(&mp4.audio_start_ms), "{mp4:?}");
    }

    /// Needs ffmpeg and ffprobe: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn refuses_a_generated_video_without_sound() {
        let dir = std::env::temp_dir().join("kreokit-voice-probe-test");
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("silent.mp4");
        let status = Command::new(ffmpeg::locate("ffmpeg").unwrap())
            .args(["-y", "-v", "error", "-f", "lavfi"])
            .args(["-i", "testsrc=duration=1:size=320x240:rate=24"])
            .args(["-pix_fmt", "yuv420p", "-c:v", "mpeg4"])
            .arg(&clip)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(probe(&clip).unwrap_err(), "this file has no sound");
    }

    /// Needs ffprobe: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn rejects_a_file_that_is_not_media() {
        let err = probe(&fixture("hello.txt")).unwrap_err();
        assert!(err.contains("can't be read as audio or video"), "{err}");
    }
}
