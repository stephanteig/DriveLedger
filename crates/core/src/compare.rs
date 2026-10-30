//! Comparing a source index against a target index.
//!
//! Every source file is matched against the *entire* target, regardless of
//! folder structure, by content fingerprint. Names are only used to describe
//! a match (same name or not) and to explain a miss (a file with the same
//! name exists but holds different content). A name and size match alone is
//! never treated as a copy: cameras reuse file names on every card.
//!
//! The full rules, with the real cases they came from, are in
//! `docs/matching.md`.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use chrono::{DateTime, SecondsFormat, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::error::{Error, Result};
use crate::index::{FileRecord, Index, ScanInfo};
use crate::scan::{ProgressFn, ScanOptions, ScanOutcome, scan_to_library};
use crate::volume::{VolumeProbe, ensure_available};

/// Classification of one source file. Ordered by precedence.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Identical content exists on the target under the same file name.
    Present,
    /// Identical content exists on the target, but only under another name.
    PresentElsewhere,
    /// Identical content exists on the target, but only inside bundles such
    /// as `.dra/MediaFiles`, which are not a proper home for the file.
    OnlyInBundle,
    /// No identical content; a file with the same name but different content
    /// exists. Counts as *not* backed up.
    SameNameDifferentContent,
    /// No identical content anywhere on the target.
    Missing,
}

impl Status {
    pub const ALL: [Status; 5] = [
        Status::Present,
        Status::PresentElsewhere,
        Status::OnlyInBundle,
        Status::SameNameDifferentContent,
        Status::Missing,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Present => "present",
            Status::PresentElsewhere => "present_elsewhere",
            Status::OnlyInBundle => "only_in_bundle",
            Status::SameNameDifferentContent => "same_name_different_content",
            Status::Missing => "missing",
        }
    }

    /// True if identical content was found somewhere on the target.
    pub fn content_found(self) -> bool {
        matches!(
            self,
            Status::Present | Status::PresentElsewhere | Status::OnlyInBundle
        )
    }
}

impl std::str::FromStr for Status {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Status::ALL
            .into_iter()
            .find(|status| status.as_str() == s)
            .ok_or_else(|| format!("unknown status `{s}`"))
    }
}

/// Which side holds the newer modification time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Source,
    Target,
    Same,
}

/// Extra observations attached to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Hint {
    /// The source is smaller than the same-name target file and its size is a
    /// multiple of 4096, the typical signature of an interrupted copy.
    PossiblyTruncated,
    /// The source file could not be read during the scan, so its content
    /// could not be matched. It is reported as missing to stay on the safe
    /// side.
    SourceUnreadable,
}

/// A file on the target that relates to a source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Location {
    pub rel_path: String,
    pub size: u64,
    /// RFC 3339 UTC modification time.
    pub modified: Option<String>,
    pub in_bundle: bool,
}

/// The comparison result for one source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileComparison {
    /// NFC path relative to the source root.
    pub rel_path: String,
    pub size: u64,
    pub modified: Option<String>,
    pub status: Status,
    /// For `present`, `present_elsewhere` and `only_in_bundle`: every target
    /// file with identical content. For `same_name_different_content`: every
    /// target file with the same name, closest match first. Empty otherwise.
    pub target_locations: Vec<Location>,
    /// For `same_name_different_content`: which side is newer, judged against
    /// the first entry of `target_locations`.
    pub newer_side: Option<Side>,
    pub hints: Vec<Hint>,
}

/// File count and byte total.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Totals {
    pub files: u64,
    pub bytes: u64,
}

impl Totals {
    fn add(&mut self, bytes: u64) {
        self.files += 1;
        self.bytes += bytes;
    }
}

/// Totals for each status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatusTotals {
    pub present: Totals,
    pub present_elsewhere: Totals,
    pub only_in_bundle: Totals,
    pub same_name_different_content: Totals,
    pub missing: Totals,
}

impl StatusTotals {
    pub fn get(&self, status: Status) -> Totals {
        match status {
            Status::Present => self.present,
            Status::PresentElsewhere => self.present_elsewhere,
            Status::OnlyInBundle => self.only_in_bundle,
            Status::SameNameDifferentContent => self.same_name_different_content,
            Status::Missing => self.missing,
        }
    }

