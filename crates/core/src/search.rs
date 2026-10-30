//! Searching files across one or more indexes.

use chrono::{DateTime, NaiveDate, Utc};
use globset::{GlobBuilder, GlobMatcher};
use rusqlite::types::Value;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::error::{Error, Result};
use crate::index::{FileRecord, Index, file_from_row};
use crate::library::IndexEntry;

/// Search criteria. All set fields must match.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SearchQuery {
    /// Case-insensitive substring of the NFC path.
    pub text: Option<String>,
    /// Glob such as `SW-FX3*.MP4`. Without a `/` it matches the file name,
    /// otherwise the whole relative path. Case-insensitive.
    pub glob: Option<String>,
    /// Extensions without the dot, case-insensitive.
    pub extensions: Vec<String>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    /// `YYYY-MM-DD` or RFC 3339. Inclusive.
    pub modified_after: Option<String>,
    /// `YYYY-MM-DD` (the whole day is included) or RFC 3339.
    pub modified_before: Option<String>,
    /// Exact content fingerprint.
    pub fingerprint: Option<String>,
}

/// One matching file and the index it was found in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SearchHit {
    pub index_id: String,
    pub volume_name: String,
    pub root_path: String,
    pub file: FileRecord,
}

/// Search every index in `indexes`. Hits are ordered by index (as given)
/// and then by path.
pub fn search(indexes: &[IndexEntry], query: &SearchQuery) -> Result<Vec<SearchHit>> {
    let compiled = CompiledQuery::new(query)?;
    let mut hits = Vec::new();
    for entry in indexes {
        let index = Index::open(&entry.path)?;
        for file in compiled.candidates(&index)? {
            if compiled.matches(&file) {
                hits.push(SearchHit {
                    index_id: entry.id(),
                    volume_name: entry.info.volume_name.clone(),
                    root_path: entry.info.root_path.clone(),
                    file,
                });
            }
        }
    }
    Ok(hits)
}

struct CompiledQuery {
    text: Option<String>,
    glob: Option<(GlobMatcher, bool)>,
    sql_where: Vec<String>,
    sql_params: Vec<Value>,
}

impl CompiledQuery {
    fn new(query: &SearchQuery) -> Result<Self> {
        let mut sql_where = Vec::new();
        let mut sql_params: Vec<Value> = Vec::new();

        if !query.extensions.is_empty() {
            let marks = vec!["?"; query.extensions.len()].join(", ");
            sql_where.push(format!("ext IN ({marks})"));
            for ext in &query.extensions {
                sql_params.push(Value::Text(ext.trim_start_matches('.').to_lowercase()));
            }
        }
        if let Some(min) = query.min_size {
            sql_where.push("size >= ?".into());
            sql_params.push(Value::Integer(min as i64));
        }
        if let Some(max) = query.max_size {
            sql_where.push("size <= ?".into());
            sql_params.push(Value::Integer(max as i64));
        }
        if let Some(after) = &query.modified_after {
            sql_where.push("mtime >= ?".into());
            sql_params.push(Value::Integer(parse_date(after, false)?));
        }
        if let Some(before) = &query.modified_before {
            sql_where.push("mtime < ?".into());
            sql_params.push(Value::Integer(parse_date(before, true)?));
        }
        if let Some(fp) = &query.fingerprint {
            sql_where.push("fingerprint = ?".into());
            sql_params.push(Value::Text(fp.to_lowercase()));
        }

        let glob = match &query.glob {
            Some(pattern) => {
                let nfc: String = pattern.nfc().collect();
                let matcher = GlobBuilder::new(&nfc)
                    .case_insensitive(true)
                    .literal_separator(true)
                    .build()
                    .map_err(|e| Error::InvalidArgument(format!("glob `{pattern}`: {e}")))?
                    .compile_matcher();
                Some((matcher, nfc.contains('/')))
            }
            None => None,
        };

        Ok(CompiledQuery {
            text: query
                .text
                .as_ref()
                .map(|t| t.nfc().collect::<String>().to_lowercase())
                .filter(|t| !t.is_empty()),
            glob,
            sql_where,
            sql_params,
        })
    }

    /// Files that pass the SQL-side filters.
    fn candidates(&self, index: &Index) -> Result<Vec<FileRecord>> {
        let mut sql = String::from(
            "SELECT rel_path, rel_path_nfc, name_nfc, ext, size, mtime, ctime, in_bundle,
                    fingerprint, fingerprint_kind, full_hash FROM file",
        );
        if !self.sql_where.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&self.sql_where.join(" AND "));
        }
        sql.push_str(" ORDER BY rel_path_nfc");
        let mut stmt = index.conn().prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(&self.sql_params), file_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn matches(&self, file: &FileRecord) -> bool {
        if let Some(text) = &self.text
            && !file.rel_path_nfc.to_lowercase().contains(text.as_str())
        {
            return false;
        }
        if let Some((glob, full_path)) = &self.glob {
            let subject = if *full_path {
                &file.rel_path_nfc
            } else {
                &file.name_nfc
            };
            if !glob.is_match(subject) {
                return false;
            }
        }
        true
    }
}

/// Parse `YYYY-MM-DD` or RFC 3339 into nanoseconds since the epoch. With
/// `end_of_day`, a plain date means the start of the following day.
fn parse_date(s: &str, end_of_day: bool) -> Result<i64> {
    let invalid = || Error::InvalidArgument(format!("date `{s}`: use YYYY-MM-DD or RFC 3339"));
    let dt: DateTime<Utc> = if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let date = if end_of_day {
            date.succ_opt().ok_or_else(invalid)?
        } else {
            date
        };
        date.and_hms_opt(0, 0, 0).ok_or_else(invalid)?.and_utc()
    } else {
        DateTime::parse_from_rfc3339(s)
            .map_err(|_| invalid())?
            .with_timezone(&Utc)
    };
    dt.timestamp_nanos_opt().ok_or_else(invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        let day = parse_date("2026-10-30", false).unwrap();
        let next = parse_date("2026-10-30", true).unwrap();
        assert_eq!(next - day, 86_400 * 1_000_000_000);
        assert!(parse_date("30.10.2026", false).is_err());
    }
}
