//! Statistics writer methods: file column stats, file variant stats, table stats.

#![allow(missing_docs)]

use rocklake_core::keys;
use rocklake_core::rows::{
    FileColumnStatsRow, FileVariantStatsRow, TableColumnStatsRow, TableStatsRow,
};
use rocklake_core::values;

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
/// Comparison priority (v0.27.8):
/// 1. `BOOLEAN`-like tokens: `"false"` < `"true"`.
/// 2. `INTEGER`/`BIGINT` and `UBIGINT`/unsigned: parsed as `i128` which
///    covers the full `u64` range (0 .. u64::MAX = 18_446_744_073_709_551_615)
///    without overflow.  Negative values are handled correctly.
/// 3. `DECIMAL`/`NUMERIC`: string-based comparison that avoids f64 precision
///    loss for large exact decimals (e.g. `"999999999999999999.999"` vs
///    `"1000000000000000000.000"`).
/// 4. `FLOAT`/`DOUBLE`: parsed as `f64` for values that have fractional parts
///    and cannot be exactly represented as integers or decimals.
/// 5. `DATE`: ISO-8601 `YYYY-MM-DD` strings sort correctly lexicographically.
/// 6. `TIMESTAMP`/`TIMESTAMPTZ`: ISO-8601 strings sort correctly lexicographically.
/// 7. `UUID`: lexicographic is correct for RFC-4122 UUIDs.
/// 8. Everything else (strings, etc.): lexicographic.
fn stats_value_less_or_equal(left: &str, right: &str) -> bool {
    // BOOLEAN
    match (left, right) {
        ("false", "false") | ("true", "true") => return true,
        ("false", "true") => return true,
        ("true", "false") => return false,
        _ => {}
    }

    // INTEGER / BIGINT / UBIGINT — i128 covers the full u64 range so unsigned
    // integer comparison (e.g. u64::MAX = 18446744073709551615) is exact.
    if let (Ok(l), Ok(r)) = (left.parse::<i128>(), right.parse::<i128>()) {
        return l <= r;
    }

    // DECIMAL / NUMERIC — string-based comparison avoids f64 precision loss for
    // large exact decimals.  Only attempted when both values look like decimal
    // strings (optional sign, digits, optional dot and more digits).
    if let Some(ord) = compare_decimal_strings(left, right) {
        return ord != std::cmp::Ordering::Greater;
    }

    // FLOAT / DOUBLE — covers remaining finite float values.
    if let (Ok(l), Ok(r)) = (left.parse::<f64>(), right.parse::<f64>()) {
        if l.is_finite() && r.is_finite() {
            return l <= r;
        }
    }

    // DATE, TIMESTAMP, TIMESTAMPTZ, UUID, and plain strings — ISO-8601 date
    // and timestamp strings sort correctly lexicographically, as do UUIDs.
    left <= right
}

/// String-based decimal comparison that preserves exact ordering for
/// `DECIMAL`/`NUMERIC` values without introducing a bigdecimal dependency.
///
/// Returns `None` if either string does not look like a decimal literal,
/// allowing the caller to fall through to the `f64` path.
fn compare_decimal_strings(left: &str, right: &str) -> Option<std::cmp::Ordering> {
    fn is_decimal(s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        let s = s.strip_prefix('-').unwrap_or(s);
        if s.is_empty() {
            return false;
        }
        // Must contain a '.' (otherwise it's just an integer — handled by i128).
        let Some(dot_pos) = s.find('.') else {
            return false;
        };
        let int_part = &s[..dot_pos];
        let frac_part = &s[dot_pos + 1..];
        !int_part.is_empty()
            && int_part.chars().all(|c| c.is_ascii_digit())
            && !frac_part.is_empty()
            && frac_part.chars().all(|c| c.is_ascii_digit())
    }

    if !is_decimal(left) || !is_decimal(right) {
        return None;
    }

    let (l_neg, l_abs) = if let Some(s) = left.strip_prefix('-') {
        (true, s)
    } else {
        (false, left)
    };
    let (r_neg, r_abs) = if let Some(s) = right.strip_prefix('-') {
        (true, s)
    } else {
        (false, right)
    };

    // Compare absolute values: integer part length first, then lexicographic.
    let abs_cmp = compare_decimal_abs(l_abs, r_abs);

    Some(match (l_neg, r_neg) {
        (false, false) => abs_cmp,
        (true, true) => abs_cmp.reverse(),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
    })
}

