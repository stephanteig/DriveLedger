//! User settings, stored as JSON in the platform config directory
//! (`~/Library/Application Support/DriveLedger/settings.json` on macOS).

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::scan::{DEFAULT_BUNDLE_EXTENSIONS, ScanOptions, SkipList};

/// Environment variable that overrides the library folder.
pub const LIBRARY_ENV: &str = "DRIVELEDGER_LIBRARY";
/// Environment variable that overrides the config directory (used by tests).
pub const CONFIG_DIR_ENV: &str = "DRIVELEDGER_CONFIG_DIR";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Settings {
    /// Library folder. `None` means the default location.
    pub library: Option<PathBuf>,
    pub skip: SkipList,
    pub bundle_extensions: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            library: None,
            skip: SkipList::default(),
            bundle_extensions: DEFAULT_BUNDLE_EXTENSIONS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

impl Settings {
    /// Where the settings file lives.
    pub fn path() -> Option<PathBuf> {
        let dir = match std::env::var_os(CONFIG_DIR_ENV) {
            Some(dir) => PathBuf::from(dir),
            None => dirs::config_dir()?.join("DriveLedger"),
        };
        Some(dir.join("settings.json"))
    }

    /// Load settings, falling back to defaults if no file exists yet.
    pub fn load() -> Result<Settings> {
        let Some(path) = Self::path() else {
            return Ok(Settings::default());
        };
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                Error::io(
                    &path,
                    std::io::Error::new(std::io::ErrorKind::InvalidData, e),
                )
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(Error::io(&path, e)),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path().ok_or_else(|| {
            Error::io(
                "settings.json",
                std::io::Error::new(std::io::ErrorKind::NotFound, "no config directory"),
            )
        })?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        let json = serde_json::to_vec_pretty(self).expect("settings serialize");
        std::fs::write(&path, json).map_err(|e| Error::io(&path, e))
    }

    /// `~/Documents/DriveLedger Library`.
    pub fn default_library() -> PathBuf {
        dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("DriveLedger Library")
    }

    /// The library to use: an explicit override (CLI flag), then the
    /// `DRIVELEDGER_LIBRARY` environment variable, then the saved setting,
    /// then the default.
    pub fn resolve_library(&self, explicit: Option<&Path>) -> PathBuf {
        if let Some(path) = explicit {
            return path.to_path_buf();
        }
        if let Some(env) = std::env::var_os(LIBRARY_ENV).filter(|v| !v.is_empty()) {
            return PathBuf::from(env);
        }
        self.library.clone().unwrap_or_else(Self::default_library)
    }

    /// Scan options built from these settings.
    pub fn scan_options(&self) -> ScanOptions {
        ScanOptions {
            skip: self.skip.clone(),
            bundle_extensions: self.bundle_extensions.clone(),
            ..ScanOptions::default()
        }
    }
}
