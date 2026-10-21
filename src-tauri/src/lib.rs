mod disk;
mod ffmpeg;
mod player;
mod preview;
mod probe;
mod recent;
mod rnnoise;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// Runs blocking work (FFmpeg, file I/O) off the main thread.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
}

/// Where `recent.json` lives.
fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|e| e.to_string())
}

/// Where the preview caches live (one folder per file, see `preview.rs`).
fn preview_root(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("preview"))
}

/// Keeps only the preview caches of files still in the recent list.
fn prune_previews(app: &AppHandle, list: &[recent::RecentFile]) -> Result<(), String> {
    let root = preview_root(app)?;
    let keep: Vec<PathBuf> = list
        .iter()
        .map(|f| preview::dir_for(&root, Path::new(&f.path)))
        // The file being played, even if it just left the list.
        .chain(app.state::<player::Player>().loaded())
        .collect();
    preview::prune(&root, &keep);
    Ok(())
}

fn recent_file(path: &Path, info: &probe::MediaInfo) -> recent::RecentFile {
    recent::RecentFile {
        path: path.to_string_lossy().into_owned(),
        opened_at: recent::now_ms(),
        duration_ms: info.duration_ms,
        is_video: info.video.is_some(),
    }
}

/// What an audio or video file contains (refused when it has no sound).
#[tauri::command]
async fn probe_media(path: PathBuf) -> Result<probe::MediaInfo, String> {
    blocking(move || probe::probe(&path)).await
}

#[tauri::command]
async fn list_recent(app: AppHandle) -> Result<Vec<recent::RecentEntry>, String> {
    let dir = data_dir(&app)?;
    blocking(move || Ok(recent::entries(&dir))).await
}

/// One dropped or chosen file: its contents, or why it can't be used.
#[derive(Serialize)]
struct AddedFile {
    path: PathBuf,
    info: Option<probe::MediaInfo>,
    error: Option<String>,
}

/// Probes `paths` and puts the usable ones at the top of the recent list, in order.
#[tauri::command]
async fn add_files(app: AppHandle, paths: Vec<PathBuf>) -> Result<Vec<AddedFile>, String> {
    let dir = data_dir(&app)?;
    let added = blocking(move || {
        let added: Vec<AddedFile> = paths
            .into_iter()
            .map(|path| match probe::probe(&path) {
                Ok(info) => AddedFile {
                    path,
                    info: Some(info),
                    error: None,
                },
                Err(e) => AddedFile {
                    path,
                    info: None,
                    error: Some(e),
                },
            })
            .collect();
        let usable = added
            .iter()
            .filter_map(|a| Some(recent_file(&a.path, a.info.as_ref()?)))
            .collect::<Vec<_>>();
        if !usable.is_empty() {
            recent::touch(&dir, usable)?;
        }
        Ok(added)
    })
    .await?;
    prune_previews(&app, &recent::load(&data_dir(&app)?))?;
    Ok(added)
}

/// Removes a file from the list (and its preview cache). The file itself is not touched.
#[tauri::command]
async fn remove_recent(app: AppHandle, path: String) -> Result<(), String> {
    let list = recent::remove(&data_dir(&app)?, &path)?;
    prune_previews(&app, &list)
}

/// A moved file found again at `new`: it takes the old entry's place.
#[tauri::command]
async fn relocate_recent(
    app: AppHandle,
    old: String,
    new: PathBuf,
) -> Result<probe::MediaInfo, String> {
    let dir = data_dir(&app)?;
    let (info, list) = blocking(move || {
        let info = probe::probe(&new)?;
        let list = recent::relocate(&dir, &old, recent_file(&new, &info))?;
        Ok((info, list))
    })
    .await?;
    prune_previews(&app, &list)?;
    Ok(info)
}

/// The running preview decode's cancel flag; a new one cancels the previous.
#[derive(Default)]
struct PreviewJob(Mutex<Option<Arc<AtomicBool>>>);

#[derive(Clone, Serialize)]
struct PreviewProgress {
    path: PathBuf,
    fraction: f64,
}

