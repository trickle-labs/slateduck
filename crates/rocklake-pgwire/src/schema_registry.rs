//! DuckLake v1.0 table schema registry.
//!
//! This module is the **single authoritative source** for the PgWire
//! `FieldInfo` definitions of every DuckLake metadata table.  All executor
//! response builders, handler `describe_fields_for_sql`, and COPY metadata
//! responses must derive their schemas from the functions in this module.
//!
//! # Design
//!
//! Each public function returns a fresh `Arc<Vec<FieldInfo>>` whose column
//! names, types, and wire formats exactly match the DuckLake v1.0 spec.
//! Callers clone the `Arc` to share the immutable schema between the
//! `QueryResponse` header and each `DataRowEncoder`.

use std::sync::Arc;

use pgwire::api::results::{FieldFormat, FieldInfo};
use pgwire::api::Type;

// ── helper macros ─────────────────────────────────────────────────────────────

macro_rules! text_col {
    ($name:expr) => {
        FieldInfo::new($name.to_string(), None, None, Type::TEXT, FieldFormat::Text)
    };
}

macro_rules! int8t {
    ($name:expr) => {
        FieldInfo::new($name.to_string(), None, None, Type::INT8, FieldFormat::Text)
    };
}

macro_rules! int8b {
    ($name:expr) => {
        FieldInfo::new(
            $name.to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Binary,
        )
    };
}

macro_rules! bool_col {
    ($name:expr) => {
        FieldInfo::new($name.to_string(), None, None, Type::BOOL, FieldFormat::Text)
    };
}

macro_rules! uuid_col {
    ($name:expr) => {
        FieldInfo::new($name.to_string(), None, None, Type::UUID, FieldFormat::Text)
    };
}

macro_rules! tstz_col {
    ($name:expr) => {
        FieldInfo::new(
            $name.to_string(),
            None,
            None,
            Type::TIMESTAMPTZ,
            FieldFormat::Text,
        )
    };
}

// ── ducklake_snapshot ─────────────────────────────────────────────────────────

/// `ducklake_snapshot(snapshot_id, snapshot_time, schema_version,
/// next_catalog_id, next_file_id)` — DuckLake v1.0 spec.
pub fn snapshot_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("snapshot_id"),
        tstz_col!("snapshot_time"),
        int8t!("schema_version"),
        int8t!("next_catalog_id"),
        int8t!("next_file_id"),
    ])
}

// ── ducklake_snapshot_changes ────────────────────────────────────────────────

/// `ducklake_snapshot_changes(snapshot_id, changes_made, author,
/// commit_message, commit_extra_info)` — DuckLake v1.0 spec.
pub fn snapshot_changes_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("snapshot_id"),
        text_col!("changes_made"),
        text_col!("author"),
        text_col!("commit_message"),
        text_col!("commit_extra_info"),
    ])
}

// ── ducklake_schema ───────────────────────────────────────────────────────────

/// `ducklake_schema(schema_id, begin_snapshot, end_snapshot, schema_uuid,
/// schema_name, path, path_is_relative)` — DuckLake v1.0 spec.
pub fn schema_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8b!("schema_id"),
        int8b!("begin_snapshot"),
        int8b!("end_snapshot"),
        uuid_col!("schema_uuid"),
        text_col!("schema_name"),
        text_col!("path"),
        bool_col!("path_is_relative"),
    ])
}

// ── ducklake_table ────────────────────────────────────────────────────────────

/// `ducklake_table(table_id, begin_snapshot, end_snapshot, schema_id,
/// table_name, table_uuid, path, path_is_relative)` — DuckLake v1.0 spec.
pub fn table_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8b!("table_id"),
        int8b!("begin_snapshot"),
        int8b!("end_snapshot"),
        int8b!("schema_id"),
        text_col!("table_name"),
        uuid_col!("table_uuid"),
        text_col!("path"),
        bool_col!("path_is_relative"),
    ])
}

// ── ducklake_column ───────────────────────────────────────────────────────────

