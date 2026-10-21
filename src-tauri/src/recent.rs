//! Recent files for the Home screen (from KreoKit Cutout).
//!
//! The list is `recent.json` in the app data dir, newest first. Each entry keeps what Home
//! shows (length, audio or video) so the list never has to probe the files again.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How many files the Home screen keeps (their preview caches are kept too).
pub const MAX_RECENT: usize = 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecentFile {
    pub path: String,
    /// Last opened, in Unix milliseconds.
    pub opened_at: u64,
    pub duration_ms: u64,
    pub is_video: bool,
    /// The file's own settings, kept between sessions.
    #[serde(default)]
    pub settings: FileSettings,
}

/// What the editor remembers per file.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FileSettings {
    /// Noise removal strength, 0–1.
    pub strength: f32,
}

impl Default for FileSettings {
    fn default() -> Self {
        Self {
            strength: crate::clean::DEFAULT_STRENGTH,
        }
    }
}

/// A list entry plus what the Home screen needs to show it.
#[derive(Debug, Clone, Serialize)]
pub struct RecentEntry {
    #[serde(flatten)]
    pub file: RecentFile,
    /// The file still exists.
    pub exists: bool,
}

fn list_file(dir: &Path) -> PathBuf {
    dir.join("recent.json")
}

/// The stored list, newest first. A missing or damaged file is an empty list.
pub fn load(dir: &Path) -> Vec<RecentFile> {
    fs::read_to_string(list_file(dir))
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn store(dir: &Path, list: &[RecentFile]) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    // Written next to the list and renamed over it, so a crash never leaves half a file.
    let temp = dir.join("recent.json.tmp");
    fs::write(&temp, json).map_err(|e| format!("cannot save recent files: {e}"))?;
    fs::rename(&temp, list_file(dir)).map_err(|e| format!("cannot save recent files: {e}"))
}

/// The list with each file's existence checked.
pub fn entries(dir: &Path) -> Vec<RecentEntry> {
    load(dir)
        .into_iter()
        .map(|file| RecentEntry {
            exists: Path::new(&file.path).is_file(),
            file,
        })
        .collect()
}

/// Puts `files` at the top of the list in the given order (adding or moving them),
/// capped at [`MAX_RECENT`]. Returns the list.
pub fn touch(dir: &Path, mut files: Vec<RecentFile>) -> Result<Vec<RecentFile>, String> {
    let mut list = load(dir);
    // A file opened again keeps its settings.
    for file in &mut files {
        if let Some(old) = list.iter().find(|f| f.path == file.path) {
            file.settings = old.settings;
        }
    }
    list.retain(|f| !files.iter().any(|new| new.path == f.path));
    list.splice(0..0, files);
    list.truncate(MAX_RECENT);
    store(dir, &list)?;
    Ok(list)
}

/// A file's settings (the defaults for a file not in the list).
pub fn settings(dir: &Path, path: &str) -> FileSettings {
    load(dir)
        .into_iter()
        .find(|f| f.path == path)
        .map(|f| f.settings)
        .unwrap_or_default()
}

pub fn save_settings(dir: &Path, path: &str, settings: FileSettings) -> Result<(), String> {
    let mut list = load(dir);
    if let Some(file) = list.iter_mut().find(|f| f.path == path) {
        file.settings = settings;
        store(dir, &list)?;
    }
    Ok(())
}

pub fn remove(dir: &Path, path: &str) -> Result<Vec<RecentFile>, String> {
    let mut list = load(dir);
    list.retain(|f| f.path != path);
    store(dir, &list)?;
    Ok(list)
}

