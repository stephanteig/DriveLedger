//! Compare engine tests. Each case reproduces a pattern recorded in
//! `reference/README.md` from the manual checks on 2026-10-30.

mod common;

use std::path::Path;
use std::time::Instant;

use common::{Fixture, pseudo_random_bytes, snapshot};
use driveledger_core::{
    CompareOptions, CompareResult, Error, FileComparison, FileRecord, FingerprintKind, Hint, Index,
    ScanInfo, ScanOptions, ScanProgress, Side, Status, SystemProbe, compare,
    compare_with_live_target, scan_to_library,
};

const MIB: usize = 1024 * 1024;

fn no_progress(_: &ScanProgress) {}

fn scan(fx: &Fixture, library: &Path) -> Index {
    let options = ScanOptions {
        incremental: false,
        ..ScanOptions::default()
    };
    let outcome = scan_to_library(&fx.root(), library, &options, &SystemProbe, &no_progress)
        .expect("scan fixture");
    Index::open(&outcome.index_path).unwrap()
}

/// Scan both fixtures into a fresh library and compare them.
fn run(source: &Fixture, target: &Fixture, options: &CompareOptions) -> CompareResult {
    let library = tempfile::tempdir().unwrap();
    let src = scan(source, library.path());
    let tgt = scan(target, library.path());
    compare(&src, &tgt, options).unwrap()
}

fn file<'a>(result: &'a CompareResult, rel: &str) -> &'a FileComparison {
    result
        .files
        .iter()
        .find(|f| f.rel_path == rel)
        .unwrap_or_else(|| panic!("{rel} not in result"))
}

/// Write the same pseudo-random content to `rel` in both trees.
fn same(source: &Fixture, src_rel: &str, target: &Fixture, tgt_rel: &str, len: usize, seed: u64) {
    let bytes = pseudo_random_bytes(len, seed);
    source.file(src_rel, &bytes);
    target.file(tgt_rel, &bytes);
}

#[test]
fn reorganised_folders_are_present() {
    let (source, target) = (Fixture::new(), Fixture::new());
    same(
        &source,
        "_HYSJ/x/clip.MP4",
        &target,
        "HYSJ/2026/x/clip.MP4",
        4 * MIB,
        1,
    );
    same(
        &source,
        "_HYSJ/x/a.jpg",
        &target,
        "HYSJ/2026/x/a.jpg",
        2_000,
        2,
    );

    let result = run(&source, &target, &CompareOptions::default());
    let clip = file(&result, "_HYSJ/x/clip.MP4");
    assert_eq!(clip.status, Status::Present);
    assert_eq!(clip.target_locations[0].rel_path, "HYSJ/2026/x/clip.MP4");
    assert_eq!(result.summary.by_status.present.files, 2);
}

#[test]
fn renamed_files_are_present_elsewhere() {
    let (source, target) = (Fixture::new(), Fixture::new());
    same(
        &source,
        "Logo/pos (1).pdf",
        &target,
        "HYSJ/2026/Logo/pos.pdf",
        3_000,
        1,
    );
    // macOS shows `/` in Finder as `:` on disk; the copy replaced it with `_`.
    same(
        &source,
        "OdaRose/Store ord  : 2.mov :: Kan brukes alene .mov",
        &target,
        "HYSJ/2026/OdaRose/Store ord _ 2.mov __ Kan brukes alene .mov",
        5_000,
        2,
    );

    let result = run(&source, &target, &CompareOptions::default());
    let pdf = file(&result, "Logo/pos (1).pdf");
    assert_eq!(pdf.status, Status::PresentElsewhere);
    assert_eq!(pdf.target_locations[0].rel_path, "HYSJ/2026/Logo/pos.pdf");
    assert_eq!(
        file(
            &result,
            "OdaRose/Store ord  : 2.mov :: Kan brukes alene .mov"
        )
        .status,
        Status::PresentElsewhere
    );
}

