//! Exports in the formats users already rely on.
//!
//! - TSV file list, column-for-column like `reference/scripts/filliste.sh`:
//!   `size_mb, modified, path`.
//! - Folder overview TSV like `reference/scripts/mapper.sh`:
//!   `size, files, path`.
//! - JSON with the scan metadata, every file and every error.

use std::io::Write;

use chrono::{DateTime, Local};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::index::{FileRecord, Index, ScanError, ScanInfo};
use crate::overview::FolderStat;

/// Write the file list as TSV. Rows are sorted by path in byte order (what
/// `LC_ALL=C sort` produces).
pub fn write_file_list_tsv(index: &Index, out: &mut dyn Write) -> Result<()> {
    let mut files = index.files()?;
    files.sort_by(|a, b| a.rel_path.as_bytes().cmp(b.rel_path.as_bytes()));
    let io = |e| Error::io("export", e);
    writeln!(out, "size_mb\tmodified\tpath").map_err(io)?;
    for f in &files {
        writeln!(
            out,
            "{:.1}\t{}\t{}",
            f.size as f64 / 1_048_576.0,
            f.mtime.map(local_date).unwrap_or_default(),
            f.rel_path
        )
        .map_err(io)?;
    }
    Ok(())
}

#[derive(Serialize)]
struct JsonExport<'a> {
    scan: &'a ScanInfo,
    files: &'a [FileRecord],
    errors: &'a [ScanError],
}

/// Write scan metadata, all files and all errors as pretty JSON.
pub fn write_json(index: &Index, out: &mut dyn Write) -> Result<()> {
    let export = JsonExport {
        scan: &index.scan_info()?,
        files: &index.files()?,
        errors: &index.errors()?,
    };
    serde_json::to_writer_pretty(&mut *out, &export)
        .map_err(|e| Error::io("export", std::io::Error::other(e)))?;
    writeln!(out).map_err(|e| Error::io("export", e))
}

/// Write a folder overview as TSV in the `mapper.sh` layout. Sizes are
/// apparent sizes in `du -h` style; `du` itself counts allocated blocks, so
/// small files can differ slightly.
pub fn write_overview_tsv(folders: &[FolderStat], out: &mut dyn Write) -> Result<()> {
    let io = |e| Error::io("export", e);
    writeln!(out, "size\tfiles\tpath").map_err(io)?;
    for f in folders {
        writeln!(out, "{}\t{} files\t{}", du_size(f.bytes), f.files, f.path).map_err(io)?;
    }
    Ok(())
}

/// `du -h` style: binary units, one decimal below 10, e.g. `4.0K`, `12M`.
pub fn du_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    if bytes < 1024 {
        return format!("{bytes}B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1023.95 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value < 9.95 {
        format!("{value:.1}{}", UNITS[unit])
    } else {
        format!("{value:.0}{}", UNITS[unit])
    }
}

fn local_date(nanos: i64) -> String {
    DateTime::from_timestamp_nanos(nanos)
        .with_timezone(&Local)
        .format("%Y-%m-%d")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn du_sizes() {
        assert_eq!(du_size(0), "0B");
        assert_eq!(du_size(4096), "4.0K");
        assert_eq!(du_size(12 * 1024), "12K");
        assert_eq!(du_size(1_610_612_736), "1.5G");
        assert_eq!(du_size(100 * 1024 * 1024), "100M");
    }
}
