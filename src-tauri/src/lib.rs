mod analysis;
mod awake;
mod clean;
mod cpu;
mod disk;
mod dpdfnet;
mod export;
mod ffmpeg;
mod loudness;
mod metrics;
mod models;
mod player;
mod polish;
mod preview;
mod probe;
mod recent;
mod rnnoise;
mod selftest;
mod speeds;
mod testsignals;

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

/// Where the Best model is downloaded.
fn models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("models"))
}

/// Where the preview caches live (one folder per file, see `preview.rs`).
fn preview_root(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("preview"))
}

/// Keeps only the preview caches of files still in the recent list; returns the bytes
/// freed.
fn prune_previews(app: &AppHandle, list: &[recent::RecentFile]) -> Result<u64, String> {
    let root = preview_root(app)?;
    let keep: Vec<PathBuf> = list
        .iter()
        .map(|f| preview::dir_for(&root, Path::new(&f.path)))
        // The file being played, even if it just left the list.
        .chain(app.state::<player::Player>().loaded())
        .collect();
    Ok(preview::prune(&root, &keep))
}

/// Stops everything that uses `dir`'s cache (the player and the jobs), so it can be
/// deleted; `None` stops them whatever file they're on.
fn release(app: &AppHandle, dir: Option<&Path>) {
    let player = app.state::<player::Player>();
    if dir.is_none() || player.loaded().as_deref() == dir {
        player.unload();
        cancel_preview(app.clone());
        cancel_clean(app.clone());
        cancel_polish(app.clone());
    }
}

