//! Shared setup for the binary tests: a small case-03-shaped pair of trees,
//! a library and a config dir, all in temp folders.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use filetime::FileTime;
use serde_json::Value;

pub struct Env {
    _dirs: Vec<tempfile::TempDir>,
    pub source: PathBuf,
    pub target: PathBuf,
    pub library: PathBuf,
    pub config: PathBuf,
}

pub fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    filetime::set_file_mtime(&path, FileTime::from_unix_time(1_785_000_000, 0)).unwrap();
}

pub fn bytes(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| {
            (i as u8)
                .wrapping_mul(seed)
                .wrapping_add(seed ^ (i >> 8) as u8)
        })
        .collect()
}

/// A small case-03-shaped pair of trees.
pub fn setup() -> Env {
    let dirs: Vec<tempfile::TempDir> = (0..4).map(|_| tempfile::tempdir().unwrap()).collect();
    let canon = |i: usize| dirs[i].path().canonicalize().unwrap();
    let (source, target, library, config) = (canon(0), canon(1), canon(2), canon(3));

    write(&source, "01_Artister/take1.mov", &bytes(5_000, 3));
    write(&target, "HYSJ/2026/01_Artister/take1.mov", &bytes(5_000, 3));
    write(&source, "04_Partnere/Logoer/pos (1).pdf", &bytes(800, 5));
    write(
        &target,
        "HYSJ/2026/04_Partnere/Logoer/pos.pdf",
        &bytes(800, 5),
    );
    write(
        &source,
        "04_Partnere/Partnervideo/SW-FX30001.MP4",
        &bytes(4_096, 7),
    );
    write(&target, "Other/SW-FX30001.MP4", &bytes(4_096, 9));
    write(
        &source,
        "04_Partnere/Partnerslipp/Red-Bull/C0001.MP4",
        &bytes(9_000, 11),
    );

    Env {
        _dirs: dirs,
        source,
        target,
        library,
        config,
    }
}

impl Env {
    pub fn run(&self, args: &[&str]) -> Output {
        bin()
            .args(args)
            .arg("--library")
            .arg(&self.library)
            .env("DRIVELEDGER_CONFIG_DIR", &self.config)
            .env_remove("DRIVELEDGER_LIBRARY")
            .output()
            .unwrap()
    }

    pub fn json(&self, args: &[&str]) -> Value {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }

    pub fn redact(&self, text: &str) -> String {
        text.replace(self.source.to_str().unwrap(), "[SOURCE]")
            .replace(self.target.to_str().unwrap(), "[TARGET]")
            .replace(self.library.to_str().unwrap(), "[LIBRARY]")
    }
}

pub fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_driveledger"))
}

/// Index ID from a `scan --json` result.
pub fn index_id(scan: &Value) -> String {
    let path = scan["index_path"].as_str().unwrap();
    Path::new(path)
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .trim_end_matches(".dlidx.sqlite")
        .to_string()
}
