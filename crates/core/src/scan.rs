//! Walking a folder tree into an index.
//!
//! Safety: this module only ever *reads* below the scan root (directory
//! listings, metadata and, for fingerprints, read-only file handles). Every
//! write goes to the index file, which is refused if it would land inside the
//! scan root.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Local, SecondsFormat, Utc};
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use walkdir::WalkDir;

use crate::error::{Error, Result};
use crate::fingerprint;
use crate::index::{FileRecord, FingerprintKind, INDEX_SUFFIX, Index, ScanError, ScanInfo};
use crate::library::find_previous_index;
use crate::volume::{VolumeInfo, VolumeProbe};

/// Names that are never indexed. Matching entries are skipped entirely
/// (including everything below a matching folder).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkipList {
    /// Exact file or folder names.
    pub names: Vec<String>,
    /// Name prefixes, e.g. `._` for AppleDouble files.
    pub prefixes: Vec<String>,
}

impl Default for SkipList {
    fn default() -> Self {
        let names = [
            ".DS_Store",
            "Icon\r",
            ".Spotlight-V100",
            ".fseventsd",
            ".Trashes",
            ".TemporaryItems",
            ".DocumentRevisions-V100",
        ];
        SkipList {
            names: names.iter().map(|s| s.to_string()).collect(),
            prefixes: vec!["._".to_string()],
        }
    }
}

impl SkipList {
    pub fn matches(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name) || self.prefixes.iter().any(|p| name.starts_with(p))
    }
}

/// Folder extensions treated as macOS packages. They are scanned like normal
/// folders, but files inside them get `in_bundle = true`.
pub const DEFAULT_BUNDLE_EXTENSIONS: &[&str] = &[
    "dra",
    "app",
    "fcpbundle",
    "rdc",
    "photoslibrary",
    "band",
    "logicx",
];

/// Options for a scan.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub skip: SkipList,
    /// Lower-case folder extensions that mark a bundle.
    pub bundle_extensions: Vec<String>,
    /// Number of files processed in parallel. `None` picks a default based on
    /// the volume type.
    pub concurrency: Option<usize>,
    /// Set to `true` from another thread to stop the scan.
    pub cancel: Option<Arc<AtomicBool>>,
    /// Reuse fingerprints from the newest earlier index of the same volume
    /// and root in the library, for files whose path, size and mtime are
    /// unchanged.
    pub incremental: bool,
    /// Use this index for reuse instead of searching the library.
    pub previous_index: Option<PathBuf>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions {
            skip: SkipList::default(),
            bundle_extensions: DEFAULT_BUNDLE_EXTENSIONS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            concurrency: None,
            cancel: None,
            incremental: true,
            previous_index: None,
        }
    }
}

impl ScanOptions {
    fn is_cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
    }
}

/// Which stage a scan is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScanPhase {
    Walking,
    Processing,
    Writing,
    Done,
}

/// A progress update, delivered through the callback passed to the scan.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScanProgress {
    pub phase: ScanPhase,
    /// Files found by the walk so far.
    pub files_seen: u64,
    /// Files whose metadata (and fingerprint) are done.
    pub files_done: u64,
    /// Bytes read for fingerprinting so far.
    pub bytes_hashed: u64,
    pub errors: u64,
    pub current_path: String,
}

/// Progress callback. Called from worker threads, so it must be `Sync`.
pub type ProgressFn<'a> = &'a (dyn Fn(&ScanProgress) + Sync);

/// Counters describing how a scan did its work.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScanStats {
    /// Files fingerprinted by reading their content.
    pub files_hashed: u64,
    /// Files whose fingerprint was reused from the previous index.
    pub files_reused: u64,
    pub bytes_hashed: u64,
    /// The index fingerprints were reused from, if any.
    pub previous_index: Option<PathBuf>,
}

/// Result of [`scan_to_library`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScanOutcome {
    pub index_path: PathBuf,
    pub info: ScanInfo,
    pub stats: ScanStats,
}