fn recent_file(path: &Path, info: &probe::MediaInfo) -> recent::RecentFile {
    recent::RecentFile {
        path: path.to_string_lossy().into_owned(),
        opened_at: recent::now_ms(),
        duration_ms: info.duration_ms,
        is_video: info.video.is_some(),
        settings: recent::FileSettings::default(),
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

/// Removes a file from Recent: its list entry (with its settings) and its preview,
/// waveform and cleaned sounds. The file itself is never touched. Returns the bytes freed.
#[tauri::command]
async fn remove_recent(app: AppHandle, path: String) -> Result<u64, String> {
    let dir = preview::dir_for(&preview_root(&app)?, Path::new(&path));
    release(&app, Some(&dir));
    let list = recent::remove(&data_dir(&app)?, &path)?;
    let app2 = app.clone();
    blocking(move || prune_previews(&app2, &list)).await
}

/// Removes every file from Recent (as `remove_recent`). Returns the bytes freed.
#[tauri::command]
async fn clear_recent(app: AppHandle) -> Result<u64, String> {
    release(&app, None);
    recent::clear(&data_dir(&app)?)?;
    let app2 = app.clone();
    blocking(move || prune_previews(&app2, &[])).await
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

/// The running cleaning job's cancel flag; a new one cancels the previous.
#[derive(Default)]
struct CleanJob(Mutex<Option<Arc<AtomicBool>>>);

/// The Best model: its size, whether it is downloaded, and whether the CPU can run it.
#[tauri::command]
async fn best_model(app: AppHandle) -> Result<models::BestModel, String> {
    let dir = models_dir(&app)?;
    blocking(move || Ok(models::best_model(&dir))).await
}

/// The running Best model download's cancel flag.
#[derive(Default)]
struct ModelJob(Mutex<Option<Arc<AtomicBool>>>);

/// Downloads the Best model once, with `model-progress` events.
#[tauri::command]
async fn download_best_model(app: AppHandle) -> Result<(), String> {
    let cancel = Arc::new(AtomicBool::new(false));
    if let Some(previous) = app
        .state::<ModelJob>()
        .0
        .lock()
        .unwrap()
        .replace(cancel.clone())
    {
        previous.store(true, Ordering::SeqCst);
    }
    let dir = models_dir(&app)?;
    let events = app.clone();
    blocking(move || {
        models::download_best(&dir, &cancel, |p| {
            let _ = events.emit("model-progress", p);
        })
        .map(|_| ())
    })
    .await
}

#[tauri::command]
fn cancel_model_download(app: AppHandle) {
    if let Some(job) = app.state::<ModelJob>().0.lock().unwrap().take() {
        job.store(true, Ordering::SeqCst);
    }
}

/// Whether `path` already has a sound cleaned with `model` (from an earlier session).
#[tauri::command]
async fn clean_status(
    app: AppHandle,
    path: PathBuf,
    model: clean::Model,
) -> Result<Option<clean::CleanMeta>, String> {
    let dir = preview::dir_for(&preview_root(&app)?, &path);
    blocking(move || {
        Ok(preview::cached(&dir, &path).and_then(|preview| clean::cached(&dir, &preview, model)))
    })
    .await
}

/// Removes the noise of `path` with `model` (its preview must be ready), with
/// `clean-progress` events. When `path` is the file being played, the result is loaded into
/// the player.
#[tauri::command]
async fn clean_file(
    app: AppHandle,
    path: PathBuf,
    model: clean::Model,
) -> Result<clean::CleanMeta, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    if let Some(previous) = app
        .state::<CleanJob>()
        .0
        .lock()
        .unwrap()
        .replace(cancel.clone())
    {
        previous.store(true, Ordering::SeqCst);
    }
    let dir = preview::dir_for(&preview_root(&app)?, &path);
    let player = app.state::<player::Player>();
    let playing_this = player.loaded().as_ref() == Some(&dir);
    if playing_this {
        // Its cleaned file may be replaced: release the old mapping.
        player.set_cleaned(None);
    }
    let events = app.clone();
    let models = models_dir(&app)?;
    let (job_dir, job_path) = (dir.clone(), path.clone());
    let meta = blocking(move || {
        let preview = preview::cached(&job_dir, &job_path).ok_or("the preview isn't ready")?;
        let meta = clean::prepare(&job_dir, &preview, model, &models, &cancel, |fraction| {
            let _ = events.emit(
                "clean-progress",
                PreviewProgress {
                    path: job_path.clone(),
                    fraction,
                },
            );
        })?;
        Ok((preview, meta))
    })
    .await;
    let (preview, meta) = meta?;
    if playing_this && player.loaded().as_ref() == Some(&dir) {
        let pcm = player::Pcm::open(&clean::audio_path(&dir, model), preview.channels)?;
        player.set_cleaned(Some(pcm));
    }
    Ok(meta)
}

#[tauri::command]
fn cancel_clean(app: AppHandle) {
    if let Some(job) = app.state::<CleanJob>().0.lock().unwrap().take() {
        job.store(true, Ordering::SeqCst);
    }
}

/// The sound cleaned with `model`: its waveform (like `waveform`).
#[tauri::command]
async fn cleaned_waveform(
    app: AppHandle,
    path: PathBuf,
    model: clean::Model,
) -> Result<tauri::ipc::Response, String> {
    let dir = preview::dir_for(&preview_root(&app)?, &path);
    blocking(move || clean::peaks(&dir, model).map(tauri::ipc::Response::new)).await
}

/// Loads `path`'s sound cleaned with `model` next to its preview in the player (none when
/// there isn't one yet).
#[tauri::command]
async fn player_load_cleaned(
    app: AppHandle,
    path: PathBuf,
    model: clean::Model,
) -> Result<player::PlayerState, String> {
    let dir = preview::dir_for(&preview_root(&app)?, &path);
    let player = app.state::<player::Player>();
    if player.loaded().as_ref() == Some(&dir) {
        let found = preview::cached(&dir, &path)
            .and_then(|p| clean::cached(&dir, &p, model).map(|_| p.channels));
        let pcm = match found {
            Some(channels) => Some(player::Pcm::open(
                &clean::audio_path(&dir, model),
                channels,
            )?),
            None => None,
        };
        player.set_cleaned(pcm);
    }
    Ok(player.state())
}

/// The running Polish measurement's cancel flag; a new one cancels the previous.
#[derive(Default)]
struct PolishJob(Mutex<Option<Arc<AtomicBool>>>);

/// Measures `path`'s sound (cleaned with `cleaned` when given, at `strength`) through the
/// Polish chain with `settings`, with `polish-progress` events. When `path` is playing, the
/// player gets the chain and the original's loudness-matching gain.
#[tauri::command]
async fn measure_polish(
    app: AppHandle,
    path: PathBuf,
    cleaned: Option<clean::Model>,
    strength: f32,
    settings: loudness::PolishSettings,
) -> Result<loudness::Measured, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    if let Some(previous) = app
        .state::<PolishJob>()
        .0
        .lock()
        .unwrap()
        .replace(cancel.clone())
    {
        previous.store(true, Ordering::SeqCst);
    }
    let dir = preview::dir_for(&preview_root(&app)?, &path);
    let events = app.clone();
    let (job_dir, job_path) = (dir.clone(), path.clone());
    let (measured, original_lufs) = blocking(move || {
        let preview = preview::cached(&job_dir, &job_path).ok_or("the preview isn't ready")?;
        let cleaned = cleaned.filter(|&m| clean::cached(&job_dir, &preview, m).is_some());
        let original_lufs = preview.analysis.integrated_lufs;
        let source = clean::BlendedPreview {
            dir: job_dir,
            preview,
            cleaned,
            strength: strength.clamp(0.0, 1.0),
        };
        let measured = loudness::measure(&source, &settings, &cancel, &mut |fraction| {
            let _ = events.emit(
                "polish-progress",
                PreviewProgress {
                    path: job_path.clone(),
                    fraction,
                },
            );
        })?;
        Ok((measured, original_lufs))
    })
    .await?;
    let player = app.state::<player::Player>();
    if player.loaded().as_ref() == Some(&dir) {
        player.set_polish(measured.params);
        if let (Some(out), Some(orig)) = (measured.output_lufs, original_lufs) {
            player.set_match_gain_db((out - orig) as f32);
        }
    }
    Ok(measured)
}

/// The quick estimate right after a change (`loudness::estimate`: excerpts of a long
/// sound): the player gets it at once; `measure_polish` replaces it with the exact one.
/// Never used for export.
#[tauri::command]
async fn estimate_polish(
    app: AppHandle,
    path: PathBuf,
    cleaned: Option<clean::Model>,
    strength: f32,
    settings: loudness::PolishSettings,
) -> Result<loudness::Measured, String> {
    let dir = preview::dir_for(&preview_root(&app)?, &path);
    let job_dir = dir.clone();
    let (measured, original_lufs) = blocking(move || {
        let preview = preview::cached(&job_dir, &path).ok_or("the preview isn't ready")?;
        let cleaned = cleaned.filter(|&m| clean::cached(&job_dir, &preview, m).is_some());
        let original_lufs = preview.analysis.integrated_lufs;
        let source = clean::BlendedPreview {
            dir: job_dir,
            preview,
            cleaned,
            strength: strength.clamp(0.0, 1.0),
        };
        let measured = loudness::estimate(&source, &settings, &AtomicBool::new(false))?;
        Ok((measured, original_lufs))
    })
    .await?;
    let player = app.state::<player::Player>();
    if player.loaded().as_ref() == Some(&dir) {
        player.set_polish(measured.params);
        if let (Some(out), Some(orig)) = (measured.output_lufs, original_lufs) {
            player.set_match_gain_db((out - orig) as f32);
        }
    }
    Ok(measured)
}

#[tauri::command]
fn cancel_polish(app: AppHandle) {
    if let Some(job) = app.state::<PolishJob>().0.lock().unwrap().take() {
        job.store(true, Ordering::SeqCst);
    }
}

/// A/B: the cleaned sound (true) or the original.
#[tauri::command]
fn player_listen(player: tauri::State<player::Player>, cleaned: bool) -> player::PlayerState {
    player.listen_cleaned(cleaned);
    player.state()
}

#[tauri::command]
fn player_strength(player: tauri::State<player::Player>, strength: f32) -> player::PlayerState {
    player.set_strength(strength);
    player.state()
}

#[tauri::command]
async fn file_settings(app: AppHandle, path: String) -> Result<recent::FileSettings, String> {
    let dir = data_dir(&app)?;
    blocking(move || Ok(recent::settings(&dir, &path))).await
}

#[tauri::command]
async fn save_settings(
    app: AppHandle,
    path: String,
    settings: recent::FileSettings,
) -> Result<(), String> {
    let dir = data_dir(&app)?;
    blocking(move || recent::save_settings(&dir, &path, settings)).await
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

/// Where exports keep their temporary f32 file.
fn export_work_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("export"))
}