#[test]
fn camera_trap_same_name_and_size_is_not_backed_up() {
    let (source, target) = (Fixture::new(), Fixture::new());
    source.random_file("GretlandReno/SW-FX30001.MP4", 4 * MIB, 10);
    target.random_file("Other shoot/SW-FX30001.MP4", 4 * MIB, 20);
    source.random_file("GretlandReno/SW-FX30001M01.XML", 1_500, 11);
    target.random_file("Other shoot/SW-FX30001M01.XML", 1_500, 21);

    let result = run(&source, &target, &CompareOptions::default());
    for rel in [
        "GretlandReno/SW-FX30001.MP4",
        "GretlandReno/SW-FX30001M01.XML",
    ] {
        let f = file(&result, rel);
        assert_eq!(f.status, Status::SameNameDifferentContent, "{rel}");
        assert_eq!(f.target_locations[0].size, f.size, "same size on purpose");
    }
    assert_eq!(result.summary.content_not_found.files, 2);
    assert_eq!(result.summary.content_found.files, 0);
}

#[test]
fn truncated_copy_is_flagged() {
    let (source, target) = (Fixture::new(), Fixture::new());
    let full = pseudo_random_bytes(5 * MIB + 1234, 7);
    target.file("SVAL/2027/05_PROJEKTER/x/SW-FX30157.MP4", &full);
    // The interrupted copy stopped on a 4096-byte boundary.
    source.file("_SVAL/2027/05_PROJEKTER/x/SW-FX30157.MP4", &full[..4 * MIB]);

    let result = run(&source, &target, &CompareOptions::default());
    let f = file(&result, "_SVAL/2027/05_PROJEKTER/x/SW-FX30157.MP4");
    assert_eq!(f.status, Status::SameNameDifferentContent);
    assert!(f.hints.contains(&Hint::PossiblyTruncated));
    assert_eq!(f.target_locations[0].size, full.len() as u64);
}

#[test]
fn version_drift_reports_newer_side() {
    let (source, target) = (Fixture::new(), Fixture::new());
    source.random_file("Design/plakat.af", 30_000, 1);
    source.set_mtime("Design/plakat.af", 1_780_000_000);
    target.random_file("ByHola/Design/plakat.af", 34_000, 2);
    target.set_mtime("ByHola/Design/plakat.af", 1_790_000_000);

    let result = run(&source, &target, &CompareOptions::default());
    let f = file(&result, "Design/plakat.af");
    assert_eq!(f.status, Status::SameNameDifferentContent);
    assert_eq!(f.newer_side, Some(Side::Target));
    assert!(!f.hints.contains(&Hint::PossiblyTruncated));
}

#[test]
fn nfc_and_nfd_names_match() {
    let (source, target) = (Fixture::new(), Fixture::new());
    // Source written with decomposed å (NFD), target with composed å (NFC).
    same(
        &source,
        "Ra\u{30A}materiale/STØY.wav",
        &target,
        "R\u{E5}materiale/STØY.wav",
        10_000,
        1,
    );

    let result = run(&source, &target, &CompareOptions::default());
    let f = file(&result, "Råmateriale/STØY.wav");
    assert_eq!(f.status, Status::Present);
}

#[test]
fn match_only_inside_dra_bundle_is_only_in_bundle() {
    let (source, target) = (Fixture::new(), Fixture::new());
    same(
        &source,
        "03_Profil/Logo/PNGs/HYSJ-kvadrat-pink-012.png",
        &target,
        "ByHola/Projects/X/Davinci/v2.dra/MediaFiles/ByHola_1/HYSJ-kvadrat-pink-02.png",
        6_000,
        1,
    );

    let result = run(&source, &target, &CompareOptions::default());
    let f = file(&result, "03_Profil/Logo/PNGs/HYSJ-kvadrat-pink-012.png");
    assert_eq!(f.status, Status::OnlyInBundle);
    assert!(f.target_locations[0].in_bundle);
}

#[test]
fn bundle_copy_ranks_above_same_name_different_content() {
    // Case 02: an old `.af` version survived inside a `.dra` bundle while the
    // proper copy on the target was newer. The old content does exist on the
    // target, so this is only_in_bundle rather than a version conflict.
    let (source, target) = (Fixture::new(), Fixture::new());
    same(
        &source,
        "Design/logo.af",
        &target,
        "P/x.dra/MediaFiles/logo.af",
        9_000,
        1,
    );
    target.random_file("Design/logo.af", 9_500, 2);

    let result = run(&source, &target, &CompareOptions::default());
    assert_eq!(file(&result, "Design/logo.af").status, Status::OnlyInBundle);
}

