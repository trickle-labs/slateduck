//! Per-shard Parquet output writing.
//!
//! Each shard writes one Parquet file per output cycle.  The file is
//! registered as a data file on the matview's output table in the catalog.
//!
//! ## File naming
//! ```text
//! {data_prefix}/matviews/{matview_id}/shards/{shard_id}/output-{seq:016x}.parquet
//! ```
//!
//! ## Compaction
//! The compaction policy is stored in `MatviewRow.state_uri` (re-used as a
//! config bag until a dedicated field is added in v0.13).  Supported values:
//! * `"never"` — no compaction (default).
//! * `"<duration>h"` / `"<duration>m"` — compact output files older than the
//!   specified duration.  Implemented by the IVM worker's background
//!   compaction loop.

use std::collections::HashMap;

use serde_json::Value;

/// Parquet output policy for a matview.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CompactionPolicy {
    #[default]
    Never,
    After(std::time::Duration),
}

/// Parquet output configuration including sort keys for optimized reads.
#[derive(Debug, Clone, Default)]
pub struct ParquetOutputConfig {
    /// Sort keys for Parquet row-group ordering (auto-populated from GROUP BY / join keys).
    pub sort_keys: Vec<String>,
    /// Compaction policy.
    pub compaction: CompactionPolicy,
}

impl CompactionPolicy {
    /// Parse a compaction policy string (e.g. `"1h"`, `"30m"`, `"never"`).
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "never" | "" => Self::Never,
            other => {
                if let Some(h) = other.strip_suffix('h') {
                    if let Ok(n) = h.parse::<u64>() {
                        return Self::After(std::time::Duration::from_secs(n * 3600));
                    }
                } else if let Some(m) = other.strip_suffix('m') {
                    if let Ok(n) = m.parse::<u64>() {
                        return Self::After(std::time::Duration::from_secs(n * 60));
                    }
                }
                Self::Never
            }
        }
    }
}

/// Compute the data-file path for a shard's output at sequence `seq`.
pub fn shard_output_path(data_prefix: &str, matview_id: u64, shard_id: u32, seq: u64) -> String {
    let prefix = data_prefix.trim_end_matches('/');
    format!("{prefix}/matviews/{matview_id}/shards/{shard_id}/output-{seq:016x}.parquet")
}

/// Serialise a set of output rows to Parquet-compatible JSON-line format.
///
/// In v0.12 SlateDuck does not embed a native Parquet writer; instead output
/// rows are written as newline-delimited JSON ("NDJSON").  A subsequent
/// DuckDB `COPY … TO … (FORMAT PARQUET)` step converts these files, which is
/// acceptable because the IVM worker is co-located with the DuckDB engine in
/// the same process in the v0.12 deployment model.
///
/// The bytes returned are suitable for storing as an inlined value or for
/// upload to an object store.
pub fn serialise_output_ndjson(rows: &[HashMap<String, Value>]) -> Vec<u8> {
    let mut buf = Vec::new();
    for row in rows {
        let s = serde_json::to_string(row).unwrap_or_default();
        buf.extend_from_slice(s.as_bytes());
        buf.push(b'\n');
    }
    buf
}

/// Estimate the size in bytes of the NDJSON representation of `rows`.
pub fn estimate_output_bytes(rows: &[HashMap<String, Value>]) -> usize {
    rows.iter()
        .map(|r| serde_json::to_string(r).map(|s| s.len() + 1).unwrap_or(2))
        .sum()
}

/// The hidden rowid column name used in DuckLake tables.
pub const ROWID_COLUMN: &str = "__sd_rowid";

/// Inject `__sd_rowid` values into output rows, starting from `start_rowid`.
///
/// This stamps each row with a stable, monotonically increasing rowid allocated
/// from the catalog's per-table counter. The rowid survives compaction and
/// file re-registration.
pub fn inject_rowids(rows: &mut [HashMap<String, Value>], start_rowid: u64) {
    for (i, row) in rows.iter_mut().enumerate() {
        row.insert(
            ROWID_COLUMN.to_string(),
            Value::from(start_rowid + i as u64),
        );
    }
}

/// Check if a row already has a `__sd_rowid` value.
pub fn has_rowid(row: &HashMap<String, Value>) -> bool {
    row.contains_key(ROWID_COLUMN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compaction_policy_parse() {
        assert_eq!(CompactionPolicy::parse("never"), CompactionPolicy::Never);
        assert_eq!(CompactionPolicy::parse(""), CompactionPolicy::Never);
        assert_eq!(
            CompactionPolicy::parse("1h"),
            CompactionPolicy::After(std::time::Duration::from_secs(3600))
        );
        assert_eq!(
            CompactionPolicy::parse("30m"),
            CompactionPolicy::After(std::time::Duration::from_secs(1800))
        );
        assert_eq!(CompactionPolicy::parse("garbage"), CompactionPolicy::Never);
    }

    #[test]
    fn output_path_format() {
        let path = shard_output_path("s3://bucket/data", 1, 0, 255);
        assert_eq!(
            path,
            "s3://bucket/data/matviews/1/shards/0/output-00000000000000ff.parquet"
        );
    }

    #[test]
    fn serialise_ndjson_is_one_line_per_row() {
        let rows = vec![[
            ("a".to_string(), Value::from(1)),
            ("b".to_string(), Value::from("x")),
        ]
        .into_iter()
        .collect::<HashMap<_, _>>()];
        let bytes = serialise_output_ndjson(&rows);
        assert_eq!(bytes.iter().filter(|&&b| b == b'\n').count(), 1);
    }
}