/// Decodes `path` into its preview cache (or finds it), with `preview-progress` events.
#[tauri::command]
async fn prepare_preview(
    app: AppHandle,
    path: PathBuf,
    channels: u32,
    duration_ms: u64,
) -> Result<preview::Meta, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    if let Some(previous) = app
        .state::<PreviewJob>()
        .0
        .lock()
        .unwrap()
        .replace(cancel.clone())
    {
        previous.store(true, Ordering::SeqCst);
    }
    let dir = preview::dir_for(&preview_root(&app)?, &path);
    // A cache is rewritten only when it isn't mapped by the player.
    let player = app.state::<player::Player>();
    if player.loaded().as_ref() == Some(&dir) {
        player.unload();
    }
    let events = app.clone();
    blocking(move || {
        preview::prepare(&dir, &path, channels, duration_ms, &cancel, |fraction| {
            let _ = events.emit(
                "preview-progress",
                PreviewProgress {
                    path: path.clone(),
                    fraction,
                },
            );
        })
    })
    .await
}

#[tauri::command]
fn cancel_preview(app: AppHandle) {
    if let Some(job) = app.state::<PreviewJob>().0.lock().unwrap().take() {
        job.store(true, Ordering::SeqCst);
    }
}

/// The prepared file's waveform: one byte per 10 ms (`preview::level`).
#[tauri::command]
async fn waveform(app: AppHandle, path: PathBuf) -> Result<tauri::ipc::Response, String> {
    let dir = preview::dir_for(&preview_root(&app)?, &path);
    blocking(move || preview::peaks(&dir).map(tauri::ipc::Response::new)).await
}

/// Loads `path`'s finished preview into the player, stopped at the start.
#[tauri::command]
async fn player_open(app: AppHandle, path: PathBuf) -> Result<player::PlayerState, String> {
    let dir = preview::dir_for(&preview_root(&app)?, &path);
    let meta = preview::cached(&dir, &path).ok_or("the preview isn't ready")?;
    let pcm = player::Pcm::open(&preview::audio_path(&dir), meta.channels)?;
    let player = app.state::<player::Player>();
    player.load(pcm, dir);
    Ok(player.state())
}

#[tauri::command]
fn player_close(player: tauri::State<player::Player>) {
    player.unload();
}

#[tauri::command]
fn player_play(player: tauri::State<player::Player>) -> Result<player::PlayerState, String> {
    player.play()?;
    Ok(player.state())
}

#[tauri::command]
fn player_pause(player: tauri::State<player::Player>) -> player::PlayerState {
    player.pause();
    player.state()
}

#[tauri::command]
fn player_seek(player: tauri::State<player::Player>, ms: f64) -> player::PlayerState {
    player.seek_ms(ms);
    player.state()
}

#[tauri::command]
fn player_state(player: tauri::State<player::Player>) -> player::PlayerState {
    player.state()
}

/// Development aid: a file to open at startup, from `KREOKIT_DEV_OPEN`. Always `None` in
/// release builds.
#[tauri::command]
fn dev_startup_file() -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var("KREOKIT_DEV_OPEN")
        .ok()
        .filter(|p| !p.is_empty())
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// Where testers report problems: the bug-report form, with the app version filled in.
fn report_problem_url() -> String {
    format!(
        "https://github.com/kreokit/voice/issues/new?template=bug_report.yml&version={}",
        env!("CARGO_PKG_VERSION")
    )
}

/// Opens the bug-report page in the default browser. Only this fixed page can be opened;
/// the webview gets no general permission to open links.
#[tauri::command]
fn report_problem(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(report_problem_url(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(PreviewJob::default())
        .manage(player::Player::default())
        .invoke_handler(tauri::generate_handler![
            probe_media,
            list_recent,
            add_files,
            remove_recent,
            relocate_recent,
            prepare_preview,
            cancel_preview,
            waveform,
            player_open,
            player_close,
            player_play,
            player_pause,
            player_seek,
            player_state,
            dev_startup_file,
            quit_app,
            report_problem
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Quitting stops the decode, so its FFmpeg ends with the app.
            if let tauri::RunEvent::Exit = event {
                cancel_preview(app.clone());
                app.state::<player::Player>().unload();
            }
        });
}

#[cfg(test)]
mod tests {
    #[test]
    fn report_problem_opens_the_bug_form() {
        assert_eq!(
            super::report_problem_url(),
            format!(
                "https://github.com/kreokit/voice/issues/new?template=bug_report.yml&version={}",
                env!("CARGO_PKG_VERSION")
            )
        );
    }
}
