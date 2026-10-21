//! Locating the FFmpeg and ffprobe binaries.
//!
//! Lookup order:
//! 1. `KREOKIT_FFMPEG` / `KREOKIT_FFPROBE` environment variables (explicit override).
//! 2. A bundled sidecar next to the app executable. Release builds declare the binaries in
//!    `tauri.release.conf.json` (`bundle.externalBin`); Tauri copies them next to the
//!    executable with the target-triple suffix stripped.
//! 3. The system install: `PATH`, then common install dirs. Apps launched from Finder
//!    don't inherit the shell `PATH`, so those dirs are checked explicitly.
//!
//! In development (step 3) the system FFmpeg is used, which is often a GPL build. Release
//! builds bundle an LGPL FFmpeg (no GPL parts) as the sidecar, as KreoKit Captions and Clips
//! do, so every command Voice runs must work without GPL-only filters and encoders:
//! [`gpl_only`] finds them, and each module's tests run their commands through it.

use std::env;
use std::path::{Path, PathBuf};

/// Directories checked after `PATH`.
const FALLBACK_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "~/.local/bin"];

/// Returns the path of `tool` ("ffmpeg" or "ffprobe").
pub fn locate(tool: &str) -> Result<PathBuf, String> {
    let env_var = format!("KREOKIT_{}", tool.to_uppercase());
    if let Some(path) = env::var_os(&env_var).map(PathBuf::from) {
        return if path.is_file() {
            Ok(path)
        } else {
            Err(format!(
                "{env_var} points to {}, which does not exist",
                path.display()
            ))
        };
    }

    let file_name = format!("{tool}{}", env::consts::EXE_SUFFIX);

    let sidecar_dir = env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let path_dirs = env::var_os("PATH")
        .map(|p| env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    let fallback_dirs = FALLBACK_DIRS.iter().filter_map(|d| expand_home(d));

    sidecar_dir
        .into_iter()
        .chain(path_dirs)
        .chain(fallback_dirs)
        .map(|dir| dir.join(&file_name))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            format!("{tool} not found. Install FFmpeg or set {env_var} to the {tool} binary.")
        })
}

fn expand_home(dir: &str) -> Option<PathBuf> {
    match dir.strip_prefix("~/") {
        Some(rest) => env::var_os("HOME").map(|home| PathBuf::from(home).join(rest)),
        None => Some(PathBuf::from(dir)),
    }
}

#[cfg(test)]
/// FFmpeg 9.0.2's GPL-only filters (`grep 'filter_deps="gpl' configure`; the list from
/// KreoKit Clips). None of them is an audio filter today; the list guards against one being
/// picked by mistake (e.g. while trying filters in a dev build with the system's GPL FFmpeg).
const GPL_FILTERS: &[&str] = &[
    "blackframe",
    "boxblur",
    "boxblur_opencl",
    "colormatrix",
    "cover_rect",
    "cropdetect",
    "delogo",
    "eq",
    "find_rect",
    "fspp",
    "histeq",
    "hqdn3d",
    "interlace",
    "kerndeint",
    "mcdeint",
    "mpdecimate",
    "mptestsrc",
    "nnedi",
    "owdenoise",
    "perspective",
    "phase",
    "pp7",
    "pullup",
    "repeatfields",
    "sab",
    "signature",
    "smartblur",
    "spp",
    "stereo3d",
    "super2xsai",
    "tinterlace",
    "uspp",
    "vaguedenoiser",
];

#[cfg(test)]
/// Encoders that need `--enable-gpl` or `--enable-nonfree`.
const GPL_ENCODERS: &[&str] = &["libx264", "libx264rgb", "libx265", "libxvid", "libfdk_aac"];

#[cfg(test)]
/// The first GPL-only filter or encoder named anywhere in an FFmpeg command's arguments,
/// or `None` when the command runs on our LGPL release build.
pub fn gpl_only<S: AsRef<str>>(args: &[S]) -> Option<&'static str> {
    args.iter()
        .flat_map(|arg| {
            arg.as_ref()
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .collect::<Vec<_>>()
        })
        .find_map(|word| {
            GPL_FILTERS
                .iter()
                .chain(GPL_ENCODERS)
                .find(|name| **name == word)
                .copied()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_gpl_only_filters_and_encoders() {
        assert_eq!(
            gpl_only(&["-vf", "scale=640:-2,boxblur=5"]),
            Some("boxblur")
        );
        assert_eq!(gpl_only(&["-c:v", "libx264"]), Some("libx264"));
        assert_eq!(gpl_only(&["-af", "eq=contrast=2"]), Some("eq"));
        // Whole words only: "equalizer" and "aeval" are LGPL audio filters.
        assert_eq!(
            gpl_only(&["-af", "equalizer=f=100:t=q:w=1:g=-3,aeval=val(0)"]),
            None
        );
        assert_eq!(gpl_only(&["-af", "aresample=async=1:first_pts=0"]), None);
        assert_eq!(gpl_only::<&str>(&[]), None);
    }
}
