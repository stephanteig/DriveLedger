//! Folder overview: size and file count per folder, a few levels deep.
//! Mirrors `reference/scripts/mapper.sh`.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::error::Result;
use crate::index::Index;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FolderStat {
    /// Folder path relative to the scan root (or to `subpath`).
    pub path: String,
    /// Depth below the root, starting at 1.
    pub depth: usize,
    /// Files anywhere below the folder, not counting names starting with `.`
    /// (like `mapper.sh`).
    pub files: u64,
    /// Bytes of all files below the folder.
    pub bytes: u64,
}

/// Every folder down to `depth` levels below the root (or below `subpath`),
/// with recursive totals, ordered by path. Top-level hidden folders are left
/// out, as in `mapper.sh`. Folders that contain no files do not appear,
/// because the index only records files.
pub fn folder_overview(
    index: &Index,
    depth: usize,
    subpath: Option<&str>,
) -> Result<Vec<FolderStat>> {
    let prefix: Option<String> = subpath
        .map(|s| s.nfc().collect::<String>().trim_matches('/').to_string())
        .filter(|s| !s.is_empty());

    let mut folders: BTreeMap<String, FolderStat> = BTreeMap::new();
    for file in index.files()? {
        let rel = match &prefix {
            Some(p) => match file.rel_path_nfc.strip_prefix(p.as_str()) {
                Some(rest) if rest.starts_with('/') => rest[1..].to_string(),
                _ => continue,
            },
            None => file.rel_path_nfc.clone(),
        };
        let parts: Vec<&str> = rel.split('/').collect();
        let dirs = &parts[..parts.len() - 1];
        if dirs.first().is_some_and(|d| d.starts_with('.')) {
            continue;
        }
        let visible = !file.name_nfc.starts_with('.');
        for level in 1..=dirs.len().min(depth) {
            let path = dirs[..level].join("/");
            let entry = folders.entry(path.clone()).or_insert(FolderStat {
                path,
                depth: level,
                files: 0,
                bytes: 0,
            });
            entry.bytes += file.size;
            if visible {
                entry.files += 1;
            }
        }
    }
    Ok(folders.into_values().collect())
}
