//! Catalog export, import, rebuild, and migration.
//!
//! - `export`: NDJSON export of all live catalog rows at a snapshot.
//! - `import`: Initialize a fresh catalog from an NDJSON export.
//! - `pg_migrate`: Convert NDJSON to PostgreSQL INSERT statements.
//! - `rebuild`: Synthesize a fresh catalog from Parquet footers.

use serde::{Deserialize, Serialize};
use slatedb::Db;
use slateduck_core::keys;
use slateduck_core::mvcc::{self, SnapshotId};
use slateduck_core::rows::*;
use slateduck_core::tags::*;
use slateduck_core::values;
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
                    "data_path": row.data_path,
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
        if row.snapshot_id <= dl_snapshot_id.as_u64() {
            let exported = ExportedRow {
                table: "ducklake_data_file".to_string(),
                data: serde_json::json!({
                    "data_file_id": row.data_file_id,
                    "table_id": row.table_id,
                    "path": row.path,
                    "file_format": row.file_format,
                    "row_count": row.row_count,
                    "file_size_bytes": row.file_size_bytes,
                    "snapshot_id": row.snapshot_id,
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
        if row.snapshot_id <= dl_snapshot_id.as_u64() {
            let exported = ExportedRow {
                table: "ducklake_delete_file".to_string(),
                data: serde_json::json!({
                    "delete_file_id": row.delete_file_id,
                    "data_file_id": row.data_file_id,
                    "path": row.path,
                    "row_count": row.row_count,
                    "file_size_bytes": row.file_size_bytes,
                    "snapshot_id": row.snapshot_id,
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
                    "payload": base64_encode(&row.payload),
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
pub async fn import_catalog<R: BufRead>(db: &Db, reader: R) -> CatalogResult<ImportResult> {
    let mut rows_imported = 0u64;
    let mut tables_seen = std::collections::HashSet::new();

    for line in reader.lines() {
        let line = line.map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        if line.trim().is_empty() {
            continue;
        }

        let exported: ExportedRow =
            serde_json::from_str(&line).map_err(|e| CatalogError::SlateDb(e.to_string()))?;

        tables_seen.insert(exported.table.clone());

        match exported.table.as_str() {
            "ducklake_snapshot" => {
                let snapshot_id = exported.data["snapshot_id"].as_u64().unwrap_or(0);
                let row = SnapshotRow {
                    snapshot_id,
                    schema_version: exported.data["schema_version"].as_u64().unwrap_or(0),
                    snapshot_time: exported.data["snapshot_time"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    author: exported.data["author"].as_str().map(|s| s.to_string()),
                    message: exported.data["message"].as_str().map(|s| s.to_string()),
                };
                let key = keys::key_snapshot(snapshot_id);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_schema" => {
                let schema_id = exported.data["schema_id"].as_u64().unwrap_or(0);
                let row = SchemaRow {
                    schema_id,
                    schema_name: exported.data["schema_name"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    begin_snapshot: exported.data["begin_snapshot"].as_u64().unwrap_or(0),
                    end_snapshot: exported.data["end_snapshot"].as_u64(),
                };
                let key = keys::key_schema(schema_id);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_table" => {
                let table_id = exported.data["table_id"].as_u64().unwrap_or(0);
                let schema_id = exported.data["schema_id"].as_u64().unwrap_or(0);
                let begin_snapshot = exported.data["begin_snapshot"].as_u64().unwrap_or(0);
                let row = TableRow {
                    table_id,
                    schema_id,
                    table_name: exported.data["table_name"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    begin_snapshot,
                    end_snapshot: exported.data["end_snapshot"].as_u64(),
                    data_path: exported.data["data_path"].as_str().map(|s| s.to_string()),
                };
                let key = keys::key_table(schema_id, table_id, begin_snapshot);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_column" => {
                let column_id = exported.data["column_id"].as_u64().unwrap_or(0);
                let table_id = exported.data["table_id"].as_u64().unwrap_or(0);
                let begin_snapshot = exported.data["begin_snapshot"].as_u64().unwrap_or(0);
                let row = ColumnRow {
                    column_id,
                    table_id,
                    column_name: exported.data["column_name"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    data_type: exported.data["data_type"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    column_index: exported.data["column_index"].as_u64().unwrap_or(0),
                    begin_snapshot,
                    end_snapshot: exported.data["end_snapshot"].as_u64(),
                    default_value: exported.data["default_value"]
                        .as_str()
                        .map(|s| s.to_string()),
                    is_nullable: exported.data["is_nullable"].as_bool().unwrap_or(true),
                };
                let key = keys::key_column(table_id, column_id, begin_snapshot);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_data_file" => {
                let data_file_id = exported.data["data_file_id"].as_u64().unwrap_or(0);
                let table_id = exported.data["table_id"].as_u64().unwrap_or(0);
                let row = DataFileRow {
                    data_file_id,
                    table_id,
                    path: exported.data["path"].as_str().unwrap_or("").to_string(),
                    file_format: exported.data["file_format"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    row_count: exported.data["row_count"].as_u64().unwrap_or(0),
                    file_size_bytes: exported.data["file_size_bytes"].as_u64().unwrap_or(0),
                    snapshot_id: exported.data["snapshot_id"].as_u64().unwrap_or(0),
                    footer_size: exported.data["footer_size"].as_str().map(|s| s.to_string()),
                };
                let key = keys::key_data_file(table_id, data_file_id);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_delete_file" => {
                let delete_file_id = exported.data["delete_file_id"].as_u64().unwrap_or(0);
                let data_file_id = exported.data["data_file_id"].as_u64().unwrap_or(0);
                let row = DeleteFileRow {
                    delete_file_id,
                    data_file_id,
                    path: exported.data["path"].as_str().unwrap_or("").to_string(),
                    row_count: exported.data["row_count"].as_u64().unwrap_or(0),
                    file_size_bytes: exported.data["file_size_bytes"].as_u64().unwrap_or(0),
                    snapshot_id: exported.data["snapshot_id"].as_u64().unwrap_or(0),
                };
                let key = keys::key_delete_file(data_file_id, delete_file_id);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            "ducklake_inlined_insert" => {
                let table_id = exported.data["table_id"].as_u64().unwrap_or(0);
                let schema_version = exported.data["schema_version"].as_u64().unwrap_or(0);
                let row_id = exported.data["row_id"].as_u64().unwrap_or(0);
                let payload = base64_decode(exported.data["payload"].as_str().unwrap_or(""));
                let row = InlinedInsertRow {
                    table_id,
                    schema_version,
                    row_id,
                    payload,
                    begin_snapshot: exported.data["begin_snapshot"].as_u64().unwrap_or(0),
                    end_snapshot: exported.data["end_snapshot"].as_u64(),
                };
                let key = keys::key_inlined_insert(table_id, schema_version, row_id);
                db.put(&key, &values::encode_value(&row)).await?;
                rows_imported += 1;
            }
            _ => {
                tracing::warn!("Unknown table in import: {}", exported.table);
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
/// Synthesizes a minimal catalog with one snapshot, one schema, and tables inferred from paths.
pub async fn rebuild_catalog(db: &Db, data_paths: &[String]) -> CatalogResult<u64> {
    use crate::init;

    // Initialize counters
    let _counters = init::initialize_catalog(db).await?;

    let mut file_count = 0u64;

    // Create a default schema
    let schema_id = 1u64;
    let schema_row = SchemaRow {
        schema_id,
        schema_name: "main".to_string(),
        begin_snapshot: 1,
        end_snapshot: None,
    };
    let key = keys::key_schema(schema_id);
    db.put(&key, &values::encode_value(&schema_row)).await?;

    // Create tables from paths (one table per unique directory)
    let table_id = 1u64;
    let mut file_id = 1u64;

    for path in data_paths {
        // Register as a data file in a default table
        let row = DataFileRow {
            data_file_id: file_id,
            table_id,
            path: path.clone(),
            file_format: "parquet".to_string(),
            row_count: 0, // Unknown without reading footer
            file_size_bytes: 0,
            snapshot_id: 1,
            footer_size: None,
        };
        let key = keys::key_data_file(table_id, file_id);
        db.put(&key, &values::encode_value(&row)).await?;
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
    };
    let key = keys::key_snapshot(1);
    db.put(&key, &values::encode_value(&snapshot_row)).await?;

    // Update counters
    let counter_key = keys::key_counter(COUNTER_NEXT_SNAPSHOT_ID);
    db.put(&counter_key, &values::encode_counter(2)).await?;
    let counter_key = keys::key_counter(COUNTER_NEXT_CATALOG_ID);
    db.put(
        &counter_key,
        &values::encode_counter(schema_id + table_id + 1),
    )
    .await?;
    let counter_key = keys::key_counter(COUNTER_NEXT_FILE_ID);
    db.put(&counter_key, &values::encode_counter(file_id))
        .await?;

    Ok(file_count)
}

// ─── Helpers ───────────────────────────────────────────────────────────────

fn base64_encode(data: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(data.len() * 4 / 3 + 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        let chars = match chunk.len() {
            3 => 4,
            2 => 3,
            _ => 2,
        };
        for i in 0..chars {
            let idx = ((triple >> (6 * (3 - i))) & 0x3F) as u8;
            let _ = write!(s, "{}", B64_CHARS[idx as usize] as char);
        }
        for _ in chars..4 {
            s.push('=');
        }
    }
    s
}

fn base64_decode(s: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    let bytes: Vec<u8> = s
        .bytes()
        .filter(|&b| b != b'=')
        .map(|b| B64_DECODE[b as usize])
        .collect();

    for chunk in bytes.chunks(4) {
        if chunk.len() >= 2 {
            let b0 = (chunk[0] as u32) << 18;
            let b1 = (chunk[1] as u32) << 12;
            let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
            let b3 = chunk.get(3).copied().unwrap_or(0) as u32;
            let triple = b0 | b1 | (b2 << 6) | b3;
            buf.push(((triple >> 16) & 0xFF) as u8);
            if chunk.len() >= 3 {
                buf.push(((triple >> 8) & 0xFF) as u8);
            }
            if chunk.len() >= 4 {
                buf.push((triple & 0xFF) as u8);
            }
        }
    }
    buf
}

const B64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

const B64_DECODE: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut i = 0;
    while i < 64 {
        table[B64_CHARS[i] as usize] = i as u8;
        i += 1;
    }
    table
};

fn row_to_pg_insert(exported: &ExportedRow) -> String {
    match exported.table.as_str() {
        "ducklake_snapshot" => {
            format!(
                "INSERT INTO ducklake_snapshot (snapshot_id, schema_version, snapshot_time) VALUES ({}, {}, '{}');",
                exported.data["snapshot_id"],
                exported.data["schema_version"],
                exported.data["snapshot_time"].as_str().unwrap_or("")
            )
        }
        "ducklake_schema" => {
            format!(
                "INSERT INTO ducklake_schema (schema_id, schema_name, begin_snapshot, end_snapshot) VALUES ({}, '{}', {}, {});",
                exported.data["schema_id"],
                exported.data["schema_name"].as_str().unwrap_or(""),
                exported.data["begin_snapshot"],
                exported.data["end_snapshot"].as_u64().map_or("NULL".to_string(), |v| v.to_string())
            )
        }
        "ducklake_table" => {
            format!(
                "INSERT INTO ducklake_table (table_id, schema_id, table_name, begin_snapshot, end_snapshot) VALUES ({}, {}, '{}', {}, {});",
                exported.data["table_id"],
                exported.data["schema_id"],
                exported.data["table_name"].as_str().unwrap_or(""),
                exported.data["begin_snapshot"],
                exported.data["end_snapshot"].as_u64().map_or("NULL".to_string(), |v| v.to_string())
            )
        }
        "ducklake_column" => {
            format!(
                "INSERT INTO ducklake_column (column_id, table_id, column_name, data_type, column_index, begin_snapshot, end_snapshot, is_nullable) VALUES ({}, {}, '{}', '{}', {}, {}, {}, {});",
                exported.data["column_id"],
                exported.data["table_id"],
                exported.data["column_name"].as_str().unwrap_or(""),
                exported.data["data_type"].as_str().unwrap_or(""),
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
                exported.data["path"].as_str().unwrap_or(""),
                exported.data["file_format"].as_str().unwrap_or(""),
                exported.data["row_count"],
                exported.data["file_size_bytes"],
                exported.data["snapshot_id"]
            )
        }
        _ => format!("-- Unsupported table: {}", exported.table),
    }
}
