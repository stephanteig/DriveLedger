//! The library folder: where index files live.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::index::{INDEX_SUFFIX, Index, ScanInfo};
use crate::volume::{VolumeInfo, VolumeProbe, list_volumes};

/// A finished index found in the library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub path: PathBuf,
    pub info: ScanInfo,
}

impl IndexEntry {
    /// The index file name without the `.dlidx.sqlite` suffix. Used as a
    /// short, stable ID in the CLI and MCP tools.
    pub fn id(&self) -> String {
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        name.strip_suffix(INDEX_SUFFIX).unwrap_or(&name).to_string()
    }

    pub fn summary(&self) -> IndexSummary {
        IndexSummary {
            id: self.id(),
            path: self.path.clone(),
            volume_name: self.info.volume_name.clone(),
            volume_uuid: self.info.volume_uuid.clone(),
            root_path: self.info.root_path.clone(),
            scanned_at: self.info.started_at.clone(),
            file_count: self.info.file_count,
            total_bytes: self.info.total_bytes,
            error_count: self.info.error_count,
        }
    }
}

/// Flat description of a saved index, as shown by `list` and `list_indexes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexSummary {
    /// File name without `.dlidx.sqlite`; accepted wherever an index is
    /// expected.
    pub id: String,
    pub path: PathBuf,
    pub volume_name: String,
    pub volume_uuid: Option<String>,
    pub root_path: String,
    /// RFC 3339 UTC start time of the scan.
    pub scanned_at: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub error_count: u64,
}

/// A mounted volume and the indexes the library holds for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeStatus {
    pub volume: VolumeInfo,
    pub index_count: usize,
    /// Start time of the newest index of this volume, if any.
    pub last_scanned_at: Option<String>,
}

/// All finished indexes in `library`, newest first. Files that cannot be
/// opened as indexes are ignored.
pub fn list_indexes(library: &Path) -> Vec<IndexEntry> {
    let Ok(entries) = std::fs::read_dir(library) else {
        return Vec::new();
    };
    let mut indexes: Vec<IndexEntry> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(INDEX_SUFFIX))
        .filter_map(|path| {
            let info = Index::open(&path).ok()?.scan_info().ok()?;
            info.finished_at.as_ref()?;
            Some(IndexEntry { path, info })
        })
        .collect();
    indexes.sort_by(|a, b| b.info.started_at.cmp(&a.info.started_at));
    indexes
}

/// The newest index of the same root on the same volume, for incremental
/// rescans. Volumes are matched by UUID when both sides have one, otherwise
/// by name. Roots are matched by their path relative to the mount point, so
/// a drive that mounts at a different path (`/Volumes/ByHola_1 1`) still
/// finds its earlier scans when the UUID matches.
pub fn find_previous_index(library: &Path, volume: &VolumeInfo, root: &Path) -> Option<IndexEntry> {
    let current_rel = root.strip_prefix(&volume.mount_point).unwrap_or(root);
    list_indexes(library).into_iter().find(|entry| {
        let same_volume = match (&entry.info.volume_uuid, &volume.uuid) {
            (Some(a), Some(b)) => a == b,
            _ => entry.info.volume_name == volume.name,
        };
        same_volume && relative_root(&entry.info) == current_rel
    })
}

fn relative_root(info: &ScanInfo) -> PathBuf {
    Path::new(&info.root_path)
        .strip_prefix(&info.mount_point)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| PathBuf::from(&info.root_path))
}

/// Find an index by ID (exact, or a unique prefix).
pub fn find_index(library: &Path, id: &str) -> Result<IndexEntry> {
    let indexes = list_indexes(library);
    if let Some(entry) = indexes.iter().find(|e| e.id() == id) {
        return Ok(entry.clone());
    }
    let mut matches = indexes.into_iter().filter(|e| e.id().starts_with(id));
    match (matches.next(), matches.next()) {
        (Some(entry), None) => Ok(entry),
        (Some(_), Some(_)) => Err(Error::AmbiguousIndex(id.to_string())),
        _ => Err(Error::IndexNotFound(id.to_string())),
    }
}

/// Mounted volumes with the number of library indexes for each.
pub fn volume_statuses(library: &Path, probe: &dyn VolumeProbe) -> Vec<VolumeStatus> {
    let indexes = list_indexes(library);
    list_volumes(probe)
        .into_iter()
        .map(|volume| {
            let mine: Vec<&IndexEntry> = indexes
                .iter()
                .filter(|e| match (&e.info.volume_uuid, &volume.uuid) {
                    (Some(a), Some(b)) => a == b,
                    _ => e.info.volume_name == volume.name,
                })
                .collect();
            VolumeStatus {
                index_count: mine.len(),
                last_scanned_at: mine.first().map(|e| e.info.started_at.clone()),
                volume,
            }
        })
        .collect()
}
