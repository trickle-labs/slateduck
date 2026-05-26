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
    pub async fn update_table_stats(
        &mut self,
        table_id: u64,
        record_count: u64,
        file_count: u64,
        file_size_bytes: u64,
    ) -> CatalogResult<()> {
        // v0.24: read existing stats to accumulate next_row_id.
        let existing_next_row_id = {
            let key = keys::key_table_stats(table_id);
            match self.db.get(&key).await? {
                Some(data) => {
                    let existing: TableStatsRow = slateduck_core::values::decode_value(&data)
                        .unwrap_or(TableStatsRow {
                            table_id,
                            record_count: 0,
                            file_count: 0,
                            file_size_bytes: 0,
                            next_row_id: None,
                        });
                    existing.next_row_id.unwrap_or(0)
                }
                None => 0,
            }
        };
        let next_row_id = existing_next_row_id.saturating_add(record_count);
        let row = TableStatsRow {
            table_id,
            record_count,
            file_count,
            file_size_bytes,
            next_row_id: Some(next_row_id),
        };
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
        let existing_file_count = {
            let key = keys::key_table_stats(table_id);
            match self.db.get(&key).await? {
                Some(data) => {
                    let existing: TableStatsRow = slateduck_core::values::decode_value(&data)
                        .unwrap_or(TableStatsRow {
                            table_id,
                            record_count: 0,
                            file_count: 0,
                            file_size_bytes: 0,
                            next_row_id: None,
                        });
                    existing.file_count
                }
                None => 0,
            }
        };
        let row = TableStatsRow {
            table_id,
            record_count,
            file_count: existing_file_count,
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
        let row = TableColumnStatsRow {
            table_id,
            column_id,
            contains_null,
            min_value: min_value.map(|s| s.to_string()),
            max_value: max_value.map(|s| s.to_string()),
            contains_nan,
            extra_stats: extra_stats.map(|s| s.to_string()),
        };
        let key = keys::key_table_column_stats(table_id, column_id);
        self.db.put(&key, values::encode_value(&row)).await?;
        Ok(())
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
