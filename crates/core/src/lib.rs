//! DriveLedger core engine.
//!
//! All scanning, indexing and comparison logic lives here. The CLI, MCP server
//! and desktop app are thin layers on top of this crate.
//!
//! Safety: this crate is read-only on every scanned path. The only writes it
//! performs are index files inside the user-chosen library folder.

pub mod error;
pub mod index;
pub mod scan;
pub mod volume;

pub use error::{Error, Result};
pub use index::{FileRecord, FingerprintKind, Index, ScanError, ScanInfo};
pub use scan::{ScanOptions, ScanOutcome, ScanProgress, SkipList, scan_to_library};
pub use volume::{SystemProbe, VolumeInfo, VolumeProbe};

/// Version of the tool, recorded in every index it writes.
pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");
