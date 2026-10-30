//! Deep verification: full BLAKE3 of matched files on both sides.
//!
//! `compare` matches on sampled fingerprints. Before anything is deleted,
//! `verify` reads every byte of each matched pair and confirms the content
//! is identical. Files on both drives are only opened read-only. The full
//! hashes are stored in the two index files (in the library) so later
//! lookups can see that a file was verified.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rayon::prelude::*;
use rusqlite::params;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::compare::{CompareOptions, IndexRef, Status, Totals, compare};
use crate::error::Result;
use crate::fingerprint;
use crate::index::{FileRecord, Index};
use crate::volume::ensure_available;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct VerifyOptions {
    /// Only verify source files below this folder.
    pub source_subpath: Option<String>,
    /// Only verify these source files (relative paths). Empty means all.
    pub paths: Vec<String>,
    /// Files hashed in parallel (default 2: full reads are I/O bound).
    pub concurrency: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VerifyStatus {
    /// Every byte is identical.
    Verified,
    /// The files differ. The sampled fingerprints matched, so the difference
    /// lies outside the sampled regions, or a file changed since its scan.
    Mismatch,
    /// One side could not be read.
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VerifyItem {
    pub source_path: String,
    pub target_path: String,
    pub size: u64,
    pub status: VerifyStatus,
    pub source_hash: Option<String>,
    pub target_hash: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VerifySummary {
    pub verified: Totals,
    pub mismatch: Totals,
    pub unreadable: Totals,
    /// Source files in scope that had no content match on the target, so
    /// there was nothing to verify against.
    pub not_found_on_target: Totals,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VerifyReport {
    pub source: IndexRef,
    pub target: IndexRef,
    pub summary: VerifySummary,
    pub items: Vec<VerifyItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct VerifyProgress {
    pub pairs_done: u64,
    pub pairs_total: u64,
    pub bytes_hashed: u64,
    pub current_path: String,
}

pub type VerifyProgressFn<'a> = &'a (dyn Fn(&VerifyProgress) + Sync);

/// Verify the matched pairs between two saved indexes. Both scan roots must
/// be mounted; otherwise this fails with [`Error::TargetUnavailable`].
pub fn verify(
    source_path: &Path,
    target_path: &Path,
    options: &VerifyOptions,
    progress: VerifyProgressFn<'_>,
) -> Result<VerifyReport> {
    let source = Index::open(source_path)?;
    let target = Index::open(target_path)?;
    let source_root = PathBuf::from(source.scan_info()?.root_path);
    let target_root = PathBuf::from(target.scan_info()?.root_path);
    ensure_available(&target_root)?;
    ensure_available(&source_root)?;

    let comparison = compare(
        &source,
        &target,
        &CompareOptions {
            source_subpath: options.source_subpath.clone(),
            ..CompareOptions::default()
        },
    )?;
    let wanted: Vec<String> = options
        .paths
        .iter()
        .map(|p| p.nfc().collect::<String>().trim_matches('/').to_string())
        .collect();
    let in_scope = comparison
        .files
        .iter()
        .filter(|f| wanted.is_empty() || wanted.contains(&f.rel_path));

    let mut summary = VerifySummary::default();
    let mut pairs = Vec::new();
    for file in in_scope {
        match (file.status, file.target_locations.first()) {
            (Status::Present | Status::PresentElsewhere | Status::OnlyInBundle, Some(location)) => {
                pairs.push((file.rel_path.clone(), location.rel_path.clone(), file.size))
            }
            _ => {
                summary.not_found_on_target.files += 1;
                summary.not_found_on_target.bytes += file.size;
            }
        }
    }

    let source_files = files_by_nfc_path(&source)?;
    let target_files = files_by_nfc_path(&target)?;
    let total = pairs.len() as u64;
    let done = AtomicU64::new(0);
    let hashed = AtomicU64::new(0);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.concurrency.unwrap_or(2).max(1))
        .build()
        .expect("failed to build verify thread pool");

    let items: Vec<VerifyItem> = pool.install(|| {
        pairs
            .par_iter()
            .map(|(src_rel, tgt_rel, size)| {
                progress(&VerifyProgress {
                    pairs_done: done.load(Ordering::Relaxed),
                    pairs_total: total,
                    bytes_hashed: hashed.load(Ordering::Relaxed),
                    current_path: src_rel.clone(),
                });
                // Index records keep the on-disk spelling (possibly NFD).
                let src_disk = source_files.get(src_rel).map_or(src_rel, |f| &f.rel_path);
                let tgt_disk = target_files.get(tgt_rel).map_or(tgt_rel, |f| &f.rel_path);
                let item = verify_pair(
                    &source_root.join(src_disk),
                    &target_root.join(tgt_disk),
                    src_rel,
                    tgt_rel,
                    *size,
                );
                hashed.fetch_add(2 * size, Ordering::Relaxed);
                done.fetch_add(1, Ordering::Relaxed);
                item
            })
            .collect()
    });

    for item in &items {
        let totals = match item.status {
            VerifyStatus::Verified => &mut summary.verified,
            VerifyStatus::Mismatch => &mut summary.mismatch,
            VerifyStatus::Unreadable => &mut summary.unreadable,
        };
        totals.files += 1;
        totals.bytes += item.size;
    }
    progress(&VerifyProgress {
        pairs_done: total,
        pairs_total: total,
        bytes_hashed: hashed.load(Ordering::Relaxed),
        current_path: String::new(),
    });

    drop((source, target));
    store_full_hashes(source_path, &items, |i| (&i.source_path, &i.source_hash))?;
    store_full_hashes(target_path, &items, |i| (&i.target_path, &i.target_hash))?;

    Ok(VerifyReport {
        source: comparison.source,
        target: comparison.target,
        summary,
        items,
    })
}

fn files_by_nfc_path(index: &Index) -> Result<std::collections::HashMap<String, FileRecord>> {
    Ok(index
        .files()?
        .into_iter()
        .map(|f| (f.rel_path_nfc.clone(), f))
        .collect())
}

fn verify_pair(
    source: &Path,
    target: &Path,
    source_rel: &str,
    target_rel: &str,
    size: u64,
) -> VerifyItem {
    let mut item = VerifyItem {
        source_path: source_rel.to_string(),
        target_path: target_rel.to_string(),
        size,
        status: VerifyStatus::Unreadable,
        source_hash: None,
        target_hash: None,
        message: None,
    };
    let source_hash = fingerprint::full_hash(source);
    let target_hash = fingerprint::full_hash(target);
    match (source_hash, target_hash) {
        (Ok(s), Ok(t)) => {
            item.status = if s.hex == t.hex {
                VerifyStatus::Verified
            } else {
                VerifyStatus::Mismatch
            };
            if s.bytes_read != size || t.bytes_read != size {
                item.message = Some("file size changed since the scan".into());
            }
            item.source_hash = Some(s.hex);
            item.target_hash = Some(t.hex);
        }
        (s, t) => {
            let mut problems = Vec::new();
            if let Err(e) = &s {
                problems.push(format!("source: {e}"));
            }
            if let Err(e) = &t {
                problems.push(format!("target: {e}"));
            }
            item.message = Some(problems.join("; "));
            item.source_hash = s.ok().map(|f| f.hex);
            item.target_hash = t.ok().map(|f| f.hex);
        }
    }
    item
}

/// Record full hashes of files whose size is unchanged. Writes only to the
/// index file in the library.
fn store_full_hashes(
    index_path: &Path,
    items: &[VerifyItem],
    side: impl Fn(&VerifyItem) -> (&String, &Option<String>),
) -> Result<()> {
    let mut index = Index::open_writable(index_path)?;
    let tx = index.conn_mut().transaction()?;
    {
        let mut stmt = tx.prepare(
            "UPDATE file SET full_hash = ?1, fingerprint_kind = 'full'
             WHERE rel_path_nfc = ?2 AND size = ?3",
        )?;
        for item in items {
            if item.message.is_some() {
                continue;
            }
            if let (path, Some(hash)) = side(item) {
                stmt.execute(params![hash, path, item.size as i64])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

impl VerifyStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            VerifyStatus::Verified => "verified",
            VerifyStatus::Mismatch => "mismatch",
            VerifyStatus::Unreadable => "unreadable",
        }
    }
}
