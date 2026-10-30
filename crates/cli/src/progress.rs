//! Live scan progress on stderr. Only shown when stderr is a terminal, so
//! piped and `--json` output stays clean.

use std::io::{IsTerminal, Write};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use driveledger_core::ScanProgress;
use driveledger_core::scan::ScanPhase;

use crate::render::{human_bytes, thousands};

const INTERVAL: Duration = Duration::from_millis(100);

pub struct Reporter {
    label: String,
    enabled: bool,
    last: Mutex<Option<Instant>>,
}

impl Reporter {
    pub fn new(label: &str) -> Self {
        Reporter {
            label: label.to_string(),
            enabled: std::io::stderr().is_terminal(),
            last: Mutex::new(None),
        }
    }

    pub fn update(&self, p: &ScanProgress) {
        if !self.enabled {
            return;
        }
        let mut last = self.last.lock().unwrap();
        let now = Instant::now();
        let due = last.is_none_or(|t| now.duration_since(t) >= INTERVAL);
        if !due && p.phase != ScanPhase::Done {
            return;
        }
        *last = Some(now);

        let counts = match p.phase {
            ScanPhase::Walking => format!("{} files found", thousands(p.files_seen)),
            _ => format!(
                "{}/{} files · {} hashed",
                thousands(p.files_done),
                thousands(p.files_seen),
                human_bytes(p.bytes_hashed)
            ),
        };
        let errors = if p.errors > 0 {
            format!(" · {} errors", thousands(p.errors))
        } else {
            String::new()
        };
        let path: String = {
            let chars: Vec<char> = p.current_path.chars().collect();
            let keep = 60;
            if chars.len() > keep {
                format!(
                    "…{}",
                    chars[chars.len() - keep..].iter().collect::<String>()
                )
            } else {
                p.current_path.clone()
            }
        };
        let mut stderr = std::io::stderr().lock();
        let _ = write!(stderr, "\r\x1b[2K{} · {counts}{errors}  {path}", self.label);
        let _ = stderr.flush();
    }

    /// Clear the progress line.
    pub fn finish(&self) {
        if self.enabled && self.last.lock().unwrap().is_some() {
            let _ = write!(std::io::stderr(), "\r\x1b[2K");
        }
    }
}
