//! Statistics writer methods: file column stats, file variant stats, table stats.

#![allow(missing_docs)]

use slateduck_core::keys;
use slateduck_core::rows::{
    FileColumnStatsRow, FileVariantStatsRow, TableColumnStatsRow, TableStatsRow,
};
use slateduck_core::values;

use crate::error::CatalogResult;

use super::hash_tag_key;
use super::CatalogWriter;

fn apply_i64_delta(value: u64, delta: i64) -> u64 {
    if delta >= 0 {
        value.saturating_add(delta as u64)
    } else {
        value.saturating_sub(delta.unsigned_abs())
    }
}

fn merge_min(existing: Option<&str>, incoming: Option<&str>) -> Option<String> {
    match (existing, incoming) {
        (Some(left), Some(right)) => Some(stats_min(left, right).to_string()),
        (Some(value), None) | (None, Some(value)) => Some(value.to_string()),
        (None, None) => None,
    }
}

fn merge_max(existing: Option<&str>, incoming: Option<&str>) -> Option<String> {
    match (existing, incoming) {
        (Some(left), Some(right)) => Some(stats_max(left, right).to_string()),
        (Some(value), None) | (None, Some(value)) => Some(value.to_string()),
        (None, None) => None,
    }
}

fn stats_min<'a>(left: &'a str, right: &'a str) -> &'a str {
    if stats_value_less_or_equal(left, right) {
        left
    } else {
        right
    }
}

fn stats_max<'a>(left: &'a str, right: &'a str) -> &'a str {
    if stats_value_less_or_equal(left, right) {
        right
    } else {
        left
    }
}

/// Type-aware less-or-equal comparison for DuckLake column stat values.
///
/// Encoded stat values are stored as their string representation. We must
/// compare them semantically so that numeric ordering is respected (e.g.
/// `-10` < `-2`, `10` < `2` lexicographically but `2` < `10` numerically).
///
/// Comparison priority:
/// 1. `BOOLEAN`-like tokens: `"false"` < `"true"`.
/// 2. `INTEGER`/`BIGINT`: parsed as `i128` for signed integer comparison.
/// 3. `FLOAT`/`DOUBLE`: parsed as `f64` for finite float comparison.
/// 4. `DATE`: ISO-8601 `YYYY-MM-DD` strings sort correctly lexicographically
///    so the string comparison path handles them correctly.
/// 5. `TIMESTAMP`/`TIMESTAMPTZ`: ISO-8601 strings sort correctly lexicographically.
/// 6. Everything else (strings, UUIDs, decimals, etc.): lexicographic.
fn stats_value_less_or_equal(left: &str, right: &str) -> bool {
    // BOOLEAN
    match (left, right) {
        ("false", "false") | ("true", "true") => return true,
        ("false", "true") => return true,
        ("true", "false") => return false,
        _ => {}
    }

    // INTEGER / BIGINT — covers negative numbers (e.g. "-10" vs "-2").
    if let (Ok(l), Ok(r)) = (left.parse::<i128>(), right.parse::<i128>()) {
        return l <= r;
    }

    // FLOAT / DOUBLE — covers finite float values with fractional parts.
    if let (Ok(l), Ok(r)) = (left.parse::<f64>(), right.parse::<f64>()) {
        if l.is_finite() && r.is_finite() {
            return l <= r;
        }
    }

    // DATE, TIMESTAMP, TIMESTAMPTZ, UUID, and plain strings — ISO-8601 date
    // and timestamp strings sort correctly lexicographically, as do UUIDs.
    left <= right
}

