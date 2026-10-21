//! The Best model's one-time download (from KreoKit Cutout's `models.rs`).
//!
//! Fast (RNNoise) is compiled into the app. Best (DPDFNet-2, 48 kHz, Apache-2.0) is
//! downloaded only from our own pre-release `models-v1` on kreokit/voice, by tag (never
//! `/releases/latest`), into `<app data>/models/`, and checked against a pinned size and
//! SHA-256 (the release's SHA256SUMS). Sources, licence and training data: the release's
//! SOURCE.md and THIRD-PARTY-LICENSES.md. This download is the app's only network call.

use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use sha2::{Digest, Sha256};

pub const CANCELLED: &str = "cancelled";

const RELEASES: &str = "https://github.com/kreokit/voice/releases/download";

/// DPDFNet-2 48 kHz (`models-v1`).
pub const BEST_FILE: &str = "dpdfnet2_48khz_hr.onnx";
const BEST_SIZE: u64 = 10_493_337;
const BEST_SHA256: &str = "7f0575a5cec0ba4ffd8f8bd657e06d007e4ccdd955d76faab922b9d3291dc14b";

fn url() -> String {
    format!("{RELEASES}/models-v1/{BEST_FILE}")
}

pub fn best_path(models_dir: &Path) -> PathBuf {
    models_dir.join(BEST_FILE)
}

/// Downloads land in a `.part` file and are only renamed once the checksum matches, so a
/// complete file is a checked one.
pub fn best_downloaded(models_dir: &Path) -> bool {
    fs::metadata(best_path(models_dir)).is_ok_and(|m| m.len() == BEST_SIZE)
}

#[derive(Debug, Clone, Serialize)]
pub struct BestModel {
    pub size_bytes: u64,
    pub downloaded: bool,
    /// The CPU can run it (ONNX Runtime needs x86-64-v2), and what it lacks if not.
    pub cpu: crate::cpu::CpuSupport,
}

pub fn best_model(models_dir: &Path) -> BestModel {
    BestModel {
        size_bytes: BEST_SIZE,
        downloaded: best_downloaded(models_dir),
        cpu: crate::cpu::check(),
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

/// Downloads the Best model into `models_dir`, verifying size and SHA-256. `on_progress`
/// is called roughly every 1 MB. Returns [`CANCELLED`] if `cancel` is set.
pub fn download_best(
    models_dir: &Path,
    cancel: &AtomicBool,
    on_progress: impl FnMut(DownloadProgress),
) -> Result<PathBuf, String> {
    let target = best_path(models_dir);
    if best_downloaded(models_dir) {
        return Ok(target);
    }
    fs::create_dir_all(models_dir).map_err(|e| format!("cannot create models folder: {e}"))?;
    let part = target.with_extension("onnx.part");

    let result = (|| {
        let response = ureq::get(&url())
            .call()
            .map_err(|e| format!("model download failed: {e}"))?;
        let reader = response.into_body().into_reader();
        let file = File::create(&part).map_err(|e| format!("cannot write model file: {e}"))?;
        save_verified(reader, file, BEST_SIZE, BEST_SHA256, cancel, on_progress).map_err(|e| {
            match e.as_str() {
                CORRUPT => "the downloaded model is damaged; please try again".to_string(),
                _ => e,
            }
        })?;
        fs::rename(&part, &target).map_err(|e| format!("cannot save model file: {e}"))?;
        Ok(target.clone())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

const CORRUPT: &str = "corrupt";

/// Copies `reader` into `file`, reporting progress, and checks the size and SHA-256 at
/// the end ([`CORRUPT`] if they don't match).
fn save_verified(
    mut reader: impl Read,
    file: File,
    total_bytes: u64,
    sha256: &str,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(DownloadProgress),
) -> Result<(), String> {
    let mut file = BufWriter::new(file);
    let mut hasher = Sha256::new();
    let mut downloaded_bytes = 0u64;
    let mut last_reported = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.to_string());
        }
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("model download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])
            .map_err(|e| format!("cannot write model file: {e}"))?;
        downloaded_bytes += n as u64;
        if downloaded_bytes - last_reported >= 1 << 20 || downloaded_bytes == total_bytes {
            last_reported = downloaded_bytes;
            on_progress(DownloadProgress {
                downloaded_bytes,
                total_bytes,
            });
        }
    }
    file.flush()
        .map_err(|e| format!("cannot write model file: {e}"))?;
    if downloaded_bytes != total_bytes || to_hex(&hasher.finalize()) != sha256 {
        return Err(CORRUPT.into());
    }
    Ok(())
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("kreokit-voice-models-tests")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn downloads_only_from_our_mirror_by_tag() {
        assert_eq!(
            url(),
            "https://github.com/kreokit/voice/releases/download/models-v1/dpdfnet2_48khz_hr.onnx"
        );
        assert!(!url().contains("latest"));
    }

    #[test]
    fn partial_file_is_not_downloaded() {
        let dir = temp_dir("partial");
        fs::write(best_path(&dir), b"half").unwrap();
        assert!(!best_downloaded(&dir));
        assert!(!best_model(&dir).downloaded);
    }

    #[test]
    fn verifies_size_and_checksum() {
        let dir = temp_dir("verify");
        let data = b"hello model";
        let wrong = "a1c8ae6c0f2b2dd3b6f5c64a4eb24a8d1c6c3bd8a2bf1a2e4d1ddc6b0b5c6f45";
        let good = to_hex(&Sha256::digest(data));
        let cancel = AtomicBool::new(false);
        let mut reports = Vec::new();
        let file = || File::create(dir.join("m.part")).unwrap();
        save_verified(&data[..], file(), 11, &good, &cancel, |p| {
            reports.push(p.downloaded_bytes)
        })
        .unwrap();
        assert_eq!(reports, [11]);
        assert_eq!(fs::read(dir.join("m.part")).unwrap(), data);
        assert_eq!(
            save_verified(&data[..], file(), 11, wrong, &cancel, |_| {}).unwrap_err(),
            CORRUPT
        );
        assert_eq!(
            save_verified(&data[..], file(), 12, &good, &cancel, |_| {}).unwrap_err(),
            CORRUPT
        );
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(
            save_verified(&data[..], file(), 11, &good, &cancel, |_| {}).unwrap_err(),
            CANCELLED
        );
    }

    #[test]
    fn hex_encodes() {
        assert_eq!(to_hex(&[0x00, 0x0f, 0xab]), "000fab");
    }

    /// The Best model for the tests that need it: downloaded once from the mirror into
    /// a lasting temp folder (or `KREOKIT_MODELS_DIR`). CI runs these ignored tests, so the
    /// published mirror and the pinned checksum are checked on every push.
    pub(crate) fn models_dir_for_tests() -> PathBuf {
        // Tests run in parallel: one download at a time into the shared folder.
        static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::var_os("KREOKIT_MODELS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("kreokit-voice-test-models"));
        download_best(&dir, &AtomicBool::new(false), |_| {}).unwrap();
        dir
    }

    /// `cargo test downloads_the_best_model -- --ignored`
    #[test]
    #[ignore]
    fn downloads_the_best_model() {
        assert!(best_downloaded(&models_dir_for_tests()));
    }
}
