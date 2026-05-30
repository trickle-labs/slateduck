//! Catalog export, import, rebuild, and migration.
//!
//! - `export`: NDJSON export of all live catalog rows at a snapshot.
//! - `import`: Initialize a fresh catalog from an NDJSON export.
//! - `pg_migrate`: Convert NDJSON to PostgreSQL INSERT statements.
//! - `rebuild`: Synthesize a fresh catalog from Parquet footers.

use rocklake_core::keys;
use rocklake_core::mvcc::{self, SnapshotId};
use rocklake_core::rows::*;
use rocklake_core::tags::*;
use rocklake_core::values;
use serde::{Deserialize, Serialize};
use slatedb::{Db, WriteBatch};
use std::io::{BufRead, Write};

use crate::error::{CatalogError, CatalogResult};

/// A single exported catalog row in NDJSON format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedRow {
    /// Table name (e.g., "ducklake_schema", "ducklake_table").
    pub table: String,
    /// The row data as JSON.
    pub data: serde_json::Value,
}

/// Result of an export operation.
///
/// Returned by [`export_catalog`] after a successful NDJSON export.
/// The counts are informational and do not affect catalog integrity.
#[derive(Debug, Clone)]
pub struct ExportResult {
    /// Number of individual catalog rows written to the output stream.
    pub rows_exported: u64,
    /// Number of distinct DuckLake table types exported (e.g.
    /// `ducklake_schema`, `ducklake_table`, `ducklake_column`, …).
    pub tables_exported: u64,
}

/// Result of an import operation.
///
/// Returned by [`import_catalog`] after a successful NDJSON import.
/// A partial import leaves the catalog in an inconsistent state; callers
/// should treat any error from `import_catalog` as unrecoverable and
/// discard the target database.
#[derive(Debug, Clone)]
pub struct ImportResult {
    /// Number of individual catalog rows inserted into the target database.
    pub rows_imported: u64,
    /// Number of distinct DuckLake table types imported.
    pub tables_imported: u64,
}