/// Scan `root` and write a new index file into `library`.
///
/// The file is named `<volume>__<root-slug>__<YYYYMMDD-HHMMSS>.dlidx.sqlite`.
/// It is written under a `.partial` name and renamed on success, so a crashed
/// or cancelled scan never leaves a half-written index that looks complete.
pub fn scan_to_library(
    root: &Path,
    library: &Path,
    options: &ScanOptions,
    probe: &dyn VolumeProbe,
    progress: ProgressFn<'_>,
) -> Result<ScanOutcome> {
    // An unmounted drive can leave an empty `/Volumes/<name>` folder behind.
    // Scanning it would produce an empty index that makes every file look
    // missing in a later compare, so refuse it up front.
    crate::volume::ensure_available(root)?;
    let root = canonical_root(root)?;
    ensure_library_outside_root(library, &root)?;
    std::fs::create_dir_all(library).map_err(|e| Error::io(library, e))?;

    let volume = probe.probe(&root)?;
    let previous_path = match &options.previous_index {
        Some(path) => Some(path.clone()),
        None if options.incremental => {
            find_previous_index(library, &volume, &root).map(|entry| entry.path)
        }
        None => None,
    };
    let previous = match &previous_path {
        Some(path) => Some(Index::open(path)?),
        None => None,
    };

    let final_path = unique_index_path(library, &volume, &root);
    let partial_path = final_path.with_extension("partial");
    // A leftover partial file from a crashed run with the same timestamp
    // lives in the library, so removing it is allowed.
    let _ = std::fs::remove_file(&partial_path);

    let result = (|| {
        let mut index = Index::create(&partial_path)?;
        let (info, mut stats) = scan_into_index(
            &root,
            &volume,
            &mut index,
            options,
            previous.as_ref(),
            progress,
        )?;
        drop(index);
        std::fs::rename(&partial_path, &final_path).map_err(|e| Error::io(&final_path, e))?;
        if stats.files_reused > 0 {
            stats.previous_index = previous_path.clone();
        }
        Ok((info, stats))
    })();

    match result {
        Ok((info, stats)) => Ok(ScanOutcome {
            index_path: final_path,
            info,
            stats,
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&partial_path);
            Err(e)
        }
    }
}

/// Scan `root` into an already-open (empty) index. `root` must be canonical.
/// When `previous` is given, fingerprints of unchanged files are reused.
pub fn scan_into_index(
    root: &Path,
    volume: &VolumeInfo,
    index: &mut Index,
    options: &ScanOptions,
    previous: Option<&Index>,
    progress: ProgressFn<'_>,
) -> Result<(ScanInfo, ScanStats)> {
    let mut info = ScanInfo {
        volume_name: volume.name.clone(),
        volume_uuid: volume.uuid.clone(),
        fs_type: volume.fs_type.clone(),
        mount_point: volume.mount_point.to_string_lossy().into_owned(),
        root_path: root.to_string_lossy().into_owned(),
        started_at: now_rfc3339(),
        finished_at: None,
        tool_version: crate::TOOL_VERSION.to_string(),
        file_count: 0,
        total_bytes: 0,
        error_count: 0,
    };
    index.write_scan_info(&info)?;

    let reusable = match previous {
        Some(prev) => reusable_fingerprints(prev)?,
        None => HashMap::new(),
    };
    let (candidates, mut errors) = walk(root, options, progress)?;

    let concurrency = options
        .concurrency
        .unwrap_or_else(|| default_concurrency(volume))
        .max(1);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(concurrency)
        .build()
        .expect("failed to build scan thread pool");

    let files_seen = candidates.len() as u64;
    let files_done = AtomicU64::new(0);
    let bytes_hashed = AtomicU64::new(0);
    let error_count = AtomicU64::new(errors.len() as u64);

    let results: Vec<Processed> = pool.install(|| {
        candidates
            .par_iter()
            .map(|candidate| {
                if options.is_cancelled() {
                    return Processed::default();
                }
                let processed = process_file(candidate, &reusable);
                bytes_hashed.fetch_add(processed.bytes_read, Ordering::Relaxed);
                if processed.error.is_some() {
                    error_count.fetch_add(1, Ordering::Relaxed);
                }
                let done = files_done.fetch_add(1, Ordering::Relaxed) + 1;
                progress(&ScanProgress {
                    phase: ScanPhase::Processing,
                    files_seen,
                    files_done: done,
                    bytes_hashed: bytes_hashed.load(Ordering::Relaxed),
                    errors: error_count.load(Ordering::Relaxed),
                    current_path: candidate.rel_path.clone(),
                });
                processed
            })
            .collect()
    });
    if options.is_cancelled() {
        return Err(Error::Cancelled);
    }

    let mut stats = ScanStats::default();
    let mut files = Vec::with_capacity(results.len());
    for processed in results {
        let fingerprinted = processed
            .file
            .as_ref()
            .is_some_and(|f| f.fingerprint.is_some());
        if processed.reused {
            stats.files_reused += 1;
        } else if fingerprinted {
            stats.files_hashed += 1;
        }
        stats.bytes_hashed += processed.bytes_read;
        files.extend(processed.file);
        errors.extend(processed.error);
    }

    progress(&ScanProgress {
        phase: ScanPhase::Writing,
        files_seen,
        files_done: files.len() as u64,
        bytes_hashed: bytes_hashed.load(Ordering::Relaxed),
        errors: errors.len() as u64,
        current_path: String::new(),
    });
    index.insert_files(&files)?;
    index.insert_errors(&errors)?;

    info.file_count = files.len() as u64;
    info.total_bytes = files.iter().map(|f| f.size).sum();
    info.error_count = errors.len() as u64;
    info.finished_at = Some(now_rfc3339());
    index.write_scan_info(&info)?;

    progress(&ScanProgress {
        phase: ScanPhase::Done,
        files_seen,
        files_done: info.file_count,
        bytes_hashed: bytes_hashed.load(Ordering::Relaxed),
        errors: info.error_count,
        current_path: String::new(),
    });
    Ok((info, stats))
}

