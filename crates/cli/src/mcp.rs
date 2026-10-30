//! MCP server: a thin wrapper that exposes `driveledger-core` to any MCP
//! client over stdio or Streamable HTTP (localhost only).
//!
//! Only the official MCP spec is used; nothing here assumes a particular
//! agent. Every tool returns a short text summary, the same data as JSON
//! text, and `structuredContent` matching its output schema. Large results
//! are paged with `limit` / `cursor`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use driveledger_core::compare::{
    CompareOptions, CompareSummary, FileComparison, FolderRollup, IndexRef, Status,
};
use driveledger_core::paging::{Page, paginate};
use driveledger_core::{
    DuplicateGroup, Error as CoreError, FolderStat, IndexArg, IndexSummary, ScanOutcome,
    ScanProgress, SearchHit, SearchQuery, Settings, SystemProbe, VolumeStatus, classify_arg,
    find_duplicates, folder_overview, list_indexes, resolve_index_entry, scan_to_library, search,
    volume_statuses,
};
use rmcp::handler::server::common::schema_for_output;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::render::{human_bytes, thousands};

const INSTRUCTIONS: &str = "DriveLedger indexes drives into local scans and answers whether files \
from one drive exist on another. Matching is content-based (BLAKE3 fingerprint of size plus three \
1 MiB samples), never by name: `missing` means no identical content exists anywhere on the target, \
and `same_name_different_content` means a file with the same name exists but is NOT the same file \
(cameras reuse names), so it is not backed up either. Typical flow: `list_volumes` / \
`list_indexes`, `scan` any drive or folder without a recent index, then `compare` source against \
target and report `summary.content_not_found`. DriveLedger never modifies scanned drives.";

/// The MCP server state.
#[derive(Clone)]
pub struct DriveLedgerMcp {
    library: PathBuf,
    settings: Settings,
    jobs: Arc<Mutex<HashMap<String, Arc<ScanJob>>>>,
    tool_router: ToolRouter<Self>,
}