    fn get_mut(&mut self, status: Status) -> &mut Totals {
        match status {
            Status::Present => &mut self.present,
            Status::PresentElsewhere => &mut self.present_elsewhere,
            Status::OnlyInBundle => &mut self.only_in_bundle,
            Status::SameNameDifferentContent => &mut self.same_name_different_content,
            Status::Missing => &mut self.missing,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompareSummary {
    pub total: Totals,
    /// Identical content found on the target (present, present elsewhere,
    /// only in bundle).
    pub content_found: Totals,
    /// No identical content on the target (same name different content,
    /// missing). These files are not backed up.
    pub content_not_found: Totals,
    pub by_status: StatusTotals,
}

/// Per-folder totals, grouped a few levels deep.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FolderRollup {
    /// Folder relative to the source root, cut to the rollup depth. `.` for
    /// files directly in the root.
    pub folder: String,
    pub total: Totals,
    pub by_status: StatusTotals,
}

/// Which index a side of the comparison came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IndexRef {
    pub index_path: Option<String>,
    pub volume_name: String,
    pub root_path: String,
    pub scanned_at: String,
    pub file_count: u64,
}

impl IndexRef {
    fn new(index: &Index, info: &ScanInfo) -> Self {
        IndexRef {
            index_path: index.path().map(|p| p.to_string_lossy().into_owned()),
            volume_name: info.volume_name.clone(),
            root_path: info.root_path.clone(),
            scanned_at: info.started_at.clone(),
            file_count: info.file_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompareResult {
    pub source: IndexRef,
    pub target: IndexRef,
    /// Source subfolder the comparison was limited to, if any.
    pub source_subpath: Option<String>,
    pub summary: CompareSummary,
    pub folders: Vec<FolderRollup>,
    /// Every compared source file, ordered by path.
    pub files: Vec<FileComparison>,
    /// Things the caller should show the user, e.g. an empty target.
    pub warnings: Vec<String>,
}

impl CompareResult {
    /// Files with one of `statuses` (all files if empty).
    pub fn files_with_status(&self, statuses: &[Status]) -> Vec<&FileComparison> {
        self.files
            .iter()
            .filter(|f| statuses.is_empty() || statuses.contains(&f.status))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompareOptions {
    /// Only compare source files below this folder (relative to the source
    /// root).
    pub source_subpath: Option<String>,
    /// How many folder levels the rollup keeps.
    pub rollup_depth: usize,
}

impl Default for CompareOptions {
    fn default() -> Self {
        CompareOptions {
            source_subpath: None,
            rollup_depth: 3,
        }
    }
}

/// Compare two saved indexes.
pub fn compare(source: &Index, target: &Index, options: &CompareOptions) -> Result<CompareResult> {
    let source_info = source.scan_info()?;
    let target_info = target.scan_info()?;
    let mut warnings = Vec::new();
    for (side, info, index) in [
        ("source", &source_info, source),
        ("target", &target_info, target),
    ] {
        if info.finished_at.is_none() {
            return Err(Error::NotAnIndex(
                index
                    .path()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| side.into()),
            ));
        }
        if info.error_count > 0 {
            warnings.push(format!(
                "the {side} scan recorded {} unreadable paths; see its error list",
                info.error_count
            ));
        }
    }

    let subpath = options.source_subpath.as_deref().map(normalize_subpath);
    let source_files: Vec<FileRecord> = source
        .files()?
        .into_iter()
        .filter(|f| match &subpath {
            Some(sub) if !sub.is_empty() => {
                f.rel_path_nfc.starts_with(sub.as_str())
                    && f.rel_path_nfc[sub.len()..].starts_with('/')
            }
            _ => true,
        })
        .collect();
    let target_files = target.files()?;

    if target_files.is_empty() && !source_files.is_empty() {
        warnings.push(
            "the target index contains no files; check that the right drive was mounted when it was scanned"
                .to_string(),
        );
    }

    let lookup = TargetLookup::new(&target_files);
    let files: Vec<FileComparison> = source_files
        .iter()
        .map(|file| classify(file, &lookup))
        .collect();

    let (summary, folders) = summarize(&files, options.rollup_depth);

    Ok(CompareResult {
        source: IndexRef::new(source, &source_info),
        target: IndexRef::new(target, &target_info),
        source_subpath: subpath.filter(|s| !s.is_empty()),
        summary,
        folders,
        files,
        warnings,
    })
}

/// Compare a saved source index against a live target folder. The target is
/// checked first: if it is not mounted and readable this returns
/// [`Error::TargetUnavailable`] and never a list of missing files. Otherwise
/// the target is scanned into `library` (read-only on the target) and then
/// compared.
#[allow(clippy::too_many_arguments)]
pub fn compare_with_live_target(
    source: &Index,
    target_root: &Path,
    library: &Path,
    scan_options: &ScanOptions,
    probe: &dyn VolumeProbe,
    progress: ProgressFn<'_>,
    options: &CompareOptions,
) -> Result<(CompareResult, ScanOutcome)> {
    ensure_available(target_root)?;
    let outcome = scan_to_library(target_root, library, scan_options, probe, progress)?;
    let target = Index::open(&outcome.index_path)?;
    let result = compare(source, &target, options)?;
    Ok((result, outcome))
}

/// Target files indexed for the lookups the classifier needs.
struct TargetLookup<'a> {
    files: &'a [FileRecord],
    by_fingerprint: HashMap<&'a str, Vec<usize>>,
    by_name: HashMap<&'a str, Vec<usize>>,
}

impl<'a> TargetLookup<'a> {
    fn new(files: &'a [FileRecord]) -> Self {
        let mut by_fingerprint: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, f) in files.iter().enumerate() {
            if let Some(fp) = f.fingerprint.as_deref() {
                by_fingerprint.entry(fp).or_default().push(i);
            }
            by_name.entry(f.name_nfc.as_str()).or_default().push(i);
        }
        TargetLookup {
            files,
            by_fingerprint,
            by_name,
        }
    }

    fn get(&self, ids: &[usize]) -> Vec<&'a FileRecord> {
        ids.iter().map(|&i| &self.files[i]).collect()
    }

    /// Target files with identical content.
    fn content_matches(&self, source: &FileRecord) -> Vec<&'a FileRecord> {
        let Some(fp) = source.fingerprint.as_deref() else {
            return Vec::new();
        };
        let matches = self.get(self.by_fingerprint.get(fp).map_or(&[], Vec::as_slice));
        if source.size == 0 {
            // Every empty file has the same fingerprint, so content says
            // nothing. Like the prototype, an empty file only counts as found
            // when an empty file with the same name exists on the target.
            return matches
                .into_iter()
                .filter(|t| t.name_nfc == source.name_nfc)
                .collect();
        }
        matches
    }