/// A regular file found by the walk, not yet stat'ed or fingerprinted.
#[derive(Debug)]
struct Candidate {
    abs_path: PathBuf,
    rel_path: String,
    in_bundle: bool,
}

/// Walk the tree and collect regular files. Unreadable folders and other
/// problems become [`ScanError`]s instead of aborting the scan.
fn walk(
    root: &Path,
    options: &ScanOptions,
    progress: ProgressFn<'_>,
) -> Result<(Vec<Candidate>, Vec<ScanError>)> {
    let mut candidates = Vec::new();
    let mut errors = Vec::new();

    let walker = WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0 || !options.skip.matches(&entry.file_name().to_string_lossy())
        });

    for entry in walker {
        if options.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                let rel_path = err
                    .path()
                    .map(|p| relative_path(root, p).0)
                    .unwrap_or_default();
                errors.push(ScanError {
                    rel_path,
                    kind: "walk".into(),
                    message: err.to_string(),
                });
                continue;
            }
        };

        let file_type = entry.file_type();
        if file_type.is_dir() {
            continue;
        }
        let (rel_path, lossless) = relative_path(root, entry.path());
        if !lossless {
            errors.push(ScanError {
                rel_path: rel_path.clone(),
                kind: "non_utf8_name".into(),
                message: "path is not valid UTF-8; stored with replacement characters".into(),
            });
        }
        if file_type.is_symlink() {
            errors.push(ScanError {
                rel_path,
                kind: "skipped_symlink".into(),
                message: "symbolic links are not followed or indexed".into(),
            });
            continue;
        }
        if !file_type.is_file() {
            errors.push(ScanError {
                rel_path,
                kind: "skipped_special".into(),
                message: "not a regular file".into(),
            });
            continue;
        }

        let in_bundle = is_in_bundle(&rel_path, &options.bundle_extensions);
        candidates.push(Candidate {
            abs_path: entry.into_path(),
            rel_path,
            in_bundle,
        });
        if candidates.len() % 256 == 1 {
            progress(&ScanProgress {
                phase: ScanPhase::Walking,
                files_seen: candidates.len() as u64,
                files_done: 0,
                bytes_hashed: 0,
                errors: errors.len() as u64,
                current_path: candidates
                    .last()
                    .map(|c| c.rel_path.clone())
                    .unwrap_or_default(),
            });
        }
    }
    Ok((candidates, errors))
}

/// Fingerprint data that may be carried over from a previous scan, keyed by
/// the on-disk relative path.
struct Reusable {
    size: u64,
    mtime: Option<i64>,
    fingerprint: String,
    full_hash: Option<String>,
}

