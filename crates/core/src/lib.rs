//! DriveLedger core engine.
//!
//! All scanning, indexing and comparison logic lives here. The CLI, MCP server
//! and desktop app are thin layers on top of this crate.
//!
//! Safety: this crate is read-only on every scanned path. The only writes it
//! performs are index files inside the user-chosen library folder.

pub mod compare;
pub mod error;
pub mod fingerprint;
pub mod index;
pub mod library;
pub mod overview;
pub mod paging;
pub mod resolve;
pub mod scan;
pub mod search;
pub mod settings;
pub mod volume;

pub use compare::{
    CompareOptions, CompareResult, CompareSummary, FileComparison, Hint, Side, Status, compare,
    compare_with_live_target,
};
pub use error::{Error, Result};
pub use index::{FileRecord, FingerprintKind, Index, ScanError, ScanInfo};
pub use library::{
    IndexEntry, IndexSummary, VolumeStatus, find_index, list_indexes, volume_statuses,
};
pub use overview::{FolderStat, folder_overview};
pub use resolve::{IndexArg, LiveOptions, OpenedIndex, classify_arg, open_or_scan};
pub use scan::{ScanOptions, ScanOutcome, ScanProgress, ScanStats, SkipList, scan_to_library};
pub use search::{SearchHit, SearchQuery, search};
pub use settings::Settings;
pub use volume::{SystemProbe, VolumeInfo, VolumeProbe};

/// Version of the tool, recorded in every index it writes.
pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");