fn merge_optional_bool_or(existing: Option<bool>, incoming: Option<bool>) -> Option<bool> {
    match (existing, incoming) {
        (Some(left), Some(right)) => Some(left || right),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

/// Input parameters for `upsert_file_column_stats`.
///
/// Introduced in v0.21 to replace the 7-argument positional API and bring the
/// call below the `clippy::too_many_arguments` threshold.
#[derive(Debug, Clone)]
pub struct FileColumnStatsInput<'a> {
    pub table_id: u64,
    pub column_id: u64,
    pub data_file_id: u64,
    /// v0.26: renamed from has_null (spec field is contains_null).
    pub contains_null: bool,
    pub min_value: Option<&'a str>,
    pub max_value: Option<&'a str>,
    pub contains_nan: bool,
    /// v0.26: total bytes for this column in the file.
    pub column_size_bytes: Option<u64>,
    /// v0.26: number of non-null values.
    pub value_count: Option<u64>,
    /// v0.26: number of null values.
    pub null_count: Option<u64>,
    /// v0.26: JSON blob for geometry or variant extra stats.
    pub extra_stats: Option<&'a str>,
}

/// Input parameters for `upsert_file_variant_stats`.
///
/// Introduced in v0.21 to replace the 7-argument positional API and bring the
/// call below the `clippy::too_many_arguments` threshold.
#[derive(Debug, Clone)]
pub struct FileVariantStatsInput<'a> {
    pub table_id: u64,
    pub column_id: u64,
    /// v0.26: renamed from variant_path (spec field is variant_key).
    pub variant_key: &'a str,
    pub data_file_id: u64,
    pub min_value: Option<&'a str>,
    pub max_value: Option<&'a str>,
    /// v0.26: the shredded type of this variant key.
    pub shredded_type: Option<&'a str>,
    /// v0.26: total bytes for this variant column.
    pub column_size_bytes: Option<u64>,
    /// v0.26: number of non-null values.
    pub value_count: Option<u64>,
    /// v0.26: number of null values.
    pub null_count: Option<u64>,
    /// v0.26: whether any NaN values are present.
    pub contains_nan: Option<bool>,
    /// v0.26: JSON blob for extra stats.
    pub extra_stats: Option<&'a str>,
}

impl CatalogWriter {
    /// Update table stats from a DuckLake v1.0 `INSERT INTO ducklake_table_stats` statement.
    /// Parameters match the DuckLake v1.0 column order: table_id, record_count,
    /// next_row_id (position 2 — NOT file_count), file_size_bytes.
    pub async fn update_table_stats(
        &mut self,
        table_id: u64,
        record_count: u64,
        next_row_id: u64,
        file_size_bytes: u64,
    ) -> CatalogResult<()> {
        let existing = self.read_table_stats_or_default(table_id).await?;
        // Advance next_row_id by at least the number of new rows inserted in this
        // batch (additive), but also honour any larger absolute value that DuckDB
        // may provide directly.  This handles both "absolute" and "batch-relative"
        // next_row_id values sent during incremental inlined-data inserts.
        let merged_next_row_id = std::cmp::max(
            existing
                .next_row_id
                .unwrap_or(0)
                .saturating_add(record_count),
            next_row_id,
        );
        let row = TableStatsRow {
            table_id,
            record_count: existing.record_count.saturating_add(record_count),
            internal_file_count: existing.internal_file_count,
            file_size_bytes: existing.file_size_bytes.saturating_add(file_size_bytes),
            next_row_id: Some(merged_next_row_id),
        };
        let key = keys::key_table_stats(table_id);
        self.db.put(&key, values::encode_value(&row)).await?;
        Ok(())
    }

    pub async fn adjust_table_record_count(
        &mut self,
        table_id: u64,
        delta: i64,
    ) -> CatalogResult<()> {
        let mut row = self.read_table_stats_or_default(table_id).await?;
        row.record_count = apply_i64_delta(row.record_count, delta);
        let key = keys::key_table_stats(table_id);
        self.db.put(&key, values::encode_value(&row)).await?;
        Ok(())
    }

    pub async fn set_table_stats(
        &mut self,
        table_id: u64,
        record_count: u64,
        file_size_bytes: u64,
        next_row_id: u64,
    ) -> CatalogResult<()> {
        let existing_internal_file_count = {
            let key = keys::key_table_stats(table_id);
            match self.db.get(&key).await? {
                Some(data) => {
                    let existing: TableStatsRow = slateduck_core::values::decode_value(&data)
                        .unwrap_or(TableStatsRow {
                            table_id,
                            record_count: 0,
                            internal_file_count: 0,
                            file_size_bytes: 0,
                            next_row_id: None,
                        });
                    existing.internal_file_count
                }
                None => 0,
            }
        };
        let row = TableStatsRow {
            table_id,
            record_count,
            internal_file_count: existing_internal_file_count,
            file_size_bytes,
            next_row_id: Some(next_row_id),
        };
        let key = keys::key_table_stats(table_id);
        self.db.put(&key, values::encode_value(&row)).await?;
        Ok(())
    }