fn reusable_fingerprints(previous: &Index) -> Result<HashMap<String, Reusable>> {
    Ok(previous
        .files()?
        .into_iter()
        .filter_map(|f| {
            let fingerprint = f.fingerprint?;
            Some((
                f.rel_path,
                Reusable {
                    size: f.size,
                    mtime: f.mtime,
                    fingerprint,
                    full_hash: f.full_hash,
                },
            ))
        })
        .collect())
}

/// Outcome of processing one file. A file that could be stat'ed but not
/// read has both a record (without fingerprint) and an error.
#[derive(Default)]
struct Processed {
    file: Option<FileRecord>,
    error: Option<ScanError>,
    reused: bool,
    bytes_read: u64,
}

/// Read metadata for one file, fingerprint it (or reuse the previous
/// fingerprint if path, size and mtime are unchanged) and build its record.
fn process_file(candidate: &Candidate, reusable: &HashMap<String, Reusable>) -> Processed {
    let error = |kind: &str, e: std::io::Error| ScanError {
        rel_path: candidate.rel_path.clone(),
        kind: kind.into(),
        message: e.to_string(),
    };
    let meta = match std::fs::symlink_metadata(&candidate.abs_path) {
        Ok(meta) => meta,
        Err(e) => {
            return Processed {
                error: Some(error("metadata", e)),
                ..Processed::default()
            };
        }
    };

    let rel_path_nfc: String = candidate.rel_path.nfc().collect();
    let name_nfc = rel_path_nfc
        .rsplit('/')
        .next()
        .unwrap_or(&rel_path_nfc)
        .to_string();
    let ext = Path::new(&name_nfc)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let size = meta.len();
    let mtime = meta.modified().ok().and_then(system_time_to_nanos);

    let mut processed = Processed::default();
    let mut record = FileRecord {
        rel_path: candidate.rel_path.clone(),
        rel_path_nfc,
        name_nfc,
        ext,
        size,
        mtime,
        ctime: meta.created().ok().and_then(system_time_to_nanos),
        in_bundle: candidate.in_bundle,
        fingerprint: None,
        fingerprint_kind: None,
        full_hash: None,
    };

    let previous = reusable
        .get(&candidate.rel_path)
        .filter(|prev| prev.size == size && prev.mtime.is_some() && prev.mtime == mtime);
    match previous {
        Some(prev) => {
            record.fingerprint = Some(prev.fingerprint.clone());
            record.full_hash = prev.full_hash.clone();
            processed.reused = true;
        }
        None => match fingerprint::sample3(&candidate.abs_path) {
            Ok(fp) => {
                record.fingerprint = Some(fp.hex);
                processed.bytes_read = fp.bytes_read;
            }
            Err(e) => processed.error = Some(error("read", e)),
        },
    }
    if record.fingerprint.is_some() {
        record.fingerprint_kind = Some(if record.full_hash.is_some() {
            FingerprintKind::Full
        } else {
            FingerprintKind::Sample3
        });
    }
    processed.file = Some(record);
    processed
}

fn default_concurrency(volume: &VolumeInfo) -> usize {
    if volume.is_network() { 2 } else { 4 }
}

/// True if any folder above the file has a bundle extension.
fn is_in_bundle(rel_path: &str, bundle_extensions: &[String]) -> bool {
    let mut parts: Vec<&str> = rel_path.split('/').collect();
    parts.pop(); // the file name itself never marks a bundle
    parts.iter().any(|dir| {
        dir.rsplit_once('.').is_some_and(|(stem, ext)| {
            !stem.is_empty()
                && bundle_extensions
                    .iter()
                    .any(|b| b.eq_ignore_ascii_case(ext))
        })
    })
}

/// `path` relative to `root`, joined with `/`. The flag is false if the path
/// was not valid UTF-8 and had to be converted lossily.
fn relative_path(root: &Path, path: &Path) -> (String, bool) {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let mut lossless = true;
    let parts: Vec<String> = rel
        .components()
        .map(|c| {
            let os = c.as_os_str();
            match os.to_str() {
                Some(s) => s.to_string(),
                None => {
                    lossless = false;
                    os.to_string_lossy().into_owned()
                }
            }
        })
        .collect();
    (parts.join("/"), lossless)
}

fn canonical_root(root: &Path) -> Result<PathBuf> {
    let canonical = root
        .canonicalize()
        .map_err(|_| Error::InvalidRoot(root.to_path_buf()))?;
    if !canonical.is_dir() {
        return Err(Error::InvalidRoot(root.to_path_buf()));
    }
    Ok(canonical)
}

