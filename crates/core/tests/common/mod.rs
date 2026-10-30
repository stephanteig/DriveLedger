//! Fixture builder shared by the integration tests.
//!
//! Trees are generated in a temp dir from deterministic pseudo-random bytes,
//! so no binaries are committed and every run sees identical content.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use filetime::FileTime;
use tempfile::TempDir;

/// A temporary folder tree that is removed when dropped.
pub struct Fixture {
    dir: TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        Fixture {
            dir: tempfile::tempdir().expect("create temp dir"),
        }
    }

    /// Canonical root of the tree (`/private/var/...` on macOS).
    pub fn root(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.root().join(rel)
    }

    /// Write `bytes` to `rel`, creating parent folders.
    pub fn file(&self, rel: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// Write `len` deterministic pseudo-random bytes derived from `seed`.
    pub fn random_file(&self, rel: &str, len: usize, seed: u64) -> PathBuf {
        self.file(rel, &pseudo_random_bytes(len, seed))
    }

    pub fn dir(&self, rel: &str) -> PathBuf {
        let path = self.path(rel);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// Set a file's modification time to `unix_secs`.
    pub fn set_mtime(&self, rel: &str, unix_secs: i64) {
        filetime::set_file_mtime(self.path(rel), FileTime::from_unix_time(unix_secs, 0)).unwrap();
    }
}

/// xorshift64* stream; fast and good enough to make distinct file contents.
pub fn pseudo_random_bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        out.extend_from_slice(&state.wrapping_mul(0x2545_F491_4F6C_DD1D).to_le_bytes());
    }
    out.truncate(len);
    out
}

/// Size, mtime and full content of every entry below a root, used to prove
/// that a scan changed nothing.
#[derive(Debug, PartialEq, Eq)]
pub struct TreeSnapshot(BTreeMap<PathBuf, EntryState>);

#[derive(Debug, PartialEq, Eq)]
pub struct EntryState {
    pub is_dir: bool,
    pub size: u64,
    pub mtime: SystemTime,
    pub content: Vec<u8>,
}

pub fn snapshot(root: &Path) -> TreeSnapshot {
    let mut entries = BTreeMap::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.expect("fixture tree must be fully readable for snapshots");
        let meta = std::fs::symlink_metadata(entry.path()).unwrap();
        let content = if meta.is_file() {
            std::fs::read(entry.path()).unwrap()
        } else {
            Vec::new()
        };
        entries.insert(
            entry.path().to_path_buf(),
            EntryState {
                is_dir: meta.is_dir(),
                size: meta.len(),
                mtime: meta.modified().unwrap(),
                content,
            },
        );
    }
    TreeSnapshot(entries)
}

impl TreeSnapshot {
    pub fn len(&self) -> usize {
        self.0.len()
    }
}
