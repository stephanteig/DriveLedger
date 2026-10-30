//! Deep verify and export formats.

mod common;

use std::path::Path;
use std::process::Command;

use common::{Fixture, pseudo_random_bytes, snapshot};
use driveledger_core::export::{write_file_list_tsv, write_overview_tsv};
use driveledger_core::{
    Error, FingerprintKind, Index, ScanOptions, ScanProgress, Status, SystemProbe, VerifyOptions,
    VerifyStatus, compare, folder_overview, scan_to_library, verify,
};

const MIB: usize = 1024 * 1024;

fn scan(fx: &Fixture, library: &Path) -> std::path::PathBuf {
    let options = ScanOptions {
        incremental: false,
        ..ScanOptions::default()
    };
    scan_to_library(
        &fx.root(),
        library,
        &options,
        &SystemProbe,
        &|_: &ScanProgress| {},
    )
    .unwrap()
    .index_path
}

#[test]
fn verify_confirms_matches_and_catches_what_sampling_misses() {
    let (source, target) = (Fixture::new(), Fixture::new());
    let clip = pseudo_random_bytes(6 * MIB, 1);
    source.file("_HYSJ/clip.MP4", &clip);
    target.file("HYSJ/2026/clip.MP4", &clip);

    // Same size, same three samples, one byte different in an unsampled gap.
    let mut almost = pseudo_random_bytes(6 * MIB, 2);
    source.file("_HYSJ/almost.MP4", &almost);
    almost[MIB + MIB / 2] ^= 0xFF;
    target.file("HYSJ/2026/almost.MP4", &almost);

    source.random_file("_HYSJ/only-here.MP4", 1_000, 3);
    let library = tempfile::tempdir().unwrap();
    let src = scan(&source, library.path());
    let tgt = scan(&target, library.path());

    let comparison = compare(
        &Index::open(&src).unwrap(),
        &Index::open(&tgt).unwrap(),
        &Default::default(),
    )
    .unwrap();
    let status = |p: &str| {
        comparison
            .files
            .iter()
            .find(|f| f.rel_path == p)
            .unwrap()
            .status
    };
    assert_eq!(
        status("_HYSJ/almost.MP4"),
        Status::Present,
        "sampling can't see it"
    );

    let before = (snapshot(&source.root()), snapshot(&target.root()));
    let report = verify(&src, &tgt, &VerifyOptions::default(), &|_| {}).unwrap();
    let after = (snapshot(&source.root()), snapshot(&target.root()));
    assert_eq!(before, after, "verify must not change either tree");

    let item = |p: &str| report.items.iter().find(|i| i.source_path == p).unwrap();
    assert_eq!(item("_HYSJ/clip.MP4").status, VerifyStatus::Verified);
    assert_eq!(item("_HYSJ/clip.MP4").target_path, "HYSJ/2026/clip.MP4");
    assert_eq!(item("_HYSJ/almost.MP4").status, VerifyStatus::Mismatch);
    assert_eq!(report.summary.verified.files, 1);
    assert_eq!(report.summary.mismatch.files, 1);
    assert_eq!(report.summary.not_found_on_target.files, 1);

    // Full hashes are cached in the library indexes.
    let stored = Index::open(&src)
        .unwrap()
        .file_by_path("_HYSJ/clip.MP4")
        .unwrap()
        .unwrap();
    assert_eq!(stored.fingerprint_kind, Some(FingerprintKind::Full));
    assert_eq!(
        stored.full_hash.as_deref(),
        Some(blake3::hash(&clip).to_hex().as_str())
    );
}

#[test]
fn verify_selected_paths_only() {
    let (source, target) = (Fixture::new(), Fixture::new());
    for n in 0..3u64 {
        let bytes = pseudo_random_bytes(2_000, n);
        source.file(&format!("a/{n}.mov"), &bytes);
        target.file(&format!("b/{n}.mov"), &bytes);
    }
    let library = tempfile::tempdir().unwrap();
    let (src, tgt) = (scan(&source, library.path()), scan(&target, library.path()));
    let options = VerifyOptions {
        paths: vec!["a/1.mov".into()],
        ..VerifyOptions::default()
    };
    let report = verify(&src, &tgt, &options, &|_| {}).unwrap();
    assert_eq!(report.items.len(), 1);
    assert_eq!(report.items[0].status, VerifyStatus::Verified);
}