/// Refuse a library folder that is (or would be) inside the scan root.
/// `root` must be canonical. The library may not exist yet, so the check
/// resolves its nearest existing ancestor.
pub fn ensure_library_outside_root(library: &Path, root: &Path) -> Result<()> {
    let library_abs = if library.is_absolute() {
        library.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| Error::io(library, e))?
            .join(library)
    };
    let mut existing = library_abs.as_path();
    let mut tail = Vec::new();
    while !existing.exists() {
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_os_string());
                existing = parent;
            }
            _ => break,
        }
    }
    let mut resolved = existing
        .canonicalize()
        .unwrap_or_else(|_| existing.to_path_buf());
    for part in tail.iter().rev() {
        resolved.push(part);
    }
    if resolved.starts_with(root) {
        return Err(Error::LibraryInsideScanRoot {
            library: library.to_path_buf(),
            root: root.to_path_buf(),
        });
    }
    Ok(())
}

/// Pick `<library>/<volume>__<root-slug>__<timestamp>.dlidx.sqlite`, adding a
/// counter if a scan of the same root already exists for this second.
fn unique_index_path(library: &Path, volume: &VolumeInfo, root: &Path) -> PathBuf {
    let volume_part = slugify(&volume.name);
    let rel_root = root.strip_prefix(&volume.mount_point).unwrap_or(root);
    let root_part = {
        let parts: Vec<String> = rel_root
            .components()
            .map(|c| slugify(&c.as_os_str().to_string_lossy()))
            .filter(|s| !s.is_empty())
            .collect();
        if parts.is_empty() {
            "root".to_string()
        } else {
            parts.join("-")
        }
    };
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    let base = format!("{volume_part}__{root_part}__{stamp}");
    let mut candidate = library.join(format!("{base}{INDEX_SUFFIX}"));
    let mut n = 2;
    while candidate.exists() || candidate.with_extension("partial").exists() {
        candidate = library.join(format!("{base}-{n}{INDEX_SUFFIX}"));
        n += 1;
    }
    candidate
}

/// Make a string safe for a file name: keep letters, digits, `-`, `_` and
/// `.`; turn spaces into `-` and everything else into `_`.
fn slugify(s: &str) -> String {
    s.nfc()
        .map(|c| match c {
            c if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' => c,
            ' ' => '-',
            _ => '_',
        })
        .collect::<String>()
        .trim_matches('.')
        .to_string()
}

fn system_time_to_nanos(t: SystemTime) -> Option<i64> {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_nanos()).ok(),
        Err(e) => i64::try_from(e.duration().as_nanos()).ok().map(|n| -n),
    }
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_skip_list() {
        let skip = SkipList::default();
        for name in [".DS_Store", "._SW-FX30001.MP4", "Icon\r", ".Trashes"] {
            assert!(skip.matches(name), "{name:?} should be skipped");
        }
        for name in ["SW-FX30001.MP4", "Icon", ".hidden"] {
            assert!(!skip.matches(name), "{name:?} should be kept");
        }
    }

    #[test]
    fn bundle_detection() {
        let exts: Vec<String> = DEFAULT_BUNDLE_EXTENSIONS
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(is_in_bundle("P/x.dra/MediaFiles/a.png", &exts));
        assert!(is_in_bundle("A001.RDC/A001_C001.R3D", &exts));
        assert!(!is_in_bundle("P/x.dra", &exts));
        assert!(!is_in_bundle("Projects/.dra/a.png", &exts));
        assert!(!is_in_bundle("HYSJ/2026/a.png", &exts));
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("ByHola 1"), "ByHola-1");
        assert_eq!(slugify("_STØY"), "_STØY");
        assert_eq!(slugify("a:b/c"), "a_b_c");
    }

    #[test]
    fn library_inside_root_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let err = ensure_library_outside_root(&root.join("lib/new"), &root).unwrap_err();
        assert!(matches!(err, Error::LibraryInsideScanRoot { .. }));
        assert!(ensure_library_outside_root(&root, &root).is_err());
        let other = tempfile::tempdir().unwrap();
        ensure_library_outside_root(other.path(), &root).unwrap();
    }
}
