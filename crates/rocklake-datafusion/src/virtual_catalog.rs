//! Virtual DuckLake catalog for DataFusion.
//!
//! Registers all 32 DuckLake catalog tables as empty, schema-only
//! `MemTable`-backed providers in an in-memory DataFusion `SessionContext`.
//! This allows complex SQL (CTEs, joins, projections) against catalog tables to
//! be logically planned and validated by DataFusion before execution, without
//! requiring real data.
//!
//! The schemas defined here align exactly with the DuckLake v1.0 specification
//! (Catalog Version 7). DuckLake v1.1 (Catalog Version 8) is out of scope.
//!
//! # Usage
//! ```rust,ignore
//! use rocklake_datafusion::virtual_catalog::VirtualCatalogContext;
//! let ctx = VirtualCatalogContext::new().await?;
//! let df = ctx.session_context().sql("SELECT * FROM ducklake_table").await?;
//! ```

use std::sync::Arc;

use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::common::DataFusionError;
use datafusion::datasource::MemTable;
use datafusion::prelude::SessionContext;

/// An in-memory DataFusion `SessionContext` pre-populated with empty
/// schema-only tables for all 32 DuckLake catalog tables.
///
/// Mitigation 1 (v0.27.11): allows PgWire executor to fall back to DataFusion
/// for complex SELECT queries (CTEs, joins, aggregations) against catalog
/// tables that the bounded SQL dispatcher cannot handle natively.
pub struct VirtualCatalogContext {
    ctx: SessionContext,
}

impl VirtualCatalogContext {
    /// Build a new context and register all 32 catalog table schemas.
    pub async fn new() -> Result<Self, DataFusionError> {
        let ctx = SessionContext::new();
        register_all_catalog_tables(&ctx)?;
        Ok(Self { ctx })
    }

    /// Access the underlying `SessionContext` for SQL execution.
    pub fn session_context(&self) -> &SessionContext {
        &self.ctx
    }
}

