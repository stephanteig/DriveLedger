//! Content fingerprints.
//!
//! `sample3` is the default: BLAKE3 over the file size followed by three
//! 1 MiB samples (start, middle, end). It reads at most 3 MiB per file, so a
//! 12 GB camera clip on an SMB share costs about as much as a small photo,
//! yet it tells apart files that merely share a name and size (Sony FX3 reuses
//! `SW-FX30001.MP4` on every shoot). Files of 3 MiB or less are hashed whole.
//!
//! `full` hashes every byte and is used by `verify` before deleting anything.
//!
//! Files are only ever opened read-only.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Size of each sample.
pub const SAMPLE_BYTES: u64 = 1024 * 1024;

/// Files up to this size are hashed whole instead of sampled.
pub const WHOLE_FILE_LIMIT: u64 = 3 * SAMPLE_BYTES;

/// A computed fingerprint and the number of content bytes read for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub hex: String,
    pub bytes_read: u64,
}

/// Compute the `sample3` fingerprint of a file.
pub fn sample3(path: &Path) -> std::io::Result<Fingerprint> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();

    let mut hasher = blake3::Hasher::new();
    hasher.update(&size.to_le_bytes());

    let bytes_read = if size <= WHOLE_FILE_LIMIT {
        let read = std::io::copy(&mut (&mut file).take(size), &mut HashWriter(&mut hasher))?;
        if read != size {
            return Err(short_read(path));
        }
        read
    } else {
        // Same offsets as the Python prototype the matching was validated with.
        let offsets = [0, size / 2 - SAMPLE_BYTES / 2, size - SAMPLE_BYTES];
        let mut buf = vec![0u8; SAMPLE_BYTES as usize];
        for offset in offsets {
            file.seek(SeekFrom::Start(offset))?;
            file.read_exact(&mut buf).map_err(|e| match e.kind() {
                std::io::ErrorKind::UnexpectedEof => short_read(path),
                _ => e,
            })?;
            hasher.update(&buf);
        }
        3 * SAMPLE_BYTES
    };

    Ok(Fingerprint {
        hex: hasher.finalize().to_hex().to_string(),
        bytes_read,
    })
}

/// Compute a BLAKE3 hash of the entire file, streaming it in 1 MiB chunks.
pub fn full_hash(path: &Path) -> std::io::Result<Fingerprint> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; SAMPLE_BYTES as usize];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok(Fingerprint {
        hex: hasher.finalize().to_hex().to_string(),
        bytes_read: total,
    })
}

fn short_read(path: &Path) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        format!("{} changed size while it was being read", path.display()),
    )
}

/// Adapter so `io::copy` can stream into a BLAKE3 hasher.
struct HashWriter<'a>(&'a mut blake3::Hasher);

impl std::io::Write for HashWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    fn write(dir: &Path, name: &str, data: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, data).unwrap();
        path
    }

    #[test]
    fn small_files_are_hashed_whole_with_size_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let data = bytes(1000, 1);
        let fp = sample3(&write(dir.path(), "a", &data)).unwrap();

        let mut expected = blake3::Hasher::new();
        expected.update(&1000u64.to_le_bytes());
        expected.update(&data);
        assert_eq!(fp.hex, expected.finalize().to_hex().to_string());
        assert_eq!(fp.bytes_read, 1000);
    }

    #[test]
    fn large_files_hash_three_samples() {
        let dir = tempfile::tempdir().unwrap();
        let size = 5 * SAMPLE_BYTES as usize;
        let data = bytes(size, 2);
        let fp = sample3(&write(dir.path(), "big", &data)).unwrap();
        assert_eq!(fp.bytes_read, 3 * SAMPLE_BYTES);

        let s = SAMPLE_BYTES as usize;
        let mid = size / 2 - s / 2;
        let mut expected = blake3::Hasher::new();
        expected.update(&(size as u64).to_le_bytes());
        expected.update(&data[..s]);
        expected.update(&data[mid..mid + s]);
        expected.update(&data[size - s..]);
        assert_eq!(fp.hex, expected.finalize().to_hex().to_string());
    }

    #[test]
    fn a_change_inside_a_sample_changes_the_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let mut data = bytes(5 * SAMPLE_BYTES as usize, 3);
        let a = sample3(&write(dir.path(), "a", &data)).unwrap();
        let last = data.len() - 1;
        data[last] ^= 0xFF;
        let b = sample3(&write(dir.path(), "b", &data)).unwrap();
        assert_ne!(a.hex, b.hex);
    }

    #[test]
    fn same_bytes_different_size_differ() {
        // A truncated copy of a small file must never match the original.
        let dir = tempfile::tempdir().unwrap();
        let data = bytes(4096 * 3, 4);
        let a = sample3(&write(dir.path(), "a", &data)).unwrap();
        let b = sample3(&write(dir.path(), "b", &data[..4096 * 2])).unwrap();
        assert_ne!(a.hex, b.hex);
    }

    #[test]
    fn full_hash_matches_plain_blake3() {
        let dir = tempfile::tempdir().unwrap();
        let data = bytes(3 * SAMPLE_BYTES as usize + 17, 5);
        let fp = full_hash(&write(dir.path(), "a", &data)).unwrap();
        assert_eq!(fp.hex, blake3::hash(&data).to_hex().to_string());
        assert_eq!(fp.bytes_read, data.len() as u64);
    }
}
