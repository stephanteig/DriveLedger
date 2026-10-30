//! `driveledger` command-line interface.
//!
//! A thin layer over `driveledger-core`: parse arguments, call core, print
//! the result as a table or (with `--json`) as JSON on stdout.
//!
//! Exit codes: 0 success, 1 error, 2 usage error, 3 target unavailable,
//! 4 verify found differences.

mod mcp;
mod progress;
mod render;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use std::io::Write as _;

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
use driveledger_core::compare::{CompareOptions, Status};
use driveledger_core::export;
use driveledger_core::paging::paginate;
use driveledger_core::{
    Index, LiveOptions, SearchQuery, Settings, SystemProbe, VerifyOptions, folder_overview,
    list_indexes, open_or_scan, scan_to_library, search, verify, volume_statuses,
};
use serde::Serialize;

use crate::render::Style;

#[derive(Parser)]
#[command(
    name = "driveledger",
    version,
    about = "Read-only drive indexer and backup checker. Never writes to scanned drives."
)]
struct Cli {
    /// Library folder for index files [env: DRIVELEDGER_LIBRARY]
    /// [default: ~/Documents/DriveLedger Library]
    #[arg(long, global = true, value_name = "DIR")]
    library: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scan a drive or folder into a new index in the library.
    Scan {
        path: PathBuf,
        /// Hash every file again instead of reusing unchanged fingerprints.
        #[arg(long)]
        full: bool,
        #[command(flatten)]
        out: Output,
    },
    /// Compare a source against a target: what from the source exists on the target?
    ///
    /// SOURCE and TARGET can each be an index file, an index ID (see `list`)
    /// or a live folder, which is scanned into the library first.
    Compare {
        source: String,
        target: String,
        /// Only compare source files below this folder.
        #[arg(long, value_name = "PATH")]
        subpath: Option<String>,
        /// Folder levels in the rollup.
        #[arg(long, default_value_t = 3)]
        depth: usize,
        /// Statuses to list file by file (repeatable). Default: everything but `present`.
        #[arg(long = "status", value_name = "STATUS", value_parser = parse_status)]
        statuses: Vec<Status>,
        /// For live folders, reuse an index of the same folder if it is recent enough.
        #[arg(long)]
        reuse: bool,
        /// Maximum age in hours of an index reused by --reuse.
        #[arg(long, default_value_t = 24, value_name = "HOURS")]
        max_age: u64,
        #[command(flatten)]
        out: Output,
    },
    /// List saved indexes in the library.
    List {
        #[command(flatten)]
        out: Output,
    },
    /// List mounted volumes and whether the library has indexes for them.
    Volumes {
        #[command(flatten)]
        out: Output,
    },
    /// Search files across all indexes (or the ones given with --index).
    Search {
        /// Case-insensitive substring of the path.
        query: Option<String>,
        /// Glob on the file name (or the path, if it contains `/`), e.g. 'SW-FX3*.MP4'.
        #[arg(long)]
        glob: Option<String>,
        /// Extension, repeatable.
        #[arg(long = "ext", value_name = "EXT")]
        extensions: Vec<String>,
        /// Minimum size, e.g. 500M or 2G (decimal units).
        #[arg(long, value_parser = parse_size)]
        min_size: Option<u64>,
        #[arg(long, value_parser = parse_size)]
        max_size: Option<u64>,
        /// Modified on or after this date (YYYY-MM-DD).
        #[arg(long, value_name = "DATE")]
        after: Option<String>,
        /// Modified on or before this date (YYYY-MM-DD).
        #[arg(long, value_name = "DATE")]
        before: Option<String>,
        /// Limit the search to these indexes (IDs or files), repeatable.
        #[arg(long = "index", value_name = "INDEX")]
        indexes: Vec<String>,
        #[arg(long, default_value_t = 200)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
        #[command(flatten)]
        out: Output,
    },
    /// Folder sizes and file counts of an index, a few levels deep.
    Overview {
        index: String,
        #[arg(long, default_value_t = 4)]
        depth: usize,
        #[arg(long, value_name = "PATH")]
        subpath: Option<String>,
        /// Print TSV in the `mapper.sh` layout (size, files, path).
        #[arg(long, conflicts_with = "json")]
        tsv: bool,
        #[command(flatten)]
        out: Output,
    },
    /// Hash matched files in full on both drives to confirm they are identical.
    ///
    /// Run this before deleting anything. Both drives must be mounted. Exit
    /// code 4 if any pair differs or could not be read.
    Verify {
        /// Source index ID or file.
        source: String,
        /// Target index ID or file.
        target: String,
        /// Only verify source files below this folder.
        #[arg(long, value_name = "PATH")]
        subpath: Option<String>,
        /// Only verify this source file (relative path), repeatable.
        #[arg(long = "path", value_name = "PATH")]
        paths: Vec<String>,
        #[command(flatten)]
        out: Output,
    },
    /// Export an index as a TSV file list (like `filliste.sh`) or JSON.
    Export {
        index: String,
        /// TSV with columns size_mb, modified, path.
        #[arg(long, required_unless_present = "json", conflicts_with = "json")]
        tsv: bool,
        /// JSON with scan metadata, files and errors.
        #[arg(long)]
        json: bool,
        /// Write to this file instead of stdout. Refused inside the scanned folder.
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Run the MCP server (stdio by default) so any MCP client can query DriveLedger.
    Mcp {
        /// Serve Streamable HTTP on this loopback address instead, e.g. 127.0.0.1:7341.
        #[arg(long, value_name = "ADDR")]
        http: Option<std::net::SocketAddr>,
    },
    /// Show settings, or save a default library folder.
    Config {
        /// Save this folder as the default library.
        #[arg(long, value_name = "DIR")]
        set_library: Option<PathBuf>,
        #[command(flatten)]
        out: Output,
    },
}

#[derive(Args, Clone, Copy)]
struct Output {
    /// Print machine-readable JSON on stdout.
    #[arg(long)]
    json: bool,
}

impl Command {
    fn json(&self) -> bool {
        match self {
            Command::Scan { out, .. }
            | Command::Compare { out, .. }
            | Command::List { out }
            | Command::Volumes { out }
            | Command::Search { out, .. }
            | Command::Overview { out, .. }
            | Command::Verify { out, .. }
            | Command::Config { out, .. } => out.json,
            Command::Export { json, out, .. } => *json && out.is_none(),
            Command::Mcp { .. } => false,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.command.json();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => report_error(&err, json),
    }
}

fn report_error(err: &anyhow::Error, json: bool) -> ExitCode {
    let core = err.downcast_ref::<driveledger_core::Error>();
    let kind = core.map(|e| e.kind()).unwrap_or("error");
    if err.downcast_ref::<VerifyFailed>().is_some() {
        // The report was already printed; only add the reason on stderr.
        eprintln!("error: {err}");
    } else if json {
        let body = serde_json::json!({ "error": { "kind": kind, "message": format!("{err:#}") } });
        println!("{}", serde_json::to_string_pretty(&body).unwrap());
    } else {
        eprintln!("error: {err:#}");
    }
    if err.downcast_ref::<VerifyFailed>().is_some() {
        return ExitCode::from(4);
    }
    match core {
        Some(driveledger_core::Error::TargetUnavailable(_)) => ExitCode::from(3),
        _ => ExitCode::from(1),
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let settings = Settings::load()?;
    let library = settings.resolve_library(cli.library.as_deref());
    let style = Style::detect();

    match cli.command {
        Command::Scan { path, full, out } => {
            let mut options = settings.scan_options();
            options.incremental = !full;
            let progress = progress::Reporter::new("scanning");
            let outcome = scan_to_library(&path, &library, &options, &SystemProbe, &|p| {
                progress.update(p)
            })?;
            progress.finish();
            if out.json {
                print_json(&outcome)
            } else {
                let errors = Index::open(&outcome.index_path)?.errors()?;
                print!("{}", render::scan(&outcome, &errors, &style));
                Ok(())
            }
        }

        Command::Compare {
            source,
            target,
            subpath,
            depth,
            statuses,
            reuse,
            max_age,
            out,
        } => {
            let live = LiveOptions {
                scan: settings.scan_options(),
                reuse_max_age: reuse.then(|| Duration::from_secs(max_age * 3600)),
            };
            let open = |arg: &str, label: &str| {
                let progress = progress::Reporter::new(&format!("scanning {label}"));
                let opened =
                    open_or_scan(arg, &library, &live, &SystemProbe, &|p| progress.update(p));
                progress.finish();
                opened.with_context(|| format!("{label} `{arg}`"))
            };
            // The target is resolved first so an unmounted drive fails fast,
            // before a long scan of the source.
            let target = open(&target, "target")?;
            let source = open(&source, "source")?;
            let options = CompareOptions {
                source_subpath: subpath,
                rollup_depth: depth,
            };
            let mut result = driveledger_core::compare(&source.index, &target.index, &options)?;
            if out.json {
                if !statuses.is_empty() {
                    result.files.retain(|f| statuses.contains(&f.status));
                }
                print_json(&result)
            } else {
                let listed: &[Status] = if statuses.is_empty() {
                    &render::DEFAULT_LISTED
                } else {
                    &statuses
                };
                print!("{}", render::compare(&result, listed, &style));
                Ok(())
            }
        }

        Command::List { out } => {
            let indexes: Vec<_> = list_indexes(&library).iter().map(|e| e.summary()).collect();
            if out.json {
                print_json(&indexes)
            } else {
                print!("{}", render::list(&indexes, &style));
                Ok(())
            }
        }

        Command::Volumes { out } => {
            let volumes = volume_statuses(&library, &SystemProbe);
            if out.json {
                print_json(&volumes)
            } else {
                print!("{}", render::volumes(&volumes, &style));
                Ok(())
            }
        }

        Command::Search {
            query,
            glob,
            extensions,
            min_size,
            max_size,
            after,
            before,
            indexes,
            limit,
            cursor,
            out,
        } => {
            let entries = if indexes.is_empty() {
                list_indexes(&library)
            } else {
                indexes
                    .iter()
                    .map(|arg| driveledger_core::resolve_index_entry(&library, arg))
                    .collect::<driveledger_core::Result<_>>()?
            };
            let hits = search(
                &entries,
                &SearchQuery {
                    text: query,
                    glob,
                    extensions,
                    min_size,
                    max_size,
                    modified_after: after,
                    modified_before: before,
                    fingerprint: None,
                },
            )?;
            let page = paginate(&hits, Some(limit), cursor.as_deref())
                .map_err(driveledger_core::Error::InvalidArgument)?;
            if out.json {
                print_json(&page)
            } else {
                print!("{}", render::search(&page, &style));
                Ok(())
            }
        }

        Command::Overview {
            index,
            depth,
            subpath,
            tsv,
            out,
        } => {
            let entry = driveledger_core::resolve_index_entry(&library, &index)?;
            let folders = folder_overview(&Index::open(&entry.path)?, depth, subpath.as_deref())?;
            if out.json {
                print_json(&folders)
            } else if tsv {
                Ok(export::write_overview_tsv(
                    &folders,
                    &mut std::io::stdout().lock(),
                )?)
            } else {
                print!("{}", render::overview(&folders, &style));
                Ok(())
            }
        }

        Command::Verify {
            source,
            target,
            subpath,
            paths,
            out,
        } => {
            let source = driveledger_core::resolve_index_entry(&library, &source)?;
            let target = driveledger_core::resolve_index_entry(&library, &target)?;
            let options = VerifyOptions {
                source_subpath: subpath,
                paths,
                concurrency: None,
            };
            let progress = progress::Reporter::new("verifying");
            let report = verify(&source.path, &target.path, &options, &|p| {
                progress.update_verify(p)
            });
            progress.finish();
            let report = report?;
            if out.json {
                print_json(&report)?;
            } else {
                print!("{}", render::verify(&report, &style));
            }
            let problems = report.summary.mismatch.files + report.summary.unreadable.files;
            if problems > 0 {
                return Err(anyhow::Error::new(VerifyFailed(problems)));
            }
            Ok(())
        }

        Command::Export {
            index,
            tsv,
            json: _,
            out,
        } => {
            let entry = driveledger_core::resolve_index_entry(&library, &index)?;
            let index = Index::open(&entry.path)?;
            let mut writer: Box<dyn std::io::Write> = match &out {
                Some(path) => {
                    refuse_inside_scanned_root(path, &entry.info.root_path)?;
                    Box::new(std::io::BufWriter::new(
                        std::fs::File::create(path)
                            .with_context(|| format!("creating {}", path.display()))?,
                    ))
                }
                None => Box::new(std::io::stdout().lock()),
            };
            if tsv {
                export::write_file_list_tsv(&index, &mut writer)?;
            } else {
                export::write_json(&index, &mut writer)?;
            }
            writer.flush()?;
            if let Some(path) = out {
                eprintln!("Wrote {}", path.display());
            }
            Ok(())
        }

        Command::Mcp { http } => {
            let server = mcp::DriveLedgerMcp::new(library, settings);
            let runtime = tokio::runtime::Runtime::new()?;
            match http {
                Some(addr) => runtime.block_on(mcp::serve_http(server, addr)),
                None => runtime.block_on(mcp::serve_stdio(server)),
            }
        }

        Command::Config { set_library, out } => {
            let mut settings = settings;
            if let Some(dir) = set_library {
                let dir = std::path::absolute(&dir).with_context(|| dir.display().to_string())?;
                settings.library = Some(dir);
                settings.save()?;
            }
            let library = settings.resolve_library(cli.library.as_deref());
            if out.json {
                #[derive(Serialize)]
                struct ConfigOut<'a> {
                    settings_path: Option<PathBuf>,
                    library: &'a Path,
                    settings: &'a Settings,
                }
                print_json(&ConfigOut {
                    settings_path: Settings::path(),
                    library: &library,
                    settings: &settings,
                })
            } else {
                print!("{}", render::config(&settings, &library));
                Ok(())
            }
        }
    }
}

/// Verify found differences or unreadable files; reported with exit code 4.
#[derive(Debug)]
struct VerifyFailed(u64);

impl std::fmt::Display for VerifyFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} file pairs did not verify; do not delete them",
            self.0
        )
    }
}

