//! Catalog executor operations: response builders, execute_commit, table_changes, next_rowid.

use std::sync::Arc;

use pgwire::api::results::{DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response};
use pgwire::api::Type;
use sqlparser::ast::{Expr, SelectItem, SetExpr, Statement};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

use slateduck_catalog::writer::stats::FileColumnStatsInput;
use slateduck_catalog::{CatalogStore, CommitResult};
use slateduck_core::rows::{ColumnRow, InlinedDataTablesRow, InlinedInsertRow};

use crate::error::SlateDuckError;
use crate::notify::NotifyManager;
use crate::session::BufferedOp;

use super::extension::hash_table_ref;

pub(super) fn parse_inlined_table_ids(table_name: &str) -> Option<(u64, u64)> {
    let normalized = table_name.trim_matches('"').to_ascii_lowercase();
    let suffix = normalized.strip_prefix("ducklake_inlined_data_")?;
    let (table_id, schema_version) = suffix.rsplit_once('_')?;
    Some((table_id.parse().ok()?, schema_version.parse().ok()?))
}

fn literal_u64_value(value: Option<&str>) -> Option<u64> {
    value?.parse::<u64>().ok()
}

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
    let mut snapshot_author: Option<String> = None;
    let mut snapshot_message: Option<String> = None;
    let mut needs_snapshot = false;

    for op in ops {
        match op {
            BufferedOp::InsertSchema { schema_name } => {
                needs_snapshot = true;
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
                needs_snapshot = true;
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
                needs_snapshot = true;
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
                needs_snapshot = true;
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
                needs_snapshot = true;
                writer
                    .register_delete_file(data_file_id, &path, delete_count, file_size_bytes)
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertSnapshot { author, message } => {
                needs_snapshot = true;
                snapshot_author = author;
                snapshot_message = message;
            }
            BufferedOp::InsertSnapshotChanges {
                change_type,
                change_info,
                schema_id,
                table_id,
            } => {
                needs_snapshot = true;
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
                needs_snapshot = true;
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
                needs_snapshot = true;
                // v0.24: apply the incoming row-count delta to existing stats.
                writer
                    .apply_table_stats_delta(table_id, row_count_delta)
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::SetTableStats {
                table_id,
                record_count,
                file_size_bytes,
                next_row_id,
            } => {
                needs_snapshot = true;
                writer
                    .set_table_stats(table_id, record_count, file_size_bytes, next_row_id)
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
                needs_snapshot = true;
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
            BufferedOp::InsertTableColumnStats {
                table_id,
                column_id,
                contains_null,
                contains_nan,
                min_value,
                max_value,
                extra_stats,
            } => {
                writer
                    .upsert_table_column_stats(
                        table_id,
                        column_id,
                        contains_null,
                        min_value.as_deref(),
                        max_value.as_deref(),
                        contains_nan,
                        extra_stats.as_deref(),
                    )
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertMetadata {
                key,
                value,
                scope,
                scope_id,
            } => {
                needs_snapshot = true;
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
            BufferedOp::InsertInlinedDataTables {
                table_id,
                table_name,
                schema_version,
            } => {
                needs_snapshot = true;
                writer
                    .register_inlined_data_table(table_id, &table_name, schema_version)
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertSchemaVersions {
                begin_snapshot,
                schema_version,
                table_id,
            } => {
                needs_snapshot = true;
                writer
                    .register_schema_version(begin_snapshot, schema_version, table_id)
                    .await
                    .map_err(SlateDuckError::from)?;
            }
            BufferedOp::InsertInlinedRow { table_name, rows } => {
                needs_snapshot = true;
                if let Some((table_id, schema_version)) = parse_inlined_table_ids(&table_name) {
                    for values in rows {
                        let Some(row_id) =
                            literal_u64_value(values.first().and_then(|v| v.as_deref()))
                        else {
                            continue;
                        };
                        let payload =
                            serde_json::to_vec(&values.iter().skip(3).cloned().collect::<Vec<_>>())
                                .map_err(|err| SlateDuckError::Internal(err.to_string()))?;
                        writer
                            .register_inlined_insert(table_id, schema_version, row_id, payload)
                            .await
                            .map_err(SlateDuckError::from)?;
                    }
                }
            }
            BufferedOp::InsertView {
                schema_id,
                view_name,
                sql,
                view_uuid,
                dialect,
                column_aliases,
            } => {
                needs_snapshot = true;
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
                needs_snapshot = true;
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
                needs_snapshot = true;
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
                needs_snapshot = true;
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
    if needs_snapshot {
        let cr = writer
            .create_snapshot(snapshot_author.as_deref(), snapshot_message.as_deref())
            .await
            .map_err(SlateDuckError::from)?;
        commit_result = Some(cr);
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

pub(super) fn make_latest_snapshot_info_response(
    snap: Option<slateduck_core::rows::SnapshotRow>,
) -> Response<'static> {
    let schema = Arc::new(vec![
        FieldInfo::new(
            "snapshot_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Binary,
        ),
        FieldInfo::new(
            "schema_version".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Binary,
        ),
        FieldInfo::new(
            "next_catalog_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Binary,
        ),
        FieldInfo::new(
            "next_file_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Binary,
        ),
    ]);

    let (snapshot_id, schema_version, next_catalog_id, next_file_id) = match snap {
        Some(s) => (
            s.snapshot_id as i64,
            s.schema_version as i64,
            s.next_catalog_id.map(|v| v as i64).unwrap_or(1),
            s.next_file_id.map(|v| v as i64).unwrap_or(1),
        ),
        None => {
            // No snapshot yet — return an empty result set (0 rows) so that
            // DuckDB knows the catalog is uninitialised.  Returning a fake
            // snapshot causes DuckDB to crash when it tries to look up schemas.
            let mut resp = QueryResponse::new(schema, futures::stream::iter(vec![]));
            resp.set_command_tag("SELECT 0");
            return Response::Query(resp);
        }
    };

    let mut encoder = DataRowEncoder::new(schema.clone());
    encoder
        .encode_field_with_type_and_format(&Some(snapshot_id), &Type::INT8, FieldFormat::Binary)
        .expect("pgwire field encoding is infallible");
    encoder
        .encode_field_with_type_and_format(&Some(schema_version), &Type::INT8, FieldFormat::Binary)
        .expect("pgwire field encoding is infallible");
    encoder
        .encode_field_with_type_and_format(&Some(next_catalog_id), &Type::INT8, FieldFormat::Binary)
        .expect("pgwire field encoding is infallible");
    encoder
        .encode_field_with_type_and_format(&Some(next_file_id), &Type::INT8, FieldFormat::Binary)
        .expect("pgwire field encoding is infallible");
    let data_rows = vec![encoder.finish()];

    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
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
            FieldFormat::Binary,
        ),
        FieldInfo::new(
            "begin_snapshot".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Binary,
        ),
        FieldInfo::new(
            "end_snapshot".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Binary,
        ),
        FieldInfo::new(
            "schema_uuid".to_string(),
            None,
            None,
            Type::UUID,
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
                &Some(s.schema_id as i64),
                &Type::INT8,
                FieldFormat::Binary,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(s.begin_snapshot as i64),
                &Type::INT8,
                FieldFormat::Binary,
            )
            .expect("pgwire field encoding is infallible");
        let end = s.end_snapshot.map(|e| e as i64);
        encoder
            .encode_field_with_type_and_format(&end, &Type::INT8, FieldFormat::Binary)
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
            FieldFormat::Binary,
        ),
        FieldInfo::new(
            "begin_snapshot".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Binary,
        ),
        FieldInfo::new(
            "end_snapshot".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Binary,
        ),
        FieldInfo::new(
            "schema_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Binary,
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
            Type::UUID,
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
                &Some(t.table_id as i64),
                &Type::INT8,
                FieldFormat::Binary,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(t.begin_snapshot as i64),
                &Type::INT8,
                FieldFormat::Binary,
            )
            .expect("pgwire field encoding is infallible");
        let end = t.end_snapshot.map(|e| e as i64);
        encoder
            .encode_field_with_type_and_format(&end, &Type::INT8, FieldFormat::Binary)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(t.schema_id as i64),
                &Type::INT8,
                FieldFormat::Binary,
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
            "column_order".to_string(),
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
            "initial_default".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "default_value".to_string(),
            None,
            None,
            Type::TEXT,
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
            "parent_column".to_string(),
            None,
            None,
            Type::INT8,
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
                &Some(c.column_index.to_string()),
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
            .encode_field_with_type_and_format(&c.initial_default, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&c.default_value, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(c.is_nullable.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let parent = c.parent_column.map(|v| v.to_string());
        encoder
            .encode_field_with_type_and_format(&parent, &Type::TEXT, FieldFormat::Text)
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
    make_table_stats_rows_response(stats.into_iter().collect())
}

pub(super) fn make_table_stats_rows_response(
    rows: Vec<slateduck_core::rows::TableStatsRow>,
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
    let mut data_rows = Vec::new();
    for s in rows {
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
        data_rows.push(encoder.finish());
    }
    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

pub(super) fn make_table_column_stats_response(
    rows: Vec<slateduck_core::rows::TableColumnStatsRow>,
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
            "column_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "contains_null".to_string(),
            None,
            None,
            Type::BOOL,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "contains_nan".to_string(),
            None,
            None,
            Type::BOOL,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "min_value".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "max_value".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "extra_stats".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
    ]);
    let mut data_rows = Vec::new();
    for row in &rows {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(row.table_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(row.column_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(row.contains_null.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let contains_nan = row.contains_nan.map(|value| value.to_string());
        encoder
            .encode_field_with_type_and_format(&contains_nan, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&row.min_value, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&row.max_value, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&row.extra_stats, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        data_rows.push(encoder.finish());
    }
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
            Type::UUID,
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

pub(super) fn make_macro_impls_response(
    rows: Vec<slateduck_core::rows::MacroImplRow>,
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
            "impl_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "dialect".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new("sql".to_string(), None, None, Type::TEXT, FieldFormat::Text),
        FieldInfo::new(
            "type".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
    ]);
    let mut data_rows = Vec::new();
    for row in &rows {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(row.macro_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(row.impl_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&row.dialect, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(row.sql.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&row.impl_type, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        data_rows.push(encoder.finish());
    }
    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

pub(super) fn make_macro_parameters_response(
    rows: Vec<slateduck_core::rows::MacroParametersRow>,
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
            "impl_id".to_string(),
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
            "parameter_name".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "parameter_type".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "default_value".to_string(),
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
    ]);
    let mut data_rows = Vec::new();
    for row in &rows {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(row.macro_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(row.impl_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(row.column_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(row.parameter_name.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(row.parameter_type.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&row.default_value, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &row.default_value_type,
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

#[derive(Clone)]
enum InlinedProjectionSource {
    RowId,
    BeginSnapshot,
    EndSnapshot,
    Column(usize),
}

#[derive(Clone)]
struct InlinedProjection {
    name: String,
    datatype: Type,
    source: InlinedProjectionSource,
}

pub(super) fn make_inlined_data_tables_response(
    rows: Vec<InlinedDataTablesRow>,
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
            "table_name".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "schema_version".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
    ]);
    let mut data_rows = Vec::new();
    for row in &rows {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(row.table_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(row.sql.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(row.schema_version.to_string()),
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

pub(super) fn make_schema_versions_response(
    rows: Vec<slateduck_core::rows::SchemaVersionsRow>,
) -> Response<'static> {
    let schema = Arc::new(vec![
        FieldInfo::new(
            "begin_snapshot".to_string(),
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
            "table_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
    ]);
    let mut data_rows = Vec::new();
    for row in &rows {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(row.begin_snapshot.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(row.schema_version.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &Some(row.table_id.to_string()),
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

pub(super) fn make_inlined_rows_response(
    sql: &str,
    columns: Vec<ColumnRow>,
    rows: Vec<InlinedInsertRow>,
) -> Response<'static> {
    let projections = inlined_projections(sql, &columns);
    let schema = Arc::new(
        projections
            .iter()
            .map(|projection| {
                FieldInfo::new(
                    projection.name.clone(),
                    None,
                    None,
                    projection.datatype.clone(),
                    FieldFormat::Binary,
                )
            })
            .collect::<Vec<_>>(),
    );

    let mut data_rows = Vec::new();
    for row in &rows {
        let values =
            serde_json::from_slice::<Vec<Option<String>>>(&row.payload).unwrap_or_default();
        let mut encoder = DataRowEncoder::new(schema.clone());
        for projection in &projections {
            match projection.source {
                InlinedProjectionSource::RowId => {
                    let value = Some(row.row_id as i64);
                    encoder
                        .encode_field_with_type_and_format(&value, &Type::INT8, FieldFormat::Binary)
                        .expect("pgwire field encoding is infallible");
                }
                InlinedProjectionSource::BeginSnapshot => {
                    let value = Some(row.begin_snapshot as i64);
                    encoder
                        .encode_field_with_type_and_format(&value, &Type::INT8, FieldFormat::Binary)
                        .expect("pgwire field encoding is infallible");
                }
                InlinedProjectionSource::EndSnapshot => {
                    let value = row.end_snapshot.map(|snapshot| snapshot as i64);
                    encoder
                        .encode_field_with_type_and_format(&value, &Type::INT8, FieldFormat::Binary)
                        .expect("pgwire field encoding is infallible");
                }
                InlinedProjectionSource::Column(index) => {
                    let value = values.get(index).cloned().flatten();
                    encode_inlined_column_value(
                        &mut encoder,
                        value.as_deref(),
                        &projection.datatype,
                    );
                }
            }
        }
        data_rows.push(encoder.finish());
    }

    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

fn inlined_projections(sql: &str, columns: &[ColumnRow]) -> Vec<InlinedProjection> {
    let items = select_projection_items(sql).unwrap_or_default();
    let mut projections = Vec::new();
    for item in items {
        match item {
            SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(_, _) => {
                projections.extend(default_inlined_column_projections(columns));
            }
            SelectItem::UnnamedExpr(expr) => {
                let source_name = expr_last_identifier(&expr);
                if let Some(projection) =
                    inlined_projection_for_name(&source_name, &source_name, columns)
                {
                    projections.push(projection);
                }
            }
            SelectItem::ExprWithAlias { expr, alias } => {
                let source_name = expr_last_identifier(&expr);
                if let Some(projection) =
                    inlined_projection_for_name(&source_name, &alias.value, columns)
                {
                    projections.push(projection);
                }
            }
        }
    }

    if projections.is_empty() {
        default_inlined_column_projections(columns)
    } else {
        projections
    }
}

fn select_projection_items(sql: &str) -> Option<Vec<SelectItem>> {
    let dialect = PostgreSqlDialect {};
    let mut statements = Parser::parse_sql(&dialect, sql).ok()?;
    let statement = statements.pop()?;
    let Statement::Query(query) = statement else {
        return None;
    };
    let SetExpr::Select(select) = query.body.as_ref() else {
        return None;
    };
    Some(select.projection.clone())
}

fn default_inlined_column_projections(columns: &[ColumnRow]) -> Vec<InlinedProjection> {
    columns
        .iter()
        .enumerate()
        .map(|(index, column)| InlinedProjection {
            name: column.column_name.clone(),
            datatype: inlined_storage_type(&column.data_type),
            source: InlinedProjectionSource::Column(index),
        })
        .collect()
}

fn inlined_projection_for_name(
    source_name: &str,
    output_name: &str,
    columns: &[ColumnRow],
) -> Option<InlinedProjection> {
    let source_name = source_name.trim_matches('"').to_ascii_lowercase();
    let output_name = output_name.trim_matches('"').to_string();
    match source_name.as_str() {
        "row_id" => Some(InlinedProjection {
            name: output_name,
            datatype: Type::INT8,
            source: InlinedProjectionSource::RowId,
        }),
        "begin_snapshot" => Some(InlinedProjection {
            name: output_name,
            datatype: Type::INT8,
            source: InlinedProjectionSource::BeginSnapshot,
        }),
        "end_snapshot" => Some(InlinedProjection {
            name: output_name,
            datatype: Type::INT8,
            source: InlinedProjectionSource::EndSnapshot,
        }),
        _ => columns
            .iter()
            .position(|column| column.column_name.eq_ignore_ascii_case(&source_name))
            .map(|index| InlinedProjection {
                name: output_name,
                datatype: inlined_storage_type(&columns[index].data_type),
                source: InlinedProjectionSource::Column(index),
            }),
    }
}

fn expr_last_identifier(expr: &Expr) -> String {
    match expr {
        Expr::Identifier(id) => id.value.to_ascii_lowercase(),
        Expr::CompoundIdentifier(parts) => parts
            .last()
            .map(|id| id.value.to_ascii_lowercase())
            .unwrap_or_default(),
        _ => expr.to_string().to_ascii_lowercase(),
    }
}

fn inlined_storage_type(logical_type: &str) -> Type {
    match logical_type.to_ascii_uppercase().as_str() {
        "BOOLEAN" | "BOOL" => Type::BOOL,
        "TINYINT" | "SMALLINT" | "INT2" | "INT16" => Type::INT2,
        "INTEGER" | "INT" | "INT4" | "INT32" => Type::INT4,
        "BIGINT" | "INT8" | "INT64" => Type::INT8,
        "VARCHAR" | "TEXT" | "STRING" | "BLOB" | "BYTEA" => Type::BYTEA,
        _ => Type::TEXT,
    }
}

fn encode_inlined_column_value(encoder: &mut DataRowEncoder, value: Option<&str>, datatype: &Type) {
    match datatype {
        datatype if datatype == &Type::BOOL => {
            let value = value.and_then(parse_bool);
            encoder
                .encode_field_with_type_and_format(&value, &Type::BOOL, FieldFormat::Binary)
                .expect("pgwire field encoding is infallible");
        }
        datatype if datatype == &Type::INT2 => {
            let value = value.and_then(|value| value.parse::<i16>().ok());
            encoder
                .encode_field_with_type_and_format(&value, &Type::INT2, FieldFormat::Binary)
                .expect("pgwire field encoding is infallible");
        }
        datatype if datatype == &Type::INT4 => {
            let value = value.and_then(|value| value.parse::<i32>().ok());
            encoder
                .encode_field_with_type_and_format(&value, &Type::INT4, FieldFormat::Binary)
                .expect("pgwire field encoding is infallible");
        }
        datatype if datatype == &Type::INT8 => {
            let value = value.and_then(|value| value.parse::<i64>().ok());
            encoder
                .encode_field_with_type_and_format(&value, &Type::INT8, FieldFormat::Binary)
                .expect("pgwire field encoding is infallible");
        }
        datatype if datatype == &Type::BYTEA => {
            let value = value.map(decode_bytea_literal);
            encoder
                .encode_field_with_type_and_format(&value, &Type::BYTEA, FieldFormat::Binary)
                .expect("pgwire field encoding is infallible");
        }
        _ => {
            let value = value.map(|value| value.to_string());
            encoder
                .encode_field_with_type_and_format(&value, &Type::TEXT, FieldFormat::Text)
                .expect("pgwire field encoding is infallible");
        }
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "t" | "1" => Some(true),
        "false" | "f" | "0" => Some(false),
        _ => None,
    }
}

fn decode_bytea_literal(value: &str) -> Vec<u8> {
    let value = value.strip_prefix("\\x").unwrap_or(value);
    if value.len() % 2 == 0 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        let mut bytes = Vec::with_capacity(value.len() / 2);
        let mut chars = value.as_bytes().chunks_exact(2);
        for pair in &mut chars {
            let Ok(hex) = std::str::from_utf8(pair) else {
                return value.as_bytes().to_vec();
            };
            let Ok(byte) = u8::from_str_radix(hex, 16) else {
                return value.as_bytes().to_vec();
            };
            bytes.push(byte);
        }
        bytes
    } else {
        value.as_bytes().to_vec()
    }
}

pub(super) fn make_metadata_table_empty_response(table_name: &str) -> Response<'static> {
    fn field(name: &str, datatype: Type) -> FieldInfo {
        FieldInfo::new(name.to_string(), None, None, datatype, FieldFormat::Text)
    }
    fn int8(name: &str) -> FieldInfo {
        field(name, Type::INT8)
    }
    fn text(name: &str) -> FieldInfo {
        field(name, Type::TEXT)
    }
    fn bool_col(name: &str) -> FieldInfo {
        field(name, Type::BOOL)
    }
    fn timestamp_tz(name: &str) -> FieldInfo {
        field(name, Type::TIMESTAMPTZ)
    }

    let schema = match table_name {
        "ducklake_snapshot_changes" => vec![
            int8("snapshot_id"),
            text("changes_made"),
            text("author"),
            text("commit_message"),
            text("commit_extra_info"),
        ],
        "ducklake_file_variant_stats" => vec![
            int8("data_file_id"),
            int8("table_id"),
            int8("column_id"),
            text("variant_path"),
            text("shredded_type"),
            int8("column_size_bytes"),
            int8("value_count"),
            int8("null_count"),
            text("min_value"),
            text("max_value"),
            bool_col("contains_nan"),
            text("extra_stats"),
        ],
        "ducklake_file_column_stats" => vec![
            int8("data_file_id"),
            int8("table_id"),
            int8("column_id"),
            int8("column_size_bytes"),
            int8("value_count"),
            int8("null_count"),
            text("min_value"),
            text("max_value"),
            bool_col("contains_nan"),
            text("extra_stats"),
        ],
        "ducklake_table_column_stats" => vec![
            int8("table_id"),
            int8("column_id"),
            bool_col("contains_null"),
            bool_col("contains_nan"),
            text("min_value"),
            text("max_value"),
            text("extra_stats"),
        ],
        "ducklake_partition_info" => vec![
            int8("partition_id"),
            int8("table_id"),
            int8("begin_snapshot"),
            int8("end_snapshot"),
        ],
        "ducklake_partition_column" => vec![
            int8("partition_id"),
            int8("table_id"),
            int8("partition_key_index"),
            int8("column_id"),
            text("transform"),
        ],
        "ducklake_file_partition_value" => vec![
            int8("data_file_id"),
            int8("table_id"),
            int8("partition_key_index"),
            text("partition_value"),
        ],
        "ducklake_files_scheduled_for_deletion" => vec![
            int8("data_file_id"),
            text("path"),
            bool_col("path_is_relative"),
            timestamp_tz("schedule_start"),
        ],
        "ducklake_inlined_data_tables" => {
            vec![int8("table_id"), text("table_name"), int8("schema_version")]
        }
        "ducklake_column_mapping" => {
            vec![int8("mapping_id"), int8("table_id"), text("type")]
        }
        "ducklake_name_mapping" => vec![
            int8("mapping_id"),
            int8("column_id"),
            text("source_name"),
            int8("target_field_id"),
            int8("parent_column"),
            bool_col("is_partition"),
        ],
        "ducklake_schema_versions" => vec![
            int8("begin_snapshot"),
            int8("schema_version"),
            int8("table_id"),
        ],
        "ducklake_sort_expression" => vec![
            int8("sort_id"),
            int8("table_id"),
            int8("sort_key_index"),
            text("expression"),
            text("dialect"),
            text("sort_direction"),
            text("null_order"),
        ],
        _ => vec![],
    };

    let schema = Arc::new(schema);
    let mut resp = QueryResponse::new(schema, futures::stream::iter(vec![]));
    resp.set_command_tag("SELECT 0");
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
