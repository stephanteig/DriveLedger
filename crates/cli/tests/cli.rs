//! End-to-end tests of the `driveledger` binary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use filetime::FileTime;
use serde_json::Value;

struct Env {
    _dirs: Vec<tempfile::TempDir>,
    source: PathBuf,
    target: PathBuf,
    library: PathBuf,
    config: PathBuf,
}

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    filetime::set_file_mtime(&path, FileTime::from_unix_time(1_785_000_000, 0)).unwrap();
}

fn bytes(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| {
            (i as u8)
                .wrapping_mul(seed)
                .wrapping_add(seed ^ (i >> 8) as u8)
        })
        .collect()
}

/// A small case-03-shaped pair of trees.
fn setup() -> Env {
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
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_driveledger"))
            .args(args)
            .arg("--library")
            .arg(&self.library)
            .env("DRIVELEDGER_CONFIG_DIR", &self.config)
            .env_remove("DRIVELEDGER_LIBRARY")
            .output()
            .unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }

    fn redact(&self, text: &str) -> String {
        text.replace(self.source.to_str().unwrap(), "[SOURCE]")
            .replace(self.target.to_str().unwrap(), "[TARGET]")
            .replace(self.library.to_str().unwrap(), "[LIBRARY]")
    }
}

#[test]
fn compare_live_folders_json() {
    let env = setup();
    let src = env.source.to_str().unwrap();
    let tgt = env.target.to_str().unwrap();
    let result = env.json(&["compare", src, tgt, "--json"]);
    let text = env.redact(&serde_json::to_string_pretty(&result).unwrap());
    let redacted: Value = serde_json::from_str(&text).unwrap();
    insta::assert_json_snapshot!(redacted, {
        ".source.index_path" => "[INDEX]",
        ".target.index_path" => "[INDEX]",
        ".source.scanned_at" => "[TIME]",
        ".target.scanned_at" => "[TIME]",
        ".source.volume_name" => "[VOLUME]",
        ".target.volume_name" => "[VOLUME]",
    });

    // Both live folders were scanned into the library.
    let list = env.json(&["list", "--json"]);
    assert_eq!(list.as_array().unwrap().len(), 2);
}

#[test]
fn compare_human_output_lists_problem_files() {
    let env = setup();
    let out = env.run(&[
        "compare",
        env.source.to_str().unwrap(),
        env.target.to_str().unwrap(),
    ]);
    assert!(out.status.success());
    let text = env.redact(&String::from_utf8(out.stdout).unwrap());
    // Volume names differ between machines; drop the two header lines.
    let body: String = text.lines().skip(2).collect::<Vec<_>>().join("\n");
    insta::assert_snapshot!(body);
}

#[test]
fn indexes_can_be_used_by_id_and_status_filter() {
    let env = setup();
    let scan_src = env.json(&["scan", env.source.to_str().unwrap(), "--json"]);
    let scan_tgt = env.json(&["scan", env.target.to_str().unwrap(), "--json"]);
    let id = |scan: &Value| {
        let path = scan["index_path"].as_str().unwrap();
        Path::new(path)
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .trim_end_matches(".dlidx.sqlite")
            .to_string()
    };
    let result = env.json(&[
        "compare",
        &id(&scan_src),
        &id(&scan_tgt),
        "--status",
        "missing",
        "--json",
    ]);
    let files = result["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(
        files[0]["rel_path"],
        "04_Partnere/Partnerslipp/Red-Bull/C0001.MP4"
    );
    assert_eq!(result["summary"]["by_status"]["missing"]["files"], 1);
}

#[test]
fn unmounted_target_exits_3_with_json_error() {
    let env = setup();
    let out = env.run(&[
        "compare",
        env.source.to_str().unwrap(),
        "/Volumes/DriveLedger-test-not-mounted/ByHola",
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(3));
    let body: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(body["error"]["kind"], "target_unavailable");
    // Nothing was scanned: the target is checked before the source.
    assert_eq!(env.json(&["list", "--json"]).as_array().unwrap().len(), 0);
}

#[test]
fn search_and_overview() {
    let env = setup();
    env.json(&["scan", env.target.to_str().unwrap(), "--json"]);

    let hits = env.json(&["search", "--glob", "sw-fx3*.mp4", "--json"]);
    assert_eq!(hits["total"], 1);
    assert_eq!(hits["items"][0]["file"]["rel_path"], "Other/SW-FX30001.MP4");

    let hits = env.json(&["search", "logoer", "--ext", "pdf", "--json"]);
    assert_eq!(hits["total"], 1);

    let id = env.json(&["list", "--json"])[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let overview = env.json(&["overview", &id, "--depth", "2", "--json"]);
    let paths: Vec<&str> = overview
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["HYSJ", "HYSJ/2026", "Other"]);
    assert_eq!(overview[0]["files"], 2);
}

#[test]
fn config_saves_library() {
    let env = setup();
    let chosen = env.library.join("chosen");
    let out = Command::new(env!("CARGO_BIN_EXE_driveledger"))
        .args(["config", "--set-library"])
        .arg(&chosen)
        .arg("--json")
        .env("DRIVELEDGER_CONFIG_DIR", &env.config)
        .env_remove("DRIVELEDGER_LIBRARY")
        .output()
        .unwrap();
    assert!(out.status.success());
    let body: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(body["library"], chosen.to_str().unwrap());
    assert!(env.config.join("settings.json").is_file());
}