    fn same_name(&self, source: &FileRecord) -> Vec<&'a FileRecord> {
        self.get(
            self.by_name
                .get(source.name_nfc.as_str())
                .map_or(&[], Vec::as_slice),
        )
    }
}

/// Apply the classification rules to one source file.
fn classify(source: &FileRecord, lookup: &TargetLookup<'_>) -> FileComparison {
    let mut result = FileComparison {
        rel_path: source.rel_path_nfc.clone(),
        size: source.size,
        modified: source.mtime.map(format_time),
        status: Status::Missing,
        target_locations: Vec::new(),
        newer_side: None,
        hints: Vec::new(),
    };
    if source.fingerprint.is_none() {
        result.hints.push(Hint::SourceUnreadable);
    }

    let mut matches = lookup.content_matches(source);
    if !matches.is_empty() {
        sort_by_closeness(&mut matches, source);
        // A match inside a bundle (`.dra/MediaFiles`, ...) is a copy, but not
        // a proper home for the file. The exception is a source file that is
        // itself inside a bundle (R3D clips live in `.RDC` folders): then a
        // bundle on the target is exactly where it belongs.
        let proper: Vec<&FileRecord> = matches
            .iter()
            .copied()
            .filter(|t| source.in_bundle || !t.in_bundle)
            .collect();
        result.status = if proper.iter().any(|t| t.name_nfc == source.name_nfc) {
            Status::Present
        } else if !proper.is_empty() {
            Status::PresentElsewhere
        } else {
            Status::OnlyInBundle
        };
        result.target_locations = matches.into_iter().map(location).collect();
        return result;
    }

    let mut same_name = lookup.same_name(source);
    if !same_name.is_empty() {
        sort_by_closeness(&mut same_name, source);
        let closest = same_name[0];
        result.status = Status::SameNameDifferentContent;
        result.newer_side = match (source.mtime, closest.mtime) {
            (Some(s), Some(t)) if s > t => Some(Side::Source),
            (Some(s), Some(t)) if s < t => Some(Side::Target),
            (Some(_), Some(_)) => Some(Side::Same),
            _ => None,
        };
        if source.size < closest.size && source.size > 0 && source.size.is_multiple_of(4096) {
            result.hints.push(Hint::PossiblyTruncated);
        }
        result.target_locations = same_name.into_iter().map(location).collect();
    }
    result
}

