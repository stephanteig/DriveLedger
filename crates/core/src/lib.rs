//! DriveLedger core engine.
//!
//! All scanning, indexing and comparison logic lives here. The CLI, MCP server
//! and desktop app are thin layers on top of this crate.
//!
//! Safety: this crate is read-only on every scanned path. The only writes it
//! performs are index files inside the user-chosen library folder.

/// Version of the tool, recorded in every index it writes.
pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_version_is_set() {
        assert!(!TOOL_VERSION.is_empty());
    }
}