/// The running export's (or batch's) cancel flag.
#[derive(Default)]
struct ExportJob(Mutex<Option<Arc<AtomicBool>>>);

impl ExportJob {
    fn start(&self) -> Result<Arc<AtomicBool>, String> {
        let mut slot = self.0.lock().unwrap();
        if slot.as_ref().is_some_and(|c| !c.load(Ordering::SeqCst)) {
            return Err(
                "An export is already running. Wait for it to finish, or stop it first.".into(),
            );
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *slot = Some(cancel.clone());
        Ok(cancel)
    }

    fn end(&self) {
        *self.0.lock().unwrap() = None;
    }
}

/// What to export a file with: what the editor plays (its model only when it was
/// cleaned), or a batch's shared settings.
#[derive(Debug, Clone, Copy, serde::Deserialize, Serialize)]
struct ExportSettings {
    /// The model to clean with, `None` for Polish only.
    cleaned: Option<clean::Model>,
    strength: f32,
    polish: loudness::PolishSettings,
}

/// The file's own settings as the editor plays them.
fn editor_settings(app: &AppHandle, path: &Path) -> Result<(ExportSettings, PathBuf), String> {
    let saved = recent::settings(&data_dir(app)?, &path.to_string_lossy());
    let preview_dir = preview::dir_for(&preview_root(app)?, path);
    let cleaned = preview::cached(&preview_dir, path)
        .and_then(|p| clean::cached(&preview_dir, &p, saved.model).map(|_| saved.model));
    let strength = match saved.model {
        clean::Model::Fast => saved.fast_strength,
        clean::Model::Best => saved.best_strength,
    };
    Ok((
        ExportSettings {
            cleaned,
            strength,
            polish: saved.polish,
        },
        preview_dir,
    ))
}

/// The export dialog's facts for one file.
#[derive(Serialize)]
struct ExportOptions {
    format: export::Format,
    /// `<name>-clean.<ext>` next to the original.
    destination: PathBuf,
    exists: bool,
    nothing_to_do: bool,
    cleaned: Option<clean::Model>,
    mp3: bool,
    is_video: bool,
}

#[tauri::command]
async fn export_options(app: AppHandle, path: PathBuf) -> Result<ExportOptions, String> {
    let (settings, _) = editor_settings(&app, &path)?;
    blocking(move || {
        let info = probe::probe(&path)?;
        let format = export::Format::default_for(&path, &info);
        let folder = path
            .parent()
            .ok_or("This file has no folder. Open it again from its folder.")?;
        let destination = export::destination(folder, &path, format);
        Ok(ExportOptions {
            format,
            exists: destination.exists(),
            destination,
            nothing_to_do: export::nothing_to_do(settings.cleaned, &settings.polish),
            cleaned: settings.cleaned,
            mp3: export::mp3_available(),
            is_video: info.video.is_some(),
        })
    })
    .await
}

#[derive(Clone, Serialize)]
struct ExportProgress {
    path: PathBuf,
    /// The file's place in a batch (0 for a single export).
    index: usize,
    stage: export::Stage,
    fraction: f64,
}

/// Exports one file with `settings` to `destination`; `export-progress` events.
fn export_one(
    app: &AppHandle,
    path: &Path,
    settings: ExportSettings,
    preview_dir: Option<PathBuf>,
    format: export::Format,
    destination: PathBuf,
    replace: bool,
    index: usize,
    cancel: &AtomicBool,
) -> Result<export::Done, String> {
    let info = probe::probe(path)?;
    let job = export::Job {
        source: path.to_path_buf(),
        info,
        cleaned: settings.cleaned,
        strength: settings.strength,
        polish: settings.polish,
        format,
        destination,
        replace,
        preview_dir,
    };
    let data = data_dir(app)?;
    if let Some(folder) = job.destination.parent() {
        speeds::remember_folder(&data, folder);
    }
    let events = app.clone();
    let done = export::export(
        &job,
        &export_work_dir(app)?,
        &models_dir(app)?,
        cancel,
        &mut |p| {
            let _ = events.emit(
                "export-progress",
                ExportProgress {
                    path: job.source.clone(),
                    index,
                    stage: p.stage,
                    fraction: p.fraction,
                },
            );
        },
    )?;
    let mut s = speeds::load(&data);
    s.learn(settings.cleaned, done.realtime_factor);
    speeds::store(&data, &s);
    Ok(done)
}

/// Exports the open file as the editor plays it.
#[tauri::command]
async fn export_file(
    app: AppHandle,
    path: PathBuf,
    format: export::Format,
    destination: PathBuf,
    replace: bool,
) -> Result<export::Done, String> {
    let (settings, preview_dir) = editor_settings(&app, &path)?;
    let cancel = app.state::<ExportJob>().start()?;
    let app2 = app.clone();
    let result = blocking(move || {
        let _awake = awake::KeepAwake::new("exporting");
        export_one(
            &app2,
            &path,
            settings,
            Some(preview_dir),
            format,
            destination,
            replace,
            0,
            &cancel,
        )
    })
    .await;
    app.state::<ExportJob>().end();
    result
}

#[tauri::command]
fn cancel_export(app: AppHandle) {
    if let Some(job) = app.state::<ExportJob>().0.lock().unwrap().as_ref() {
        job.store(true, Ordering::SeqCst);
    }
}

/// One file of a batch, as planned.
#[derive(Serialize)]
struct PlanItem {
    path: PathBuf,
    destination: PathBuf,
    format: export::Format,
    exists: bool,
    duration_ms: u64,
    /// Rough size of the output.
    bytes: u64,
    /// Why it can't be exported (no sound, not readable…).
    error: Option<String>,
}

#[derive(Serialize)]
struct Plan {
    items: Vec<PlanItem>,
    nothing_to_do: bool,
    total_bytes: u64,
    free_bytes: Option<u64>,
    /// Free space to keep (2 GB, as in Clips' Export all).
    margin_bytes: u64,
    speeds: speeds::Speeds,
}

/// Export all, before it starts: names, conflicts, size and free space.
#[tauri::command]
async fn export_plan(
    app: AppHandle,
    paths: Vec<PathBuf>,
    folder: PathBuf,
    format: Option<export::Format>,
    settings: ExportSettings,
) -> Result<Plan, String> {
    let data = data_dir(&app)?;
    blocking(move || {
        let mut taken = std::collections::HashSet::new();
        let items: Vec<PlanItem> = paths
            .into_iter()
            .map(|path| match probe::probe(&path) {
                Ok(info) => {
                    let f = format
                        .filter(|f| (*f == export::Format::Video) == info.video.is_some())
                        .unwrap_or_else(|| export::Format::default_for(&path, &info));
                    let mut destination = export::destination(&folder, &path, f);
                    // Two files with the same name in one batch: the second gets -2.
                    while !taken.insert(destination.clone()) {
                        destination = export::keep_both(&destination);
                    }
                    let channels = preview::output_channels(info.audio.channels);
                    let bytes = f.bytes_per_second(channels) * info.duration_ms / 1000
                        + if f == export::Format::Video {
                            std::fs::metadata(&path).map_or(0, |m| m.len())
                        } else {
                            0
                        };
                    PlanItem {
                        exists: destination.exists(),
                        destination,
                        format: f,
                        duration_ms: info.duration_ms,
                        bytes,
                        error: None,
                        path,
                    }
                }
                Err(e) => PlanItem {
                    destination: PathBuf::new(),
                    format: export::Format::Wav,
                    exists: false,
                    duration_ms: 0,
                    bytes: 0,
                    error: Some(e),
                    path,
                },
            })
            .collect();
        Ok(Plan {
            total_bytes: items.iter().map(|i| i.bytes).sum(),
            nothing_to_do: export::nothing_to_do(settings.cleaned, &settings.polish),
            items,
            free_bytes: disk::free_bytes(&folder),
            margin_bytes: 2 * 1024 * 1024 * 1024,
            speeds: speeds::load(&data),
        })
    })
    .await
}

/// One file of Export all, as the user decided it (Replace / Keep both already applied).
#[derive(serde::Deserialize)]
struct BatchItem {
    path: PathBuf,
    destination: PathBuf,
    format: export::Format,
    replace: bool,
}

#[derive(Serialize)]
struct BatchResult {
    path: PathBuf,
    done: Option<export::Done>,
    error: Option<String>,
}

/// Export all with the same settings; `export-progress` events carry each file's index.
/// Stop (`cancel_export`) ends it after removing the current file's temporary files.
#[tauri::command]
async fn export_batch(
    app: AppHandle,
    items: Vec<BatchItem>,
    settings: ExportSettings,
) -> Result<Vec<BatchResult>, String> {
    let cancel = app.state::<ExportJob>().start()?;
    let root = preview_root(&app)?;
    let app2 = app.clone();
    let results = blocking(move || {
        let _awake = awake::KeepAwake::new("exporting");
        let mut results = Vec::new();
        for (index, item) in items.into_iter().enumerate() {
            if cancel.load(Ordering::SeqCst) {
                results.push(BatchResult {
                    path: item.path,
                    done: None,
                    error: Some(export::CANCELLED.into()),
                });
                continue;
            }
            let preview_dir = Some(preview::dir_for(&root, &item.path));
            let result = export_one(
                &app2,
                &item.path,
                settings,
                preview_dir,
                item.format,
                item.destination,
                item.replace,
                index,
                &cancel,
            );
            results.push(match result {
                Ok(done) => BatchResult {
                    path: item.path,
                    done: Some(done),
                    error: None,
                },
                Err(e) => BatchResult {
                    path: item.path,
                    done: None,
                    error: Some(e),
                },
            });
        }
        Ok(results)
    })
    .await;
    app.state::<ExportJob>().end();
    results
}

/// A free name next to a taken one (Keep both): `name-2.ext`, `name-3.ext`…
#[tauri::command]
fn free_name(path: PathBuf) -> PathBuf {
    export::keep_both(&path)
}

/// Shows an exported file in Finder / Explorer. Only files Voice wrote (a `-clean.`
/// name) can be revealed; the webview gets no general permission.
#[tauri::command]
fn reveal_export(app: AppHandle, path: PathBuf) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    if !name.contains("-clean") || !path.is_file() {
        return Err("only exported files can be shown".into());
    }
    app.opener()
        .reveal_item_in_dir(&path)
        .map_err(|e| e.to_string())
}