/// `ducklake_column(column_id, begin_snapshot, end_snapshot, table_id,
/// column_order, column_name, column_type, initial_default, default_value,
/// nulls_allowed, parent_column, default_value_type, default_value_dialect)`
/// — DuckLake v1.0 spec.
pub fn column_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("column_id"),
        int8t!("begin_snapshot"),
        int8t!("end_snapshot"),
        int8t!("table_id"),
        int8t!("column_order"),
        text_col!("column_name"),
        text_col!("column_type"),
        text_col!("initial_default"),
        text_col!("default_value"),
        bool_col!("nulls_allowed"),
        int8t!("parent_column"),
        text_col!("default_value_type"),
        text_col!("default_value_dialect"),
    ])
}

// ── ducklake_data_file ────────────────────────────────────────────────────────

/// `ducklake_data_file(data_file_id, table_id, begin_snapshot, end_snapshot,
/// file_order, path, path_is_relative, file_format, record_count,
/// file_size_bytes, row_id_start)` — DuckLake v1.0 spec.
pub fn data_file_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("data_file_id"),
        int8t!("table_id"),
        int8t!("begin_snapshot"),
        int8t!("end_snapshot"),
        int8t!("file_order"),
        text_col!("path"),
        bool_col!("path_is_relative"),
        text_col!("file_format"),
        int8t!("record_count"),
        int8t!("file_size_bytes"),
        int8t!("row_id_start"),
    ])
}

// ── ducklake_delete_file ──────────────────────────────────────────────────────

/// `ducklake_delete_file` — DuckLake v1.0 spec presentation columns.
/// Note: `delete_file_id` is a synthesized surrogate; `path` is the file path.
pub fn delete_file_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("delete_file_id"),
        int8t!("table_id"),
        text_col!("path"),
        int8t!("delete_count"),
        int8t!("file_size_bytes"),
        int8t!("begin_snapshot"),
        int8t!("end_snapshot"),
    ])
}

// ── ducklake_table_stats ──────────────────────────────────────────────────────

/// `ducklake_table_stats(table_id, record_count, next_row_id, file_size_bytes)`
/// — DuckLake v1.0 spec.
pub fn table_stats_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("table_id"),
        int8t!("record_count"),
        int8t!("next_row_id"),
        int8t!("file_size_bytes"),
    ])
}

// ── ducklake_table_column_stats ───────────────────────────────────────────────

/// `ducklake_table_column_stats(table_id, column_id, contains_null,
/// contains_nan, min_value, max_value, extra_stats)` — DuckLake v1.0 spec.
pub fn table_column_stats_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("table_id"),
        int8t!("column_id"),
        bool_col!("contains_null"),
        bool_col!("contains_nan"),
        text_col!("min_value"),
        text_col!("max_value"),
        text_col!("extra_stats"),
    ])
}

// ── ducklake_file_column_stats ────────────────────────────────────────────────

/// `ducklake_file_column_stats(data_file_id, table_id, column_id,
/// column_size_bytes, value_count, null_count, min_value, max_value,
/// contains_nan, extra_stats)` — DuckLake v1.0 spec.
pub fn file_column_stats_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("data_file_id"),
        int8t!("table_id"),
        int8t!("column_id"),
        int8t!("column_size_bytes"),
        int8t!("value_count"),
        int8t!("null_count"),
        text_col!("min_value"),
        text_col!("max_value"),
        bool_col!("contains_nan"),
        text_col!("extra_stats"),
    ])
}

// ── ducklake_metadata ─────────────────────────────────────────────────────────

/// `ducklake_metadata(key, value, scope, scope_id)` — DuckLake v1.0 spec
/// (Catalog Version 7).  `key` and `value` are the canonical spec column names;
/// earlier RockLake releases used `metadata_key` / `metadata_value`.
pub fn metadata_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        text_col!("key"),
        text_col!("value"),
        text_col!("scope"),
        int8t!("scope_id"),
    ])
}

// ── ducklake_view ─────────────────────────────────────────────────────────────

/// `ducklake_view(view_id, begin_snapshot, end_snapshot, schema_id, view_name,
/// view_uuid, sql, dialect, column_aliases)` — DuckLake v1.0 spec
/// (Catalog Version 7).  `sql` is the canonical spec column name;
/// earlier RockLake releases used `view_definition`.
pub fn view_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("view_id"),
        int8t!("begin_snapshot"),
        int8t!("end_snapshot"),
        int8t!("schema_id"),
        text_col!("view_name"),
        uuid_col!("view_uuid"),
        text_col!("sql"),
        text_col!("dialect"),
        text_col!("column_aliases"),
    ])
}

