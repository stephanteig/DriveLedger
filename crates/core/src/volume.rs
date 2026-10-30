//! Volume detection: which volume a path lives on, its name, UUID and file
//! system type, and whether it is actually mounted.
//!
//! The platform-specific parts sit behind [`VolumeProbe`] so the rest of the
//! core stays portable and tests can inject a fake.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Identity of a mounted volume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeInfo {
    pub name: String,
    pub uuid: Option<String>,
    /// File system type as reported by the OS, e.g. `apfs`, `exfat`, `smbfs`.
    pub fs_type: Option<String>,
    pub mount_point: PathBuf,
}

impl VolumeInfo {
    /// True for network file systems, where I/O is slow and hashing
    /// concurrency should be lower.
    pub fn is_network(&self) -> bool {
        matches!(
            self.fs_type.as_deref(),
            Some("smbfs" | "nfs" | "afpfs" | "webdav" | "cifs" | "smb3" | "nfs4")
        )
    }
}

/// Looks up volume information for a path.
pub trait VolumeProbe: Send + Sync {
    fn probe(&self, path: &Path) -> Result<VolumeInfo>;
}

/// The real probe for the current OS. On macOS it asks `diskutil` for the
/// volume name and UUID; everywhere (and when `diskutil` has no answer, as for
/// SMB shares) it falls back to the mount table.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemProbe;

impl VolumeProbe for SystemProbe {
    fn probe(&self, path: &Path) -> Result<VolumeInfo> {
        let path = path.canonicalize().map_err(|e| Error::io(path, e))?;
        let mount_point = mount_point_of(&path)?;
        let mount_entry = read_mount_table()
            .into_iter()
            .find(|entry| entry.mount_point == mount_point);

        #[cfg(target_os = "macos")]
        if let Some(info) = diskutil_info(&mount_point) {
            return Ok(VolumeInfo {
                fs_type: info.fs_type.or_else(|| mount_entry.and_then(|e| e.fs_type)),
                ..info
            });
        }

        Ok(VolumeInfo {
            name: fallback_volume_name(&mount_point),
            uuid: None,
            fs_type: mount_entry.and_then(|e| e.fs_type),
            mount_point,
        })
    }
}

/// Check that `root` is reachable right now: it exists, can be listed, and if
/// it lives under `/Volumes/<name>` that folder is a real mount point rather
/// than an empty placeholder left behind by an unmounted drive.
pub fn is_mounted_and_readable(root: &Path) -> bool {
    if !root.is_dir() || std::fs::read_dir(root).is_err() {
        return false;
    }
    match volumes_child(root) {
        Some(volume_dir) => is_mount_point(&volume_dir),
        None => true,
    }
}

/// Error-returning form of [`is_mounted_and_readable`].
pub fn ensure_available(root: &Path) -> Result<()> {
    if is_mounted_and_readable(root) {
        Ok(())
    } else {
        Err(Error::TargetUnavailable(root.to_path_buf()))
    }
}

/// Mounted volumes that a user would think of as drives: the boot volume and
/// everything mounted under `/Volumes` (macOS) or `/media`, `/mnt`,
/// `/run/media` (Linux).
pub fn list_volumes(probe: &dyn VolumeProbe) -> Vec<VolumeInfo> {
    let mut volumes = Vec::new();
    if let Ok(info) = probe.probe(Path::new("/")) {
        volumes.push(info);
    }
    for parent in ["/Volumes", "/media", "/mnt", "/run/media"] {
        let Ok(entries) = std::fs::read_dir(parent) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // `/Volumes/Macintosh HD` is a symlink to `/`; skip links.
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if !is_dir || !is_mount_point(&path) {
                continue;
            }
            if let Ok(info) = probe.probe(&path) {
                if !volumes.iter().any(|v| v.mount_point == info.mount_point) {
                    volumes.push(info);
                }
            }
        }
    }
    volumes
}

/// The nearest ancestor of `path` (inclusive) that is a mount point.
pub fn mount_point_of(path: &Path) -> Result<PathBuf> {
    let mut current = path.to_path_buf();
    loop {
        if is_mount_point(&current) {
            return Ok(current);
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => return Ok(current),
        }
    }
}

/// True if `path` is the root of a file system (its device differs from its
/// parent's, or it is `/`).
#[cfg(unix)]
pub fn is_mount_point(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    match path.parent() {
        None => true,
        Some(parent) => match std::fs::metadata(parent) {
            Ok(parent_meta) => parent_meta.dev() != meta.dev(),
            Err(_) => false,
        },
    }
}