#[test]
fn red_clips_inside_rdc_folders_are_present() {
    // `.RDC` is a bundle, but it is also the real home of R3D footage. A
    // source file inside a bundle may match a target file inside a bundle.
    let (source, target) = (Fixture::new(), Fixture::new());
    same(
        &source,
        "_LUSH/A001.RDC/A001_C001.R3D",
        &target,
        "LUSH/2026/A001.RDC/A001_C001.R3D",
        4 * MIB,
        1,
    );

    let result = run(&source, &target, &CompareOptions::default());
    assert_eq!(
        file(&result, "_LUSH/A001.RDC/A001_C001.R3D").status,
        Status::Present
    );
}

#[test]
fn empty_files_match_by_name_only() {
    let (source, target) = (Fixture::new(), Fixture::new());
    source.file("a/notes.txt", b"");
    source.file("a/lonely.txt", b"");
    target.file("b/notes.txt", b"");
    target.file("b/other.txt", b"");

    let result = run(&source, &target, &CompareOptions::default());
    assert_eq!(file(&result, "a/notes.txt").status, Status::Present);
    assert_eq!(file(&result, "a/lonely.txt").status, Status::Missing);
}

#[test]
fn subpath_limits_the_source() {
    let (source, target) = (Fixture::new(), Fixture::new());
    source.random_file("HYSJ/a.mov", 100, 1);
    source.random_file("HYSJ2/b.mov", 100, 2);
    source.random_file("LUSH/c.mov", 100, 3);

    let options = CompareOptions {
        source_subpath: Some("/HYSJ/".into()),
        ..CompareOptions::default()
    };
    let result = run(&source, &target, &options);
    let paths: Vec<&str> = result.files.iter().map(|f| f.rel_path.as_str()).collect();
    assert_eq!(paths, vec!["HYSJ/a.mov"]);
    assert_eq!(result.source_subpath.as_deref(), Some("HYSJ"));
}

#[test]
fn unmounted_target_is_unavailable_never_missing() {
    let source = Fixture::new();
    source.random_file("ByHola/clip.MP4", 1_000, 1);
    let library = tempfile::tempdir().unwrap();
    let src = scan(&source, library.path());

    for target in [
        Path::new("/Volumes/DriveLedger-test-not-mounted/ByHola"),
        &source.path("does-not-exist"),
    ] {
        let err = compare_with_live_target(
            &src,
            target,
            library.path(),
            &ScanOptions::default(),
            &SystemProbe,
            &no_progress,
            &CompareOptions::default(),
        )
        .unwrap_err();
        assert!(
            matches!(err, Error::TargetUnavailable(_)),
            "{target:?}: expected TargetUnavailable, got {err:?}"
        );
    }

    // Scanning an unmounted path is refused too, so no empty index can exist.
    let err = scan_to_library(
        Path::new("/Volumes/DriveLedger-test-not-mounted"),
        library.path(),
        &ScanOptions::default(),
        &SystemProbe,
        &no_progress,
    )
    .unwrap_err();
    assert!(matches!(err, Error::TargetUnavailable(_)));
}

#[test]
fn live_compare_is_read_only_on_both_sides() {
    let (source, target) = (Fixture::new(), Fixture::new());
    same(
        &source,
        "_HYSJ/a.MP4",
        &target,
        "HYSJ/2026/a.MP4",
        4 * MIB,
        1,
    );
    source.random_file("_HYSJ/missing.MP4", 1_000, 2);
    let library = tempfile::tempdir().unwrap();

    let before = (snapshot(&source.root()), snapshot(&target.root()));
    let src = scan(&source, library.path());
    let (result, _) = compare_with_live_target(
        &src,
        &target.root(),
        library.path(),
        &ScanOptions::default(),
        &SystemProbe,
        &no_progress,
        &CompareOptions::default(),
    )
    .unwrap();
    let after = (snapshot(&source.root()), snapshot(&target.root()));

    assert_eq!(before, after);
    assert_eq!(result.summary.by_status.present.files, 1);
    assert_eq!(result.summary.by_status.missing.files, 1);
}

#[test]
fn empty_target_index_produces_a_warning() {
    let (source, target) = (Fixture::new(), Fixture::new());
    source.random_file("a.mov", 100, 1);
    let result = run(&source, &target, &CompareOptions::default());
    assert_eq!(result.summary.by_status.missing.files, 1);
    assert!(result.warnings.iter().any(|w| w.contains("no files")));
}

// ---------------------------------------------------------------------------
// Replays of the recorded cases (scaled down, same patterns).
// ---------------------------------------------------------------------------

