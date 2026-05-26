//! Catalog executor operations: response builders, execute_commit, table_changes, next_rowid.

use std::sync::Arc;

use pgwire::api::results::{DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response};
use pgwire::api::Type;

use slateduck_catalog::writer::stats::FileColumnStatsInput;
use slateduck_catalog::{CatalogStore, CommitResult};

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
    let mut commit_result: Option<CommitResult> = None;

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
                initial_default,
                default_value_type,
                default_value_dialect,
                parent_column,
            } => {
                writer
                    .add_column_with_opts(
                        table_id,
                        &column_name,
                        &data_type,
                        column_index,
                        is_nullable,
                        default_value.as_deref(),
                        initial_default.as_deref(),
                        default_value_type.as_deref(),
                        default_value_dialect.as_deref(),
                        parent_column,
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
                delete_count,
                file_size_bytes,
            } => {
                writer
                    .register_delete_file(data_file_id, &path, delete_count, file_size_bytes)
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertSnapshot { author, message } => {
                let cr = writer
                    .create_snapshot(author.as_deref(), message.as_deref())
                    .await
                    .map_err(SlateDuckError::from)?;
                commit_result = Some(cr);
            }
            BufferedOp::InsertSnapshotChanges {
                change_type,
                change_info,
                schema_id,
                table_id,
            } => {
                // v0.24: persist SnapshotChangesRow transactionally (staged).
                writer
                    .add_snapshot_changes(change_type, change_info, schema_id, table_id)
                    .await
                    .map_err(SlateDuckError::from)?;
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
                row_count_delta,
            } => {
                // v0.24: apply the incoming row-count delta to existing stats.
                writer
                    .apply_table_stats_delta(table_id, row_count_delta)
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertFileColumnStats {
                table_id,
                column_id,
                data_file_id,
                contains_null,
                min_value,
                max_value,
                contains_nan,
            } => {
                writer
                    .upsert_file_column_stats(FileColumnStatsInput {
                        table_id,
                        column_id,
                        data_file_id,
                        contains_null,
                        min_value: min_value.as_deref(),
                        max_value: max_value.as_deref(),
                        contains_nan,
                        column_size_bytes: None,
                        value_count: None,
                        null_count: None,
                        extra_stats: None,
                    })
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertMetadata {
                key,
                value,
                scope,
                scope_id,
            } => {
                use slateduck_core::keys::MetadataScope;
                let resolved_scope = match scope.as_deref() {
                    Some("schema") => MetadataScope::Schema,
                    Some("table") => MetadataScope::Table,
                    _ => MetadataScope::Global,
                };
                let resolved_scope_id = scope_id.unwrap_or(0);
                writer
                    .set_metadata(resolved_scope, resolved_scope_id, &key, &value)
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertInlinedDataTables { .. } => {
                // Inlined data table registration accepted (persisted via future InlinedDataTablesRow writer)
            }
            BufferedOp::InsertView {
                schema_id,
                view_name,
                sql,
                view_uuid,
                dialect,
                column_aliases,
            } => {
                writer
                    .create_view_with_opts(
                        schema_id,
                        &view_name,
                        &sql,
                        view_uuid.as_deref(),
                        dialect.as_deref(),
                        column_aliases.as_deref(),
                    )
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertMacro {
                schema_id,
                macro_name,
                macro_type,
                macro_uuid: _,
            } => {
                writer
                    .create_macro(schema_id, &macro_name, &macro_type)
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertMacroImpl {
                macro_id,
                sql,
                dialect,
                impl_type,
            } => {
                writer
                    .add_macro_impl_with_opts(
                        macro_id,
                        &sql,
                        dialect.as_deref(),
                        impl_type.as_deref(),
                    )
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertMacroParams {
                macro_id,
                impl_id,
                column_id,
                parameter_name,
                parameter_type,
                default_value,
                default_value_type,
            } => {
                writer
                    .add_macro_parameter_with_opts(
                        macro_id,
                        impl_id,
                        column_id,
                        &parameter_name,
                        &parameter_type,
                        default_value.as_deref(),
                        default_value_type.as_deref(),
                    )
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertTableStats {
                table_id,
                record_count,
                file_count,
                file_size_bytes,
            } => {
                writer
                    .update_table_stats(table_id, record_count, file_count, file_size_bytes)
                    .await
                    .map_err(SlateDuckError::from)?;
            }
        }
    }
    // Synchronise the store's in-memory counters from the committed snapshot
    // (F-01: ensures read_latest() and subsequent begin_write() see the new state).
    if let Some(cr) = commit_result {
        s.commit_writer(cr);
    }
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
        FieldInfo::new(
            "next_catalog_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "next_file_id".to_string(),
            None,
            None,
            Type::INT8,
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
        .expect("pgwire field encoding is infallible");
    encoder
        .encode_field_with_type_and_format(
            &Some(snap.schema_version.to_string()),
            &Type::TEXT,
            FieldFormat::Text,
        )
        .expect("pgwire field encoding is infallible");
    encoder
        .encode_field_with_type_and_format(
            &Some(snap.snapshot_time.clone()),
            &Type::TEXT,
            FieldFormat::Text,
        )
        .expect("pgwire field encoding is infallible");
    encoder
        .encode_field_with_type_and_format(&snap.author, &Type::TEXT, FieldFormat::Text)
        .expect("pgwire field encoding is infallible");
    encoder
        .encode_field_with_type_and_format(&snap.message, &Type::TEXT, FieldFormat::Text)
        .expect("pgwire field encoding is infallible");
    let next_catalog_id = snap.next_catalog_id.map(|v| v.to_string());
    encoder
        .encode_field_with_type_and_format(&next_catalog_id, &Type::TEXT, FieldFormat::Text)
        .expect("pgwire field encoding is infallible");
    let next_file_id = snap.next_file_id.map(|v| v.to_string());
    encoder
        .encode_field_with_type_and_format(&next_file_id, &Type::TEXT, FieldFormat::Text)
        .expect("pgwire field encoding is infallible");
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
            "schema_uuid".to_string(),
            None,
            None,
            Type::TEXT,
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
            "path".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "path_is_relative".to_string(),
            None,
            None,
            Type::BOOL,
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
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(s.begin_snapshot.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let end = s.end_snapshot.map(|e| e.to_string());
        encoder
            .encode_field_with_type_and_format(&end, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&s.schema_uuid, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(s.schema_name.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&s.path, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        let pir = s.path_is_relative.map(|b| b.to_string());
        encoder
            .encode_field_with_type_and_format(&pir, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
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
            "table_uuid".to_string(),
            None,
            None,
            Type::TEXT,
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
            "path_is_relative".to_string(),
            None,
            None,
            Type::BOOL,
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
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(t.begin_snapshot.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let end = t.end_snapshot.map(|e| e.to_string());
        encoder
            .encode_field_with_type_and_format(&end, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(t.schema_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(t.table_name.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&t.table_uuid, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&t.path, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        let pir = t.path_is_relative.map(|b| b.to_string());
        encoder
            .encode_field_with_type_and_format(&pir, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
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
            "column_type".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "column_order".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "nulls_allowed".to_string(),
            None,
            None,
            Type::BOOL,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "initial_default".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "default_value_type".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "default_value_dialect".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "parent_column".to_string(),
            None,
            None,
            Type::INT8,
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
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(c.begin_snapshot.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let end = c.end_snapshot.map(|e| e.to_string());
        encoder
            .encode_field_with_type_and_format(&end, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(c.table_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(c.column_name.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(c.data_type.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(c.column_index.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(c.is_nullable.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&c.initial_default, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &c.default_value_type,
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &c.default_value_dialect,
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let parent = c.parent_column.map(|v| v.to_string());
        encoder
            .encode_field_with_type_and_format(&parent, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
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
            "file_order".to_string(),
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
            "path_is_relative".to_string(),
            None,
            None,
            Type::BOOL,
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
            "record_count".to_string(),
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
            "row_id_start".to_string(),
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
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(f.table_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let begin = f.begin_snapshot.map(|s| s.to_string());
        encoder
            .encode_field_with_type_and_format(&begin, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        let end = f.end_snapshot.map(|e| e.to_string());
        encoder
            .encode_field_with_type_and_format(&end, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        let file_order = f.file_order.map(|o| o.to_string());
        encoder
            .encode_field_with_type_and_format(&file_order, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(f.path.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let path_is_relative = f.path_is_relative.map(|b| b.to_string());
        encoder
            .encode_field_with_type_and_format(&path_is_relative, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(f.file_format.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(f.record_count.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(f.file_size_bytes.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let row_id_start = f.row_id_start.map(|r| r.to_string());
        encoder
            .encode_field_with_type_and_format(&row_id_start, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
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
            .expect("pgwire field encoding is infallible");
        data_rows.push(encoder.finish());
    }
    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

// ─── v0.27.1: CDC Completeness & Real Parquet Row Scanning ─────────────────

/// Execute `table_changes(table_ref, start_snapshot, end_snapshot)`.
///
/// v0.27.1: Uses real Parquet scanning via the catalog's `ObjectStore`.
/// File paths are resolved relative to the same object store root used to
/// open the catalog. Pass `data_root` as `None` to use the catalog's own
/// object store directly (the common case for relative file paths).
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

    // v0.27.1: Extract rows from Parquet data files using the catalog's object store.
    // The object store root is shared between catalog metadata and data files
    // when data files are registered with relative paths.
    let object_store = store_lock.object_store();
    drop(store_lock); // release lock before async I/O

    let mut added_rows = Vec::new();
    let mut base_rowid = 0u64;
    for file in &diff.added_data_files {
        let rows = slateduck_sql::table_changes::extract_rows_from_parquet(
            &object_store,
            &file.path,
            base_rowid,
            Some(file.record_count),
            slateduck_sql::DEFAULT_CDC_BATCH_SIZE,
        )
        .await
        .map_err(|e| SlateDuckError::SqlState {
            code: e.sqlstate().to_string(),
            message: e.to_string(),
        })?;
        base_rowid += rows.len() as u64;
        added_rows.extend(rows);
    }

    let mut removed_rows = Vec::new();
    base_rowid = 0;
    for file in &diff.retired_data_files {
        let rows = slateduck_sql::table_changes::extract_rows_from_parquet(
            &object_store,
            &file.path,
            base_rowid,
            Some(file.record_count),
            slateduck_sql::DEFAULT_CDC_BATCH_SIZE,
        )
        .await
        .map_err(|e| SlateDuckError::SqlState {
            code: e.sqlstate().to_string(),
            message: e.to_string(),
        })?;
        base_rowid += rows.len() as u64;
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
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(record.change_type.as_str().to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(record.columns_json.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
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
        .expect("pgwire field encoding is infallible");
    encoder
        .encode_field_with_type_and_format(&Some(end.to_string()), &Type::INT8, FieldFormat::Text)
        .expect("pgwire field encoding is infallible");
    let mut resp = QueryResponse::new(schema, futures::stream::iter(vec![encoder.finish()]));
    resp.set_command_tag("SELECT 1");
    Ok(vec![Response::Query(resp)])
}

/// Build a PgWire response for `SELECT * FROM ducklake_table_stats WHERE table_id = $1`.
pub(super) fn make_table_stats_response(
    stats: Option<slateduck_core::rows::TableStatsRow>,
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
            "record_count".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "file_count".to_string(),
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
            "next_row_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
    ]);
    let data_rows = if let Some(s) = stats {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(s.table_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(s.record_count.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(s.file_count.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(s.file_size_bytes.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let next_row_id = s.next_row_id.map(|v| v.to_string());
        encoder
            .encode_field_with_type_and_format(&next_row_id, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        vec![encoder.finish()]
    } else {
        vec![]
    };
    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

/// Build a PgWire response for `SELECT * FROM ducklake_delete_file WHERE table_id = $1`.
pub(super) fn make_delete_files_response(
    files: Vec<slateduck_core::rows::DeleteFileRow>,
) -> Response<'static> {
    let schema = Arc::new(vec![
        FieldInfo::new(
            "delete_file_id".to_string(),
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
            "delete_count".to_string(),
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
    for f in &files {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(f.delete_file_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let tid = f.table_id.map(|t| t.to_string());
        encoder
            .encode_field_with_type_and_format(&tid, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(f.path.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(f.delete_count.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(f.file_size_bytes.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let begin = f.begin_snapshot.map(|b| b.to_string());
        encoder
            .encode_field_with_type_and_format(&begin, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        let end = f.end_snapshot.map(|e| e.to_string());
        encoder
            .encode_field_with_type_and_format(&end, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        data_rows.push(encoder.finish());
    }
    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

/// v0.25: Build a PgWire response for `SELECT * FROM ducklake_metadata`.
pub(super) fn make_metadata_response(
    rows: Vec<slateduck_core::rows::MetadataRow>,
) -> Response<'static> {
    let schema = Arc::new(vec![
        FieldInfo::new(
            "metadata_key".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "metadata_value".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "scope".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "scope_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
    ]);
    let mut data_rows = Vec::new();
    for r in &rows {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(&Some(r.key.clone()), &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(r.value.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let scope_str = r.scope.clone().or_else(|| Some("global".to_string()));
        encoder
            .encode_field_with_type_and_format(&scope_str, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        let scope_id = r.scope_id.map(|v| v.to_string());
        encoder
            .encode_field_with_type_and_format(&scope_id, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        data_rows.push(encoder.finish());
    }
    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

/// v0.25: Build a PgWire response for `SELECT * FROM ducklake_view`.
pub(super) fn make_views_response(views: Vec<slateduck_core::rows::ViewRow>) -> Response<'static> {
    let schema = Arc::new(vec![
        FieldInfo::new(
            "view_id".to_string(),
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
            "schema_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "view_name".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "view_uuid".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "view_definition".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "dialect".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "column_aliases".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
    ]);
    let mut data_rows = Vec::new();
    for v in &views {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(v.view_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(v.begin_snapshot.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let end = v.end_snapshot.map(|e| e.to_string());
        encoder
            .encode_field_with_type_and_format(&end, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(v.schema_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(v.view_name.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&v.view_uuid, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&Some(v.sql.clone()), &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&v.dialect, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&v.column_aliases, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        data_rows.push(encoder.finish());
    }
    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

/// v0.25: Build a PgWire response for `SELECT * FROM ducklake_macro`.
pub(super) fn make_macros_response(
    macros: Vec<slateduck_core::rows::MacroRow>,
) -> Response<'static> {
    let schema = Arc::new(vec![
        FieldInfo::new(
            "macro_id".to_string(),
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
            "schema_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "macro_name".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "macro_uuid".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
    ]);
    let mut data_rows = Vec::new();
    for m in &macros {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(m.macro_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(m.begin_snapshot.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let end = m.end_snapshot.map(|e| e.to_string());
        encoder
            .encode_field_with_type_and_format(&end, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(m.schema_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(m.macro_name.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&m.macro_uuid, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        data_rows.push(encoder.finish());
    }
    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

// ─── v0.27: ducklake_tag / ducklake_column_tag / ducklake_sort_info ──────────

/// v0.27: Build a PgWire response for `SELECT * FROM ducklake_tag`.
///
/// Spec column names: tag_id, begin_snapshot, end_snapshot, object_id,
/// tag_name, tag_value.  The internal `TagRow.tag_key` is exposed as `tag_name`.
/// `tag_id` is synthesized as `object_id`.
pub(super) fn make_tags_response(rows: Vec<slateduck_core::rows::TagRow>) -> Response<'static> {
    let schema = Arc::new(vec![
        FieldInfo::new(
            "tag_id".to_string(),
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
            "object_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "tag_name".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "tag_value".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
    ]);
    let mut data_rows = Vec::new();
    for r in &rows {
        let mut encoder = DataRowEncoder::new(schema.clone());
        // tag_id synthesized as object_id (stable surrogate within this snapshot)
        encoder
            .encode_field_with_type_and_format(
                &Some(r.object_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(r.begin_snapshot.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let end = r.end_snapshot.map(|e| e.to_string());
        encoder
            .encode_field_with_type_and_format(&end, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(r.object_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        // Internal field tag_key is exposed as spec column tag_name.
        encoder
            .encode_field_with_type_and_format(
                &Some(r.tag_key.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(r.tag_value.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        data_rows.push(encoder.finish());
    }
    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

/// v0.27: Build a PgWire response for `SELECT * FROM ducklake_column_tag`.
///
/// Spec column names: tag_id, begin_snapshot, end_snapshot, column_id,
/// tag_name, tag_value.  The internal `ColumnTagRow.tag_key` is exposed as `tag_name`.
/// `tag_id` is synthesized as `column_id`.
pub(super) fn make_column_tags_response(
    rows: Vec<slateduck_core::rows::ColumnTagRow>,
) -> Response<'static> {
    let schema = Arc::new(vec![
        FieldInfo::new(
            "tag_id".to_string(),
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
            "column_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "tag_name".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "tag_value".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
    ]);
    let mut data_rows = Vec::new();
    for r in &rows {
        let mut encoder = DataRowEncoder::new(schema.clone());
        // tag_id synthesized as column_id (stable surrogate within this snapshot)
        encoder
            .encode_field_with_type_and_format(
                &Some(r.column_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(r.begin_snapshot.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let end = r.end_snapshot.map(|e| e.to_string());
        encoder
            .encode_field_with_type_and_format(&end, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(r.column_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        // Internal field tag_key is exposed as spec column tag_name.
        encoder
            .encode_field_with_type_and_format(
                &Some(r.tag_key.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(r.tag_value.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        data_rows.push(encoder.finish());
    }
    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

/// v0.27: Build a PgWire response for `SELECT * FROM ducklake_sort_info`.
///
/// Spec column names: sort_id, begin_snapshot, end_snapshot, table_id,
/// sort_order, column_id.
/// `SortInfoRow` carries `sort_id`, `table_id`, MVCC windows.
/// `sort_order` and `column_id` default to 0 until sort-expression storage is
/// implemented in a future version.
pub(super) fn make_sort_info_response(
    rows: Vec<slateduck_core::rows::SortInfoRow>,
) -> Response<'static> {
    let schema = Arc::new(vec![
        FieldInfo::new(
            "sort_id".to_string(),
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
            "table_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "sort_order".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "column_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
    ]);
    let mut data_rows = Vec::new();
    for r in &rows {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(r.sort_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(r.begin_snapshot.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let end = r.end_snapshot.map(|e| e.to_string());
        encoder
            .encode_field_with_type_and_format(&end, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(r.table_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        // sort_order and column_id: default to 0 until sort-expression rows are stored.
        encoder
            .encode_field_with_type_and_format(
                &Some("0".to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some("0".to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        data_rows.push(encoder.finish());
    }
    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

/// v0.27: Build a PgWire response for `SELECT * FROM ducklake_schema_version`.
///
/// Spec column names: schema_version, schema_version_info.
/// Returns the single global catalog schema version row.
pub(super) fn make_schema_version_response(catalog_schema_version: u64) -> Response<'static> {
    let schema = Arc::new(vec![
        FieldInfo::new(
            "schema_version".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "schema_version_info".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
    ]);
    let mut encoder = DataRowEncoder::new(schema.clone());
    encoder
        .encode_field_with_type_and_format(
            &Some(catalog_schema_version.to_string()),
            &Type::TEXT,
            FieldFormat::Text,
        )
        .expect("pgwire field encoding is infallible");
    // Human-readable description of the DuckLake catalog schema version.
    let info = Some(format!("DuckLake catalog schema v{catalog_schema_version}"));
    encoder
        .encode_field_with_type_and_format(&info, &Type::TEXT, FieldFormat::Text)
        .expect("pgwire field encoding is infallible");
    let data_rows = vec![encoder.finish()];
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag("SELECT 1");
    Response::Query(resp)
}
