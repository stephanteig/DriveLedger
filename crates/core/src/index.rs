//! The on-disk index format: one SQLite file per scan.
//!
//! The schema is documented for third-party tools in `docs/index-format.md`.
//! Any change to tables or columns must bump [`SCHEMA_VERSION`] and add a
//! migration step in [`Index::open_with_flags`].

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Current version of the index schema, stored in `meta.schema_version`.
pub const SCHEMA_VERSION: i64 = 1;

/// File name suffix of every index written to the library.
pub const INDEX_SUFFIX: &str = ".dlidx.sqlite";

const SCHEMA_SQL: &str = r#"
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE scan (
    id            INTEGER PRIMARY KEY,
    volume_name   TEXT NOT NULL,
    volume_uuid   TEXT,
    fs_type       TEXT,
    mount_point   TEXT NOT NULL,
    root_path     TEXT NOT NULL,
    started_at    TEXT NOT NULL,
    finished_at   TEXT,
    tool_version  TEXT NOT NULL,
    file_count    INTEGER NOT NULL DEFAULT 0,
    total_bytes   INTEGER NOT NULL DEFAULT 0,
    error_count   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE file (
    id               INTEGER PRIMARY KEY,
    rel_path         TEXT NOT NULL,
    rel_path_nfc     TEXT NOT NULL,
    name_nfc         TEXT NOT NULL,
    ext              TEXT NOT NULL,
    size             INTEGER NOT NULL,
    mtime            INTEGER,
    ctime            INTEGER,
    in_bundle        INTEGER NOT NULL DEFAULT 0,
    fingerprint      TEXT,
    fingerprint_kind TEXT,
    full_hash        TEXT
);

CREATE TABLE error (
    id       INTEGER PRIMARY KEY,
    rel_path TEXT NOT NULL,
    kind     TEXT NOT NULL,
    message  TEXT NOT NULL
);

CREATE INDEX idx_file_fingerprint  ON file(fingerprint);
CREATE INDEX idx_file_name_nfc     ON file(name_nfc);
CREATE INDEX idx_file_size         ON file(size);
CREATE INDEX idx_file_rel_path_nfc ON file(rel_path_nfc);
"#;

/// How the `fingerprint` column of a file was computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FingerprintKind {
    /// BLAKE3 over size and three 1 MiB samples (or the whole file if ≤ 3 MiB).
    Sample3,
    /// BLAKE3 over the whole file, filled in by `verify`.
    Full,
}

impl FingerprintKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FingerprintKind::Sample3 => "sample3",
            FingerprintKind::Full => "full",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "sample3" => Some(FingerprintKind::Sample3),
            "full" => Some(FingerprintKind::Full),
            _ => None,
        }
    }
}

/// Metadata about the scan that produced an index (the single `scan` row).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScanInfo {
    pub volume_name: String,
    pub volume_uuid: Option<String>,
    pub fs_type: Option<String>,
    /// Mount point of the volume the root lives on, e.g. `/Volumes/ByHola_1`.
    pub mount_point: String,
    /// Absolute path that was scanned.
    pub root_path: String,
    /// RFC 3339 UTC timestamp.
    pub started_at: String,
    /// RFC 3339 UTC timestamp; `None` while the scan is running.
    pub finished_at: Option<String>,
    pub tool_version: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub error_count: u64,
}

/// One indexed file. Paths are relative to the scan root and use `/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileRecord {
    /// Path as found on disk (may be NFD on some file systems).
    pub rel_path: String,
    /// NFC-normalised `rel_path`, used for all comparisons.
    pub rel_path_nfc: String,
    /// NFC-normalised file name.
    pub name_nfc: String,
    /// Lower-case extension without the dot, or an empty string.
    pub ext: String,
    pub size: u64,
    /// Modification time in nanoseconds since the Unix epoch.
    pub mtime: Option<i64>,
    /// Creation (birth) time in nanoseconds since the Unix epoch, where the
    /// file system provides it.
    pub ctime: Option<i64>,
    /// True if the file lives inside a macOS package such as `.dra` or `.app`.
    pub in_bundle: bool,
    /// Hex-encoded BLAKE3 fingerprint, `None` if the file could not be read.
    pub fingerprint: Option<String>,
    pub fingerprint_kind: Option<FingerprintKind>,
    /// Hex-encoded BLAKE3 of the whole file, filled in by `verify`.
    pub full_hash: Option<String>,
}

