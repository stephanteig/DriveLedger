//! Integration tests for scanning, including the read-only guard.

mod common;

use std::sync::atomic::{AtomicU64, Ordering};

use common::{Fixture, snapshot};
use driveledger_core::{Error, Index, ScanOptions, ScanProgress, SystemProbe, scan_to_library};

/// A small tree covering the scanner's special cases.
fn build_tree(fx: &Fixture) {
    fx.random_file("_HYSJ/2026/SW-FX30001.MP4", 200_000, 1);
    fx.random_file("_HYSJ/2026/SW-FX30001M01.XML", 1_500, 2);
    fx.file("_HYSJ/empty.txt", b"");
    // NFD-encoded "Råmateriale" (a + combining ring above).
    fx.random_file("Ra\u{30A}materiale/STØY.mov", 4_096, 3);
    fx.random_file("Projects/v2.dra/MediaFiles/pink-02.png", 8_000, 4);
    fx.random_file("Footage/A001.RDC/A001_C001.R3D", 10_000, 5);
    fx.file(".DS_Store", b"junk");
    fx.file("_HYSJ/._SW-FX30001.MP4", b"appledouble");
    fx.random_file(".Spotlight-V100/store.db", 100, 6);
}

fn no_progress(_: &ScanProgress) {}

#[test]
fn scan_is_read_only() {
    let fx = Fixture::new();
    build_tree(&fx);
    let library = tempfile::tempdir().unwrap();

    let before = snapshot(&fx.root());
    let outcome = scan_to_library(
        &fx.root(),
        library.path(),
        &ScanOptions::default(),
        &SystemProbe,
        &no_progress,
    )
    .unwrap();
    let after = snapshot(&fx.root());

    assert!(before.len() > 10);
    assert_eq!(before, after, "scanning must not change the scanned tree");

    // The only output is one finished index in the library.
    let outputs: Vec<_> = std::fs::read_dir(library.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(outputs, vec![outcome.index_path.clone()]);
    assert!(
        outcome
            .index_path
            .to_string_lossy()
            .ends_with(".dlidx.sqlite")
    );
}

#[test]
fn scan_records_expected_files() {
    let fx = Fixture::new();
    build_tree(&fx);
    let library = tempfile::tempdir().unwrap();
    let updates = AtomicU64::new(0);
    let progress = |_: &ScanProgress| {
        updates.fetch_add(1, Ordering::Relaxed);
    };

    let outcome = scan_to_library(
        &fx.root(),
        library.path(),
        &ScanOptions::default(),
        &SystemProbe,
        &progress,
    )
    .unwrap();
    assert!(updates.load(Ordering::Relaxed) > 0);

    let index = Index::open(&outcome.index_path).unwrap();
    let files = index.files().unwrap();
    let paths: Vec<&str> = files.iter().map(|f| f.rel_path_nfc.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "Footage/A001.RDC/A001_C001.R3D",
            "Projects/v2.dra/MediaFiles/pink-02.png",
            "Råmateriale/STØY.mov",
            "_HYSJ/2026/SW-FX30001.MP4",
            "_HYSJ/2026/SW-FX30001M01.XML",
            "_HYSJ/empty.txt",
        ]
    );

    let by_path = |p: &str| files.iter().find(|f| f.rel_path_nfc == p).unwrap();
    assert!(by_path("Projects/v2.dra/MediaFiles/pink-02.png").in_bundle);
    assert!(by_path("Footage/A001.RDC/A001_C001.R3D").in_bundle);
    assert!(!by_path("_HYSJ/2026/SW-FX30001.MP4").in_bundle);
    assert_eq!(by_path("_HYSJ/2026/SW-FX30001.MP4").ext, "mp4");
    assert_eq!(by_path("_HYSJ/2026/SW-FX30001.MP4").size, 200_000);
    assert_eq!(by_path("Råmateriale/STØY.mov").name_nfc, "STØY.mov");
    assert!(by_path("_HYSJ/empty.txt").mtime.is_some());

    let info = index.scan_info().unwrap();
    assert_eq!(info.file_count, 6);
    assert_eq!(info.total_bytes, files.iter().map(|f| f.size).sum::<u64>());
    assert_eq!(info.root_path, fx.root().to_string_lossy());
    assert!(info.finished_at.is_some());
}

#[test]
fn library_inside_scan_root_is_refused() {
    let fx = Fixture::new();
    build_tree(&fx);
    let library = fx.path("_HYSJ/library");

    let err = scan_to_library(
        &fx.root(),
        &library,
        &ScanOptions::default(),
        &SystemProbe,
        &no_progress,
    )
    .unwrap_err();

    assert!(matches!(err, Error::LibraryInsideScanRoot { .. }));
    assert!(!library.exists(), "refused library must not be created");
}