/// Register all 32 DuckLake v1.0 catalog tables as empty memory-backed tables.
fn register_all_catalog_tables(ctx: &SessionContext) -> Result<(), DataFusionError> {
    let tables: &[(&str, Vec<Field>)] = &[
        (
            "ducklake_snapshot",
            vec![
                Field::new("snapshot_id", DataType::Int64, false),
                Field::new("snapshot_time", DataType::Utf8, true),
                Field::new("schema_version", DataType::Int64, false),
                Field::new("next_catalog_id", DataType::Int64, false),
            ],
        ),
        (
            "ducklake_snapshot_changes",
            vec![
                Field::new("snapshot_id", DataType::Int64, false),
                Field::new("change_type", DataType::Utf8, false),
                Field::new("catalog_id", DataType::Int64, true),
            ],
        ),
        (
            "ducklake_schema",
            vec![
                Field::new("schema_id", DataType::Int64, false),
                Field::new("begin_snapshot", DataType::Int64, false),
                Field::new("end_snapshot", DataType::Int64, true),
                Field::new("schema_name", DataType::Utf8, false),
                Field::new("schema_uuid", DataType::Utf8, true),
            ],
        ),
        (
            "ducklake_table",
            vec![
                Field::new("table_id", DataType::Int64, false),
                Field::new("begin_snapshot", DataType::Int64, false),
                Field::new("end_snapshot", DataType::Int64, true),
                Field::new("schema_id", DataType::Int64, false),
                Field::new("table_name", DataType::Utf8, false),
                Field::new("table_uuid", DataType::Utf8, true),
            ],
        ),
        (
            "ducklake_column",
            vec![
                Field::new("column_id", DataType::Int64, false),
                Field::new("begin_snapshot", DataType::Int64, false),
                Field::new("end_snapshot", DataType::Int64, true),
                Field::new("table_id", DataType::Int64, false),
                Field::new("column_index", DataType::Int32, false),
                Field::new("column_name", DataType::Utf8, false),
                Field::new("column_type", DataType::Utf8, false),
                Field::new("initial_default", DataType::Utf8, true),
                Field::new("write_default", DataType::Utf8, true),
            ],
        ),
        (
            "ducklake_data_file",
            vec![
                Field::new("data_file_id", DataType::Int64, false),
                Field::new("begin_snapshot", DataType::Int64, false),
                Field::new("end_snapshot", DataType::Int64, true),
                Field::new("table_id", DataType::Int64, false),
                Field::new("path", DataType::Utf8, false),
                Field::new("path_is_relative", DataType::Boolean, false),
                Field::new("row_count", DataType::Int64, false),
                Field::new("file_size_bytes", DataType::Int64, false),
                Field::new("footer_size", DataType::Int64, true),
            ],
        ),
        (
            "ducklake_delete_file",
            vec![
                Field::new("delete_file_id", DataType::Int64, false),
                Field::new("begin_snapshot", DataType::Int64, false),
                Field::new("end_snapshot", DataType::Int64, true),
                Field::new("data_file_id", DataType::Int64, false),
                Field::new("path", DataType::Utf8, false),
                Field::new("path_is_relative", DataType::Boolean, false),
                Field::new("row_count", DataType::Int64, false),
                Field::new("file_size_bytes", DataType::Int64, false),
            ],
        ),
        (
            "ducklake_table_stats",
            vec![
                Field::new("table_id", DataType::Int64, false),
                Field::new("record_count", DataType::Int64, false),
                Field::new("next_row_id", DataType::Int64, false),
                Field::new("file_size_bytes", DataType::Int64, false),
            ],
        ),
        (
            "ducklake_table_column_stats",
            vec![
                Field::new("table_id", DataType::Int64, false),
                Field::new("column_id", DataType::Int64, false),
                Field::new("null_count", DataType::Int64, true),
                Field::new("lower_bound", DataType::Utf8, true),
                Field::new("upper_bound", DataType::Utf8, true),
            ],
        ),
        (
            "ducklake_file_column_stats",
            vec![
                Field::new("data_file_id", DataType::Int64, false),
                Field::new("column_id", DataType::Int64, false),
                Field::new("null_count", DataType::Int64, true),
                Field::new("lower_bound", DataType::Utf8, true),
                Field::new("upper_bound", DataType::Utf8, true),
            ],
        ),
        (
            "ducklake_metadata",
            vec![
                Field::new("key", DataType::Utf8, false),
                Field::new("value", DataType::Utf8, true),
            ],
        ),
        (
            "ducklake_view",
            vec![
                Field::new("view_id", DataType::Int64, false),
                Field::new("begin_snapshot", DataType::Int64, false),
                Field::new("end_snapshot", DataType::Int64, true),
                Field::new("schema_id", DataType::Int64, false),
                Field::new("view_name", DataType::Utf8, false),
                Field::new("sql", DataType::Utf8, false),
                Field::new("column_aliases", DataType::Utf8, true),
                Field::new("dialect", DataType::Utf8, true),
            ],
        ),
        (
            "ducklake_macro",
            vec![
                Field::new("macro_id", DataType::Int64, false),
                Field::new("begin_snapshot", DataType::Int64, false),
                Field::new("end_snapshot", DataType::Int64, true),
                Field::new("schema_id", DataType::Int64, false),
                Field::new("macro_name", DataType::Utf8, false),
            ],
        ),
        (
            "ducklake_macro_impl",
            vec![
                Field::new("macro_id", DataType::Int64, false),
                Field::new("macro_impl_id", DataType::Int64, false),
                Field::new("return_type", DataType::Utf8, false),
                Field::new("macro_body", DataType::Utf8, false),
            ],
        ),
        (
            "ducklake_macro_parameters",
            vec![
                Field::new("macro_id", DataType::Int64, false),
                Field::new("macro_impl_id", DataType::Int64, false),
                Field::new("parameter_index", DataType::Int32, false),
                Field::new("parameter_name", DataType::Utf8, false),
                Field::new("parameter_type", DataType::Utf8, false),
            ],
        ),
        (
            "ducklake_tag",
            vec![
                Field::new("begin_snapshot", DataType::Int64, false),
                Field::new("end_snapshot", DataType::Int64, true),
                Field::new("object_id", DataType::Int64, false),
                Field::new("key", DataType::Utf8, false),
                Field::new("value", DataType::Utf8, true),
            ],
        ),
        (
            "ducklake_column_tag",
            vec![
                Field::new("begin_snapshot", DataType::Int64, false),
                Field::new("end_snapshot", DataType::Int64, true),
                Field::new("column_id", DataType::Int64, false),
                Field::new("key", DataType::Utf8, false),
                Field::new("value", DataType::Utf8, true),
            ],
        ),
        (
            "ducklake_partition_info",
            vec![
                Field::new("partition_id", DataType::Int64, false),
                Field::new("begin_snapshot", DataType::Int64, false),
                Field::new("end_snapshot", DataType::Int64, true),
                Field::new("table_id", DataType::Int64, false),
                Field::new("partition_type", DataType::Utf8, false),
            ],
        ),
        (
            "ducklake_partition_column",
            vec![
                Field::new("partition_id", DataType::Int64, false),
                Field::new("partition_key_index", DataType::Int32, false),
                Field::new("column_id", DataType::Int64, false),
                Field::new("transform", DataType::Utf8, false),
            ],
        ),
        (
            "ducklake_partition_value",
            vec![
                Field::new("data_file_id", DataType::Int64, false),
                Field::new("partition_key_index", DataType::Int32, false),
                Field::new("value", DataType::Utf8, true),
            ],
        ),
        (
            "ducklake_sort_info",
            vec![
                Field::new("sort_id", DataType::Int64, false),
                Field::new("begin_snapshot", DataType::Int64, false),
                Field::new("end_snapshot", DataType::Int64, true),
                Field::new("table_id", DataType::Int64, false),
            ],
        ),
        (
            "ducklake_sort_expression",
            vec![
                Field::new("sort_id", DataType::Int64, false),
                Field::new("sort_key_index", DataType::Int32, false),
                Field::new("column_id", DataType::Int64, false),
                Field::new("sort_order", DataType::Utf8, false),
                Field::new("null_order", DataType::Utf8, false),
            ],
        ),
        (
            "ducklake_files_scheduled_for_deletion",
            vec![
                Field::new("path", DataType::Utf8, false),
                Field::new("path_is_relative", DataType::Boolean, false),
            ],
        ),
        (
            "ducklake_inlined_data_tables",
            vec![
                Field::new("table_id", DataType::Int64, false),
                Field::new("table_name", DataType::Utf8, false),
                Field::new("schema_version", DataType::Int64, false),
            ],
        ),
        (
            "ducklake_schema_version",
            vec![
                Field::new("schema_version", DataType::Int64, false),
                Field::new("schema_version_info", DataType::Utf8, true),
            ],
        ),
        (
            "ducklake_schema_changes",
            vec![
                Field::new("changes_id", DataType::Int64, false),
                Field::new("snapshot_id", DataType::Int64, false),
                Field::new("change_type", DataType::Utf8, false),
                Field::new("catalog_id", DataType::Int64, true),
            ],
        ),
        (
            "ducklake_encrypted_secret",
            vec![
                Field::new("secret_id", DataType::Int64, false),
                Field::new("secret_name", DataType::Utf8, false),
                Field::new("encrypted_secret", DataType::Utf8, false),
            ],
        ),
        (
            "ducklake_encryption_key",
            vec![
                Field::new("catalog_id", DataType::Int64, false),
                Field::new("begin_snapshot", DataType::Int64, false),
                Field::new("end_snapshot", DataType::Int64, true),
                Field::new("encryption_type", DataType::Utf8, false),
                Field::new("key_id", DataType::Utf8, true),
                Field::new("encryption_key", DataType::Utf8, true),
            ],
        ),
        (
            "ducklake_file_partition_value",
            vec![
                Field::new("data_file_id", DataType::Int64, false),
                Field::new("table_id", DataType::Int64, false),
                Field::new("partition_key_index", DataType::Int32, false),
                Field::new("partition_value", DataType::Utf8, true),
            ],
        ),
        // --- v0.27.11 extension tables ---
        (
            "ducklake_file_variant_stats",
            vec![
                Field::new("data_file_id", DataType::Int64, false),
                Field::new("variant_column_id", DataType::Int64, false),
                Field::new("variant_stats", DataType::Utf8, true),
            ],
        ),
        (
            "ducklake_column_mapping",
            vec![
                Field::new("column_id", DataType::Int64, false),
                Field::new("begin_snapshot", DataType::Int64, false),
                Field::new("end_snapshot", DataType::Int64, true),
                Field::new("mapping_type", DataType::Utf8, false),
                Field::new("mapping_key", DataType::Utf8, false),
            ],
        ),
        (
            "ducklake_name_mapping",
            vec![
                Field::new("table_id", DataType::Int64, false),
                Field::new("begin_snapshot", DataType::Int64, false),
                Field::new("end_snapshot", DataType::Int64, true),
                Field::new("name_mapping", DataType::Utf8, false),
            ],
        ),
    ];

    for (table_name, fields) in tables {
        let schema = Arc::new(Schema::new(fields.clone()));
        let empty_batch = RecordBatch::new_empty(schema.clone());
        let mem_table = MemTable::try_new(schema, vec![vec![empty_batch]])?;
        ctx.register_table(*table_name, Arc::new(mem_table))?;
    }

    Ok(())
}

