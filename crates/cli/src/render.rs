//! Human-readable output: sizes, dates, colours and tables. Every renderer
//! returns a `String` so it can be snapshot-tested.

use std::fmt::Write as _;

use chrono::{DateTime, Local};
use driveledger_core::compare::{CompareResult, FileComparison, Hint, Side, Status};
use driveledger_core::index::ScanInfo;
use driveledger_core::paging::Page;
use driveledger_core::{FolderStat, IndexSummary, ScanOutcome, SearchHit, Settings, VolumeStatus};

/// Whether to emit ANSI colours.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub color: bool,
}

impl Style {
    /// Colour only when stdout is a terminal and `NO_COLOR` is unset.
    pub fn detect() -> Self {
        use std::io::IsTerminal;
        Style {
            color: std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }

    #[cfg(test)]
    pub fn plain() -> Self {
        Style { color: false }
    }

    fn paint(&self, text: &str, color: Color) -> String {
        if self.color {
            format!("\x1b[{}m{text}\x1b[0m", color.code())
        } else {
            text.to_string()
        }
    }
}

/// The DriveLedger palette, mapped to 256-colour ANSI codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Cyan,
    Violet,
    Amber,
    Magenta,
    Dim,
    Bold,
}

impl Color {
    fn code(self) -> &'static str {
        match self {
            Color::Cyan => "38;5;51",
            Color::Violet => "38;5;141",
            Color::Amber => "38;5;220",
            Color::Magenta => "38;5;211",
            Color::Dim => "2",
            Color::Bold => "1",
        }
    }
}

pub fn status_color(status: Status) -> Color {
    match status {
        Status::Present => Color::Cyan,
        Status::PresentElsewhere | Status::OnlyInBundle => Color::Violet,
        Status::SameNameDifferentContent => Color::Amber,
        Status::Missing => Color::Magenta,
    }
}

pub fn status_label(status: Status) -> &'static str {
    match status {
        Status::Present => "present",
        Status::PresentElsewhere => "present elsewhere",
        Status::OnlyInBundle => "only in bundle",
        Status::SameNameDifferentContent => "same name, different content",
        Status::Missing => "missing",
    }
}

fn status_short(status: Status) -> &'static str {
    match status {
        Status::Present => "present",
        Status::PresentElsewhere => "elsewhere",
        Status::OnlyInBundle => "bundle",
        Status::SameNameDifferentContent => "different",
        Status::Missing => "missing",
    }
}

/// Decimal units, as Finder shows them: `100.18 GB`.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    match unit {
        0 => format!("{bytes} B"),
        1 | 2 => format!("{value:.1} {}", UNITS[unit]),
        _ => format!("{value:.2} {}", UNITS[unit]),
    }
}

/// `1156` → `1,156`.
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// RFC 3339 → `YYYY-MM-DD` in local time.
pub fn date(rfc3339: &str) -> String {
    DateTime::parse_from_rfc3339(rfc3339)
        .map(|t| t.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| rfc3339.to_string())
}

fn opt_date(rfc3339: Option<&str>) -> String {
    rfc3339.map(date).unwrap_or_else(|| "-".to_string())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Right,
}

struct Cell {
    text: String,
    color: Option<Color>,
}

impl From<String> for Cell {
    fn from(text: String) -> Self {
        Cell { text, color: None }
    }
}

impl From<&str> for Cell {
    fn from(text: &str) -> Self {
        Cell {
            text: text.to_string(),
            color: None,
        }
    }
}

fn colored(text: impl Into<String>, color: Color) -> Cell {
    Cell {
        text: text.into(),
        color: Some(color),
    }
}

/// A plain text table with padded columns. The last column is never padded,
/// so long paths do not create trailing whitespace.
struct Table {
    headers: Vec<&'static str>,
    align: Vec<Align>,
    rows: Vec<Vec<Cell>>,
}