/// Export all live catalog rows at the given snapshot to NDJSON.
///
/// Each row is written as a JSON object on a single line (newline-delimited JSON).
/// Rows are serialised in table-group order: `ducklake_snapshot`, `ducklake_schema`,
/// `ducklake_table`, `ducklake_column`, `ducklake_data_file`, \u2026
///
/// # Atomicity
///
/// The export reads from the SlateDB key-value store using separate prefix scans.
/// It is **not** a snapshot-isolated read at the storage level — if a concurrent
/// writer commits between two table scans the export may contain rows from mixed
/// snapshot generations. For a consistent export, pause writes before calling
/// this function or take a catalog checkpoint first.
///
/// # Completeness
///
/// Only rows that are *visible* at `snapshot_id` (i.e. `begin_snapshot <=
/// snapshot_id < end_snapshot`) are included. Retired rows and rows created after
/// the snapshot are silently omitted. When `snapshot_id` is `None` the latest
/// committed snapshot is used.
///
/// # Errors
///
/// Returns `CatalogError::SlateDb` on any database read failure or JSON
/// serialisation error.  Partial output may have been written to `writer` before
/// the error is returned; callers should discard incomplete output.
pub async fn export_catalog<W: Write>(
    db: &Db,
    snapshot_id: Option<u64>,
    writer: &mut W,
) -> CatalogResult<ExportResult> {
    let dl_snapshot_id = match snapshot_id {
        Some(id) => SnapshotId::new(id),
        None => {
            let key = keys::key_counter(COUNTER_NEXT_SNAPSHOT_ID);
            let next = match db.get(&key).await? {
                Some(data) => values::decode_counter(&data)?,
                None => 1,
            };
            SnapshotId::new(if next > 0 { next - 1 } else { 0 })
        }
    };

    let mut rows_exported = 0u64;
    let mut tables_exported = 0u64;

    // Export snapshots
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_SNAPSHOT);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: SnapshotRow = values::decode_value(&kv.value)?;
        if row.snapshot_id <= dl_snapshot_id.as_u64() {
            let exported = ExportedRow {
                table: "ducklake_snapshot".to_string(),
                data: serde_json::json!({
                    "snapshot_id": row.snapshot_id,
                    "schema_version": row.schema_version,
                    "snapshot_time": row.snapshot_time,
                    "author": row.author,
                    "message": row.message,
                }),
            };
            serde_json::to_writer(&mut *writer, &exported)
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            rows_exported += 1;
        }
    }

    // Export schemas
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_SCHEMA);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: SchemaRow = values::decode_value(&kv.value)?;
        if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, dl_snapshot_id) {
            let exported = ExportedRow {
                table: "ducklake_schema".to_string(),
                data: serde_json::json!({
                    "schema_id": row.schema_id,
                    "schema_name": row.schema_name,
                    "begin_snapshot": row.begin_snapshot,
                    "end_snapshot": row.end_snapshot,
                }),
            };
            serde_json::to_writer(&mut *writer, &exported)
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            rows_exported += 1;
        }
    }

    // Export tables
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_TABLE);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: TableRow = values::decode_value(&kv.value)?;
        if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, dl_snapshot_id) {
            let exported = ExportedRow {
                table: "ducklake_table".to_string(),
                data: serde_json::json!({
                    "table_id": row.table_id,
                    "schema_id": row.schema_id,
                    "table_name": row.table_name,
                    "begin_snapshot": row.begin_snapshot,
                    "end_snapshot": row.end_snapshot,
                    "path": row.path,
                    "table_uuid": row.table_uuid,
                }),
            };
            serde_json::to_writer(&mut *writer, &exported)
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            rows_exported += 1;
        }
    }

    // Export columns
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_COLUMN);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: ColumnRow = values::decode_value(&kv.value)?;
        if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, dl_snapshot_id) {
            let exported = ExportedRow {
                table: "ducklake_column".to_string(),
                data: serde_json::json!({
                    "column_id": row.column_id,
                    "table_id": row.table_id,
                    "column_name": row.column_name,
                    "data_type": row.data_type,
                    "column_index": row.column_index,
                    "begin_snapshot": row.begin_snapshot,
                    "end_snapshot": row.end_snapshot,
                    "default_value": row.default_value,
                    "is_nullable": row.is_nullable,
                }),
            };
            serde_json::to_writer(&mut *writer, &exported)
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            rows_exported += 1;
        }
    }

    // Export data files
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_DATA_FILE);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: DataFileRow = values::decode_value(&kv.value)?;
        let begin = row.begin_snapshot.unwrap_or(0);
        // MVCC: only export rows visible at dl_snapshot_id (not yet retired).
        let live_at_snapshot = begin <= dl_snapshot_id.as_u64()
            && row
                .end_snapshot
                .is_none_or(|end| end > dl_snapshot_id.as_u64());
        if live_at_snapshot {
            let exported = ExportedRow {
                table: "ducklake_data_file".to_string(),
                data: serde_json::json!({
                    "data_file_id": row.data_file_id,
                    "table_id": row.table_id,
                    "path": row.path,
                    "file_format": row.file_format,
                    "record_count": row.record_count,
                    "file_size_bytes": row.file_size_bytes,
                    "begin_snapshot": begin,
                    "end_snapshot": row.end_snapshot,
                    "footer_size": row.footer_size,
                }),
            };
            serde_json::to_writer(&mut *writer, &exported)
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            rows_exported += 1;
        }
    }

    // Export delete files
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_DELETE_FILE);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: DeleteFileRow = values::decode_value(&kv.value)?;
        let begin = row.begin_snapshot.unwrap_or(row.snapshot_id);
        // MVCC: only export rows visible at dl_snapshot_id (not yet retired).
        let live_at_snapshot = begin <= dl_snapshot_id.as_u64()
            && row
                .end_snapshot
                .is_none_or(|end| end > dl_snapshot_id.as_u64());
        if live_at_snapshot {
            let exported = ExportedRow {
                table: "ducklake_delete_file".to_string(),
                data: serde_json::json!({
                    "delete_file_id": row.delete_file_id,
                    "data_file_id": row.data_file_id,
                    "path": row.path,
                    "delete_count": row.delete_count,
                    "file_size_bytes": row.file_size_bytes,
                    "snapshot_id": row.snapshot_id,
                    "begin_snapshot": begin,
                    "end_snapshot": row.end_snapshot,
                }),
            };
            serde_json::to_writer(&mut *writer, &exported)
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            rows_exported += 1;
        }
    }

    // Export inlined inserts
    let prefix = vec![TAG_INLINED_ROWS, INLINED_SUBTYPE_INSERT];
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: InlinedInsertRow = values::decode_value(&kv.value)?;
        if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, dl_snapshot_id) {
            let exported = ExportedRow {
                table: "ducklake_inlined_insert".to_string(),
                data: serde_json::json!({
                    "table_id": row.table_id,
                    "schema_version": row.schema_version,
                    "row_id": row.row_id,
                    "payload": base64_encode_crate(&row.payload),
                    "begin_snapshot": row.begin_snapshot,
                    "end_snapshot": row.end_snapshot,
                }),
            };
            serde_json::to_writer(&mut *writer, &exported)
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            rows_exported += 1;
        }
    }

    // Export snapshot changes
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_SNAPSHOT_CHANGES);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: SnapshotChangesRow = values::decode_value(&kv.value)?;
        if row.snapshot_id <= dl_snapshot_id.as_u64() {
            let exported = ExportedRow {
                table: "ducklake_snapshot_changes".to_string(),
                data: serde_json::json!({
                    "snapshot_id": row.snapshot_id,
                    "change_type": row.change_type,
                    "change_info": row.change_info,
                    "schema_id": row.schema_id,
                    "table_id": row.table_id,
                    "author": row.author,
                    "commit_message": row.commit_message,
                    "commit_extra_info": row.commit_extra_info,
                    "changes_made": row.changes_made,
                }),
            };
            serde_json::to_writer(&mut *writer, &exported)
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            rows_exported += 1;
        }
    }

    // Export views
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_VIEW);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: ViewRow = values::decode_value(&kv.value)?;
        if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, dl_snapshot_id) {
            let exported = ExportedRow {
                table: "ducklake_view".to_string(),
                data: serde_json::json!({
                    "view_id": row.view_id,
                    "schema_id": row.schema_id,
                    "view_name": row.view_name,
                    "sql": row.sql,
                    "begin_snapshot": row.begin_snapshot,
                    "end_snapshot": row.end_snapshot,
                    "view_uuid": row.view_uuid,
                    "dialect": row.dialect,
                    "column_aliases": row.column_aliases,
                }),
            };
            serde_json::to_writer(&mut *writer, &exported)
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            rows_exported += 1;
        }
    }

    // Export macros
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_MACRO);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: MacroRow = values::decode_value(&kv.value)?;
        if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, dl_snapshot_id) {
            let exported = ExportedRow {
                table: "ducklake_macro".to_string(),
                data: serde_json::json!({
                    "macro_id": row.macro_id,
                    "schema_id": row.schema_id,
                    "macro_name": row.macro_name,
                    "macro_type": row.macro_type,
                    "begin_snapshot": row.begin_snapshot,
                    "end_snapshot": row.end_snapshot,
                    "macro_uuid": row.macro_uuid,
                }),
            };
            serde_json::to_writer(&mut *writer, &exported)
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            rows_exported += 1;
        }
    }

    // Export macro impls
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_MACRO_IMPL);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: MacroImplRow = values::decode_value(&kv.value)?;
        let exported = ExportedRow {
            table: "ducklake_macro_impl".to_string(),
            data: serde_json::json!({
                "impl_id": row.impl_id,
                "macro_id": row.macro_id,
                "sql": row.sql,
                "dialect": row.dialect,
                "impl_type": row.impl_type,
            }),
        };
        serde_json::to_writer(&mut *writer, &exported)
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        rows_exported += 1;
    }

    // Export macro parameters
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_MACRO_PARAMETERS);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: MacroParametersRow = values::decode_value(&kv.value)?;
        let exported = ExportedRow {
            table: "ducklake_macro_parameters".to_string(),
            data: serde_json::json!({
                "macro_id": row.macro_id,
                "impl_id": row.impl_id,
                "column_id": row.column_id,
                "parameter_name": row.parameter_name,
                "parameter_type": row.parameter_type,
                "default_value": row.default_value,
                "default_value_type": row.default_value_type,
            }),
        };
        serde_json::to_writer(&mut *writer, &exported)
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        rows_exported += 1;
    }

    // Export tags (ducklake_tag)
    // The key includes tag_key_hash; export the hash so import can reconstruct
    // the exact key without relying on a non-deterministic hasher.
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_TAG);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: TagRow = values::decode_value(&kv.value)?;
        if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, dl_snapshot_id) {
            // Key layout: 0x1A | object_id(8) | tag_key_hash(8) | begin_snapshot(8)
            let tag_key_hash = if kv.key.len() >= 17 {
                keys::decode_u64(&kv.key[9..]).unwrap_or(0)
            } else {
                0
            };
            let exported = ExportedRow {
                table: "ducklake_tag".to_string(),
                data: serde_json::json!({
                    "object_id": row.object_id,
                    "tag_key": row.tag_key,
                    "tag_value": row.tag_value,
                    "begin_snapshot": row.begin_snapshot,
                    "end_snapshot": row.end_snapshot,
                    "tag_key_hash": tag_key_hash,
                }),
            };
            serde_json::to_writer(&mut *writer, &exported)
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            rows_exported += 1;
        }
    }

    // Export column tags (ducklake_column_tag)
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_COLUMN_TAG);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: ColumnTagRow = values::decode_value(&kv.value)?;
        if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, dl_snapshot_id) {
            // Key layout: 0x1B | table_id(8) | column_id(8) | tag_key_hash(8) | begin_snapshot(8)
            let tag_key_hash = if kv.key.len() >= 25 {
                keys::decode_u64(&kv.key[17..]).unwrap_or(0)
            } else {
                0
            };
            let exported = ExportedRow {
                table: "ducklake_column_tag".to_string(),
                data: serde_json::json!({
                    "table_id": row.table_id,
                    "column_id": row.column_id,
                    "tag_key": row.tag_key,
                    "tag_value": row.tag_value,
                    "begin_snapshot": row.begin_snapshot,
                    "end_snapshot": row.end_snapshot,
                    "tag_key_hash": tag_key_hash,
                }),
            };
            serde_json::to_writer(&mut *writer, &exported)
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            rows_exported += 1;
        }
    }

    // Export partition info
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_PARTITION_INFO);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: PartitionInfoRow = values::decode_value(&kv.value)?;
        if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, dl_snapshot_id) {
            let exported = ExportedRow {
                table: "ducklake_partition_info".to_string(),
                data: serde_json::json!({
                    "partition_id": row.partition_id,
                    "table_id": row.table_id,
                    "begin_snapshot": row.begin_snapshot,
                    "end_snapshot": row.end_snapshot,
                }),
            };
            serde_json::to_writer(&mut *writer, &exported)
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            rows_exported += 1;
        }
    }

    // Export sort info
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_SORT_INFO);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: SortInfoRow = values::decode_value(&kv.value)?;
        if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, dl_snapshot_id) {
            let exported = ExportedRow {
                table: "ducklake_sort_info".to_string(),
                data: serde_json::json!({
                    "sort_id": row.sort_id,
                    "table_id": row.table_id,
                    "begin_snapshot": row.begin_snapshot,
                    "end_snapshot": row.end_snapshot,
                }),
            };
            serde_json::to_writer(&mut *writer, &exported)
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            rows_exported += 1;
        }
    }

    // Export sort expressions
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_SORT_EXPRESSION);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: SortExpressionRow = values::decode_value(&kv.value)?;
        let exported = ExportedRow {
            table: "ducklake_sort_expression".to_string(),
            data: serde_json::json!({
                "sort_id": row.sort_id,
                "sort_key_index": row.sort_key_index,
                "column_id": row.column_id,
                "sort_direction": row.sort_direction,
                "null_order": row.null_order,
                "table_id": row.table_id,
                "expression": row.expression,
                "dialect": row.dialect,
            }),
        };
        serde_json::to_writer(&mut *writer, &exported)
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        rows_exported += 1;
    }

    // Export schema versions
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_SCHEMA_VERSIONS);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: SchemaVersionsRow = values::decode_value(&kv.value)?;
        let exported = ExportedRow {
            table: "ducklake_schema_version".to_string(),
            data: serde_json::json!({
                "table_id": row.table_id,
                "begin_snapshot": row.begin_snapshot,
                "schema_version": row.schema_version,
            }),
        };
        serde_json::to_writer(&mut *writer, &exported)
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        rows_exported += 1;
    }

    // Export table stats
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_TABLE_STATS);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: TableStatsRow = values::decode_value(&kv.value)?;
        let exported = ExportedRow {
            table: "ducklake_table_stats".to_string(),
            data: serde_json::json!({
                "table_id": row.table_id,
                "record_count": row.record_count,
                "file_size_bytes": row.file_size_bytes,
                "next_row_id": row.next_row_id,
            }),
        };
        serde_json::to_writer(&mut *writer, &exported)
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        rows_exported += 1;
    }

    // Export table column stats
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_TABLE_COLUMN_STATS);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: TableColumnStatsRow = values::decode_value(&kv.value)?;
        let exported = ExportedRow {
            table: "ducklake_table_column_stats".to_string(),
            data: serde_json::json!({
                "table_id": row.table_id,
                "column_id": row.column_id,
                "contains_null": row.contains_null,
                "min_value": row.min_value,
                "max_value": row.max_value,
                "contains_nan": row.contains_nan,
                "extra_stats": row.extra_stats,
            }),
        };
        serde_json::to_writer(&mut *writer, &exported)
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        rows_exported += 1;
    }

    // Export file column stats
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_FILE_COLUMN_STATS);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: FileColumnStatsRow = values::decode_value(&kv.value)?;
        let exported = ExportedRow {
            table: "ducklake_file_column_stats".to_string(),
            data: serde_json::json!({
                "table_id": row.table_id,
                "column_id": row.column_id,
                "data_file_id": row.data_file_id,
                "contains_null": row.contains_null,
                "min_value": row.min_value,
                "max_value": row.max_value,
                "contains_nan": row.contains_nan,
                "column_size_bytes": row.column_size_bytes,
                "value_count": row.value_count,
                "null_count": row.null_count,
                "extra_stats": row.extra_stats,
            }),
        };
        serde_json::to_writer(&mut *writer, &exported)
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        rows_exported += 1;
    }

    // Export column mappings
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_COLUMN_MAPPING);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: ColumnMappingRow = values::decode_value(&kv.value)?;
        let exported = ExportedRow {
            table: "ducklake_column_mapping".to_string(),
            data: serde_json::json!({
                "mapping_id": row.mapping_id,
                "table_id": row.table_id,
                "file_column_name": row.file_column_name,
                "column_id": row.column_id,
                "mapping_type": row.mapping_type,
            }),
        };
        serde_json::to_writer(&mut *writer, &exported)
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        rows_exported += 1;
    }

    // Export name mappings
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_NAME_MAPPING);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: NameMappingRow = values::decode_value(&kv.value)?;
        let exported = ExportedRow {
            table: "ducklake_name_mapping".to_string(),
            data: serde_json::json!({
                "mapping_id": row.mapping_id,
                "column_id": row.column_id,
                "name": row.name,
                "source_name_hash": row.source_name_hash,
                "target_field_id": row.target_field_id,
                "parent_column": row.parent_column,
                "is_partition": row.is_partition,
            }),
        };
        serde_json::to_writer(&mut *writer, &exported)
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        rows_exported += 1;
    }

    // Export file partition values
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_FILE_PARTITION_VALUE);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: FilePartitionValueRow = values::decode_value(&kv.value)?;
        let exported = ExportedRow {
            table: "ducklake_file_partition_value".to_string(),
            data: serde_json::json!({
                "table_id": row.table_id,
                "partition_key_index": row.partition_key_index,
                "data_file_id": row.data_file_id,
                "partition_value": row.partition_value,
            }),
        };
        serde_json::to_writer(&mut *writer, &exported)
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        rows_exported += 1;
    }

    // Export file variant stats
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_FILE_VARIANT_STATS);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: FileVariantStatsRow = values::decode_value(&kv.value)?;
        // Key layout: TAG(1) | table_id(8) | column_id(8) | variant_path_hash(8) | data_file_id(8)
        let variant_path_hash = if kv.key.len() >= 25 {
            keys::decode_u64(&kv.key[17..]).unwrap_or(0)
        } else {
            0
        };
        let exported = ExportedRow {
            table: "ducklake_file_variant_stats".to_string(),
            data: serde_json::json!({
                "table_id": row.table_id,
                "column_id": row.column_id,
                "data_file_id": row.data_file_id,
                "variant_key": row.variant_key,
                "variant_path_hash": variant_path_hash,
                "min_value": row.min_value,
                "max_value": row.max_value,
                "shredded_type": row.shredded_type,
                "column_size_bytes": row.column_size_bytes,
                "value_count": row.value_count,
                "null_count": row.null_count,
                "contains_nan": row.contains_nan,
                "extra_stats": row.extra_stats,
            }),
        };
        serde_json::to_writer(&mut *writer, &exported)
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        rows_exported += 1;
    }

    // Export encrypted secrets
    // NOTE: encrypted_secret fields are redacted for security; import restores
    // the row with an empty placeholder and requires manual secret rotation.
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_ENCRYPTED_SECRET);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: EncryptedSecretRow = values::decode_value(&kv.value)?;
        let exported = ExportedRow {
            table: "ducklake_encrypted_secret".to_string(),
            data: serde_json::json!({
                "secret_id": row.secret_id,
                "secret_name": row.secret_name,
                "encrypted_secret": "<redacted>",
            }),
        };
        serde_json::to_writer(&mut *writer, &exported)
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        rows_exported += 1;
    }

    // Export encryption keys
    // NOTE: encryption_key fields are redacted for security; restore requires
    // manual re-keying after import.
    tables_exported += 1;
    let prefix = keys::prefix_for_tag(TAG_ENCRYPTION_KEY);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: EncryptionKeyRow = values::decode_value(&kv.value)?;
        if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, dl_snapshot_id) {
            let exported = ExportedRow {
                table: "ducklake_encryption_key".to_string(),
                data: serde_json::json!({
                    "catalog_id": row.catalog_id,
                    "begin_snapshot": row.begin_snapshot,
                    "end_snapshot": row.end_snapshot,
                    "encryption_type": row.encryption_type,
                    "key_id": row.key_id,
                    "encryption_key": "<redacted>",
                }),
            };
            serde_json::to_writer(&mut *writer, &exported)
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            writeln!(writer).map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            rows_exported += 1;
        }
    }

    Ok(ExportResult {
        rows_exported,
        tables_exported,
    })
}