// ---------------------------------------------------------------------------
// Tool inputs and outputs
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct PageArgs {
    /// Maximum items to return (default 200, max 5000).
    #[serde(default)]
    pub limit: Option<usize>,
    /// `next_cursor` from the previous page.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListIndexesOutput {
    pub library: PathBuf,
    pub indexes: Page<IndexSummary>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct EmptyArgs {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListVolumesOutput {
    pub volumes: Vec<VolumeStatus>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// Case-insensitive substring of the file path.
    #[serde(default)]
    pub text: Option<String>,
    /// Glob, e.g. `SW-FX3*.MP4`. Matches the file name, or the relative path if it contains `/`.
    #[serde(default)]
    pub glob: Option<String>,
    /// File extensions without the dot, e.g. `["mp4", "mov"]`.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Minimum size in bytes.
    #[serde(default)]
    pub min_size: Option<u64>,
    /// Maximum size in bytes.
    #[serde(default)]
    pub max_size: Option<u64>,
    /// Modified on or after this date (`YYYY-MM-DD` or RFC 3339).
    #[serde(default)]
    pub modified_after: Option<String>,
    /// Modified on or before this date (`YYYY-MM-DD` or RFC 3339).
    #[serde(default)]
    pub modified_before: Option<String>,
    /// Index IDs to search. Default: all indexes in the library.
    #[serde(default)]
    pub indexes: Vec<String>,
    #[serde(flatten)]
    pub page: PageArgs,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OverviewArgs {
    /// Index ID (from `list_indexes`) or index file path.
    pub index: String,
    /// Folder levels to show (default 4).
    #[serde(default)]
    pub depth: Option<usize>,
    /// Only show folders below this relative path.
    #[serde(default)]
    pub subpath: Option<String>,
    #[serde(flatten)]
    pub page: PageArgs,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct OverviewOutput {
    pub index: IndexSummary,
    pub folders: Page<FolderStat>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CompareArgs {
    /// Source index ID or index file: the drive you want to know is backed up.
    pub source: String,
    /// Target index ID or index file: where the backup should be.
    pub target: String,
    /// Only compare source files below this folder (relative to the source root).
    #[serde(default)]
    pub source_subpath: Option<String>,
    /// Folder levels in the rollup (default 3).
    #[serde(default)]
    pub depth: Option<usize>,
    /// Only return files with these statuses: present, present_elsewhere, only_in_bundle,
    /// same_name_different_content, missing. Default: all. Summary and folders always cover
    /// every file.
    #[serde(default)]
    pub statuses: Vec<Status>,
    /// Maximum folder rows to return (default 200). Folders are ordered by path.
    #[serde(default)]
    pub max_folders: Option<usize>,
    #[serde(flatten)]
    pub page: PageArgs,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CompareOutput {
    pub source: IndexRef,
    pub target: IndexRef,
    pub source_subpath: Option<String>,
    pub summary: CompareSummary,
    pub warnings: Vec<String>,
    pub folders: Vec<FolderRollup>,
    /// True if there were more folders than `max_folders`.
    pub folders_truncated: bool,
    pub files: Page<FileComparison>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct FindFileArgs {
    /// Content fingerprint to look up. Alternatively give `index` and `path`.
    #[serde(default)]
    pub fingerprint: Option<String>,
    /// Index ID containing the file.
    #[serde(default)]
    pub index: Option<String>,
    /// Path of the file relative to that index's root.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FindFileOutput {
    pub fingerprint: String,
    /// Every file with this content, in every index of the library.
    pub locations: Vec<SearchHit>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct DuplicatesArgs {
    /// Index IDs to look in (within and across them). Default: all indexes.
    #[serde(default)]
    pub indexes: Vec<String>,
    /// Ignore files smaller than this many bytes (default 1).
    #[serde(default)]
    pub min_size: Option<u64>,
    #[serde(flatten)]
    pub page: PageArgs,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DuplicatesOutput {
    /// Bytes that keeping one copy of every group would free, across all groups.
    pub total_redundant_bytes: u64,
    pub groups: Page<DuplicateGroup>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ScanArgs {
    /// Drive or folder to scan, e.g. `/Volumes/ByHola_1`. Starts a new scan.
    #[serde(default)]
    pub path: Option<String>,
    /// ID of a scan started earlier, to check its progress.
    #[serde(default)]
    pub scan_id: Option<String>,
    /// Hash every file again instead of reusing unchanged fingerprints from the last scan.
    #[serde(default)]
    pub full: bool,
    /// Wait up to this many seconds for the scan to finish (default 20, max 300).
    #[serde(default)]
    pub wait_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScanState {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ScanStatusOutput {
    pub scan_id: String,
    pub path: String,
    pub state: ScanState,
    /// Latest progress while running.
    pub progress: Option<ScanProgress>,
    /// The finished scan. Its index ID is the index file name without `.dlidx.sqlite`.
    pub result: Option<ScanOutcome>,
    pub index_id: Option<String>,
    pub error: Option<ErrorBody>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ErrorBody {
    pub kind: String,
    pub message: String,
}

struct ScanJob {
    status: Mutex<ScanStatusOutput>,
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

#[tool_router(router = tool_router)]
impl DriveLedgerMcp {
    pub fn new(library: PathBuf, settings: Settings) -> Self {
        DriveLedgerMcp {
            library,
            settings,
            jobs: Arc::default(),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "List saved scans (indexes) in the library: volume, root path, scan date, file count and size. Index IDs from here are accepted by the other tools.",
        output_schema = schema_for_output::<ListIndexesOutput>(),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_indexes(&self, Parameters(args): Parameters<PageArgs>) -> CallToolResult {
        let library = self.library.clone();
        run(move || {
            let all: Vec<IndexSummary> =
                list_indexes(&library).iter().map(|e| e.summary()).collect();
            let indexes = page(&all, &args)?;
            let summary = format!(
                "{} indexes in {}{}",
                all.len(),
                library.display(),
                more_note(&indexes)
            );
            Ok((summary, ListIndexesOutput { library, indexes }))
        })
        .await
    }

    #[tool(
        description = "List mounted volumes (drives and network shares) and how many saved indexes each has. A volume with no recent index should be scanned before comparing.",
        output_schema = schema_for_output::<ListVolumesOutput>(),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_volumes(&self, Parameters(_): Parameters<EmptyArgs>) -> CallToolResult {
        let library = self.library.clone();
        run(move || {
            let volumes = volume_statuses(&library, &SystemProbe);
            let names: Vec<String> = volumes
                .iter()
                .map(|v| format!("{} ({} indexes)", v.volume.name, v.index_count))
                .collect();
            let summary = format!("{} mounted volumes: {}", volumes.len(), names.join(", "));
            Ok((summary, ListVolumesOutput { volumes }))
        })
        .await
    }

    #[tool(
        description = "Search files by name or path substring, glob, extension, size range or modification date, across all indexes or the ones given.",
        output_schema = schema_for_output::<Page<SearchHit>>(),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn search_files(&self, Parameters(args): Parameters<SearchArgs>) -> CallToolResult {
        let library = self.library.clone();
        run(move || {
            let entries = entries_for(&library, &args.indexes)?;
            let hits = search(
                &entries,
                &SearchQuery {
                    text: args.text.clone(),
                    glob: args.glob.clone(),
                    extensions: args.extensions.clone(),
                    min_size: args.min_size,
                    max_size: args.max_size,
                    modified_after: args.modified_after.clone(),
                    modified_before: args.modified_before.clone(),
                    fingerprint: None,
                },
            )?;
            let result = page(&hits, &args.page)?;
            let summary = format!(
                "{} matching files in {} indexes{}",
                hits.len(),
                entries.len(),
                more_note(&result)
            );
            Ok((summary, result))
        })
        .await
    }

    #[tool(
        description = "Folder tree of an index with total size and file count per folder, down to `depth` levels (default 4).",
        output_schema = schema_for_output::<OverviewOutput>(),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn folder_overview(&self, Parameters(args): Parameters<OverviewArgs>) -> CallToolResult {
        let library = self.library.clone();
        run(move || {
            let entry = resolve_index_entry(&library, &args.index)?;
            let index = driveledger_core::Index::open(&entry.path)?;
            let folders =
                folder_overview(&index, args.depth.unwrap_or(4), args.subpath.as_deref())?;
            let folders_page = page(&folders, &args.page)?;
            let summary = format!(
                "{} · {} · {} files, {} · {} folders{}",
                entry.info.volume_name,
                entry.info.root_path,
                thousands(entry.info.file_count),
                human_bytes(entry.info.total_bytes),
                folders.len(),
                more_note(&folders_page)
            );
            Ok((
                summary,
                OverviewOutput {
                    index: entry.summary(),
                    folders: folders_page,
                },
            ))
        })
        .await
    }

    #[tool(
        description = "Compare a source index against a target index: for every source file, does identical CONTENT exist anywhere on the target? Matching is by content fingerprint regardless of folder structure, never by name. Statuses: present (same content, same name), present_elsewhere (same content, other name/path), only_in_bundle (same content only inside e.g. .dra/MediaFiles), same_name_different_content (a same-name file exists but is a different file, NOT a backup; has newer_side and possibly_truncated hints), missing (no identical content anywhere on the target). `summary.content_not_found` is what is not backed up. Both sides must be saved indexes; use `scan` first for drives without a recent index.",
        output_schema = schema_for_output::<CompareOutput>(),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn compare(&self, Parameters(args): Parameters<CompareArgs>) -> CallToolResult {
        let library = self.library.clone();
        run(move || {
            // Target first, so an unmounted drive is reported before anything else.
            let target = open_saved_index(&library, &args.target)?;
            let source = open_saved_index(&library, &args.source)?;
            let options = CompareOptions {
                source_subpath: args.source_subpath.clone(),
                rollup_depth: args.depth.unwrap_or(3),
            };
            let result = driveledger_core::compare(&source, &target, &options)?;
            let files: Vec<FileComparison> = result
                .files_with_status(&args.statuses)
                .into_iter()
                .cloned()
                .collect();
            let files_page = page(&files, &args.page)?;
            let max_folders = args.max_folders.unwrap_or(200);
            let s = &result.summary;
            let mut summary = format!(
                "{} → {}: {} files ({}). Content found: {} ({}). Content NOT found: {} ({}): {} missing, {} same name but different content.",
                result.source.volume_name,
                result.target.volume_name,
                thousands(s.total.files),
                human_bytes(s.total.bytes),
                thousands(s.content_found.files),
                human_bytes(s.content_found.bytes),
                thousands(s.content_not_found.files),
                human_bytes(s.content_not_found.bytes),
                thousands(s.by_status.missing.files),
                thousands(s.by_status.same_name_different_content.files),
            );
            for warning in &result.warnings {
                summary.push_str(&format!(" Warning: {warning}."));
            }
            summary.push_str(&more_note(&files_page));
            let output = CompareOutput {
                source: result.source,
                target: result.target,
                source_subpath: result.source_subpath,
                summary: result.summary,
                warnings: result.warnings,
                folders_truncated: result.folders.len() > max_folders,
                folders: result.folders.into_iter().take(max_folders).collect(),
                files: files_page,
            };
            Ok((summary, output))
        })
        .await
    }

    #[tool(
        description = "Find every copy of one file's content across all indexes. Give a `fingerprint`, or an `index` plus the file's relative `path`.",
        output_schema = schema_for_output::<FindFileOutput>(),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn find_file(&self, Parameters(args): Parameters<FindFileArgs>) -> CallToolResult {
        let library = self.library.clone();
        run(move || {
            let fingerprint = match (&args.fingerprint, &args.index, &args.path) {
                (Some(fp), _, _) => fp.clone(),
                (None, Some(index), Some(path)) => {
                    let entry = resolve_index_entry(&library, index)?;
                    let file = driveledger_core::Index::open(&entry.path)?
                        .file_by_path(path)?
                        .ok_or_else(|| {
                            CoreError::InvalidArgument(format!(
                                "no file `{path}` in index `{index}`"
                            ))
                        })?;
                    file.fingerprint.ok_or_else(|| {
                        CoreError::InvalidArgument(format!(
                            "`{path}` could not be read during the scan and has no fingerprint"
                        ))
                    })?
                }
                _ => {
                    return Err(CoreError::InvalidArgument(
                        "give `fingerprint`, or both `index` and `path`".into(),
                    ));
                }
            };
            let locations = search(
                &list_indexes(&library),
                &SearchQuery {
                    fingerprint: Some(fingerprint.clone()),
                    ..SearchQuery::default()
                },
            )?;
            let volumes: std::collections::BTreeSet<&str> =
                locations.iter().map(|h| h.volume_name.as_str()).collect();
            let summary = format!(
                "{} copies on {} volumes: {}",
                locations.len(),
                volumes.len(),
                volumes.into_iter().collect::<Vec<_>>().join(", ")
            );
            Ok((
                summary,
                FindFileOutput {
                    fingerprint,
                    locations,
                },
            ))
        })
        .await
    }

    #[tool(
        description = "Groups of files with identical content (same fingerprint), within or across indexes, largest redundant size first.",
        output_schema = schema_for_output::<DuplicatesOutput>(),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn find_duplicates(
        &self,
        Parameters(args): Parameters<DuplicatesArgs>,
    ) -> CallToolResult {
        let library = self.library.clone();
        run(move || {
            let entries = entries_for(&library, &args.indexes)?;
            let groups = find_duplicates(&entries, args.min_size.unwrap_or(1))?;
            let total_redundant_bytes = groups.iter().map(|g| g.redundant_bytes).sum();
            let groups_page = page(&groups, &args.page)?;
            let summary = format!(
                "{} duplicate groups, {} redundant{}",
                groups.len(),
                human_bytes(total_redundant_bytes),
                more_note(&groups_page)
            );
            Ok((
                summary,
                DuplicatesOutput {
                    total_redundant_bytes,
                    groups: groups_page,
                },
            ))
        })
        .await
    }

    #[tool(
        description = "Scan a drive or folder into a new index in the DriveLedger library (pass `path`), or check a running scan (pass `scan_id`). Read-only on the scanned path; it only writes the index file into the library. Unchanged files reuse fingerprints from the previous scan. Waits up to `wait_seconds` (default 20) and returns progress if the scan is still running; call again with `scan_id` to follow it. Fails with target_unavailable if the drive is not mounted.",
        output_schema = schema_for_output::<ScanStatusOutput>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn scan(&self, Parameters(args): Parameters<ScanArgs>) -> CallToolResult {
        let job = match (&args.path, &args.scan_id) {
            (Some(path), _) => match self.start_scan(path, args.full) {
                Ok(job) => job,
                Err(e) => return tool_error(&e),
            },
            (None, Some(id)) => match self.jobs.lock().unwrap().get(id) {
                Some(job) => job.clone(),
                None => {
                    return tool_error(&CoreError::InvalidArgument(format!("no scan `{id}`")));
                }
            },
            (None, None) => {
                return tool_error(&CoreError::InvalidArgument(
                    "give `path` to start a scan or `scan_id` to check one".into(),
                ));
            }
        };

        let wait = Duration::from_secs(args.wait_seconds.unwrap_or(20).min(300));
        let deadline = Instant::now() + wait;
        loop {
            let status = job.status.lock().unwrap().clone();
            if status.state != ScanState::Running || Instant::now() >= deadline {
                return scan_result(status);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

impl DriveLedgerMcp {
    /// Check the path, then run the scan on a background thread.
    fn start_scan(&self, path: &str, full: bool) -> Result<Arc<ScanJob>, CoreError> {
        let root = PathBuf::from(path);
        driveledger_core::volume::ensure_available(&root)?;
        let scan_id = format!(
            "scan-{}-{}",
            chrono::Utc::now().format("%Y%m%d%H%M%S"),
            self.jobs.lock().unwrap().len() + 1
        );
        let job = Arc::new(ScanJob {
            status: Mutex::new(ScanStatusOutput {
                scan_id: scan_id.clone(),
                path: path.to_string(),
                state: ScanState::Running,
                progress: None,
                result: None,
                index_id: None,
                error: None,
            }),
        });
        self.jobs.lock().unwrap().insert(scan_id, job.clone());

        let mut options = self.settings.scan_options();
        options.incremental = !full;
        let library = self.library.clone();
        let worker = job.clone();
        std::thread::spawn(move || {
            let outcome = scan_to_library(&root, &library, &options, &SystemProbe, &|p| {
                worker.status.lock().unwrap().progress = Some(p.clone());
            });
            let mut status = worker.status.lock().unwrap();
            match outcome {
                Ok(outcome) => {
                    status.state = ScanState::Done;
                    status.index_id = Some(index_id_of(&outcome.index_path));
                    status.result = Some(outcome);
                }
                Err(e) => {
                    status.state = ScanState::Failed;
                    status.error = Some(ErrorBody {
                        kind: e.kind().into(),
                        message: e.to_string(),
                    });
                }
            }
        });
        Ok(job)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for DriveLedgerMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "driveledger",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(INSTRUCTIONS)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Run blocking core work off the async runtime and turn the result into a
/// tool result.
async fn run<T, F>(work: F) -> CallToolResult
where
    T: Serialize + Send + 'static,
    F: FnOnce() -> Result<(String, T), CoreError> + Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(Ok((summary, value))) => tool_ok(summary, &value),
        Ok(Err(e)) => tool_error(&e),
        Err(join) => tool_error(&CoreError::InvalidArgument(format!(
            "internal error: {join}"
        ))),
    }
}

/// Text summary, the JSON as text (for clients that ignore structured
/// content), and the structured content itself.
fn tool_ok<T: Serialize>(summary: String, value: &T) -> CallToolResult {
    let json = serde_json::to_value(value).expect("tool output serializes");
    let mut result = CallToolResult::success(vec![
        ContentBlock::text(summary),
        ContentBlock::text(json.to_string()),
    ]);
    result.structured_content = Some(json);
    result
}

fn tool_error(e: &CoreError) -> CallToolResult {
    let mut message = e.to_string();
    if matches!(e, CoreError::TargetUnavailable(_)) {
        message.push_str(". Do not treat files as missing: the drive is not connected.");
    }
    let mut result =
        CallToolResult::error(vec![ContentBlock::text(format!("{}: {message}", e.kind()))]);
    result.structured_content =
        Some(serde_json::json!({ "error": { "kind": e.kind(), "message": message } }));
    result
}

fn scan_result(status: ScanStatusOutput) -> CallToolResult {
    let summary = match status.state {
        ScanState::Running => {
            let p = status.progress.as_ref();
            format!(
                "Scan {} of {} is still running: {}/{} files. Call `scan` again with scan_id to follow it.",
                status.scan_id,
                status.path,
                thousands(p.map_or(0, |p| p.files_done)),
                thousands(p.map_or(0, |p| p.files_seen)),
            )
        }
        ScanState::Done => {
            let r = status.result.as_ref().expect("done scans have a result");
            format!(
                "Scanned {} files ({}) on {}, {} errors. Index ID: {}",
                thousands(r.info.file_count),
                human_bytes(r.info.total_bytes),
                r.info.volume_name,
                thousands(r.info.error_count),
                status.index_id.as_deref().unwrap_or_default()
            )
        }
        ScanState::Failed => {
            let e = status.error.as_ref().expect("failed scans have an error");
            format!("Scan of {} failed: {}: {}", status.path, e.kind, e.message)
        }
    };
    let mut result = tool_ok(summary, &status);
    if status.state == ScanState::Failed {
        result.is_error = Some(true);
    }
    result
}

/// Open a saved index for compare. A folder argument is refused (scan it
/// first); a path that does not exist is reported as an unavailable drive.
fn open_saved_index(library: &Path, arg: &str) -> Result<driveledger_core::Index, CoreError> {
    match classify_arg(library, arg)? {
        IndexArg::Index(path) => driveledger_core::Index::open(&path),
        IndexArg::LivePath(path) => {
            driveledger_core::volume::ensure_available(&path)?;
            Err(CoreError::InvalidArgument(format!(
                "`{arg}` is a folder; scan it first with the `scan` tool and pass the returned index_id"
            )))
        }
    }
}

fn entries_for(
    library: &Path,
    ids: &[String],
) -> Result<Vec<driveledger_core::IndexEntry>, CoreError> {
    if ids.is_empty() {
        Ok(list_indexes(library))
    } else {
        ids.iter()
            .map(|id| resolve_index_entry(library, id))
            .collect()
    }
}

fn page<T: Clone>(items: &[T], args: &PageArgs) -> Result<Page<T>, CoreError> {
    paginate(items, args.limit, args.cursor.as_deref()).map_err(CoreError::InvalidArgument)
}

fn more_note<T>(page: &Page<T>) -> String {
    match &page.next_cursor {
        Some(cursor) => format!(
            ". Showing {} of {}; pass cursor \"{cursor}\" for more.",
            page.items.len(),
            page.total
        ),
        None => String::new(),
    }
}

fn index_id_of(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.strip_suffix(driveledger_core::index::INDEX_SUFFIX)
        .unwrap_or(&name)
        .to_string()
}

// ---------------------------------------------------------------------------
// Transports
// ---------------------------------------------------------------------------

/// Serve over stdio until the client disconnects.
pub async fn serve_stdio(server: DriveLedgerMcp) -> anyhow::Result<()> {
    let running = server.serve(rmcp::transport::stdio()).await?;
    running.waiting().await?;
    Ok(())
}

/// Serve Streamable HTTP on a loopback address at `/mcp`.
pub async fn serve_http(server: DriveLedgerMcp, addr: SocketAddr) -> anyhow::Result<()> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };

    if !addr.ip().is_loopback() {
        anyhow::bail!(
            "refusing to listen on {addr}: the MCP HTTP transport only binds to loopback (127.0.0.1 or ::1)"
        );
    }
    let port = addr.port();
    let config = StreamableHttpServerConfig::default().with_allowed_origins([
        format!("http://localhost:{port}"),
        format!("http://127.0.0.1:{port}"),
        format!("http://[::1]:{port}"),
    ]);
    let cancel = config.cancellation_token.clone();
    let service: StreamableHttpService<DriveLedgerMcp, LocalSessionManager> =
        StreamableHttpService::new(move || Ok(server.clone()), Default::default(), config);
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("DriveLedger MCP listening on http://{addr}/mcp");
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            cancel.cancel();
        })
        .await?;
    Ok(())
}