impl Table {
    fn new(columns: &[(&'static str, Align)]) -> Self {
        Table {
            headers: columns.iter().map(|c| c.0).collect(),
            align: columns.iter().map(|c| c.1).collect(),
            rows: Vec::new(),
        }
    }

    fn row(&mut self, cells: Vec<Cell>) {
        self.rows.push(cells);
    }

    fn render(&self, style: &Style) -> String {
        let width = |i: usize| {
            self.rows
                .iter()
                .map(|r| r[i].text.chars().count())
                .chain([self.headers[i].len()])
                .max()
                .unwrap_or(0)
        };
        let widths: Vec<usize> = (0..self.headers.len()).map(width).collect();
        let last = self.headers.len() - 1;

        let mut out = String::new();
        let format_row = |out: &mut String, cells: Vec<(String, Option<Color>)>| {
            let mut line = String::new();
            for (i, (text, color)) in cells.into_iter().enumerate() {
                let pad = widths[i].saturating_sub(text.chars().count());
                let padded = match (self.align[i], i == last) {
                    (Align::Right, _) => format!("{}{text}", " ".repeat(pad)),
                    (Align::Left, true) => text,
                    (Align::Left, false) => format!("{text}{}", " ".repeat(pad)),
                };
                if i > 0 {
                    line.push_str("  ");
                }
                match color {
                    Some(c) => line.push_str(&style.paint(&padded, c)),
                    None => line.push_str(&padded),
                }
            }
            out.push_str(line.trim_end());
            out.push('\n');
        };
        format_row(
            &mut out,
            self.headers
                .iter()
                .map(|h| (h.to_string(), Some(Color::Dim)))
                .collect(),
        );
        for row in &self.rows {
            format_row(
                &mut out,
                row.iter().map(|c| (c.text.clone(), c.color)).collect(),
            );
        }
        out
    }
}

fn index_line(label: &str, volume: &str, root: &str, scanned_at: &str, files: u64) -> String {
    format!(
        "{label:<7} {volume} · {root} · scanned {} · {} files\n",
        date(scanned_at),
        thousands(files)
    )
}

/// Which statuses the file list shows by default: everything that is not a
/// plain `present`.
pub const DEFAULT_LISTED: [Status; 4] = [
    Status::PresentElsewhere,
    Status::OnlyInBundle,
    Status::SameNameDifferentContent,
    Status::Missing,
];

pub fn compare(result: &CompareResult, listed: &[Status], style: &Style) -> String {
    let mut out = String::new();
    out.push_str(&index_line(
        "Source",
        &result.source.volume_name,
        &result.source.root_path,
        &result.source.scanned_at,
        result.source.file_count,
    ));
    out.push_str(&index_line(
        "Target",
        &result.target.volume_name,
        &result.target.root_path,
        &result.target.scanned_at,
        result.target.file_count,
    ));
    if let Some(sub) = &result.source_subpath {
        let _ = writeln!(out, "Subpath {sub}");
    }
    for warning in &result.warnings {
        out.push_str(&style.paint(&format!("! {warning}"), Color::Amber));
        out.push('\n');
    }
    out.push('\n');

    // Summary by status.
    let s = &result.summary;
    let mut table = Table::new(&[
        ("STATUS", Align::Left),
        ("FILES", Align::Right),
        ("SIZE", Align::Right),
    ]);
    for status in Status::ALL {
        let t = s.by_status.get(status);
        table.row(vec![
            colored(status_label(status), status_color(status)),
            thousands(t.files).into(),
            human_bytes(t.bytes).into(),
        ]);
    }
    table.row(vec![
        colored("content found", Color::Bold),
        thousands(s.content_found.files).into(),
        human_bytes(s.content_found.bytes).into(),
    ]);
    let not_found_color = if s.content_not_found.files > 0 {
        Color::Magenta
    } else {
        Color::Bold
    };
    table.row(vec![
        colored("content not found", not_found_color),
        thousands(s.content_not_found.files).into(),
        human_bytes(s.content_not_found.bytes).into(),
    ]);
    out.push_str(&table.render(style));
    out.push('\n');

    // Folder rollup.
    let mut folders = Table::new(&[
        ("FILES", Align::Right),
        ("SIZE", Align::Right),
        ("PRESENT", Align::Right),
        ("ELSEWHERE", Align::Right),
        ("BUNDLE", Align::Right),
        ("DIFFERENT", Align::Right),
        ("MISSING", Align::Right),
        ("NOT FOUND SIZE", Align::Right),
        ("FOLDER", Align::Left),
    ]);
    let count = |n: u64, status: Status| -> Cell {
        if n == 0 {
            colored("·", Color::Dim)
        } else {
            colored(thousands(n), status_color(status))
        }
    };
    for f in &result.folders {
        let b = &f.by_status;
        let not_found = b.same_name_different_content.bytes + b.missing.bytes;
        folders.row(vec![
            thousands(f.total.files).into(),
            human_bytes(f.total.bytes).into(),
            count(b.present.files, Status::Present),
            count(b.present_elsewhere.files, Status::PresentElsewhere),
            count(b.only_in_bundle.files, Status::OnlyInBundle),
            count(
                b.same_name_different_content.files,
                Status::SameNameDifferentContent,
            ),
            count(b.missing.files, Status::Missing),
            if not_found == 0 {
                colored("·", Color::Dim)
            } else {
                colored(human_bytes(not_found), Color::Magenta)
            },
            f.folder.as_str().into(),
        ]);
    }
    out.push_str(&folders.render(style));

    // File list.
    let files = result.files_with_status(listed);
    if !files.is_empty() {
        let _ = writeln!(
            out,
            "\n{}",
            style.paint(
                &format!("FILES ({})", thousands(files.len() as u64)),
                Color::Dim
            )
        );
        for f in files {
            out.push_str(&file_line(f, style));
        }
    }
    out
}

fn file_line(f: &FileComparison, style: &Style) -> String {
    let mut out = format!(
        "{}  {:>9}  {}  {}",
        style.paint(
            &format!("{:<9}", status_short(f.status)),
            status_color(f.status)
        ),
        human_bytes(f.size),
        opt_date(f.modified.as_deref()),
        f.rel_path
    );
    let mut notes: Vec<&str> = f
        .hints
        .iter()
        .map(|h| match h {
            Hint::PossiblyTruncated => "possibly truncated",
            Hint::SourceUnreadable => "source unreadable",
        })
        .collect();
    match f.newer_side {
        Some(Side::Target) => notes.push("target is newer"),
        Some(Side::Source) => notes.push("source is newer"),
        Some(Side::Same) => notes.push("same mtime"),
        None => {}
    }
    if !notes.is_empty() {
        out.push_str(&style.paint(&format!("  [{}]", notes.join(", ")), Color::Amber));
    }
    out.push('\n');

    if let Some(first) = f.target_locations.first() {
        let arrow = if f.status == Status::SameNameDifferentContent {
            "≠"
        } else {
            "→"
        };
        let more = match f.target_locations.len() {
            1 => String::new(),
            n => format!("  (+{} more)", n - 1),
        };
        let line = format!(
            "{:<9}  {:>9}  {}  {arrow} {}{more}",
            "",
            human_bytes(first.size),
            opt_date(first.modified.as_deref()),
            first.rel_path
        );
        out.push_str(&style.paint(&line, Color::Dim));
        out.push('\n');
    }
    out
}

pub fn scan(
    outcome: &ScanOutcome,
    errors: &[driveledger_core::ScanError],
    style: &Style,
) -> String {
    let info: &ScanInfo = &outcome.info;
    let mut out = format!(
        "Scanned {} · {}\n{} files · {} · {} errors\n",
        info.volume_name,
        info.root_path,
        thousands(info.file_count),
        human_bytes(info.total_bytes),
        thousands(info.error_count),
    );
    let stats = &outcome.stats;
    let _ = writeln!(
        out,
        "{} fingerprinted ({} read) · {} reused from the previous scan",
        thousands(stats.files_hashed),
        human_bytes(stats.bytes_hashed),
        thousands(stats.files_reused)
    );
    let _ = writeln!(out, "Index  {}", outcome.index_path.display());
    if !errors.is_empty() {
        out.push('\n');
        for e in errors.iter().take(20) {
            out.push_str(&style.paint(
                &format!("! {:<16} {}  {}", e.kind, e.rel_path, e.message),
                Color::Amber,
            ));
            out.push('\n');
        }
        if errors.len() > 20 {
            let _ = writeln!(out, "… and {} more", errors.len() - 20);
        }
    }
    out
}

pub fn list(indexes: &[IndexSummary], style: &Style) -> String {
    if indexes.is_empty() {
        return "No indexes in the library yet. Run `driveledger scan <path>`.\n".to_string();
    }
    let mut table = Table::new(&[
        ("SCANNED", Align::Left),
        ("VOLUME", Align::Left),
        ("FILES", Align::Right),
        ("SIZE", Align::Right),
        ("ERRORS", Align::Right),
        ("ID", Align::Left),
    ]);
    for i in indexes {
        table.row(vec![
            date(&i.scanned_at).into(),
            colored(i.volume_name.clone(), Color::Cyan),
            thousands(i.file_count).into(),
            human_bytes(i.total_bytes).into(),
            if i.error_count == 0 {
                colored("·", Color::Dim)
            } else {
                colored(thousands(i.error_count), Color::Amber)
            },
            i.id.clone().into(),
        ]);
    }
    table.render(style)
}

pub fn volumes(volumes: &[VolumeStatus], style: &Style) -> String {
    let mut table = Table::new(&[
        ("VOLUME", Align::Left),
        ("FS", Align::Left),
        ("INDEXES", Align::Right),
        ("LAST SCAN", Align::Left),
        ("MOUNT POINT", Align::Left),
    ]);
    for v in volumes {
        table.row(vec![
            colored(v.volume.name.clone(), Color::Cyan),
            v.volume
                .fs_type
                .clone()
                .unwrap_or_else(|| "-".into())
                .into(),
            thousands(v.index_count as u64).into(),
            opt_date(v.last_scanned_at.as_deref()).into(),
            v.volume.mount_point.display().to_string().into(),
        ]);
    }
    table.render(style)
}

pub fn search(page: &Page<SearchHit>, style: &Style) -> String {
    if page.items.is_empty() {
        return "No matches.\n".to_string();
    }
    let mut table = Table::new(&[
        ("VOLUME", Align::Left),
        ("SIZE", Align::Right),
        ("MODIFIED", Align::Left),
        ("PATH", Align::Left),
    ]);
    for hit in &page.items {
        let path = std::path::Path::new(&hit.root_path).join(&hit.file.rel_path_nfc);
        table.row(vec![
            colored(hit.volume_name.clone(), Color::Cyan),
            human_bytes(hit.file.size).into(),
            opt_date(
                hit.file
                    .mtime
                    .map(driveledger_core::compare::format_time)
                    .as_deref(),
            )
            .into(),
            path.display().to_string().into(),
        ]);
    }
    let mut out = table.render(style);
    if page.next_cursor.is_some() {
        let _ = writeln!(
            out,
            "… showing {} of {} matches; use --limit or --cursor {}",
            thousands(page.items.len() as u64),
            thousands(page.total as u64),
            page.next_cursor.as_deref().unwrap_or_default()
        );
    }
    out
}

pub fn overview(folders: &[FolderStat], style: &Style) -> String {
    let mut table = Table::new(&[
        ("SIZE", Align::Right),
        ("FILES", Align::Right),
        ("FOLDER", Align::Left),
    ]);
    for f in folders {
        let indent = "  ".repeat(f.depth.saturating_sub(1));
        let name = f.path.rsplit('/').next().unwrap_or(&f.path);
        table.row(vec![
            human_bytes(f.bytes).into(),
            thousands(f.files).into(),
            format!("{indent}{name}").into(),
        ]);
    }
    table.render(style)
}

pub fn config(settings: &Settings, library: &std::path::Path) -> String {
    let path = Settings::path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "-".into());
    let skip: Vec<String> = settings
        .skip
        .names
        .iter()
        .map(|n| format!("{n:?}"))
        .chain(settings.skip.prefixes.iter().map(|p| format!("{p:?}*")))
        .collect();
    format!(
        "Settings  {path}\nLibrary   {}\nSkip      {}\nBundles   {}\n",
        library.display(),
        skip.join(" "),
        settings.bundle_extensions.join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use driveledger_core::compare::{
        CompareSummary, FolderRollup, IndexRef, Location, StatusTotals, Totals,
    };

    #[test]
    fn units() {
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1_783_848_960), "1.78 GB");
        assert_eq!(human_bytes(100_180_000_000), "100.18 GB");
        assert_eq!(human_bytes(12_300_000), "12.3 MB");
        assert_eq!(thousands(1_156), "1,156");
        assert_eq!(thousands(100), "100");
    }