impl std::error::Error for VerifyFailed {}

/// DriveLedger never writes to a scanned drive, so an export into the
/// scanned folder is refused.
fn refuse_inside_scanned_root(out: &Path, root: &str) -> anyhow::Result<()> {
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let resolved = parent
        .canonicalize()
        .with_context(|| format!("output folder {}", parent.display()))?;
    if resolved.starts_with(root) {
        bail!(
            "refusing to write {} inside the scanned folder {root}; DriveLedger never writes to scanned drives",
            out.display()
        );
    }
    Ok(())
}

fn print_json<T: Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn parse_status(s: &str) -> Result<Status, String> {
    s.parse()
}

/// Parse `1234`, `500K`, `2.5G`, `1TB` (decimal units) or `4MiB` (binary).
fn parse_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (number, unit) = s.split_at(split);
    let value: f64 = number.parse().map_err(|_| format!("invalid size `{s}`"))?;
    let multiplier: f64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1.0,
        "k" | "kb" => 1e3,
        "m" | "mb" => 1e6,
        "g" | "gb" => 1e9,
        "t" | "tb" => 1e12,
        "kib" => 1024.0,
        "mib" => 1024.0 * 1024.0,
        "gib" => 1024.0 * 1024.0 * 1024.0,
        other => return Err(format!("unknown size unit `{other}`")),
    };
    Ok((value * multiplier).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("1234").unwrap(), 1234);
        assert_eq!(parse_size("2.5G").unwrap(), 2_500_000_000);
        assert_eq!(parse_size("4MiB").unwrap(), 4 * 1024 * 1024);
        assert!(parse_size("1X").is_err());
    }

    #[test]
    fn cli_definition_is_valid() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