#[cfg(unix)]
#[test]
fn unreadable_folder_is_recorded_as_error() {
    use std::os::unix::fs::PermissionsExt;

    let fx = Fixture::new();
    fx.random_file("ok/a.bin", 100, 7);
    let locked = fx.dir("locked");
    fx.random_file("locked/b.bin", 100, 8);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read_dir(&locked).is_ok() {
        // Running as root: permissions are not enforced, nothing to test.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }

    let library = tempfile::tempdir().unwrap();
    let result = scan_to_library(
        &fx.root(),
        library.path(),
        &ScanOptions::default(),
        &SystemProbe,
        &no_progress,
    );
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

    let index = Index::open(&result.unwrap().index_path).unwrap();
    let errors = index.errors().unwrap();
    assert_eq!(index.files().unwrap().len(), 1);
    assert!(
        errors
            .iter()
            .any(|e| e.rel_path == "locked" && e.kind == "walk"),
        "expected a walk error for the locked folder, got {errors:?}"
    );
    assert_eq!(index.scan_info().unwrap().error_count, errors.len() as u64);
}

fn scan(fx: &Fixture, library: &std::path::Path) -> driveledger_core::ScanOutcome {
    scan_to_library(
        &fx.root(),
        library,
        &ScanOptions::default(),
        &SystemProbe,
        &no_progress,
    )
    .unwrap()
}

fn fingerprint_of(outcome: &driveledger_core::ScanOutcome, rel: &str) -> String {
    Index::open(&outcome.index_path)
        .unwrap()
        .files()
        .unwrap()
        .into_iter()
        .find(|f| f.rel_path_nfc == rel)
        .unwrap()
        .fingerprint
        .unwrap()
}

#[test]
fn every_readable_file_gets_a_sample3_fingerprint() {
    let fx = Fixture::new();
    build_tree(&fx);
    fx.random_file("big/clip.MP4", 5 * 1024 * 1024 + 123, 9);
    let library = tempfile::tempdir().unwrap();
    let outcome = scan(&fx, library.path());

    let files = Index::open(&outcome.index_path).unwrap().files().unwrap();
    for f in &files {
        assert!(f.fingerprint.is_some(), "{} has no fingerprint", f.rel_path);
        assert_eq!(
            f.fingerprint_kind,
            Some(driveledger_core::FingerprintKind::Sample3)
        );
    }
    assert_eq!(outcome.stats.files_hashed, files.len() as u64);
    assert_eq!(outcome.stats.files_reused, 0);
}

#[test]
fn camera_trap_same_name_and_size_get_different_fingerprints() {
    // Sony FX3 restarts numbering on every card: two shoots, same file name,
    // same size, different footage.
    let shoot_a = Fixture::new();
    let shoot_b = Fixture::new();
    shoot_a.random_file("SW-FX30001.MP4", 4 * 1024 * 1024, 100);
    shoot_b.random_file("SW-FX30001.MP4", 4 * 1024 * 1024, 200);
    shoot_a.random_file("SW-FX30001M01.XML", 1_500, 101);
    shoot_b.random_file("SW-FX30001M01.XML", 1_500, 201);
    let library = tempfile::tempdir().unwrap();

    let a = scan(&shoot_a, library.path());
    let b = scan(&shoot_b, library.path());
    for name in ["SW-FX30001.MP4", "SW-FX30001M01.XML"] {
        assert_ne!(fingerprint_of(&a, name), fingerprint_of(&b, name), "{name}");
    }
}

#[test]
fn incremental_rescan_reuses_unchanged_fingerprints() {
    let fx = Fixture::new();
    fx.random_file("keep.MP4", 50_000, 1);
    fx.random_file("touched.af", 50_000, 2);
    fx.random_file("sneaky.bin", 50_000, 3);
    for rel in ["keep.MP4", "touched.af", "sneaky.bin"] {
        fx.set_mtime(rel, 1_780_000_000);
    }
    let library = tempfile::tempdir().unwrap();
    let first = scan(&fx, library.path());
    assert_eq!(first.stats.files_hashed, 3);

    // A real edit: new content and a new mtime, so it must be re-hashed.
    fx.random_file("touched.af", 50_000, 22);
    fx.set_mtime("touched.af", 1_790_000_000);
    // Content changed but size and mtime were restored. The rescan trusts
    // (path, size, mtime) and reuses the old fingerprint; that is the
    // documented trade-off, and it proves reuse actually happened.
    fx.random_file("sneaky.bin", 50_000, 33);
    fx.set_mtime("sneaky.bin", 1_780_000_000);
    fx.random_file("new.txt", 10, 4);

    let second = scan(&fx, library.path());
    assert_eq!(second.stats.files_reused, 2);
    assert_eq!(second.stats.files_hashed, 2);
    assert_eq!(
        second.stats.previous_index.as_ref(),
        Some(&first.index_path)
    );
    assert_eq!(
        fingerprint_of(&first, "keep.MP4"),
        fingerprint_of(&second, "keep.MP4")
    );
    assert_eq!(
        fingerprint_of(&first, "sneaky.bin"),
        fingerprint_of(&second, "sneaky.bin")
    );
    assert_ne!(
        fingerprint_of(&first, "touched.af"),
        fingerprint_of(&second, "touched.af")
    );

    let fresh = scan_to_library(
        &fx.root(),
        library.path(),
        &ScanOptions {
            incremental: false,
            ..ScanOptions::default()
        },
        &SystemProbe,
        &no_progress,
    )
    .unwrap();
    assert_eq!(fresh.stats.files_reused, 0);
    assert_ne!(
        fingerprint_of(&first, "sneaky.bin"),
        fingerprint_of(&fresh, "sneaky.bin")
    );
}

#[cfg(unix)]
#[test]
fn unreadable_file_is_indexed_without_fingerprint_and_recorded() {
    use std::os::unix::fs::PermissionsExt;

    let fx = Fixture::new();
    let locked = fx.random_file("locked.MP4", 100, 1);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&locked).is_ok() {
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
        return;
    }
    let library = tempfile::tempdir().unwrap();
    let outcome = scan(&fx, library.path());
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();

    let index = Index::open(&outcome.index_path).unwrap();
    let file = &index.files().unwrap()[0];
    assert_eq!(file.fingerprint, None);
    let errors = index.errors().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].kind, "read");
    assert_eq!(errors[0].rel_path, "locked.MP4");
}