#[cfg(not(unix))]
pub fn is_mount_point(path: &Path) -> bool {
    path.parent().is_none()
}

/// If `path` is `/Volumes/<name>/...`, return `/Volumes/<name>`.
fn volumes_child(path: &Path) -> Option<PathBuf> {
    let mut components = path.components();
    let first_two: Vec<_> = components.by_ref().take(3).collect();
    match first_two.as_slice() {
        [
            std::path::Component::RootDir,
            std::path::Component::Normal(v),
            std::path::Component::Normal(name),
        ] if *v == "Volumes" => Some(Path::new("/Volumes").join(name)),
        _ => None,
    }
}

fn fallback_volume_name(mount_point: &Path) -> String {
    mount_point
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "root".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MountEntry {
    mount_point: PathBuf,
    fs_type: Option<String>,
}

/// Parse the output of `mount`. macOS prints `dev on /path (type, opts)` and
/// Linux prints `dev on /path type fstype (opts)`.
fn read_mount_table() -> Vec<MountEntry> {
    let Ok(output) = Command::new("mount").output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_mount_line)
        .collect()
}

fn parse_mount_line(line: &str) -> Option<MountEntry> {
    let (_, rest) = line.split_once(" on ")?;
    if let Some((path, fs_and_opts)) = rest.split_once(" type ") {
        let fs_type = fs_and_opts.split_whitespace().next().map(str::to_string);
        return Some(MountEntry {
            mount_point: PathBuf::from(path),
            fs_type,
        });
    }
    let open = rest.rfind(" (")?;
    let path = &rest[..open];
    let fs_type = rest[open + 2..]
        .split([',', ')'])
        .next()
        .map(|s| s.trim().to_string());
    Some(MountEntry {
        mount_point: PathBuf::from(path),
        fs_type,
    })
}

/// Ask `diskutil info -plist` about a mount point. Returns `None` when
/// diskutil has no record (SMB shares, for example).
#[cfg(target_os = "macos")]
fn diskutil_info(mount_point: &Path) -> Option<VolumeInfo> {
    let output = Command::new("/usr/sbin/diskutil")
        .args(["info", "-plist"])
        .arg(mount_point)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value: plist::Value = plist::from_bytes(&output.stdout).ok()?;
    let dict = value.as_dictionary()?;
    let get = |key: &str| {
        dict.get(key)
            .and_then(|v| v.as_string())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    Some(VolumeInfo {
        name: get("VolumeName").unwrap_or_else(|| fallback_volume_name(mount_point)),
        uuid: get("VolumeUUID"),
        fs_type: get("FilesystemType"),
        mount_point: mount_point.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_macos_mount_lines() {
        let entry =
            parse_mount_line("//guest@100.119.237.113/ByHola on /Volumes/By Hola (smbfs, nodev)")
                .unwrap();
        assert_eq!(entry.mount_point, PathBuf::from("/Volumes/By Hola"));
        assert_eq!(entry.fs_type.as_deref(), Some("smbfs"));
    }

    #[test]
    fn parses_linux_mount_lines() {
        let entry = parse_mount_line("/dev/sda1 on /media/x type exfat (rw,relatime)").unwrap();
        assert_eq!(entry.mount_point, PathBuf::from("/media/x"));
        assert_eq!(entry.fs_type.as_deref(), Some("exfat"));
    }

    #[test]
    fn missing_volume_is_unavailable() {
        let root = Path::new("/Volumes/DriveLedger-does-not-exist/HYSJ");
        assert!(!is_mounted_and_readable(root));
        assert!(matches!(
            ensure_available(root),
            Err(Error::TargetUnavailable(_))
        ));
    }

    #[test]
    fn volumes_child_extracts_volume_dir() {
        assert_eq!(
            volumes_child(Path::new("/Volumes/ByHola_1/HYSJ/2026")),
            Some(PathBuf::from("/Volumes/ByHola_1"))
        );
        assert_eq!(volumes_child(Path::new("/Users/x")), None);
    }

    #[test]
    fn probes_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let info = SystemProbe.probe(dir.path()).unwrap();
        assert!(!info.name.is_empty());
        assert!(
            dir.path()
                .canonicalize()
                .unwrap()
                .starts_with(&info.mount_point)
        );
    }
}
