//! Catalog export, import, rebuild, and migration.
//!
//! - `export`: NDJSON export of all live catalog rows at a snapshot.
//! - `import`: Initialize a fresh catalog from an NDJSON export.
//! - `pg_migrate`: Convert NDJSON to PostgreSQL INSERT statements.
//! - `rebuild`: Synthesize a fresh catalog from Parquet footers.

#![allow(missing_docs)]

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
#[derive(Debug, Clone)]
pub struct ExportResult {
    pub rows_exported: u64,
    pub tables_exported: u64,
}

/// Result of an import operation.
#[derive(Debug, Clone)]
pub struct ImportResult {
    pub rows_imported: u64,
    pub tables_imported: u64,
}

/// Export all live catalog rows at the given snapshot to NDJSON.
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
                // Primary key.
                let key = keys::key_data_file(table_id, data_file_id);
                db.put(&key, &encoded).await?;
                // Secondary index (TAG_DATA_FILE_BY_SNAPSHOT) required by
                // list_data_files() so that readers see the file after import.
                let idx_begin = begin_snapshot.unwrap_or(0);
                let idx_key = keys::key_data_file_by_snapshot(table_id, idx_begin, data_file_id);
                db.put(&idx_key, &encoded).await?;
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
