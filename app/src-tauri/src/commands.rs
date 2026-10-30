//! Tauri commands. Each one loads settings, calls core and returns
//! serialisable data. Long-running work (scans) runs on a background thread
//! and reports through events.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use driveledger_core::compare::{CompareOptions, CompareResult};
use driveledger_core::export;
use driveledger_core::paging::{Page, paginate};
use driveledger_core::{
    Index, IndexSummary, ScanOutcome, ScanProgress, SearchHit, SearchQuery, Settings, SystemProbe,
    VerifyOptions, VerifyReport, VolumeStatus, list_indexes, resolve_index_entry, scan_to_library,
    search, volume_statuses,
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

/// Compare two saved indexes (IDs or index files).
#[tauri::command]
pub async fn compare(
    source: String,
    target: String,
    subpath: Option<String>,
    depth: Option<usize>,
) -> CommandResult<CompareResult> {
    let (_, library) = library()?;
    blocking(move || run_compare(&library, &source, &target, subpath, depth)).await
}

fn run_compare(
    library: &std::path::Path,
    source: &str,
    target: &str,
    subpath: Option<String>,
    depth: Option<usize>,
) -> CommandResult<CompareResult> {
    let target = resolve_index_entry(library, target)?;
    let source = resolve_index_entry(library, source)?;
    let options = CompareOptions {
        source_subpath: subpath.filter(|s| !s.trim().is_empty()),
        rollup_depth: depth.unwrap_or(3),
    };
    Ok(driveledger_core::compare(
        &Index::open(&source.path)?,
        &Index::open(&target.path)?,
        &options,
    )?)
}

/// Write a comparison report to a file the user picked. Refused inside
/// either scanned folder.
#[tauri::command]
pub async fn export_compare(
    source: String,
    target: String,
    subpath: Option<String>,
    out: String,
    format: String,
) -> CommandResult<()> {
    let (_, library) = library()?;
    blocking(move || {
        let result = run_compare(&library, &source, &target, subpath, None)?;
        let out = PathBuf::from(out);
        export::ensure_outside_scanned_roots(
            &out,
            &[&result.source.root_path, &result.target.root_path],
        )?;
        let file = std::fs::File::create(&out).map_err(|e| CommandError {
            kind: "io".into(),
            message: format!("{}: {e}", out.display()),
        })?;
        let mut writer = std::io::BufWriter::new(file);
        if format == "json" {
            serde_json::to_writer_pretty(&mut writer, &result).map_err(|e| CommandError {
                kind: "io".into(),
                message: e.to_string(),
            })?;
        } else {
            export::write_compare_tsv(&result, &mut writer)?;
        }
        Ok(())
    })
    .await
}

/// Full-hash verify of selected source files (all matched files if empty).
#[tauri::command]
pub async fn verify(
    source: String,
    target: String,
    paths: Vec<String>,
) -> CommandResult<VerifyReport> {
    let (_, library) = library()?;
    blocking(move || {
        let source = resolve_index_entry(&library, &source)?;
        let target = resolve_index_entry(&library, &target)?;
        let options = VerifyOptions {
            paths,
            ..VerifyOptions::default()
        };
        Ok(driveledger_core::verify(
            &source.path,
            &target.path,
            &options,
            &|_| {},
        )?)
    })
    .await
}

/// Search across all indexes.
#[tauri::command]
pub async fn search_files(
    query: SearchQuery,
    limit: Option<usize>,
) -> CommandResult<Page<SearchHit>> {
    let (_, library) = library()?;
    blocking(move || {
        let hits = search(&list_indexes(&library), &query)?;
        paginate(&hits, Some(limit.unwrap_or(2000)), None).map_err(|m| CommandError {
            kind: "invalid_argument".into(),
            message: m,
        })
    })
    .await
}

/// Whether a scanned root is reachable right now (for Reveal in Finder and
/// the "Connect drive" state).
#[tauri::command]
pub fn is_available(path: String) -> bool {
    driveledger_core::volume::is_mounted_and_readable(std::path::Path::new(&path))
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