// ── ducklake_macro ────────────────────────────────────────────────────────────

/// `ducklake_macro(macro_id, begin_snapshot, end_snapshot, schema_id,
/// macro_name, macro_uuid)` — DuckLake v1.0 spec.
pub fn macro_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("macro_id"),
        int8t!("begin_snapshot"),
        int8t!("end_snapshot"),
        int8t!("schema_id"),
        text_col!("macro_name"),
        text_col!("macro_uuid"),
    ])
}

// ── ducklake_macro_impl ───────────────────────────────────────────────────────

/// `ducklake_macro_impl(macro_id, impl_id, dialect, sql, type)` — DuckLake
/// v1.0 spec.
pub fn macro_impl_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("macro_id"),
        int8t!("impl_id"),
        text_col!("dialect"),
        text_col!("sql"),
        text_col!("type"),
    ])
}

// ── ducklake_macro_parameters ─────────────────────────────────────────────────

/// `ducklake_macro_parameters(macro_id, impl_id, column_id, parameter_name,
/// parameter_type, default_value, default_value_type)` — DuckLake v1.0 spec.
pub fn macro_parameters_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("macro_id"),
        int8t!("impl_id"),
        int8t!("column_id"),
        text_col!("parameter_name"),
        text_col!("parameter_type"),
        text_col!("default_value"),
        text_col!("default_value_type"),
    ])
}

// ── ducklake_tag ──────────────────────────────────────────────────────────────

/// `ducklake_tag(begin_snapshot, end_snapshot, object_id, key, value)` —
/// DuckLake v1.0 spec (Catalog Version 7).  The synthesized `tag_id` surrogate
/// has been removed per spec alignment; `key` and `value` are the canonical
/// spec column names for the tag key and tag value respectively.
pub fn tag_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("begin_snapshot"),
        int8t!("end_snapshot"),
        int8t!("object_id"),
        text_col!("key"),
        text_col!("value"),
    ])
}

// ── ducklake_column_tag ───────────────────────────────────────────────────────

/// `ducklake_column_tag(begin_snapshot, end_snapshot, column_id, key, value)` —
/// DuckLake v1.0 spec (Catalog Version 7).  The synthesized `tag_id` surrogate
/// has been removed per spec alignment; `key` and `value` are the canonical
/// spec column names.
pub fn column_tag_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("begin_snapshot"),
        int8t!("end_snapshot"),
        int8t!("column_id"),
        text_col!("key"),
        text_col!("value"),
    ])
}

// ── ducklake_partition_info ───────────────────────────────────────────────────

/// `ducklake_partition_info(partition_id, table_id, begin_snapshot,
/// end_snapshot)` — DuckLake v1.0 spec.
pub fn partition_info_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("partition_id"),
        int8t!("table_id"),
        int8t!("begin_snapshot"),
        int8t!("end_snapshot"),
    ])
}

// ── ducklake_partition_column ─────────────────────────────────────────────────

/// `ducklake_partition_column(partition_id, partition_index, column_id,
/// transform, transform_param)` — DuckLake v1.0 spec.
pub fn partition_column_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("partition_id"),
        int8t!("partition_index"),
        int8t!("column_id"),
        text_col!("transform"),
        text_col!("transform_param"),
    ])
}

// ── ducklake_partition_value ──────────────────────────────────────────────────

/// `ducklake_partition_value(data_file_id, partition_index, partition_value)` —
/// DuckLake v1.0 spec.
pub fn partition_value_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("data_file_id"),
        int8t!("partition_index"),
        text_col!("partition_value"),
    ])
}

// ── ducklake_sort_info ────────────────────────────────────────────────────────

/// `ducklake_sort_info(sort_id, table_id, begin_snapshot, end_snapshot)` —
/// DuckLake v1.0 spec.
pub fn sort_info_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("sort_id"),
        int8t!("table_id"),
        int8t!("begin_snapshot"),
        int8t!("end_snapshot"),
    ])
}

