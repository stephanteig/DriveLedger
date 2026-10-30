//! Turning a user argument (index file, index ID or live folder) into an
//! open index. Shared by the CLI, MCP server and app.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::error::{Error, Result};
use crate::index::{INDEX_SUFFIX, Index};
use crate::library::{find_index, find_previous_index};
use crate::scan::{ProgressFn, ScanOptions, ScanOutcome, scan_to_library};
use crate::volume::{VolumeProbe, ensure_available};

/// What an argument refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexArg {
    /// An existing index file.
    Index(PathBuf),
    /// A folder to scan (or reuse a recent scan of).
    LivePath(PathBuf),
}

/// Classify `arg`: an existing file is an index, an existing folder is a live
/// path, otherwise it is looked up as an index ID. Something that looks like
/// a path but does not exist is treated as a live path, so the caller reports
/// it as unavailable (an unmounted drive) rather than "index not found".
pub fn classify_arg(library: &Path, arg: &str) -> Result<IndexArg> {
    let path = Path::new(arg);
    if path.is_file() {
        return Ok(IndexArg::Index(path.to_path_buf()));
    }
    if path.is_dir() {
        return Ok(IndexArg::LivePath(path.to_path_buf()));
    }
    match find_index(library, arg) {
        Ok(entry) => Ok(IndexArg::Index(entry.path)),
        Err(Error::IndexNotFound(_))
            if arg.contains(std::path::MAIN_SEPARATOR) && !arg.ends_with(INDEX_SUFFIX) =>
        {
            Ok(IndexArg::LivePath(path.to_path_buf()))
        }
        Err(e) => Err(e),
    }
}

/// How to handle a live path.
#[derive(Debug, Clone, Default)]
pub struct LiveOptions {
    pub scan: ScanOptions,
    /// Reuse the newest index of the same root if it is younger than this,
    /// instead of scanning again.
    pub reuse_max_age: Option<Duration>,
}

/// An index ready for use, plus how it was obtained.
pub struct OpenedIndex {
    pub index: Index,
    pub path: PathBuf,
    /// Set when the argument was a live path that was scanned just now.
    pub scanned: Option<ScanOutcome>,
    /// True when a recent index of a live path was reused.
    pub reused: bool,
}

/// Open the index `arg` refers to, scanning a live path into `library` when
/// needed. A live path that is not mounted fails with
/// [`Error::TargetUnavailable`].
pub fn open_or_scan(
    arg: &str,
    library: &Path,
    live: &LiveOptions,
    probe: &dyn VolumeProbe,
    progress: ProgressFn<'_>,
) -> Result<OpenedIndex> {
    match classify_arg(library, arg)? {
        IndexArg::Index(path) => Ok(OpenedIndex {
            index: Index::open(&path)?,
            path,
            scanned: None,
            reused: false,
        }),
        IndexArg::LivePath(root) => {
            ensure_available(&root)?;
            if let Some(max_age) = live.reuse_max_age {
                let canonical = root.canonicalize().map_err(|e| Error::io(&root, e))?;
                let volume = probe.probe(&canonical)?;
                if let Some(entry) = find_previous_index(library, &volume, &canonical)
                    .filter(|e| is_younger_than(&e.info.started_at, max_age))
                {
                    return Ok(OpenedIndex {
                        index: Index::open(&entry.path)?,
                        path: entry.path,
                        scanned: None,
                        reused: true,
                    });
                }
            }
            let outcome = scan_to_library(&root, library, &live.scan, probe, progress)?;
            Ok(OpenedIndex {
                index: Index::open(&outcome.index_path)?,
                path: outcome.index_path.clone(),
                scanned: Some(outcome),
                reused: false,
            })
        }
    }
}

fn is_younger_than(rfc3339: &str, max_age: Duration) -> bool {
    DateTime::parse_from_rfc3339(rfc3339)
        .map(|t| {
            let age = Utc::now().signed_duration_since(t.with_timezone(&Utc));
            age.to_std().map(|a| a <= max_age).unwrap_or(true)
        })
        .unwrap_or(false)
}
