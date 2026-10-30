//! The library folder: where index files live.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::index::{INDEX_SUFFIX, Index, ScanInfo};
use crate::volume::VolumeInfo;

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
