//! Tauri commands. Each one loads settings, calls core and returns
//! serialisable data. Long-running work (scans) runs on a background thread
//! and reports through events.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use driveledger_core::{
    IndexSummary, ScanOutcome, ScanProgress, Settings, SystemProbe, VolumeStatus, list_indexes,
    scan_to_library, volume_statuses,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

/// Error shape returned to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct CommandError {
    pub kind: String,
    pub message: String,
}

impl From<driveledger_core::Error> for CommandError {
    fn from(e: driveledger_core::Error) -> Self {
        CommandError {
            kind: e.kind().to_string(),
            message: e.to_string(),
        }
    }
}

type CommandResult<T> = Result<T, CommandError>;

/// Running scans, by ID, with their cancel flags.
#[derive(Default)]
pub struct AppState {
    scans: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

/// Settings plus the library folder they resolve to.
#[derive(Debug, Serialize)]
pub struct SettingsView {
    pub settings: Settings,
    pub library: PathBuf,
    pub default_library: PathBuf,
    pub settings_path: Option<PathBuf>,
}

fn library() -> CommandResult<(Settings, PathBuf)> {
    let settings = Settings::load()?;
    let library = settings.resolve_library(None);
    Ok((settings, library))
}

#[tauri::command]
pub fn get_settings() -> CommandResult<SettingsView> {
    let (settings, library) = library()?;
    Ok(SettingsView {
        settings,
        library,
        default_library: Settings::default_library(),
        settings_path: Settings::path(),
    })
}

#[tauri::command]
pub fn save_settings(settings: Settings) -> CommandResult<SettingsView> {
    settings.save()?;
    get_settings()
}

#[tauri::command]
pub async fn list_volumes() -> CommandResult<Vec<VolumeStatus>> {
    let (_, library) = library()?;
    blocking(move || Ok(volume_statuses(&library, &SystemProbe))).await
}

#[tauri::command]
pub async fn list_saved_indexes() -> CommandResult<Vec<IndexSummary>> {
    let (_, library) = library()?;
    blocking(move || Ok(list_indexes(&library).iter().map(|e| e.summary()).collect())).await
}

#[derive(Debug, Clone, Serialize)]
struct ScanProgressEvent<'a> {
    scan_id: &'a str,
    progress: &'a ScanProgress,
}

#[derive(Debug, Clone, Serialize)]
struct ScanFinishedEvent {
    scan_id: String,
    outcome: Option<ScanOutcome>,
    error: Option<CommandError>,
}

/// Start scanning `path` into the library. Returns a scan ID right away;
/// progress arrives as `scan://progress` events and the result as
/// `scan://finished`.
#[tauri::command]
pub fn start_scan(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    full: bool,
) -> CommandResult<String> {
    let (settings, library) = library()?;
    let root = PathBuf::from(&path);
    driveledger_core::volume::ensure_available(&root)?;

    let cancel = Arc::new(AtomicBool::new(false));
    let scan_id = {
        let mut scans = state.scans.lock().unwrap();
        let id = format!("scan-{}", scans.len() + 1);
        scans.insert(id.clone(), cancel.clone());
        id
    };
    let mut options = settings.scan_options();
    options.incremental = !full;
    options.cancel = Some(cancel);

    let id = scan_id.clone();
    std::thread::spawn(move || {
        // Throttle events so the UI is not flooded on fast local drives.
        let last = Mutex::new(Instant::now() - Duration::from_secs(1));
        let progress = |p: &ScanProgress| {
            let mut last = last.lock().unwrap();
            if last.elapsed() >= Duration::from_millis(80) {
                *last = Instant::now();
                let _ = app.emit(
                    "scan://progress",
                    ScanProgressEvent {
                        scan_id: &id,
                        progress: p,
                    },
                );
            }
        };
        let result = scan_to_library(&root, &library, &options, &SystemProbe, &progress);
        let event = match result {
            Ok(outcome) => ScanFinishedEvent {
                scan_id: id,
                outcome: Some(outcome),
                error: None,
            },
            Err(e) => ScanFinishedEvent {
                scan_id: id,
                outcome: None,
                error: Some(e.into()),
            },
        };
        let _ = app.emit("scan://finished", event);
    });
    Ok(scan_id)
}

#[tauri::command]
pub fn cancel_scan(state: State<'_, AppState>, scan_id: String) {
    if let Some(flag) = state.scans.lock().unwrap().get(&scan_id) {
        flag.store(true, Ordering::Relaxed);
    }
}

/// Run blocking core work off the async runtime.
async fn blocking<T, F>(work: F) -> CommandResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> CommandResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| CommandError {
            kind: "internal".into(),
            message: e.to_string(),
        })?
}