/// Case 01: Reserve `ByHola` → `ByHola_1`. Folder prefixes `_HYSJ` → `HYSJ`,
/// everything present except one truncated `SW-FX30157.MP4`, nothing missing.
#[test]
fn case01_reserve_byhola() {
    let (source, target) = (Fixture::new(), Fixture::new());
    for (i, prefix) in ["HYSJ", "LUSH", "STØY", "SVAL"].iter().enumerate() {
        for n in 0..3 {
            let rel = format!("{prefix}/2026/clip{n}.MP4");
            same(
                &source,
                &format!("_{rel}"),
                &target,
                &rel,
                20_000 + n,
                (i * 10 + n) as u64,
            );
        }
    }
    let full = pseudo_random_bytes(5 * MIB + 99, 77);
    target.file("SVAL/2027/05_PROJEKTER/SW-FX30157.MP4", &full);
    source.file("_SVAL/2027/05_PROJEKTER/SW-FX30157.MP4", &full[..4 * MIB]);

    let result = run(&source, &target, &CompareOptions::default());
    let s = &result.summary;
    assert_eq!(s.by_status.present.files, 12);
    assert_eq!(s.by_status.same_name_different_content.files, 1);
    assert_eq!(s.by_status.missing.files, 0);
    let truncated = file(&result, "_SVAL/2027/05_PROJEKTER/SW-FX30157.MP4");
    assert!(truncated.hints.contains(&Hint::PossiblyTruncated));
}

/// Case 02: `HYSJ-2026_1/.../ByHola` → `ByHola_1`. All present except `.af`
/// files that are newer on the target; nothing missing.
#[test]
fn case02_af_version_drift() {
    let (source, target) = (Fixture::new(), Fixture::new());
    for n in 0..5 {
        same(
            &source,
            &format!("ByHola/Video/c{n}.MP4"),
            &target,
            &format!("ByHola/Video/c{n}.MP4"),
            10_000,
            n,
        );
    }
    for n in 0..3u64 {
        let rel = format!("ByHola/Design/poster{n}.af");
        source.random_file(&rel, 40_000, 100 + n);
        source.set_mtime(&rel, 1_780_000_000);
        target.random_file(&rel, 41_000, 200 + n);
        target.set_mtime(&rel, 1_790_000_000);
    }

    let result = run(&source, &target, &CompareOptions::default());
    let s = &result.summary;
    assert_eq!(s.by_status.present.files, 5);
    assert_eq!(s.by_status.same_name_different_content.files, 3);
    assert_eq!(s.by_status.missing.files, 0);
    for f in result.files_with_status(&[Status::SameNameDifferentContent]) {
        assert!(f.rel_path.ends_with(".af"));
        assert_eq!(f.newer_side, Some(Side::Target));
    }
}

