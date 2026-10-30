//! Cursor-based paging for large results (MCP tools, CLI `--json`).
//!
//! Cursors are opaque strings. Today they encode an offset into a result
//! that is computed deterministically, so the same request returns the same
//! pages.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Default page size when a caller does not pass `limit`.
pub const DEFAULT_LIMIT: usize = 200;
/// Upper bound for `limit`, so a single response stays manageable.
pub const MAX_LIMIT: usize = 5000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// Number of items across all pages.
    pub total: usize,
    /// Pass this as `cursor` to get the next page; `None` on the last page.
    pub next_cursor: Option<String>,
}

/// Return one page of `items`. An invalid cursor is an error message.
pub fn paginate<T: Clone>(
    items: &[T],
    limit: Option<usize>,
    cursor: Option<&str>,
) -> Result<Page<T>, String> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = match cursor {
        None | Some("") => 0,
        Some(c) => c
            .parse::<usize>()
            .map_err(|_| format!("invalid cursor `{c}`"))?,
    };
    let end = offset.saturating_add(limit).min(items.len());
    let page_items = items.get(offset..end).unwrap_or(&[]).to_vec();
    Ok(Page {
        items: page_items,
        total: items.len(),
        next_cursor: (end < items.len()).then(|| end.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_through_items() {
        let items: Vec<u32> = (0..5).collect();
        let first = paginate(&items, Some(2), None).unwrap();
        assert_eq!(first.items, vec![0, 1]);
        let second = paginate(&items, Some(2), first.next_cursor.as_deref()).unwrap();
        assert_eq!(second.items, vec![2, 3]);
        let last = paginate(&items, Some(2), second.next_cursor.as_deref()).unwrap();
        assert_eq!(last.items, vec![4]);
        assert_eq!(last.next_cursor, None);
        assert!(paginate(&items, None, Some("x")).is_err());
    }
}