// ── ducklake_sort_expression ──────────────────────────────────────────────────

/// `ducklake_sort_expression(sort_id, sort_index, column_id, sort_order,
/// null_order)` — DuckLake v1.0 spec.
pub fn sort_expression_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("sort_id"),
        int8t!("sort_index"),
        int8t!("column_id"),
        text_col!("sort_order"),
        text_col!("null_order"),
    ])
}

// ── ducklake_files_scheduled_for_deletion ────────────────────────────────────

/// `ducklake_files_scheduled_for_deletion(path, path_is_relative,
/// deletion_scheduled_at)` — DuckLake v1.0 spec.
pub fn files_scheduled_for_deletion_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        text_col!("path"),
        bool_col!("path_is_relative"),
        tstz_col!("deletion_scheduled_at"),
    ])
}

// ── ducklake_inlined_data_tables ──────────────────────────────────────────────

/// `ducklake_inlined_data_tables(table_id, table_name, schema_version)` —
/// DuckLake v1.0 spec.
pub fn inlined_data_tables_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("table_id"),
        text_col!("table_name"),
        int8t!("schema_version"),
    ])
}

// ── ducklake_schema_version ───────────────────────────────────────────────────

/// `ducklake_schema_version(schema_version, schema_version_info)` —
/// DuckLake v1.0 spec.
pub fn schema_version_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("schema_version"),
        text_col!("schema_version_info"),
    ])
}

// ── ducklake_schema_changes ───────────────────────────────────────────────────

/// `ducklake_schema_changes(changes_id, snapshot_id, table_id, schema_id,
/// change_type, change_info)` — DuckLake v1.0 spec.
pub fn schema_changes_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("changes_id"),
        int8t!("snapshot_id"),
        int8t!("table_id"),
        int8t!("schema_id"),
        text_col!("change_type"),
        text_col!("change_info"),
    ])
}

// ── ducklake_encrypted_secret ─────────────────────────────────────────────────

/// `ducklake_encrypted_secret(secret_id, begin_snapshot, end_snapshot,
/// secret_name, secret_type, encrypted_secret)` — DuckLake v1.0 spec.
pub fn encrypted_secret_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("secret_id"),
        int8t!("begin_snapshot"),
        int8t!("end_snapshot"),
        text_col!("secret_name"),
        text_col!("secret_type"),
        text_col!("encrypted_secret"),
    ])
}

// ── ducklake_encryption_key ───────────────────────────────────────────────────

/// `ducklake_encryption_key(catalog_id, begin_snapshot, end_snapshot,
/// encryption_type, key_id, encryption_key)` — DuckLake v1.0 spec.
pub fn encryption_key_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("catalog_id"),
        int8t!("begin_snapshot"),
        int8t!("end_snapshot"),
        text_col!("encryption_type"),
        text_col!("key_id"),
        text_col!("encryption_key"),
    ])
}

// ── ducklake_file_partition_value ─────────────────────────────────────────────

/// `ducklake_file_partition_value(data_file_id, table_id, partition_key_index,
/// partition_value)` — DuckLake v1.0 spec.
pub fn file_partition_value_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("data_file_id"),
        int8t!("table_id"),
        int8t!("partition_key_index"),
        text_col!("partition_value"),
    ])
}

// ── Global stats (combined table_stats + column_stats) ────────────────────────

/// Combined schema for `SELECT ... FROM ducklake_table_stats INNER JOIN ...`
/// or the global stats combined query shape used by DuckLake.
pub fn global_table_stats_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("table_id"),
        int8t!("column_id"),
        int8t!("record_count"),
        int8t!("next_row_id"),
        int8t!("file_size_bytes"),
        bool_col!("contains_null"),
        bool_col!("contains_nan"),
        text_col!("min_value"),
        text_col!("max_value"),
        text_col!("extra_stats"),
    ])
}

// ── Utility: latest snapshot info ────────────────────────────────────────────

/// Schema for the 4-column `SelectLatestSnapshotInfo` response.
pub fn latest_snapshot_info_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8b!("snapshot_id"),
        int8b!("schema_version"),
        int8b!("next_catalog_id"),
        int8b!("next_file_id"),
    ])
}