/// Case 03: `HYSJ` → `ByHola_1` laid out as `HYSJ/2026/...`. Renamed files,
/// bundle-only matches, the camera trap and a block of missing partner
/// footage.
#[test]
fn case03_hysj() {
    let (source, target) = (Fixture::new(), Fixture::new());
    // Reorganised but intact.
    for n in 0..4 {
        same(
            &source,
            &format!("01_Artister/OdaRose/take{n}.mov"),
            &target,
            &format!("HYSJ/2026/01_Artister/OdaRose/take{n}.mov"),
            8_000,
            n,
        );
    }
    // Renamed: `:` → `_`, and `(1)` differences.
    same(
        &source,
        "01_Artister/OdaRose/Magenta __ 4K/Store ord  : 2.mov :: Kan brukes alene .mov",
        &target,
        "HYSJ/2026/01_Artister/OdaRose/Store ord _ 2.mov __ Kan brukes alene .mov",
        9_000,
        50,
    );
    same(
        &source,
        "04_Partnere/Logoer/cmyk_pos (1).pdf",
        &target,
        "HYSJ/2026/04_Partnere/Logoer/cmyk_pos.pdf",
        3_000,
        51,
    );
    same(
        &source,
        "04_Partnere/Logoer/cmyk_neg.eps",
        &target,
        "HYSJ/2026/04_Partnere/Logoer/cmyk_neg (1).eps",
        3_000,
        52,
    );
    // Only inside a DaVinci bundle, under another name.
    same(
        &source,
        "03_Profil/Logo/PNGs/HYSJ-kvadrat-pink-012.png",
        &target,
        "ByHola/Projects/R/Davinci/v2.dra/MediaFiles/HYSJ-kvadrat-pink-02.png",
        4_000,
        53,
    );
    // Camera trap: same names (and one same size), different shoots.
    source.random_file(
        "04_Partnere/Partnervideo/GretlandReno/SW-FX30001.MP4",
        4 * MIB,
        60,
    );
    target.random_file("HYSJ/2026/Shoot B/SW-FX30001.MP4", 4 * MIB + 4096, 61);
    source.random_file(
        "04_Partnere/Partnervideo/GretlandReno/SW-FX30001M01.XML",
        1_500,
        62,
    );
    target.random_file("HYSJ/2026/Shoot B/SW-FX30001M01.XML", 1_500, 63);
    // Missing partner footage.
    for n in 0..3u64 {
        source.random_file(
            &format!("04_Partnere/Partnerslipp/Red-Bull/C{n:04}.MP4"),
            30_000,
            70 + n,
        );
    }

    let result = run(&source, &target, &CompareOptions::default());
    let s = &result.summary;
    assert_eq!(s.by_status.present.files, 4);
    assert_eq!(s.by_status.present_elsewhere.files, 3);
    assert_eq!(s.by_status.only_in_bundle.files, 1);
    assert_eq!(s.by_status.same_name_different_content.files, 2);
    assert_eq!(s.by_status.missing.files, 3);
    // The oracle's split: found = present + elsewhere + bundle.
    assert_eq!(s.content_found.files, 8);
    assert_eq!(s.content_not_found.files, 5);

    let red_bull = result
        .folders
        .iter()
        .find(|f| f.folder == "04_Partnere/Partnerslipp/Red-Bull")
        .expect("rollup has the Red-Bull folder");
    assert_eq!(red_bull.by_status.missing.files, 3);
    assert_eq!(red_bull.by_status.missing.bytes, 90_000);
}

/// 100k source files against 100k target files must compare in under 2 s.
#[test]
fn benchmark_100k_against_100k() {
    const N: usize = 100_000;
    let record = |i: usize, dir: &str, fp: usize, in_bundle: bool| {
        let name = format!("SW-FX3{:04}.MP4", i % 2_000);
        let rel = format!("{dir}/{:03}/{name}", i / 2_000);
        FileRecord {
            rel_path: rel.clone(),
            rel_path_nfc: rel,
            name_nfc: name,
            ext: "mp4".into(),
            size: 1_000_000 + (i as u64 % 977),
            mtime: Some(1_780_000_000_000_000_000 + i as i64),
            ctime: None,
            in_bundle,
            fingerprint: Some(format!("{fp:064x}")),
            fingerprint_kind: Some(FingerprintKind::Sample3),
            full_hash: None,
        }
    };
    let make_index = |files: Vec<FileRecord>| {
        let mut index = Index::in_memory().unwrap();
        index
            .write_scan_info(&ScanInfo {
                volume_name: "bench".into(),
                volume_uuid: None,
                fs_type: None,
                mount_point: "/".into(),
                root_path: "/bench".into(),
                started_at: "2026-10-30T00:00:00.000Z".into(),
                finished_at: Some("2026-10-30T00:00:01.000Z".into()),
                tool_version: "test".into(),
                file_count: files.len() as u64,
                total_bytes: 0,
                error_count: 0,
            })
            .unwrap();
        index.insert_files(&files).unwrap();
        index
    };
    // 80% shared content (half renamed into another layout), 20% unique on
    // each side, and heavy file-name reuse like a camera card.
    let source = make_index((0..N).map(|i| record(i, "_HYSJ", i, false)).collect());
    let target = make_index(
        (0..N)
            .map(|i| {
                let fp = if i % 5 == 0 { N + i } else { i };
                record((i * 7) % N, "HYSJ/2026", fp, i % 50 == 0)
            })
            .collect(),
    );

    let start = Instant::now();
    let result = compare(&source, &target, &CompareOptions::default()).unwrap();
    let elapsed = start.elapsed();

    eprintln!("compare 100k x 100k: {elapsed:?}");
    assert_eq!(result.files.len(), N);
    assert_eq!(result.summary.content_found.files, (N - N / 5) as u64);
    assert!(
        elapsed.as_secs_f64() < 2.0,
        "compare took {elapsed:?}, budget is 2 s"
    );
}