/// A path that could not be indexed. Unreadable files are always recorded,
/// never silently skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScanError {
    pub rel_path: String,
    /// Short machine-readable category, e.g. `walk`, `metadata`, `read`.
    pub kind: String,
    pub message: String,
}

/// An open index database.
pub struct Index {
    conn: Connection,
    path: Option<PathBuf>,
}

impl Index {
    /// Create a new, empty index at `path`. Fails if the file already exists.
    pub fn create(path: &Path) -> Result<Self> {
        if path.exists() {
            return Err(Error::io(
                path,
                std::io::Error::new(std::io::ErrorKind::AlreadyExists, "index already exists"),
            ));
        }
        let conn = Connection::open(path)?;
        let index = Index {
            conn,
            path: Some(path.to_path_buf()),
        };
        index.init_schema()?;
        Ok(index)
    }

    /// Create an index that lives only in memory (tests and benchmarks).
    pub fn in_memory() -> Result<Self> {
        let index = Index {
            conn: Connection::open_in_memory()?,
            path: None,
        };
        index.init_schema()?;
        Ok(index)
    }

    /// Open an existing index read-only.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
    }

    /// Open an existing index for updates (used by `verify` to store full
    /// hashes). Only ever call this on files inside the library.
    pub fn open_writable(path: &Path) -> Result<Self> {
        Self::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
    }

    fn open_with_flags(path: &Path, flags: OpenFlags) -> Result<Self> {
        if !path.is_file() {
            return Err(Error::NotAnIndex(path.to_path_buf()));
        }
        let conn = Connection::open_with_flags(path, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        let version: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| Error::NotAnIndex(path.to_path_buf()))?;
        let found: i64 = version
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| Error::NotAnIndex(path.to_path_buf()))?;
        // Future migrations go here: upgrade older versions in place when the
        // index was opened writable, otherwise refuse.
        if found != SCHEMA_VERSION {
            return Err(Error::UnsupportedSchema {
                path: path.to_path_buf(),
                found,
                supported: SCHEMA_VERSION,
            });
        }
        Ok(Index {
            conn,
            path: Some(path.to_path_buf()),
        })
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "PRAGMA journal_mode = DELETE;
             PRAGMA synchronous = NORMAL;",
        )?;
        self.conn.execute_batch(SCHEMA_SQL)?;
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', ?1), ('created_by', ?2)",
            params![
                SCHEMA_VERSION.to_string(),
                format!("driveledger {}", crate::TOOL_VERSION)
            ],
        )?;
        Ok(())
    }

    /// Raw connection, for read queries in other core modules.
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    pub(crate) fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// Path of the index file, `None` for in-memory indexes.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Write the scan row. An index holds exactly one scan.
    pub fn write_scan_info(&self, info: &ScanInfo) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO scan (id, volume_name, volume_uuid, fs_type, mount_point,
                root_path, started_at, finished_at, tool_version, file_count, total_bytes,
                error_count)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                info.volume_name,
                info.volume_uuid,
                info.fs_type,
                info.mount_point,
                info.root_path,
                info.started_at,
                info.finished_at,
                info.tool_version,
                info.file_count as i64,
                info.total_bytes as i64,
                info.error_count as i64,
            ],
        )?;
        Ok(())
    }

    pub fn scan_info(&self) -> Result<ScanInfo> {
        let info = self.conn.query_row(
            "SELECT volume_name, volume_uuid, fs_type, mount_point, root_path, started_at,
                    finished_at, tool_version, file_count, total_bytes, error_count
             FROM scan WHERE id = 1",
            [],
            |row| {
                Ok(ScanInfo {
                    volume_name: row.get(0)?,
                    volume_uuid: row.get(1)?,
                    fs_type: row.get(2)?,
                    mount_point: row.get(3)?,
                    root_path: row.get(4)?,
                    started_at: row.get(5)?,
                    finished_at: row.get(6)?,
                    tool_version: row.get(7)?,
                    file_count: row.get::<_, i64>(8)? as u64,
                    total_bytes: row.get::<_, i64>(9)? as u64,
                    error_count: row.get::<_, i64>(10)? as u64,
                })
            },
        )?;
        Ok(info)
    }

    /// Insert a batch of files in one transaction.
    pub fn insert_files(&mut self, files: &[FileRecord]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO file (rel_path, rel_path_nfc, name_nfc, ext, size, mtime, ctime,
                    in_bundle, fingerprint, fingerprint_kind, full_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?;
            for f in files {
                stmt.execute(params![
                    f.rel_path,
                    f.rel_path_nfc,
                    f.name_nfc,
                    f.ext,
                    f.size as i64,
                    f.mtime,
                    f.ctime,
                    f.in_bundle,
                    f.fingerprint,
                    f.fingerprint_kind.map(FingerprintKind::as_str),
                    f.full_hash,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn insert_errors(&mut self, errors: &[ScanError]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO error (rel_path, kind, message) VALUES (?1, ?2, ?3)",
            )?;
            for e in errors {
                stmt.execute(params![e.rel_path, e.kind, e.message])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// All files, ordered by NFC path.
    pub fn files(&self) -> Result<Vec<FileRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT rel_path, rel_path_nfc, name_nfc, ext, size, mtime, ctime, in_bundle,
                    fingerprint, fingerprint_kind, full_hash
             FROM file ORDER BY rel_path_nfc",
        )?;
        let rows = stmt.query_map([], file_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Look up one file by its relative path (NFC-normalised before lookup).
    pub fn file_by_path(&self, rel_path: &str) -> Result<Option<FileRecord>> {
        use unicode_normalization::UnicodeNormalization;
        let nfc: String = rel_path
            .nfc()
            .collect::<String>()
            .trim_matches('/')
            .to_string();
        let mut stmt = self.conn.prepare_cached(
            "SELECT rel_path, rel_path_nfc, name_nfc, ext, size, mtime, ctime, in_bundle,
                    fingerprint, fingerprint_kind, full_hash
             FROM file WHERE rel_path_nfc = ?1",
        )?;
        Ok(stmt.query_row([nfc], file_from_row).optional()?)
    }

    pub fn errors(&self) -> Result<Vec<ScanError>> {
        let mut stmt = self
            .conn
            .prepare("SELECT rel_path, kind, message FROM error ORDER BY id")?;
        let rows = stmt.query_map([], |row| {
            Ok(ScanError {
                rel_path: row.get(0)?,
                kind: row.get(1)?,
                message: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

/// Map a `SELECT rel_path, rel_path_nfc, name_nfc, ext, size, mtime, ctime,
/// in_bundle, fingerprint, fingerprint_kind, full_hash` row.
pub(crate) fn file_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FileRecord> {
    let kind: Option<String> = row.get(9)?;
    Ok(FileRecord {
        rel_path: row.get(0)?,
        rel_path_nfc: row.get(1)?,
        name_nfc: row.get(2)?,
        ext: row.get(3)?,
        size: row.get::<_, i64>(4)? as u64,
        mtime: row.get(5)?,
        ctime: row.get(6)?,
        in_bundle: row.get(7)?,
        fingerprint: row.get(8)?,
        fingerprint_kind: kind.as_deref().and_then(FingerprintKind::parse),
        full_hash: row.get(10)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_file() -> FileRecord {
        FileRecord {
            rel_path: "a/b.mp4".into(),
            rel_path_nfc: "a/b.mp4".into(),
            name_nfc: "b.mp4".into(),
            ext: "mp4".into(),
            size: 42,
            mtime: Some(1_700_000_000_000_000_000),
            ctime: None,
            in_bundle: false,
            fingerprint: Some("abc".into()),
            fingerprint_kind: Some(FingerprintKind::Sample3),
            full_hash: None,
        }
    }

    #[test]
    fn roundtrip_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.dlidx.sqlite");
        {
            let mut idx = Index::create(&path).unwrap();
            idx.insert_files(&[sample_file()]).unwrap();
        }
        let idx = Index::open(&path).unwrap();
        assert_eq!(idx.files().unwrap(), vec![sample_file()]);
        assert!(Index::create(&path).is_err(), "create must not overwrite");
    }

    #[test]
    fn rejects_non_index_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("other.sqlite");
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE t (x)")
            .unwrap();
        assert!(matches!(Index::open(&path), Err(Error::NotAnIndex(_))));
    }
}
