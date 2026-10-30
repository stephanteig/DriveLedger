//! Error type shared by the whole core crate.

use std::path::PathBuf;

/// Errors returned by `driveledger-core`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("index database error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// The library folder would be written inside the tree being scanned.
    /// Refused because DriveLedger must never write to a scanned path.
    #[error("library folder {library} is inside the scan root {root}; choose a library outside it")]
    LibraryInsideScanRoot { library: PathBuf, root: PathBuf },

    /// The scan root does not exist or is not a directory.
    #[error("scan root {0} does not exist or is not a directory")]
    InvalidRoot(PathBuf),

    /// A live path (usually a comparison target) is not mounted or not
    /// readable. Callers must surface this instead of reporting every file as
    /// missing.
    #[error("{0} is not mounted or not readable; connect the drive and try again")]
    TargetUnavailable(PathBuf),

    #[error("index {path} has schema version {found}, this build supports {supported}")]
    UnsupportedSchema {
        path: PathBuf,
        found: i64,
        supported: i64,
    },

    #[error("{0} is not a DriveLedger index")]
    NotAnIndex(PathBuf),

    #[error("no index `{0}` in the library")]
    IndexNotFound(String),

    #[error("`{0}` matches more than one index; use more of the ID")]
    AmbiguousIndex(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("scan was cancelled")]
    Cancelled,
}

impl Error {
    /// Short machine-readable error kind for JSON output.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::Io { .. } => "io",
            Error::Sqlite(_) => "index_database",
            Error::LibraryInsideScanRoot { .. } => "library_inside_scan_root",
            Error::InvalidRoot(_) => "invalid_root",
            Error::TargetUnavailable(_) => "target_unavailable",
            Error::UnsupportedSchema { .. } => "unsupported_schema",
            Error::NotAnIndex(_) => "not_an_index",
            Error::IndexNotFound(_) => "index_not_found",
            Error::AmbiguousIndex(_) => "ambiguous_index",
            Error::InvalidArgument(_) => "invalid_argument",
            Error::Cancelled => "cancelled",
        }
    }

    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