/// A moved file found again: `old` is replaced by `new` in place, keeping its position.
pub fn relocate(dir: &Path, old: &str, new: RecentFile) -> Result<Vec<RecentFile>, String> {
    let mut list = load(dir);
    list.retain(|f| f.path != new.path || f.path == old);
    match list.iter_mut().find(|f| f.path == old) {
        Some(entry) => {
            let settings = entry.settings;
            *entry = RecentFile { settings, ..new }
        }
        None => list.insert(0, new),
    }
    list.truncate(MAX_RECENT);
    store(dir, &list)?;
    Ok(list)
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("kreokit-voice-recent-tests")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn file(path: &str, at: u64) -> RecentFile {
        RecentFile {
            path: path.into(),
            opened_at: at,
            duration_ms: 1_000,
            is_video: false,
            settings: FileSettings::default(),
        }
    }

    fn paths(dir: &Path) -> Vec<String> {
        load(dir).into_iter().map(|f| f.path).collect()
    }

    #[test]
    fn newest_first_without_duplicates() {
        let dir = temp_dir("order");
        touch(&dir, vec![file("/a.wav", 1)]).unwrap();
        touch(&dir, vec![file("/b.wav", 2)]).unwrap();
        touch(&dir, vec![file("/a.wav", 3)]).unwrap();
        assert_eq!(paths(&dir), ["/a.wav", "/b.wav"]);
        assert_eq!(load(&dir)[0].opened_at, 3);
    }

    #[test]
    fn a_batch_keeps_its_order_at_the_top() {
        let dir = temp_dir("batch");
        touch(&dir, vec![file("/old.wav", 1), file("/b.wav", 1)]).unwrap();
        touch(
            &dir,
            vec![file("/a.wav", 2), file("/b.wav", 2), file("/c.wav", 2)],
        )
        .unwrap();
        assert_eq!(paths(&dir), ["/a.wav", "/b.wav", "/c.wav", "/old.wav"]);
    }

    #[test]
    fn keeps_at_most_twenty() {
        let dir = temp_dir("cap");
        for i in 0..30 {
            touch(&dir, vec![file(&format!("/{i}.wav"), i)]).unwrap();
        }
        let list = load(&dir);
        assert_eq!(list.len(), MAX_RECENT);
        assert_eq!(list[0].path, "/29.wav");
    }

    #[test]
    fn relocates_in_place() {
        let dir = temp_dir("relocate");
        touch(
            &dir,
            vec![
                file("/a.wav", 1),
                file("/moved.wav", 1),
                file("/new.wav", 1),
            ],
        )
        .unwrap();
        relocate(&dir, "/moved.wav", file("/new.wav", 2)).unwrap();
        assert_eq!(paths(&dir), ["/a.wav", "/new.wav"]);
    }

    #[test]
    fn keeps_settings_per_file() {
        let dir = temp_dir("settings");
        touch(&dir, vec![file("/a.wav", 1)]).unwrap();
        assert_eq!(
            settings(&dir, "/a.wav").strength,
            crate::clean::DEFAULT_STRENGTH
        );
        save_settings(&dir, "/a.wav", FileSettings { strength: 0.5 }).unwrap();
        // Opened again, and moved: still 0.5. An older list without settings reads defaults.
        touch(&dir, vec![file("/a.wav", 2)]).unwrap();
        relocate(&dir, "/a.wav", file("/moved/a.wav", 3)).unwrap();
        assert_eq!(settings(&dir, "/moved/a.wav").strength, 0.5);
        fs::write(
            list_file(&dir),
            r#"[{"path":"/old.wav","opened_at":1,"duration_ms":5,"is_video":false}]"#,
        )
        .unwrap();
        assert_eq!(settings(&dir, "/old.wav"), FileSettings::default());
    }

    #[test]
    fn marks_missing_files_removes_and_survives_damage() {
        let dir = temp_dir("remove");
        touch(&dir, vec![file("/no/such/file.wav", 1)]).unwrap();
        assert!(!entries(&dir)[0].exists);
        remove(&dir, "/no/such/file.wav").unwrap();
        assert!(load(&dir).is_empty());
        fs::write(list_file(&dir), "not json").unwrap();
        assert!(load(&dir).is_empty());
    }
}
