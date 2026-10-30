//! Groups of files with identical content, within or across indexes.

use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::index::Index;
use crate::library::IndexEntry;

/// One copy of a duplicated file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Copy {
    pub index_id: String,
    pub volume_name: String,
    pub rel_path: String,
    pub in_bundle: bool,
}

/// Files that share one fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DuplicateGroup {
    pub fingerprint: String,
    pub size: u64,
    pub copies: Vec<Copy>,
    /// Bytes that would be freed by keeping one copy: `size * (copies - 1)`.
    pub redundant_bytes: u64,
}

/// Every fingerprint that occurs more than once across `indexes`, largest
/// redundant size first. Empty files are ignored (they all share one
/// fingerprint).
pub fn find_duplicates(indexes: &[IndexEntry], min_size: u64) -> Result<Vec<DuplicateGroup>> {
    let mut groups: HashMap<String, DuplicateGroup> = HashMap::new();
    for entry in indexes {
        let index = Index::open(&entry.path)?;
        let id = entry.id();
        for file in index.files()? {
            let Some(fingerprint) = file.fingerprint else {
                continue;
            };
            if file.size == 0 || file.size < min_size {
                continue;
            }
            groups
                .entry(fingerprint.clone())
                .or_insert_with(|| DuplicateGroup {
                    fingerprint,
                    size: file.size,
                    copies: Vec::new(),
                    redundant_bytes: 0,
                })
                .copies
                .push(Copy {
                    index_id: id.clone(),
                    volume_name: entry.info.volume_name.clone(),
                    rel_path: file.rel_path_nfc,
                    in_bundle: file.in_bundle,
                });
        }
    }
    let mut result: Vec<DuplicateGroup> = groups
        .into_values()
        .filter(|g| g.copies.len() > 1)
        .map(|mut g| {
            g.redundant_bytes = g.size * (g.copies.len() as u64 - 1);
            g
        })
        .collect();
    result.sort_by(|a, b| {
        b.redundant_bytes
            .cmp(&a.redundant_bytes)
            .then_with(|| a.fingerprint.cmp(&b.fingerprint))
    });
    Ok(result)
}