/// Import catalog rows from an NDJSON reader into a fresh catalog.
/// Returns a typed `ImportError` with line number on malformed input.
pub async fn import_catalog<R: BufRead>(db: &Db, reader: R) -> CatalogResult<ImportResult> {
    use base64::Engine as _;

    let mut rows_imported = 0u64;
    let mut tables_seen = std::collections::HashSet::new();
    let mut line_no = 0usize;

    // Helper closures capture line_no and table name for error context.
    macro_rules! req_u64 {
        ($data:expr, $field:expr, $table:expr) => {
            $data[$field].as_u64().ok_or_else(|| CatalogError::Import {
                line: line_no,
                table: $table.to_string(),
                message: format!("missing or invalid u64 field '{}'", $field),
            })?
        };
    }
    macro_rules! req_str {
        ($data:expr, $field:expr, $table:expr) => {
            $data[$field]
                .as_str()
                .ok_or_else(|| CatalogError::Import {
                    line: line_no,
                    table: $table.to_string(),
                    message: format!("missing or invalid string field '{}'", $field),
                })?
                .to_string()
        };
    }

    for line in reader.lines() {
        line_no += 1;
        let line = line.map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        if line.trim().is_empty() {
            continue;
        }

        let exported: ExportedRow =
            serde_json::from_str(&line).map_err(|e| CatalogError::Import {
                line: line_no,
                table: "unknown".to_string(),
                message: format!("JSON parse error: {e}"),
            })?;

        tables_seen.insert(exported.table.clone());
        let d = &exported.data;
        let tbl = exported.table.as_str();

        match tbl {
            "ducklake_snapshot" => {
                let snapshot_id = req_u64!(d, "snapshot_id", tbl);
                let row = SnapshotRow {
                    snapshot_id,
                    schema_version: req_u64!(d, "schema_version", tbl),
                    snapshot_time: req_str!(d, "snapshot_time", tbl),
                    author: d["author"].as_str().map(|s| s.to_string()),
                    message: d["message"].as_str().map(|s| s.to_string()),
                    next_catalog_id: d["next_catalog_id"].as_u64(),
                    next_file_id: d["next_file_id"].as_u64(),
                };
                let key = keys::key_snapshot(snapshot_id);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_schema" => {
                let schema_id = req_u64!(d, "schema_id", tbl);
                let row = SchemaRow {
                    schema_id,
                    schema_name: req_str!(d, "schema_name", tbl),
                    begin_snapshot: req_u64!(d, "begin_snapshot", tbl),
                    end_snapshot: d["end_snapshot"].as_u64(),
                    schema_uuid: d["schema_uuid"].as_str().map(|s| s.to_string()),
                    path: d["path"].as_str().map(|s| s.to_string()),
                    path_is_relative: d["path_is_relative"].as_bool(),
                };
                let key = keys::key_schema(schema_id);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_table" => {
                let table_id = req_u64!(d, "table_id", tbl);
                let schema_id = req_u64!(d, "schema_id", tbl);
                let begin_snapshot = req_u64!(d, "begin_snapshot", tbl);
                let row = TableRow {
                    table_id,
                    schema_id,
                    table_name: req_str!(d, "table_name", tbl),
                    begin_snapshot,
                    end_snapshot: d["end_snapshot"].as_u64(),
                    path: d["path"]
                        .as_str()
                        .or_else(|| d["data_path"].as_str())
                        .map(|s| s.to_string()),
                    table_uuid: d["table_uuid"].as_str().map(|s| s.to_string()),
                    path_is_relative: d["path_is_relative"].as_bool(),
                };
                let key = keys::key_table(schema_id, table_id, begin_snapshot);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_column" => {
                let column_id = req_u64!(d, "column_id", tbl);
                let table_id = req_u64!(d, "table_id", tbl);
                let begin_snapshot = req_u64!(d, "begin_snapshot", tbl);
                let row = ColumnRow {
                    column_id,
                    table_id,
                    column_name: req_str!(d, "column_name", tbl),
                    data_type: req_str!(d, "data_type", tbl),
                    column_index: req_u64!(d, "column_index", tbl),
                    begin_snapshot,
                    end_snapshot: d["end_snapshot"].as_u64(),
                    default_value: d["default_value"].as_str().map(|s| s.to_string()),
                    is_nullable: d["is_nullable"].as_bool().ok_or_else(|| {
                        CatalogError::Import {
                            line: line_no,
                            table: tbl.to_string(),
                            message: "missing or invalid bool field 'is_nullable'".to_string(),
                        }
                    })?,
                    initial_default: d["initial_default"].as_str().map(|s| s.to_string()),
                    default_value_type: d["default_value_type"].as_str().map(|s| s.to_string()),
                    default_value_dialect: d["default_value_dialect"]
                        .as_str()
                        .map(|s| s.to_string()),
                    parent_column: d["parent_column"].as_u64(),
                };
                let key = keys::key_column(table_id, column_id, begin_snapshot);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_data_file" => {
                let data_file_id = req_u64!(d, "data_file_id", tbl);
                let table_id = req_u64!(d, "table_id", tbl);
                let begin_snapshot = d["begin_snapshot"]
                    .as_u64()
                    .or_else(|| d["snapshot_id"].as_u64());
                let row = DataFileRow {
                    data_file_id,
                    table_id,
                    path: req_str!(d, "path", tbl),
                    file_format: req_str!(d, "file_format", tbl),
                    record_count: d["record_count"]
                        .as_u64()
                        .or_else(|| d["row_count"].as_u64())
                        .unwrap_or(0),
                    file_size_bytes: req_u64!(d, "file_size_bytes", tbl),
                    footer_size: d["footer_size"].as_i64(),
                    encryption_key: d["encryption_key"].as_str().map(|s| s.to_string()),
                    begin_snapshot,
                    end_snapshot: d["end_snapshot"].as_u64(),
                    file_order: d["file_order"].as_u64(),
                    path_is_relative: d["path_is_relative"].as_bool(),
                    row_id_start: d["row_id_start"].as_u64(),
                    partition_id: d["partition_id"].as_u64(),
                    mapping_id: d["mapping_id"].as_u64(),
                    partial_max: d["partial_max"].as_str().map(|s| s.to_string()),
                };
                let encoded = values::encode_value(&row);
                // Write primary key and secondary index atomically so a
                // crash between the two puts cannot leave list_data_files()
                // seeing a missing secondary entry.
                let mut batch = WriteBatch::new();
                batch.put(keys::key_data_file(table_id, data_file_id), encoded.clone());
                let idx_begin = begin_snapshot.unwrap_or(0);
                batch.put(
                    keys::key_data_file_by_snapshot(table_id, idx_begin, data_file_id),
                    encoded,
                );
                db.write(batch).await?;
                rows_imported += 1;
            }
            "ducklake_delete_file" => {
                let delete_file_id = req_u64!(d, "delete_file_id", tbl);
                let data_file_id = req_u64!(d, "data_file_id", tbl);
                let snapshot_id = d["snapshot_id"].as_u64().unwrap_or(0);
                let row = DeleteFileRow {
                    delete_file_id,
                    data_file_id,
                    path: req_str!(d, "path", tbl),
                    delete_count: d["delete_count"]
                        .as_u64()
                        .or_else(|| d["row_count"].as_u64())
                        .unwrap_or(0),
                    file_size_bytes: req_u64!(d, "file_size_bytes", tbl),
                    snapshot_id,
                    table_id: d["table_id"].as_u64(),
                    begin_snapshot: d["begin_snapshot"].as_u64(),
                    end_snapshot: d["end_snapshot"].as_u64(),
                    path_is_relative: d["path_is_relative"].as_bool(),
                    format: d["format"].as_str().map(|s| s.to_string()),
                    footer_size: d["footer_size"].as_i64(),
                    partial_max: d["partial_max"].as_str().map(|s| s.to_string()),
                };
                let key = keys::key_delete_file(data_file_id, delete_file_id);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_inlined_insert" => {
                let table_id = req_u64!(d, "table_id", tbl);
                let schema_version = req_u64!(d, "schema_version", tbl);
                let row_id = req_u64!(d, "row_id", tbl);
                let payload_b64 = req_str!(d, "payload", tbl);
                let payload = base64::engine::general_purpose::STANDARD
                    .decode(&payload_b64)
                    .map_err(|e| CatalogError::Import {
                        line: line_no,
                        table: tbl.to_string(),
                        message: format!("invalid base64 in 'payload': {e}"),
                    })?;
                let row = InlinedInsertRow {
                    table_id,
                    schema_version,
                    row_id,
                    payload,
                    begin_snapshot: req_u64!(d, "begin_snapshot", tbl),
                    end_snapshot: d["end_snapshot"].as_u64(),
                };
                let key = keys::key_inlined_insert(table_id, schema_version, row_id);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_snapshot_changes" => {
                let snapshot_id = req_u64!(d, "snapshot_id", tbl);
                let row = SnapshotChangesRow {
                    snapshot_id,
                    change_type: req_str!(d, "change_type", tbl),
                    change_info: d["change_info"].as_str().map(|s| s.to_string()),
                    schema_id: d["schema_id"].as_u64(),
                    table_id: d["table_id"].as_u64(),
                    author: d["author"].as_str().map(|s| s.to_string()),
                    commit_message: d["commit_message"].as_str().map(|s| s.to_string()),
                    commit_extra_info: d["commit_extra_info"].as_str().map(|s| s.to_string()),
                    changes_made: d["changes_made"].as_str().map(|s| s.to_string()),
                };
                let key = keys::key_snapshot_changes(snapshot_id);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_view" => {
                let view_id = req_u64!(d, "view_id", tbl);
                let schema_id = req_u64!(d, "schema_id", tbl);
                let begin_snapshot = req_u64!(d, "begin_snapshot", tbl);
                let row = ViewRow {
                    view_id,
                    schema_id,
                    view_name: req_str!(d, "view_name", tbl),
                    sql: req_str!(d, "sql", tbl),
                    begin_snapshot,
                    end_snapshot: d["end_snapshot"].as_u64(),
                    view_uuid: d["view_uuid"].as_str().map(|s| s.to_string()),
                    dialect: d["dialect"].as_str().map(|s| s.to_string()),
                    column_aliases: d["column_aliases"].as_str().map(|s| s.to_string()),
                };
                let key = keys::key_view(schema_id, view_id, begin_snapshot);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_macro" => {
                let macro_id = req_u64!(d, "macro_id", tbl);
                let schema_id = req_u64!(d, "schema_id", tbl);
                let begin_snapshot = req_u64!(d, "begin_snapshot", tbl);
                let row = MacroRow {
                    macro_id,
                    schema_id,
                    macro_name: req_str!(d, "macro_name", tbl),
                    macro_type: req_str!(d, "macro_type", tbl),
                    begin_snapshot,
                    end_snapshot: d["end_snapshot"].as_u64(),
                    macro_uuid: d["macro_uuid"].as_str().map(|s| s.to_string()),
                };
                let key = keys::key_macro(schema_id, macro_id, begin_snapshot);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_macro_impl" => {
                let impl_id = req_u64!(d, "impl_id", tbl);
                let macro_id = req_u64!(d, "macro_id", tbl);
                let row = MacroImplRow {
                    impl_id,
                    macro_id,
                    sql: req_str!(d, "sql", tbl),
                    dialect: d["dialect"].as_str().map(|s| s.to_string()),
                    impl_type: d["impl_type"].as_str().map(|s| s.to_string()),
                };
                let key = keys::key_macro_impl(macro_id, impl_id);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_macro_parameters" => {
                let macro_id = req_u64!(d, "macro_id", tbl);
                let impl_id = req_u64!(d, "impl_id", tbl);
                let column_id = req_u64!(d, "column_id", tbl);
                let row = MacroParametersRow {
                    macro_id,
                    impl_id,
                    column_id,
                    parameter_name: req_str!(d, "parameter_name", tbl),
                    parameter_type: req_str!(d, "parameter_type", tbl),
                    default_value: d["default_value"].as_str().map(|s| s.to_string()),
                    default_value_type: d["default_value_type"].as_str().map(|s| s.to_string()),
                };
                let key = keys::key_macro_parameters(macro_id, impl_id, column_id);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_tag" => {
                let object_id = req_u64!(d, "object_id", tbl);
                let begin_snapshot = req_u64!(d, "begin_snapshot", tbl);
                // Use the stored hash to reconstruct the exact key.
                let tag_key_hash = d["tag_key_hash"].as_u64().unwrap_or(0);
                let row = TagRow {
                    object_id,
                    tag_key: req_str!(d, "tag_key", tbl),
                    tag_value: req_str!(d, "tag_value", tbl),
                    begin_snapshot,
                    end_snapshot: d["end_snapshot"].as_u64(),
                };
                let key = keys::key_tag(object_id, tag_key_hash, begin_snapshot);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_column_tag" => {
                let table_id = req_u64!(d, "table_id", tbl);
                let column_id = req_u64!(d, "column_id", tbl);
                let begin_snapshot = req_u64!(d, "begin_snapshot", tbl);
                let tag_key_hash = d["tag_key_hash"].as_u64().unwrap_or(0);
                let row = ColumnTagRow {
                    table_id,
                    column_id,
                    tag_key: req_str!(d, "tag_key", tbl),
                    tag_value: req_str!(d, "tag_value", tbl),
                    begin_snapshot,
                    end_snapshot: d["end_snapshot"].as_u64(),
                };
                let key = keys::key_column_tag(table_id, column_id, tag_key_hash, begin_snapshot);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_partition_info" => {
                let partition_id = req_u64!(d, "partition_id", tbl);
                let table_id = req_u64!(d, "table_id", tbl);
                let begin_snapshot = req_u64!(d, "begin_snapshot", tbl);
                let row = PartitionInfoRow {
                    partition_id,
                    table_id,
                    begin_snapshot,
                    end_snapshot: d["end_snapshot"].as_u64(),
                };
                let key = keys::key_partition_info(table_id, partition_id, begin_snapshot);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_sort_info" => {
                let sort_id = req_u64!(d, "sort_id", tbl);
                let table_id = req_u64!(d, "table_id", tbl);
                let begin_snapshot = req_u64!(d, "begin_snapshot", tbl);
                let row = SortInfoRow {
                    sort_id,
                    table_id,
                    begin_snapshot,
                    end_snapshot: d["end_snapshot"].as_u64(),
                };
                let key = keys::key_sort_info(table_id, sort_id, begin_snapshot);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_sort_expression" => {
                let sort_id = req_u64!(d, "sort_id", tbl);
                let sort_key_index = req_u64!(d, "sort_key_index", tbl);
                let row = SortExpressionRow {
                    sort_id,
                    sort_key_index,
                    column_id: req_u64!(d, "column_id", tbl),
                    sort_direction: d["sort_direction"].as_str().map(|s| s.to_string()),
                    null_order: d["null_order"].as_str().map(|s| s.to_string()),
                    table_id: d["table_id"].as_u64(),
                    expression: d["expression"].as_str().map(|s| s.to_string()),
                    dialect: d["dialect"].as_str().map(|s| s.to_string()),
                };
                let key = keys::key_sort_expression(sort_id, sort_key_index);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_schema_version" => {
                let table_id = req_u64!(d, "table_id", tbl);
                let begin_snapshot = req_u64!(d, "begin_snapshot", tbl);
                let row = SchemaVersionsRow {
                    table_id,
                    begin_snapshot,
                    schema_version: req_u64!(d, "schema_version", tbl),
                };
                let key = keys::key_schema_versions(table_id, begin_snapshot);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_table_stats" => {
                let table_id = req_u64!(d, "table_id", tbl);
                let row = TableStatsRow {
                    table_id,
                    record_count: d["record_count"].as_u64().unwrap_or(0),
                    internal_file_count: 0,
                    file_size_bytes: d["file_size_bytes"].as_u64().unwrap_or(0),
                    next_row_id: d["next_row_id"].as_u64(),
                };
                let key = keys::key_table_stats(table_id);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_table_column_stats" => {
                let table_id = req_u64!(d, "table_id", tbl);
                let column_id = req_u64!(d, "column_id", tbl);
                let row = TableColumnStatsRow {
                    table_id,
                    column_id,
                    contains_null: d["contains_null"].as_bool().unwrap_or(false),
                    min_value: d["min_value"].as_str().map(|s| s.to_string()),
                    max_value: d["max_value"].as_str().map(|s| s.to_string()),
                    contains_nan: d["contains_nan"].as_bool(),
                    extra_stats: d["extra_stats"].as_str().map(|s| s.to_string()),
                };
                let key = keys::key_table_column_stats(table_id, column_id);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_file_column_stats" => {
                let table_id = req_u64!(d, "table_id", tbl);
                let column_id = req_u64!(d, "column_id", tbl);
                let data_file_id = req_u64!(d, "data_file_id", tbl);
                let row = FileColumnStatsRow {
                    table_id,
                    column_id,
                    data_file_id,
                    contains_null: d["contains_null"].as_bool().unwrap_or(false),
                    min_value: d["min_value"].as_str().map(|s| s.to_string()),
                    max_value: d["max_value"].as_str().map(|s| s.to_string()),
                    contains_nan: d["contains_nan"].as_bool().unwrap_or(false),
                    column_size_bytes: d["column_size_bytes"].as_u64(),
                    value_count: d["value_count"].as_u64(),
                    null_count: d["null_count"].as_u64(),
                    extra_stats: d["extra_stats"].as_str().map(|s| s.to_string()),
                };
                let key = keys::key_file_column_stats(table_id, column_id, data_file_id);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_column_mapping" => {
                let mapping_id = req_u64!(d, "mapping_id", tbl);
                let table_id = req_u64!(d, "table_id", tbl);
                let row = ColumnMappingRow {
                    mapping_id,
                    table_id,
                    file_column_name: d["file_column_name"].as_str().map(|s| s.to_string()),
                    column_id: d["column_id"].as_u64(),
                    mapping_type: d["mapping_type"].as_str().map(|s| s.to_string()),
                };
                let key = keys::key_column_mapping(table_id, mapping_id);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_name_mapping" => {
                let mapping_id = req_u64!(d, "mapping_id", tbl);
                let column_id = req_u64!(d, "column_id", tbl);
                let source_name_hash = d["source_name_hash"].as_u64().unwrap_or(0);
                let row = NameMappingRow {
                    mapping_id,
                    column_id,
                    name: req_str!(d, "name", tbl),
                    source_name_hash: d["source_name_hash"].as_u64(),
                    target_field_id: d["target_field_id"].as_u64(),
                    parent_column: d["parent_column"].as_u64(),
                    is_partition: d["is_partition"].as_bool(),
                };
                let key = keys::key_name_mapping(mapping_id, column_id, source_name_hash);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_file_partition_value" => {
                let table_id = req_u64!(d, "table_id", tbl);
                let partition_key_index = req_u64!(d, "partition_key_index", tbl);
                let data_file_id = req_u64!(d, "data_file_id", tbl);
                let row = FilePartitionValueRow {
                    table_id,
                    partition_key_index,
                    data_file_id,
                    partition_value: d["partition_value"].as_str().map(|s| s.to_string()),
                };
                let key =
                    keys::key_file_partition_value(table_id, partition_key_index, data_file_id);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_file_variant_stats" => {
                let table_id = req_u64!(d, "table_id", tbl);
                let column_id = req_u64!(d, "column_id", tbl);
                let data_file_id = req_u64!(d, "data_file_id", tbl);
                let variant_path_hash = d["variant_path_hash"].as_u64().unwrap_or(0);
                let row = FileVariantStatsRow {
                    table_id,
                    column_id,
                    #[allow(deprecated)]
                    deprecated_variant_path_hash: None,
                    data_file_id,
                    variant_key: req_str!(d, "variant_key", tbl),
                    min_value: d["min_value"].as_str().map(|s| s.to_string()),
                    max_value: d["max_value"].as_str().map(|s| s.to_string()),
                    shredded_type: d["shredded_type"].as_str().map(|s| s.to_string()),
                    column_size_bytes: d["column_size_bytes"].as_u64(),
                    value_count: d["value_count"].as_u64(),
                    null_count: d["null_count"].as_u64(),
                    contains_nan: d["contains_nan"].as_bool(),
                    extra_stats: d["extra_stats"].as_str().map(|s| s.to_string()),
                };
                let key = keys::key_file_variant_stats(
                    table_id,
                    column_id,
                    variant_path_hash,
                    data_file_id,
                );
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_encrypted_secret" => {
                let secret_id = req_u64!(d, "secret_id", tbl);
                // Redacted on export; import as a placeholder requiring manual rotation.
                let row = EncryptedSecretRow {
                    secret_id,
                    secret_name: req_str!(d, "secret_name", tbl),
                    encrypted_secret: d["encrypted_secret"]
                        .as_str()
                        .unwrap_or("<redacted>")
                        .to_string(),
                };
                let key = keys::key_encrypted_secret(secret_id);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_encryption_key" => {
                let catalog_id = req_u64!(d, "catalog_id", tbl);
                let begin_snapshot = req_u64!(d, "begin_snapshot", tbl);
                let row = EncryptionKeyRow {
                    catalog_id,
                    begin_snapshot,
                    end_snapshot: d["end_snapshot"].as_u64(),
                    encryption_type: req_str!(d, "encryption_type", tbl),
                    key_id: d["key_id"].as_str().map(|s| s.to_string()),
                    // Redacted on export; import as placeholder.
                    encryption_key: d["encryption_key"].as_str().map(|s| s.to_string()),
                };
                let key = keys::key_encryption_key(catalog_id, begin_snapshot);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            _ => {
                tracing::warn!("Unknown table in import at line {line_no}: {tbl}");
            }
        }
    }

    Ok(ImportResult {
        rows_imported,
        tables_imported: tables_seen.len() as u64,
    })
}

/// Convert an NDJSON export to PostgreSQL INSERT statements.
pub fn pg_migrate<R: BufRead, W: Write>(reader: R, writer: &mut W) -> CatalogResult<u64> {
    let mut count = 0u64;

    for line in reader.lines() {
        let line = line.map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        if line.trim().is_empty() {
            continue;
        }

        let exported: ExportedRow =
            serde_json::from_str(&line).map_err(|e| CatalogError::SlateDb(e.to_string()))?;

        let sql = row_to_pg_insert(&exported);
        writeln!(writer, "{sql}").map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        count += 1;
    }

    Ok(count)
}

/// Rebuild a catalog from Parquet files in the data path.
/// Synthesizes a minimal catalog with one snapshot, one schema, one table,
/// and data files for every path supplied.
///
/// v0.28.0: All rows are staged in a `WriteBatch` and committed atomically so a
/// mid-rebuild crash leaves the catalog either fully present or fully absent.
pub async fn rebuild_catalog(db: &Db, data_paths: &[String]) -> CatalogResult<u64> {
    use crate::init;
    use crate::verify;

    // Initialize counters (idempotent; writes to SlateDB outside the batch
    // since the catalog must be initialized before the batch can be composed).
    let _counters = init::initialize_catalog(db).await?;

    let mut file_count = 0u64;
    let mut batch = WriteBatch::new();

    // Create a default schema (schema_id = 1)
    let schema_id = 1u64;
    let schema_row = SchemaRow {
        schema_id,
        schema_name: "main".to_string(),
        begin_snapshot: 1,
        end_snapshot: None,
        schema_uuid: None,
        path: None,
        path_is_relative: None,
    };
    batch.put(
        keys::key_schema(schema_id),
        values::encode_value(&schema_row),
    );

    // Create the default table (table_id = 2, because catalog IDs 1..=2 are used)
    let table_id = 2u64;
    let table_row = TableRow {
        table_id,
        schema_id,
        table_name: "default".to_string(),
        begin_snapshot: 1,
        end_snapshot: None,
        path: None,
        table_uuid: None,
        path_is_relative: None,
    };
    batch.put(
        keys::key_table(schema_id, table_id, 1),
        values::encode_value(&table_row),
    );

    // Register data files under the default table
    let mut file_id = 1u64;
    for path in data_paths {
        let row = DataFileRow {
            data_file_id: file_id,
            table_id,
            path: path.clone(),
            file_format: "parquet".to_string(),
            record_count: 0, // Unknown without reading footer
            file_size_bytes: 0,
            footer_size: None,
            encryption_key: None,
            begin_snapshot: Some(1),
            end_snapshot: None,
            file_order: None,
            path_is_relative: Some(false),
            row_id_start: None,
            partition_id: None,
            mapping_id: None,
            partial_max: None,
        };
        let encoded = values::encode_value(&row);
        batch.put(keys::key_data_file(table_id, file_id), &encoded);
        // Write secondary index for O(log N) snapshot-bounded scans.
        batch.put(
            keys::key_data_file_by_snapshot(table_id, 1, file_id),
            &encoded,
        );
        file_id += 1;
        file_count += 1;
    }

    // Create initial snapshot
    let snapshot_row = SnapshotRow {
        snapshot_id: 1,
        schema_version: 1,
        snapshot_time: chrono::Utc::now().to_rfc3339(),
        author: Some("rebuild".to_string()),
        message: Some("Catalog rebuilt from Parquet files".to_string()),
        next_catalog_id: None,
        next_file_id: None,
    };
    batch.put(keys::key_snapshot(1), values::encode_value(&snapshot_row));

    // Update counters: next_snapshot = 2, next_catalog_id = table_id + 1 = 3,
    // next_file_id = file_id (already incremented past the last used id)
    batch.put(
        keys::key_counter(COUNTER_NEXT_SNAPSHOT_ID),
        values::encode_counter(2),
    );
    batch.put(
        keys::key_counter(COUNTER_NEXT_CATALOG_ID),
        values::encode_counter(table_id + 1),
    );
    batch.put(
        keys::key_counter(COUNTER_NEXT_FILE_ID),
        values::encode_counter(file_id),
    );

    // Commit all rows atomically.
    db.write(batch).await?;

    // Verify the rebuilt catalog is coherent
    verify::verify_catalog(db).await?;

    Ok(file_count)
}

// ─── Helpers ───────────────────────────────────────────────────────────────

/// Encode bytes as standard base64 using the `base64` crate.
fn base64_encode_crate(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// Escape a string value for use inside a SQL single-quoted literal.
/// Doubles any embedded single quotes per the SQL standard.
fn sql_escape(s: &str) -> String {
    s.replace('\'', "''")
}

fn row_to_pg_insert(exported: &ExportedRow) -> String {
    match exported.table.as_str() {
        "ducklake_snapshot" => {
            format!(
                "INSERT INTO ducklake_snapshot (snapshot_id, schema_version, snapshot_time) VALUES ({}, {}, '{}');",
                exported.data["snapshot_id"],
                exported.data["schema_version"],
                sql_escape(exported.data["snapshot_time"].as_str().unwrap_or(""))
            )
        }
        "ducklake_schema" => {
            format!(
                "INSERT INTO ducklake_schema (schema_id, schema_name, begin_snapshot, end_snapshot) VALUES ({}, '{}', {}, {});",
                exported.data["schema_id"],
                sql_escape(exported.data["schema_name"].as_str().unwrap_or("")),
                exported.data["begin_snapshot"],
                exported.data["end_snapshot"].as_u64().map_or("NULL".to_string(), |v| v.to_string())
            )
        }
        "ducklake_table" => {
            format!(
                "INSERT INTO ducklake_table (table_id, schema_id, table_name, begin_snapshot, end_snapshot) VALUES ({}, {}, '{}', {}, {});",
                exported.data["table_id"],
                exported.data["schema_id"],
                sql_escape(exported.data["table_name"].as_str().unwrap_or("")),
                exported.data["begin_snapshot"],
                exported.data["end_snapshot"].as_u64().map_or("NULL".to_string(), |v| v.to_string())
            )
        }
        "ducklake_column" => {
            format!(
                "INSERT INTO ducklake_column (column_id, table_id, column_name, data_type, column_index, begin_snapshot, end_snapshot, is_nullable) VALUES ({}, {}, '{}', '{}', {}, {}, {}, {});",
                exported.data["column_id"],
                exported.data["table_id"],
                sql_escape(exported.data["column_name"].as_str().unwrap_or("")),
                sql_escape(exported.data["data_type"].as_str().unwrap_or("")),
                exported.data["column_index"],
                exported.data["begin_snapshot"],
                exported.data["end_snapshot"].as_u64().map_or("NULL".to_string(), |v| v.to_string()),
                exported.data["is_nullable"].as_bool().unwrap_or(true)
            )
        }
        "ducklake_data_file" => {
            format!(
                "INSERT INTO ducklake_data_file (data_file_id, table_id, path, file_format, row_count, file_size_bytes, snapshot_id) VALUES ({}, {}, '{}', '{}', {}, {}, {});",
                exported.data["data_file_id"],
                exported.data["table_id"],
                sql_escape(exported.data["path"].as_str().unwrap_or("")),
                sql_escape(exported.data["file_format"].as_str().unwrap_or("")),
                exported.data["row_count"],
                exported.data["file_size_bytes"],
                exported.data["snapshot_id"]
            )
        }
        _ => format!("-- Unsupported table: {}", sql_escape(&exported.table)),
    }
}