/// Return the list of all registered DuckLake v1.0 catalog table names.
/// Useful for introspection and test assertions.
pub fn catalog_table_names() -> &'static [&'static str] {
    &[
        "ducklake_snapshot",
        "ducklake_snapshot_changes",
        "ducklake_schema",
        "ducklake_table",
        "ducklake_column",
        "ducklake_data_file",
        "ducklake_delete_file",
        "ducklake_table_stats",
        "ducklake_table_column_stats",
        "ducklake_file_column_stats",
        "ducklake_metadata",
        "ducklake_view",
        "ducklake_macro",
        "ducklake_macro_impl",
        "ducklake_macro_parameters",
        "ducklake_tag",
        "ducklake_column_tag",
        "ducklake_partition_info",
        "ducklake_partition_column",
        "ducklake_partition_value",
        "ducklake_sort_info",
        "ducklake_sort_expression",
        "ducklake_files_scheduled_for_deletion",
        "ducklake_inlined_data_tables",
        "ducklake_schema_version",
        "ducklake_schema_changes",
        "ducklake_encrypted_secret",
        "ducklake_encryption_key",
        "ducklake_file_partition_value",
        "ducklake_file_variant_stats",
        "ducklake_column_mapping",
        "ducklake_name_mapping",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn virtual_catalog_registers_all_32_tables() {
        let vc = VirtualCatalogContext::new()
            .await
            .expect("VirtualCatalogContext::new must not fail");
        let ctx = vc.session_context();

        // Verify all 32 tables are discoverable via DataFusion's catalog.
        let names = catalog_table_names();
        assert_eq!(names.len(), 32, "must have exactly 32 catalog tables");
        for name in names {
            let result = ctx.sql(&format!("SELECT * FROM {name}")).await;
            assert!(
                result.is_ok(),
                "SELECT * FROM {name} must plan successfully; error: {:?}",
                result.err()
            );
        }
    }

    #[tokio::test]
    async fn virtual_catalog_accepts_cte_query() {
        let vc = VirtualCatalogContext::new()
            .await
            .expect("VirtualCatalogContext::new must not fail");
        let ctx = vc.session_context();

        // A CTE joining snapshot and schema — tests complex SQL planning.
        let sql = r#"
            WITH latest AS (
                SELECT snapshot_id FROM ducklake_snapshot
            )
            SELECT t.table_name, t.table_id
            FROM ducklake_table t
            JOIN latest l ON l.snapshot_id >= t.begin_snapshot
        "#;
        let result = ctx.sql(sql).await;
        assert!(
            result.is_ok(),
            "CTE query over catalog tables must plan successfully; error: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn virtual_catalog_accepts_projection_over_renamed_columns() {
        let vc = VirtualCatalogContext::new()
            .await
            .expect("VirtualCatalogContext::new must not fail");
        let ctx = vc.session_context();

        // ducklake_metadata uses 'key'/'value' (v0.27.11 spec rename).
        let sql = "SELECT key, value FROM ducklake_metadata WHERE key = 'version'";
        let result = ctx.sql(sql).await;
        assert!(
            result.is_ok(),
            "ducklake_metadata key/value projection must plan; error: {:?}",
            result.err()
        );
    }
}