/// Order candidates so the most plausible counterpart comes first: the
/// longest shared path suffix with the source (so `_HYSJ/2026/x` pairs with
/// `HYSJ/2026/x`), then the same name, then the smallest size difference,
/// then the path for a stable order.
fn sort_by_closeness(candidates: &mut [&FileRecord], source: &FileRecord) {
    candidates.sort_by_cached_key(|t| {
        (
            std::cmp::Reverse(shared_suffix_len(&t.rel_path_nfc, &source.rel_path_nfc)),
            t.name_nfc != source.name_nfc,
            t.size.abs_diff(source.size),
            t.rel_path_nfc.clone(),
        )
    });
}

/// Number of trailing path components two paths have in common.
fn shared_suffix_len(a: &str, b: &str) -> usize {
    a.rsplit('/')
        .zip(b.rsplit('/'))
        .take_while(|(x, y)| x == y)
        .count()
}

fn location(file: &FileRecord) -> Location {
    Location {
        rel_path: file.rel_path_nfc.clone(),
        size: file.size,
        modified: file.mtime.map(format_time),
        in_bundle: file.in_bundle,
    }
}

fn summarize(files: &[FileComparison], depth: usize) -> (CompareSummary, Vec<FolderRollup>) {
    let mut summary = CompareSummary::default();
    let mut folders: BTreeMap<String, FolderRollup> = BTreeMap::new();
    for file in files {
        summary.total.add(file.size);
        summary.by_status.get_mut(file.status).add(file.size);
        if file.status.content_found() {
            summary.content_found.add(file.size);
        } else {
            summary.content_not_found.add(file.size);
        }

        let folder = rollup_folder(&file.rel_path, depth);
        let entry = folders
            .entry(folder.clone())
            .or_insert_with(|| FolderRollup {
                folder,
                total: Totals::default(),
                by_status: StatusTotals::default(),
            });
        entry.total.add(file.size);
        entry.by_status.get_mut(file.status).add(file.size);
    }
    (summary, folders.into_values().collect())
}

/// The file's folder, cut to at most `depth` levels.
fn rollup_folder(rel_path: &str, depth: usize) -> String {
    let mut parts: Vec<&str> = rel_path.split('/').collect();
    parts.pop();
    parts.truncate(depth.max(1));
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

/// NFC-normalise a user-supplied subpath and strip leading/trailing slashes.
fn normalize_subpath(subpath: &str) -> String {
    subpath
        .nfc()
        .collect::<String>()
        .trim_matches('/')
        .to_string()
}

/// Format nanoseconds since the epoch as RFC 3339 UTC.
pub fn format_time(nanos: i64) -> String {
    let dt: DateTime<Utc> = DateTime::from_timestamp_nanos(nanos);
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollup_folders() {
        assert_eq!(rollup_folder("a.txt", 3), ".");
        assert_eq!(rollup_folder("a/b/c/d/e.mp4", 3), "a/b/c");
        assert_eq!(rollup_folder("a/b.mp4", 3), "a");
    }

    #[test]
    fn suffix_lengths() {
        assert_eq!(
            shared_suffix_len("HYSJ/2026/a/x.mp4", "_HYSJ/2026/a/x.mp4"),
            3
        );
        assert_eq!(shared_suffix_len("a/x.mp4", "b/y.mp4"), 0);
    }

    #[test]
    fn status_roundtrip() {
        for status in Status::ALL {
            assert_eq!(status.as_str().parse::<Status>().unwrap(), status);
        }
    }
}
