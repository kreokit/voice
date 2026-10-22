//! How fast exports run here, per kind of work, for "time left" in a batch (the user's
//! request, 2026-10-22): seconds of work per second of sound, learned from each finished
//! export (an average that follows the last few), starting from what the dev Mac measured.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::clean::Model;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Speeds {
    /// Fast (RNNoise) + Polish.
    pub fast: f64,
    /// Best (DPDFNet) + Polish.
    pub best: f64,
    /// Polish only (the file wasn't cleaned).
    pub polish: f64,
}

impl Default for Speeds {
    fn default() -> Self {
        // Whole exports on the dev Mac's i5 (dev build, short files; stereo): Fast ~0.3,
        // Best's cleaning alone ~0.43, Polish ~0.2.
        Self {
            fast: 0.3,
            best: 0.7,
            polish: 0.2,
        }
    }
}

impl Speeds {
    #[cfg(test)]
    pub fn for_model(&self, model: Option<Model>) -> f64 {
        match model {
            Some(Model::Fast) => self.fast,
            Some(Model::Best) => self.best,
            None => self.polish,
        }
    }

    /// Learns from a finished export (half old, half new).
    pub fn learn(&mut self, model: Option<Model>, realtime_factor: f64) {
        if !realtime_factor.is_finite() || realtime_factor <= 0.0 {
            return;
        }
        let slot = match model {
            Some(Model::Fast) => &mut self.fast,
            Some(Model::Best) => &mut self.best,
            None => &mut self.polish,
        };
        *slot = 0.5 * *slot + 0.5 * realtime_factor;
    }
}

fn file(dir: &Path) -> std::path::PathBuf {
    dir.join("speeds.json")
}

pub fn load(dir: &Path) -> Speeds {
    fs::read_to_string(file(dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn store(dir: &Path, speeds: &Speeds) {
    if let Ok(json) = serde_json::to_string_pretty(speeds) {
        let _ = fs::create_dir_all(dir);
        let _ = fs::write(file(dir), json);
    }
}

/// The folders exports went to (the latest 20), so leftovers of a crash can be removed
/// there at the next start.
pub fn remember_folder(dir: &Path, folder: &Path) {
    let path = dir.join("export-folders.json");
    let mut list: Vec<String> = fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let folder = folder.to_string_lossy().into_owned();
    list.retain(|f| f != &folder);
    list.insert(0, folder);
    list.truncate(20);
    if let Ok(json) = serde_json::to_string_pretty(&list) {
        let _ = fs::write(path, json);
    }
}

pub fn export_folders(dir: &Path) -> Vec<std::path::PathBuf> {
    fs::read_to_string(dir.join("export-folders.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
        .into_iter()
        .map(Into::into)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_and_remembers() {
        let dir = std::env::temp_dir().join("kreokit-voice-speeds");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut s = load(&dir);
        assert_eq!(s, Speeds::default());
        s.learn(Some(Model::Best), 0.5);
        assert!((s.for_model(Some(Model::Best)) - 0.6).abs() < 1e-9);
        s.learn(None, f64::NAN);
        assert_eq!(s.polish, Speeds::default().polish);
        store(&dir, &s);
        assert_eq!(load(&dir), s);
        remember_folder(&dir, Path::new("/a"));
        remember_folder(&dir, Path::new("/b"));
        remember_folder(&dir, Path::new("/a"));
        assert_eq!(
            export_folders(&dir),
            vec![std::path::PathBuf::from("/a"), "/b".into()]
        );
    }
}
