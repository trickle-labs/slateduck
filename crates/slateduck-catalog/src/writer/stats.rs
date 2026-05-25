//! Statistics writer methods: file column stats, file variant stats, table stats.

use slateduck_core::keys;
use slateduck_core::rows::{FileColumnStatsRow, FileVariantStatsRow, TableStatsRow};
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
    pub has_null: bool,
    pub min_value: Option<&'a str>,
    pub max_value: Option<&'a str>,
    pub contains_nan: bool,
}

/// Input parameters for `upsert_file_variant_stats`.
///
/// Introduced in v0.21 to replace the 7-argument positional API and bring the
/// call below the `clippy::too_many_arguments` threshold.
#[derive(Debug, Clone)]
pub struct FileVariantStatsInput<'a> {
    pub table_id: u64,
    pub column_id: u64,
    pub variant_path: &'a str,
    pub data_file_id: u64,
    pub min_value: Option<&'a str>,
    pub max_value: Option<&'a str>,
}

impl CatalogWriter {
    pub async fn update_table_stats(
        &mut self,
        table_id: u64,
        row_count: u64,
        file_count: u64,
        total_size_bytes: u64,
    ) -> CatalogResult<()> {
        let row = TableStatsRow {
            table_id,
            row_count,
            file_count,
            total_size_bytes,
        };
        let key = keys::key_table_stats(table_id);
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
            has_null: input.has_null,
            min_value: input.min_value.map(|s| s.to_string()),
            max_value: input.max_value.map(|s| s.to_string()),
            contains_nan: input.contains_nan,
        };
        let key = keys::key_file_column_stats(input.table_id, input.column_id, input.data_file_id);
        self.db.put(&key, values::encode_value(&row)).await?;
        Ok(())
    }

    pub async fn upsert_file_variant_stats(
        &mut self,
        input: FileVariantStatsInput<'_>,
    ) -> CatalogResult<()> {
        let variant_path_hash = hash_tag_key(input.variant_path);
        let row = FileVariantStatsRow {
            table_id: input.table_id,
            column_id: input.column_id,
            variant_path_hash,
            data_file_id: input.data_file_id,
            variant_path: input.variant_path.to_string(),
            min_value: input.min_value.map(|s| s.to_string()),
            max_value: input.max_value.map(|s| s.to_string()),
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
