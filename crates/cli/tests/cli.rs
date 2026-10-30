//! End-to-end tests of the `driveledger` binary.

mod common;

use common::{bin, index_id, setup};
use serde_json::Value;

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
    let id = index_id;
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
    let out = bin()
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

#[test]
fn verify_and_export() {
    let env = setup();
    let src = index_id(&env.json(&["scan", env.source.to_str().unwrap(), "--json"]));
    let tgt = index_id(&env.json(&["scan", env.target.to_str().unwrap(), "--json"]));

    let report = env.json(&["verify", &src, &tgt, "--json"]);
    assert_eq!(report["summary"]["verified"]["files"], 2);
    assert_eq!(report["summary"]["not_found_on_target"]["files"], 2);

    let tsv = env.run(&["export", &tgt, "--tsv"]);
    assert!(tsv.status.success());
    let tsv = String::from_utf8(tsv.stdout).unwrap();
    let lines: Vec<&str> = tsv.lines().collect();
    assert_eq!(lines[0], "size_mb\tmodified\tpath");
    assert_eq!(lines.len(), 4);
    assert!(lines.iter().any(|l| l.ends_with("\tOther/SW-FX30001.MP4")));

    let json = env.json(&["export", &tgt, "--json"]);
    assert_eq!(json["files"].as_array().unwrap().len(), 3);

    let overview = env.run(&["overview", &tgt, "--tsv", "--depth", "1"]);
    let overview = String::from_utf8(overview.stdout).unwrap();
    assert_eq!(overview.lines().next(), Some("size\tfiles\tpath"));
    assert!(overview.contains("\t2 files\tHYSJ"));
}

#[test]
fn export_refuses_to_write_into_the_scanned_folder() {
    let env = setup();
    let tgt = index_id(&env.json(&["scan", env.target.to_str().unwrap(), "--json"]));
    let inside = env.target.join("HYSJ/list.tsv");
    let out = env.run(&["export", &tgt, "--tsv", "--out", inside.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("never writes to scanned drives"));
    assert!(!inside.exists());

    let outside = env.library.join("list.tsv");
    let out = env.run(&["export", &tgt, "--tsv", "--out", outside.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(outside.is_file());
}

#[test]
fn verify_exits_4_when_a_pair_differs() {
    // Same size and same three 1 MiB samples, one byte different in a gap.
    let env = setup();
    let mib = 1024 * 1024;
    let mut data = common::bytes(6 * mib, 13);
    common::write(&env.source, "big/almost.MP4", &data);
    data[mib + mib / 2] ^= 0xFF;
    common::write(&env.target, "big/almost.MP4", &data);
    let src = index_id(&env.json(&["scan", env.source.to_str().unwrap(), "--json"]));
    let tgt = index_id(&env.json(&["scan", env.target.to_str().unwrap(), "--json"]));

    let out = env.run(&["verify", &src, &tgt, "--path", "big/almost.MP4"]);
    assert_eq!(out.status.code(), Some(4));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("mismatch"), "{text}");
}