    fn t(files: u64, bytes: u64) -> Totals {
        Totals { files, bytes }
    }

    fn loc(path: &str, size: u64) -> Location {
        Location {
            rel_path: path.into(),
            size,
            modified: Some("2026-08-07T12:00:00Z".into()),
            in_bundle: false,
        }
    }

    /// A small result shaped like case 03.
    pub(crate) fn sample_result() -> CompareResult {
        let files = vec![
            FileComparison {
                rel_path: "01_Artister/OdaRose/take1.mov".into(),
                size: 812_000_000,
                modified: Some("2026-06-01T12:00:00Z".into()),
                status: Status::Present,
                target_locations: vec![loc("HYSJ/2026/01_Artister/OdaRose/take1.mov", 812_000_000)],
                newer_side: None,
                hints: vec![],
            },
            FileComparison {
                rel_path: "04_Partnere/Logoer/cmyk_pos (1).pdf".into(),
                size: 120_000,
                modified: Some("2026-06-02T12:00:00Z".into()),
                status: Status::PresentElsewhere,
                target_locations: vec![loc("HYSJ/2026/04_Partnere/Logoer/cmyk_pos.pdf", 120_000)],
                newer_side: None,
                hints: vec![],
            },
            FileComparison {
                rel_path: "_SVAL/2027/SW-FX30157.MP4".into(),
                size: 1_783_848_960,
                modified: Some("2026-08-07T12:00:00Z".into()),
                status: Status::SameNameDifferentContent,
                target_locations: vec![
                    loc("SVAL/2027/SW-FX30157.MP4", 2_416_201_903),
                    loc("Other/SW-FX30157.MP4", 900_000_000),
                ],
                newer_side: Some(Side::Target),
                hints: vec![Hint::PossiblyTruncated],
            },
            FileComparison {
                rel_path: "04_Partnere/Partnerslipp/Red-Bull/C0001.MP4".into(),
                size: 36_700_000_000,
                modified: Some("2026-05-01T12:00:00Z".into()),
                status: Status::Missing,
                target_locations: vec![],
                newer_side: None,
                hints: vec![],
            },
        ];
        let by_status = StatusTotals {
            present: t(1, 812_000_000),
            present_elsewhere: t(1, 120_000),
            same_name_different_content: t(1, 1_783_848_960),
            missing: t(1, 36_700_000_000),
            ..StatusTotals::default()
        };
        let index_ref = |volume: &str, root: &str, files| IndexRef {
            index_path: None,
            volume_name: volume.into(),
            root_path: root.into(),
            scanned_at: "2026-10-30T12:00:00Z".into(),
            file_count: files,
        };
        let redbull = StatusTotals {
            missing: t(1, 36_700_000_000),
            ..StatusTotals::default()
        };
        CompareResult {
            source: index_ref(
                "HYSJ-2026_1",
                "/Volumes/HYSJ-2026_1/untitled folder2/HYSJ",
                4,
            ),
            target: index_ref("ByHola_1", "/Volumes/ByHola_1", 12_000),
            source_subpath: None,
            summary: CompareSummary {
                total: t(4, 39_296_088_960),
                content_found: t(2, 812_120_000),
                content_not_found: t(2, 38_483_848_960),
                by_status,
            },
            folders: vec![FolderRollup {
                folder: "04_Partnere/Partnerslipp/Red-Bull".into(),
                total: t(1, 36_700_000_000),
                by_status: redbull,
            }],
            files,
            warnings: vec![],
        }
    }

    #[test]
    fn compare_human_output() {
        insta::assert_snapshot!(compare(&sample_result(), &DEFAULT_LISTED, &Style::plain()));
    }

    #[test]
    fn compare_json_output() {
        insta::assert_json_snapshot!(sample_result());
    }
}