#[test]
fn verify_with_a_disconnected_side_is_unavailable() {
    let source = Fixture::new();
    let target = Fixture::new();
    source.random_file("a.mov", 100, 1);
    target.random_file("a.mov", 100, 1);
    let library = tempfile::tempdir().unwrap();
    let (src, tgt) = (scan(&source, library.path()), scan(&target, library.path()));
    drop(target); // the "drive" is gone

    let err = verify(&src, &tgt, &VerifyOptions::default(), &|_| {}).unwrap_err();
    assert!(matches!(err, Error::TargetUnavailable(_)), "{err:?}");
}

/// Build a tree without empty folders or `Icon\r` files (cases where the
/// shell scripts and the index intentionally differ).
fn export_fixture() -> Fixture {
    let fx = Fixture::new();
    fx.random_file("HYSJ/2026/SW-FX30001.MP4", 3 * MIB + 17, 1);
    fx.random_file("HYSJ/2026/SW-FX30001M01.XML", 1_500, 2);
    fx.random_file("Ra\u{30A}materiale/STØY.wav", 200_000, 3);
    fx.random_file("Projects/v2.dra/MediaFiles/pink-02.png", 8_000, 4);
    fx.random_file("a b/c (1).pdf", 52_429, 5);
    fx.file("HYSJ/.DS_Store", b"junk");
    fx.file("HYSJ/._SW-FX30001.MP4", b"x");
    fx.file("top.txt", b"hello");
    for (i, rel) in [
        "HYSJ/2026/SW-FX30001.MP4",
        "HYSJ/2026/SW-FX30001M01.XML",
        "Ra\u{30A}materiale/STØY.wav",
        "Projects/v2.dra/MediaFiles/pink-02.png",
        "a b/c (1).pdf",
        "top.txt",
    ]
    .iter()
    .enumerate()
    {
        fx.set_mtime(rel, 1_780_000_000 + i as i64 * 86_400 * 40);
    }
    fx
}

fn shell(dir: &Path, script: &str) -> String {
    let out = Command::new("/bin/zsh")
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .env("LC_ALL", "C")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// The pipeline from `reference/scripts/filliste.sh`, printing to stdout.
const FILLISTE: &str = r#"TAB=$'\t'
printf "size_mb\tmodified\tpath\n"
find . \( -name '.Spotlight-V100' -o -name '.fseventsd' -o -name '.Trashes' \
          -o -name '.TemporaryItems' -o -name '.DocumentRevisions-V100' \) -prune \
     -o -type f ! -name '.DS_Store' ! -name '._*' \
     -exec stat -f "%z${TAB}%Sm${TAB}%N" -t '%Y-%m-%d' {} + \
| awk -F'\t' -v OFS='\t' '{$1=sprintf("%.1f",$1/1048576); sub(/^\.\//,"",$3); print}' \
| sort -t"$TAB" -k3"#;

/// The pipeline from `reference/scripts/mapper.sh` with depth 4, without the
/// `du` column (du counts allocated blocks, the index stores apparent sizes).
const MAPPER: &str = r#"printf "files\tpath\n"
find . -mindepth 1 -maxdepth 4 -type d ! -path './.*' | sort | while IFS= read -r d; do
  printf "%s files\t%s\n" "$(find "$d" -type f ! -name '.*' | wc -l | tr -d ' ')" "${d#./}"
done"#;

#[cfg(target_os = "macos")]
#[test]
fn tsv_export_matches_filliste_sh() {
    let fx = export_fixture();
    let library = tempfile::tempdir().unwrap();
    let index = Index::open(&scan(&fx, library.path())).unwrap();

    let mut ours = Vec::new();
    write_file_list_tsv(&index, &mut ours).unwrap();
    assert_eq!(
        String::from_utf8(ours).unwrap(),
        shell(&fx.root(), FILLISTE)
    );
}

#[cfg(target_os = "macos")]
#[test]
fn overview_tsv_matches_mapper_sh_paths_and_counts() {
    let fx = export_fixture();
    let library = tempfile::tempdir().unwrap();
    let index = Index::open(&scan(&fx, library.path())).unwrap();

    let mut ours = Vec::new();
    write_overview_tsv(&folder_overview(&index, 4, None).unwrap(), &mut ours).unwrap();
    let ours = String::from_utf8(ours).unwrap();
    // Drop our size column to compare with the du-less mapper pipeline.
    let without_size: String = ours
        .lines()
        .map(|l| l.split_once('\t').unwrap().1)
        .map(|l| format!("{l}\n"))
        .collect();
    let expected = shell(&fx.root(), MAPPER);
    // mapper prints the on-disk (NFD) folder name; the overview uses NFC.
    use unicode_normalization::UnicodeNormalization;
    let expected: String = expected.nfc().collect();
    assert_eq!(without_size, expected);
    assert!(ours.starts_with("size\tfiles\tpath\n"));
}