    /// v0.26: Write or update table-level column stats.
    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_table_column_stats(
        &mut self,
        table_id: u64,
        column_id: u64,
        contains_null: bool,
        min_value: Option<&str>,
        max_value: Option<&str>,
        contains_nan: Option<bool>,
        extra_stats: Option<&str>,
    ) -> CatalogResult<()> {
        let existing = {
            let key = keys::key_table_column_stats(table_id, column_id);
            match self.db.get(&key).await? {
                Some(data) => {
                    slateduck_core::values::decode_value::<TableColumnStatsRow>(&data).ok()
                }
                None => None,
            }
        };
        let row = TableColumnStatsRow {
            table_id,
            column_id,
            contains_null: existing
                .as_ref()
                .map(|row| row.contains_null || contains_null)
                .unwrap_or(contains_null),
            min_value: merge_min(
                existing.as_ref().and_then(|row| row.min_value.as_deref()),
                min_value,
            ),
            max_value: merge_max(
                existing.as_ref().and_then(|row| row.max_value.as_deref()),
                max_value,
            ),
            contains_nan: merge_optional_bool_or(
                existing.as_ref().and_then(|row| row.contains_nan),
                contains_nan,
            ),
            extra_stats: extra_stats
                .map(|s| s.to_string())
                .or_else(|| existing.and_then(|row| row.extra_stats)),
        };
        let key = keys::key_table_column_stats(table_id, column_id);
        self.db.put(&key, values::encode_value(&row)).await?;
        Ok(())
    }

    async fn read_table_stats_or_default(&self, table_id: u64) -> CatalogResult<TableStatsRow> {
        let key = keys::key_table_stats(table_id);
        Ok(match self.db.get(&key).await? {
            Some(data) => slateduck_core::values::decode_value(&data).unwrap_or(TableStatsRow {
                table_id,
                record_count: 0,
                internal_file_count: 0,
                file_size_bytes: 0,
                next_row_id: None,
            }),
            None => TableStatsRow {
                table_id,
                record_count: 0,
                internal_file_count: 0,
                file_size_bytes: 0,
                next_row_id: None,
            },
        })
    }

    pub async fn upsert_file_column_stats(
        &mut self,
        input: FileColumnStatsInput<'_>,
    ) -> CatalogResult<()> {
        let row = FileColumnStatsRow {
            table_id: input.table_id,
            column_id: input.column_id,
            data_file_id: input.data_file_id,
            contains_null: input.contains_null,
            min_value: input.min_value.map(|s| s.to_string()),
            max_value: input.max_value.map(|s| s.to_string()),
            contains_nan: input.contains_nan,
            column_size_bytes: input.column_size_bytes,
            value_count: input.value_count,
            null_count: input.null_count,
            extra_stats: input.extra_stats.map(|s| s.to_string()),
        };
        let key = keys::key_file_column_stats(input.table_id, input.column_id, input.data_file_id);
        self.db.put(&key, values::encode_value(&row)).await?;
        Ok(())
    }

    pub async fn upsert_file_variant_stats(
        &mut self,
        input: FileVariantStatsInput<'_>,
    ) -> CatalogResult<()> {
        let variant_path_hash = hash_tag_key(input.variant_key);
        #[allow(deprecated)]
        let row = FileVariantStatsRow {
            table_id: input.table_id,
            column_id: input.column_id,
            deprecated_variant_path_hash: None,
            data_file_id: input.data_file_id,
            variant_key: input.variant_key.to_string(),
            min_value: input.min_value.map(|s| s.to_string()),
            max_value: input.max_value.map(|s| s.to_string()),
            shredded_type: input.shredded_type.map(|s| s.to_string()),
            column_size_bytes: input.column_size_bytes,
            value_count: input.value_count,
            null_count: input.null_count,
            contains_nan: input.contains_nan,
            extra_stats: input.extra_stats.map(|s| s.to_string()),
        };
        let key = keys::key_file_variant_stats(
            input.table_id,
            input.column_id,
            variant_path_hash,
            input.data_file_id,
        );
        self.db.put(&key, values::encode_value(&row)).await?;
        Ok(())
    }
}