/// Development aid: Export uses this folder instead of asking (`KREOKIT_DEV_EXPORT_DIR`).
/// Always `None` in release builds.
#[tauri::command]
fn dev_export_folder() -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var("KREOKIT_DEV_EXPORT_DIR")
        .ok()
        .filter(|d| Path::new(d).is_dir())
}

/// At start: leftovers of a crashed export (only our own temporary names) in the work
/// folder and in the folders exports went to.
fn remove_export_leftovers(app: &AppHandle) {
    let (Ok(work), Ok(data)) = (export_work_dir(app), data_dir(app)) else {
        return;
    };
    std::thread::spawn(move || {
        let mut removed = export::remove_leftovers(&work);
        for folder in speeds::export_folders(&data) {
            removed += export::remove_leftovers(&folder);
        }
        if removed > 0 {
            eprintln!("removed {removed} leftover temporary files of an earlier export");
        }
    });
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
/// The hidden `--self-test` mode for release CI (see `selftest.rs`). `args` excludes the
/// program name; returns `None` when the app should start normally.
pub fn self_test(args: &[String]) -> Option<std::process::ExitCode> {
    selftest::run_if_requested(args)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(PreviewJob::default())
        .manage(CleanJob::default())
        .manage(ModelJob::default())
        .manage(PolishJob::default())
        .manage(ExportJob::default())
        .manage(player::Player::default())
        .invoke_handler(tauri::generate_handler![
            probe_media,
            list_recent,
            add_files,
            remove_recent,
            clear_recent,
            relocate_recent,
            prepare_preview,
            cancel_preview,
            waveform,
            best_model,
            download_best_model,
            cancel_model_download,
            clean_status,
            clean_file,
            cancel_clean,
            cleaned_waveform,
            player_load_cleaned,
            estimate_polish,
            measure_polish,
            cancel_polish,
            player_listen,
            player_strength,
            file_settings,
            save_settings,
            player_open,
            player_close,
            player_play,
            player_pause,
            player_seek,
            player_state,
            dev_startup_file,
            export_options,
            export_file,
            cancel_export,
            export_plan,
            export_batch,
            dev_export_folder,
            reveal_export,
            free_name,
            quit_app,
            report_problem
        ])
        .setup(|app| {
            remove_export_leftovers(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Quitting stops the decode, so its FFmpeg ends with the app.
            if let tauri::RunEvent::Exit = event {
                cancel_preview(app.clone());
                cancel_clean(app.clone());
                cancel_model_download(app.clone());
                cancel_polish(app.clone());
                cancel_export(app.clone());
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