// ── ducklake_file_variant_stats ───────────────────────────────────────────────

/// `ducklake_file_variant_stats(data_file_id, column_id, value_count,
/// null_count, bloom_filter_offset, bloom_filter_length)` —
/// DuckLake v1.0 spec (Catalog Version 7): per-file variant-type statistics.
pub fn file_variant_stats_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("data_file_id"),
        int8t!("column_id"),
        int8t!("value_count"),
        int8t!("null_count"),
        int8t!("bloom_filter_offset"),
        int8t!("bloom_filter_length"),
    ])
}

// ── ducklake_column_mapping ───────────────────────────────────────────────────

/// `ducklake_column_mapping(table_id, column_id, field_id, mapping_type)` —
/// DuckLake v1.0 spec (Catalog Version 7): maps logical column IDs to physical
/// field IDs for Iceberg-compatible column evolution.
pub fn column_mapping_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("table_id"),
        int8t!("column_id"),
        int8t!("field_id"),
        text_col!("mapping_type"),
    ])
}

// ── ducklake_name_mapping ─────────────────────────────────────────────────────

/// `ducklake_name_mapping(table_id, field_name, field_id, column_id)` —
/// DuckLake v1.0 spec (Catalog Version 7): maps physical field names to logical
/// column IDs; required for by-name column evolution in Iceberg-format catalogs.
pub fn name_mapping_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        int8t!("table_id"),
        text_col!("field_name"),
        int8t!("field_id"),
        int8t!("column_id"),
    ])
}

// ── Registry lookup by table name ────────────────────────────────────────────

/// Look up the canonical `FieldInfo` list for a named DuckLake metadata table.
///
/// Returns `Some(schema)` for all 32 DuckLake v1.0 tables (28 core tables +
/// 3 extension tables added in v0.27.11 + 1 extra); `None` for unknown names.
pub fn fields_for_table(table_name: &str) -> Option<Arc<Vec<FieldInfo>>> {
    match table_name {
        "ducklake_snapshot" => Some(snapshot_schema()),
        "ducklake_snapshot_changes" => Some(snapshot_changes_schema()),
        "ducklake_schema" => Some(schema_schema()),
        "ducklake_table" => Some(table_schema()),
        "ducklake_column" => Some(column_schema()),
        "ducklake_data_file" => Some(data_file_schema()),
        "ducklake_delete_file" => Some(delete_file_schema()),
        "ducklake_table_stats" => Some(table_stats_schema()),
        "ducklake_table_column_stats" => Some(table_column_stats_schema()),
        "ducklake_file_column_stats" => Some(file_column_stats_schema()),
        "ducklake_metadata" => Some(metadata_schema()),
        "ducklake_view" => Some(view_schema()),
        "ducklake_macro" => Some(macro_schema()),
        "ducklake_macro_impl" => Some(macro_impl_schema()),
        "ducklake_macro_parameters" => Some(macro_parameters_schema()),
        "ducklake_tag" => Some(tag_schema()),
        "ducklake_column_tag" => Some(column_tag_schema()),
        "ducklake_partition_info" => Some(partition_info_schema()),
        "ducklake_partition_column" => Some(partition_column_schema()),
        "ducklake_partition_value" => Some(partition_value_schema()),
        "ducklake_sort_info" => Some(sort_info_schema()),
        "ducklake_sort_expression" => Some(sort_expression_schema()),
        "ducklake_files_scheduled_for_deletion" => Some(files_scheduled_for_deletion_schema()),
        "ducklake_inlined_data_tables" => Some(inlined_data_tables_schema()),
        "ducklake_schema_version" => Some(schema_version_schema()),
        "ducklake_schema_changes" => Some(schema_changes_schema()),
        "ducklake_encrypted_secret" => Some(encrypted_secret_schema()),
        "ducklake_encryption_key" => Some(encryption_key_schema()),
        "ducklake_file_partition_value" => Some(file_partition_value_schema()),
        // Added in v0.27.11: three extension tables.
        "ducklake_file_variant_stats" => Some(file_variant_stats_schema()),
        "ducklake_column_mapping" => Some(column_mapping_schema()),
        "ducklake_name_mapping" => Some(name_mapping_schema()),
        _ => None,
    }
}
