//! Catalog executor operations: response builders, execute_commit, table_changes, next_rowid.

use std::sync::Arc;

use pgwire::api::results::{DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response};
use pgwire::api::Type;

use slateduck_catalog::writer::stats::FileColumnStatsInput;
use slateduck_catalog::CatalogStore;

use crate::error::SlateDuckError;
use crate::notify::NotifyManager;
use crate::session::BufferedOp;

use super::extension::hash_table_ref;

pub(super) async fn execute_commit(
    ops: Vec<BufferedOp>,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
    notify_manager: &Arc<NotifyManager>,
) -> Result<(), SlateDuckError> {
    if ops.is_empty() {
        return Ok(());
    }

    // Collect table IDs from InsertDataFile ops for post-commit notifications.
    let mut affected_table_ids: Vec<u64> = ops
        .iter()
        .filter_map(|op| match op {
            BufferedOp::InsertDataFile { table_id, .. } => Some(*table_id),
            _ => None,
        })
        .collect();
    affected_table_ids.dedup();

    let mut s = store.lock().await;
    let mut writer = s.begin_write();

    for op in ops {
        match op {
            BufferedOp::InsertSchema { schema_name } => {
                writer
                    .create_schema(&schema_name)
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertTable {
                schema_id,
                table_name,
                data_path,
            } => {
                writer
                    .create_table(schema_id, &table_name, data_path.as_deref())
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertColumn {
                table_id,
                column_name,
                data_type,
                column_index,
                is_nullable,
                default_value,
            } => {
                writer
                    .add_column(
                        table_id,
                        &column_name,
                        &data_type,
                        column_index,
                        is_nullable,
                        default_value.as_deref(),
                    )
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertDataFile {
                table_id,
                path,
                file_format,
                row_count,
                file_size_bytes,
            } => {
                writer
                    .register_data_file(table_id, &path, &file_format, row_count, file_size_bytes)
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertDeleteFile {
                data_file_id,
                path,
                row_count,
                file_size_bytes,
            } => {
                writer
                    .register_delete_file(data_file_id, &path, row_count, file_size_bytes)
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertSnapshot { author, message } => {
                writer
                    .create_snapshot(author.as_deref(), message.as_deref())
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertSnapshotChanges { .. } => {
                // Snapshot changes are informational; accepted but not stored separately
            }
            BufferedOp::UpdateEndSnapshot {
                table_name,
                entity_id,
                begin_snapshot,
                end_snapshot: _,
            } => {
                match table_name.as_str() {
                    "ducklake_table" => {
                        // Resolve schema_id by scanning for the live table row
                        // (F-04: do not hard-code schema_id = 0).
                        let schema_id = writer
                            .find_table_schema_id(entity_id)
                            .await
                            .map_err(SlateDuckError::from)?
                            .unwrap_or(0);
                        writer
                            .drop_table(schema_id, entity_id, begin_snapshot)
                            .await
                            .map_err(SlateDuckError::from)?;
                    }
                    "ducklake_column" => {
                        // Resolve table_id by scanning for the live column row
                        // (F-04: entity_id is column_id, not table_id).
                        let table_id = writer
                            .find_column_table_id(entity_id)
                            .await
                            .map_err(SlateDuckError::from)?
                            .unwrap_or(entity_id);
                        writer
                            .drop_column(table_id, entity_id, begin_snapshot)
                            .await
                            .map_err(SlateDuckError::from)?;
                    }
                    _ => {
                        // Other end_snapshot updates accepted
                    }
                }
            }
            BufferedOp::UpdateTableStats {
                table_id,
                row_count_delta: _,
            } => {
                writer
                    .update_table_stats(table_id, 0, 0, 0)
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertFileColumnStats {
                table_id,
                column_id,
                data_file_id,
                has_null,
                min_value,
                max_value,
                contains_nan,
            } => {
                writer
                    .upsert_file_column_stats(FileColumnStatsInput {
                        table_id,
                        column_id,
                        data_file_id,
                        has_null,
                        min_value: min_value.as_deref(),
                        max_value: max_value.as_deref(),
                        contains_nan,
                    })
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertMetadata { .. } => {
                // Metadata writes accepted
            }
            BufferedOp::InsertInlinedDataTables { .. } => {
                // Inlined data table registration accepted
            }
            BufferedOp::InsertView { .. } => {
                // Views accepted
            }
            BufferedOp::InsertMacro { .. } => {
                // Macros accepted
            }
            BufferedOp::InsertTableStats {
                table_id,
                row_count,
                file_count,
                total_size_bytes,
            } => {
                writer
                    .update_table_stats(table_id, row_count, file_count, total_size_bytes)
                    .await
                    .map_err(SlateDuckError::from)?;
            }
        }
    }
    // Synchronise the store's in-memory counters from the committed writer
    // (F-01: ensures read_latest() and subsequent begin_write() see the new state).
    s.commit_writer(&writer);
    drop(s); // Release the store lock before async notification I/O.

    // Fire LISTEN/NOTIFY notifications for any tables that received new data files.
    if !affected_table_ids.is_empty() {
        notify_manager
            .notify_snapshot_advance(&affected_table_ids)
            .await;
    }

    Ok(())
}

// ─── Response Builders ─────────────────────────────────────────────────────

/// F-24: Require a u64 parameter; returns SQLSTATE 22023 if absent or invalid.
pub(super) fn make_snapshot_row_response(
    snap: slateduck_core::rows::SnapshotRow,
) -> Response<'static> {
    let schema = Arc::new(vec![
        FieldInfo::new(
            "snapshot_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "schema_version".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "snapshot_time".to_string(),
            None,
            None,
            Type::TIMESTAMPTZ,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "author".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "message".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
    ]);
    let mut encoder = DataRowEncoder::new(schema.clone());
    encoder
        .encode_field_with_type_and_format(
            &Some(snap.snapshot_id.to_string()),
            &Type::TEXT,
            FieldFormat::Text,
        )
        .unwrap();
    encoder
        .encode_field_with_type_and_format(
            &Some(snap.schema_version.to_string()),
            &Type::TEXT,
            FieldFormat::Text,
        )
        .unwrap();
    encoder
        .encode_field_with_type_and_format(
            &Some(snap.snapshot_time.clone()),
            &Type::TEXT,
            FieldFormat::Text,
        )
        .unwrap();
    encoder
        .encode_field_with_type_and_format(&snap.author, &Type::TEXT, FieldFormat::Text)
        .unwrap();
    encoder
        .encode_field_with_type_and_format(&snap.message, &Type::TEXT, FieldFormat::Text)
        .unwrap();
    let row = encoder.finish();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(vec![row]));
    resp.set_command_tag("SELECT 1");
    Response::Query(resp)
}

pub(super) fn make_schemas_response(
    schemas: Vec<slateduck_core::rows::SchemaRow>,
) -> Response<'static> {
    let schema = Arc::new(vec![
        FieldInfo::new(
            "schema_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "schema_name".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "begin_snapshot".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "end_snapshot".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
    ]);
    let mut data_rows = Vec::new();
    for s in &schemas {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(s.schema_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(s.schema_name.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(s.begin_snapshot.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        let end = s.end_snapshot.map(|e| e.to_string());
        encoder
            .encode_field_with_type_and_format(&end, &Type::TEXT, FieldFormat::Text)
            .unwrap();
        data_rows.push(encoder.finish());
    }
    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

pub(super) fn make_tables_response(
    tables: Vec<slateduck_core::rows::TableRow>,
) -> Response<'static> {
    let schema = Arc::new(vec![
        FieldInfo::new(
            "table_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "schema_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "table_name".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "begin_snapshot".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "end_snapshot".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "data_path".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
    ]);
    let mut data_rows = Vec::new();
    for t in &tables {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(t.table_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(t.schema_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(t.table_name.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(t.begin_snapshot.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        let end = t.end_snapshot.map(|e| e.to_string());
        encoder
            .encode_field_with_type_and_format(&end, &Type::TEXT, FieldFormat::Text)
            .unwrap();
        encoder
            .encode_field_with_type_and_format(&t.data_path.clone(), &Type::TEXT, FieldFormat::Text)
            .unwrap();
        data_rows.push(encoder.finish());
    }
    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

pub(super) fn make_columns_response(
    columns: Vec<slateduck_core::rows::ColumnRow>,
) -> Response<'static> {
    let schema = Arc::new(vec![
        FieldInfo::new(
            "column_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "table_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "column_name".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "data_type".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "column_index".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "begin_snapshot".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "end_snapshot".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "is_nullable".to_string(),
            None,
            None,
            Type::BOOL,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "default_value".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
    ]);
    let mut data_rows = Vec::new();
    for c in &columns {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(c.column_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(c.table_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(c.column_name.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(c.data_type.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(c.column_index.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(c.begin_snapshot.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        let end = c.end_snapshot.map(|e| e.to_string());
        encoder
            .encode_field_with_type_and_format(&end, &Type::TEXT, FieldFormat::Text)
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(c.is_nullable.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &c.default_value.clone(),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        data_rows.push(encoder.finish());
    }
    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

pub(super) fn make_data_files_response(
    files: Vec<slateduck_core::rows::DataFileRow>,
) -> Response<'static> {
    let schema = Arc::new(vec![
        FieldInfo::new(
            "data_file_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "table_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "path".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "file_format".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "row_count".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "file_size_bytes".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "snapshot_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
    ]);
    let mut data_rows = Vec::new();
    for f in &files {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(f.data_file_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(f.table_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(f.path.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(f.file_format.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(f.row_count.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(f.file_size_bytes.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(f.snapshot_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        data_rows.push(encoder.finish());
    }
    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

pub(super) fn make_file_ids_response(file_ids: Vec<u64>) -> Response<'static> {
    let schema = Arc::new(vec![FieldInfo::new(
        "data_file_id".to_string(),
        None,
        None,
        Type::INT8,
        FieldFormat::Text,
    )]);
    let mut data_rows = Vec::new();
    for id in &file_ids {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        data_rows.push(encoder.finish());
    }
    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

// ─── v0.18: DuckLake Standard Interface Executors ──────────────────────────

pub(super) async fn execute_table_changes<'a>(
    table_ref: &str,
    start_snapshot: u64,
    end_snapshot: u64,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
) -> Result<Vec<Response<'a>>, SlateDuckError> {
    let store_lock = store.lock().await;

    // Check GC boundary
    let retain_from = slateduck_catalog::gc::read_retain_from(store_lock.db())
        .await
        .map_err(SlateDuckError::from)?;

    if start_snapshot < retain_from && retain_from > 0 {
        return Err(SlateDuckError::SqlState {
            code: "55000".to_string(),
            message: format!(
                "snapshot {} has been garbage collected (retain_from={})",
                start_snapshot, retain_from
            ),
        });
    }

    // Compute diff using SnapshotDiff
    let from_reader = store_lock
        .read_at(slateduck_core::mvcc::SnapshotId::new(start_snapshot))
        .map_err(SlateDuckError::from)?;

    let diff = from_reader
        .snapshot_diff(
            slateduck_core::mvcc::SnapshotId::new(start_snapshot),
            slateduck_core::mvcc::SnapshotId::new(end_snapshot),
        )
        .await
        .map_err(SlateDuckError::from)?;

    // v0.19: Build row-level change records using real data from files.
    // Extract rows from added/retired data files and compute CDC records.
    let mut added_rows = Vec::new();
    let mut base_rowid = 0u64;
    for file in &diff.added_data_files {
        let rows = slateduck_sql::table_changes::extract_rows_from_file(
            &file.path,
            file.row_count,
            base_rowid,
            "{}",
        );
        base_rowid += file.row_count;
        added_rows.extend(rows);
    }

    let mut removed_rows = Vec::new();
    base_rowid = 0;
    for file in &diff.retired_data_files {
        let rows = slateduck_sql::table_changes::extract_rows_from_file(
            &file.path,
            file.row_count,
            base_rowid,
            "{}",
        );
        base_rowid += file.row_count;
        removed_rows.extend(rows);
    }

    let cdc_result = slateduck_sql::table_changes::compute_table_changes(
        table_ref,
        start_snapshot,
        end_snapshot,
        retain_from,
        &added_rows,
        &removed_rows,
    )
    .map_err(|e| SlateDuckError::SqlState {
        code: e.sqlstate().to_string(),
        message: e.to_string(),
    })?;

    // Build response with change records
    let schema = Arc::new(vec![
        FieldInfo::new("rowid".into(), None, None, Type::INT8, FieldFormat::Text),
        FieldInfo::new(
            "change_type".into(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "columns_json".into(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
    ]);

    let mut data_rows = Vec::new();

    for record in &cdc_result.records {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &record.rowid.map(|r| r.to_string()),
                &Type::INT8,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(record.change_type.as_str().to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(record.columns_json.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        data_rows.push(encoder.finish());
    }

    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Ok(vec![Response::Query(resp)])
}

pub(super) async fn execute_next_rowid_range<'a>(
    table_ref: &str,
    count: u64,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
) -> Result<Vec<Response<'a>>, SlateDuckError> {
    let store_lock = store.lock().await;
    let table_id = hash_table_ref(table_ref);

    let db = store_lock.db();
    let (start, end) = slateduck_catalog::next_rowid_range(db, table_id, count)
        .await
        .map_err(SlateDuckError::from)?;

    let schema = Arc::new(vec![
        FieldInfo::new(
            "start_rowid".into(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "end_rowid".into(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
    ]);
    let mut encoder = DataRowEncoder::new(schema.clone());
    encoder
        .encode_field_with_type_and_format(&Some(start.to_string()), &Type::INT8, FieldFormat::Text)
        .unwrap();
    encoder
        .encode_field_with_type_and_format(&Some(end.to_string()), &Type::INT8, FieldFormat::Text)
        .unwrap();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(vec![encoder.finish()]));
    resp.set_command_tag("SELECT 1");
    Ok(vec![Response::Query(resp)])
}