/// Compare two non-negative decimal strings (no sign prefix, must contain '.').
fn compare_decimal_abs(left: &str, right: &str) -> std::cmp::Ordering {
    let (l_int, l_frac) = left.split_once('.').unwrap();
    let (r_int, r_frac) = right.split_once('.').unwrap();

    // Longer integer part means larger number (no leading zeros expected).
    match l_int.len().cmp(&r_int.len()) {
        std::cmp::Ordering::Equal => {}
        other => return other,
    }

    // Same length integer part — compare lexicographically.
    match l_int.cmp(r_int) {
        std::cmp::Ordering::Equal => {}
        other => return other,
    }

    // Integer parts equal — compare fractional parts by padding to same length.
    let max_len = l_frac.len().max(r_frac.len());
    let l_padded = format!("{l_frac:0<max_len$}");
    let r_padded = format!("{r_frac:0<max_len$}");
    l_padded.cmp(&r_padded)
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
                    let existing: TableStatsRow = rocklake_core::values::decode_value(&data)
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
                    rocklake_core::values::decode_value::<TableColumnStatsRow>(&data).ok()
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
            Some(data) => rocklake_core::values::decode_value(&data).unwrap_or(TableStatsRow {
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

// ── v0.27.6 unit tests for stats_value_less_or_equal ─────────────────────────

#[cfg(test)]
mod stats_unit_tests {
    use super::stats_value_less_or_equal;

    /// `-10` < `-2` numerically but `"-10"` > `"-2"` lexicographically.
    #[test]
    fn negative_integer_less_or_equal_is_numeric() {
        assert!(stats_value_less_or_equal("-10", "-2"), "-10 <= -2");
        assert!(!stats_value_less_or_equal("-2", "-10"), "-2 is not <= -10");
        assert!(
            stats_value_less_or_equal("-10", "-10"),
            "-10 <= -10 (equal)"
        );
    }

    /// Mixed sign: `-5` < `0` < `5`.
    #[test]
    fn negative_to_positive_crossing_zero_is_numeric() {
        assert!(stats_value_less_or_equal("-5", "0"), "-5 <= 0");
        assert!(stats_value_less_or_equal("0", "5"), "0 <= 5");
        assert!(stats_value_less_or_equal("-5", "5"), "-5 <= 5");
        assert!(!stats_value_less_or_equal("5", "-5"), "5 is not <= -5");
    }

    /// `"10"` > `"2"` numerically but `"10"` < `"2"` lexicographically.
    #[test]
    fn multi_digit_integer_is_numeric() {
        assert!(stats_value_less_or_equal("2", "10"), "2 <= 10");
        assert!(!stats_value_less_or_equal("10", "2"), "10 is not <= 2");
        assert!(stats_value_less_or_equal("10", "10"), "10 <= 10 (equal)");
    }

    /// Floats differing only in fractional part are compared numerically.
    #[test]
    fn float_fractional_part_is_numeric() {
        assert!(stats_value_less_or_equal("1.1", "1.9"), "1.1 <= 1.9");
        assert!(
            !stats_value_less_or_equal("1.9", "1.1"),
            "1.9 is not <= 1.1"
        );
    }

    /// `"1.10"` vs `"1.9"`: f64 parse gives 1.10 == 1.1 < 1.9.
    #[test]
    fn float_trailing_zero_fractional_is_numeric() {
        assert!(stats_value_less_or_equal("1.10", "1.9"), "1.10 <= 1.9");
        assert!(
            !stats_value_less_or_equal("1.9", "1.10"),
            "1.9 is not <= 1.10"
        );
    }

    /// Negative floats: `-1000.0` < `-3.14`.
    #[test]
    fn negative_float_is_numeric() {
        assert!(
            stats_value_less_or_equal("-1000.0", "-3.14"),
            "-1000.0 <= -3.14"
        );
        assert!(
            !stats_value_less_or_equal("-3.14", "-1000.0"),
            "-3.14 is not <= -1000.0"
        );
    }

    /// Decimal strings where lex order differs from numeric: `"12.5"` < `"9.8"` lex
    /// but 12.5 > 9.8 numerically. The f64 parse path corrects this.
    #[test]
    fn decimal_string_lexicographic_order_differs_from_numeric() {
        assert!(
            stats_value_less_or_equal("9.8", "12.5"),
            "9.8 <= 12.5 (numeric)"
        );
        assert!(
            !stats_value_less_or_equal("12.5", "9.8"),
            "12.5 is not <= 9.8 (numeric)"
        );
    }

    /// Integer strings where multi-digit is larger: `"100"` < `"9"` lex but 100 > 9.
    #[test]
    fn multi_digit_string_lexicographic_vs_numeric() {
        assert!(stats_value_less_or_equal("9", "100"), "9 <= 100 (numeric)");
        assert!(
            !stats_value_less_or_equal("100", "9"),
            "100 is not <= 9 (numeric)"
        );
    }

    /// Boolean ordering: `false` < `true`.
    #[test]
    fn boolean_false_less_than_true() {
        assert!(stats_value_less_or_equal("false", "true"), "false <= true");
        assert!(
            !stats_value_less_or_equal("true", "false"),
            "true is not <= false"
        );
        assert!(
            stats_value_less_or_equal("false", "false"),
            "false <= false (equal)"
        );
        assert!(
            stats_value_less_or_equal("true", "true"),
            "true <= true (equal)"
        );
    }

    /// ISO-8601 date strings sort correctly lexicographically.
    #[test]
    fn iso_date_lexicographic_order_is_correct() {
        assert!(
            stats_value_less_or_equal("2024-01-01", "2024-12-31"),
            "earlier date <= later date"
        );
        assert!(
            !stats_value_less_or_equal("2024-12-31", "2024-01-01"),
            "later is not <= earlier"
        );
        assert!(
            stats_value_less_or_equal("2023-06-15", "2024-01-01"),
            "2023 date <= 2024 date"
        );
    }

    /// ISO-8601 timestamp strings sort correctly lexicographically.
    #[test]
    fn iso_timestamp_lexicographic_order_is_correct() {
        assert!(
            stats_value_less_or_equal("2024-01-01T00:00:00", "2024-12-31T23:59:59"),
            "earlier timestamp <= later"
        );
        assert!(
            !stats_value_less_or_equal("2024-12-31T23:59:59", "2024-01-01T00:00:00"),
            "later is not <= earlier"
        );
    }

    /// Regression: original motivating cases from the v0.27.5 implementation.
    #[test]
    fn existing_numeric_comparisons_still_correct() {
        assert!(
            stats_value_less_or_equal("2", "10"),
            "2 <= 10 (multi-digit)"
        );
        assert!(
            !stats_value_less_or_equal("10", "2"),
            "10 not <= 2 (multi-digit)"
        );
        assert!(
            stats_value_less_or_equal("-10", "-2"),
            "-10 <= -2 (negative)"
        );
        assert!(
            !stats_value_less_or_equal("-2", "-10"),
            "-2 not <= -10 (negative)"
        );
        assert!(
            stats_value_less_or_equal("-3.14", "100.5"),
            "-3.14 <= 100.5 (float)"
        );
    }
}
