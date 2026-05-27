//! Catalog executor operations: response builders, execute_commit, table_changes, next_rowid.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use pgwire::api::results::{DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response};
use pgwire::api::Type;
use sqlparser::ast::{Expr, SelectItem, SetExpr, Statement};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

use rocklake_catalog::writer::stats::FileColumnStatsInput;
use rocklake_catalog::{CatalogStore, CommitResult};
use rocklake_core::rows::{ColumnRow, InlinedDataTablesRow, InlinedInsertRow};

use crate::error::RockLakeError;
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

fn collect_replaced_inlined_rows(ops: &[BufferedOp]) -> HashSet<(u64, u64, u64)> {
    let mut rows = HashSet::new();
    for op in ops {
        let BufferedOp::InsertInlinedRow {
            table_name,
            rows: values,
        } = op
        else {
            continue;
        };
        let Some((table_id, schema_version)) = parse_inlined_table_ids(table_name) else {
            continue;
        };
        for row in values {
            if let Some(row_id) = literal_u64_value(row.first().and_then(|value| value.as_deref()))
            {
                rows.insert((table_id, schema_version, row_id));
            }
        }
    }
    rows
}

fn collect_deleted_inlined_rows(ops: &[BufferedOp]) -> HashSet<(u64, u64, u64)> {
    let mut rows = HashSet::new();
    for op in ops {
        let BufferedOp::DeleteInlinedRows {
            table_name,
            row_ids,
        } = op
        else {
            continue;
        };
        let Some((table_id, schema_version)) = parse_inlined_table_ids(table_name) else {
            continue;
        };
        for row_id in row_ids {
            rows.insert((table_id, schema_version, *row_id));
        }
    }
    rows
}

pub(super) async fn execute_commit(
    ops: Vec<BufferedOp>,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
    notify_manager: &Arc<NotifyManager>,
) -> Result<(), RockLakeError> {
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
    let replaced_inlined_rows = collect_replaced_inlined_rows(&ops);
    let deleted_inlined_rows = collect_deleted_inlined_rows(&ops);
    let mut reserved_inlined_rows = HashSet::new();

    for op in ops {
        match op {
            BufferedOp::InsertSchema { schema_name } => {
                needs_snapshot = true;
                writer
                    .create_schema(&schema_name)
                    .await
                    .map_err(RockLakeError::from)?;
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
                    .map_err(RockLakeError::from)?;
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
                    .map_err(RockLakeError::from)?;
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
                    .map_err(RockLakeError::from)?;
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
                    .map_err(RockLakeError::from)?;
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
                    .map_err(RockLakeError::from)?;
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
                            .map_err(RockLakeError::from)?
                            .unwrap_or(0);
                        writer
                            .drop_table(schema_id, entity_id, begin_snapshot)
                            .await
                            .map_err(RockLakeError::from)?;
                    }
                    "ducklake_column" => {
                        // Resolve table_id by scanning for the live column row
                        // (F-04: entity_id is column_id, not table_id).
                        let table_id = writer
                            .find_column_table_id(entity_id)
                            .await
                            .map_err(RockLakeError::from)?
                            .unwrap_or(entity_id);
                        writer
                            .drop_column(table_id, entity_id, begin_snapshot)
                            .await
                            .map_err(RockLakeError::from)?;
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
                    .map_err(RockLakeError::from)?;
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
                    .map_err(RockLakeError::from)?;
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
                    .map_err(RockLakeError::from)?;
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
                    .map_err(RockLakeError::from)?;
            }
            BufferedOp::InsertMetadata {
                key,
                value,
                scope,
                scope_id,
            } => {
                needs_snapshot = true;
                use rocklake_core::keys::MetadataScope;
                let resolved_scope = match scope.as_deref() {
                    Some("schema") => MetadataScope::Schema,
                    Some("table") => MetadataScope::Table,
                    _ => MetadataScope::Global,
                };
                let resolved_scope_id = scope_id.unwrap_or(0);
                writer
                    .set_metadata(resolved_scope, resolved_scope_id, &key, &value)
                    .map_err(RockLakeError::from)?;
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
                    .map_err(RockLakeError::from)?;
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
                    .map_err(RockLakeError::from)?;
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
                        let mut effective_row_id = row_id;
                        if !deleted_inlined_rows.contains(&(table_id, schema_version, row_id)) {
                            while reserved_inlined_rows.contains(&(
                                table_id,
                                schema_version,
                                effective_row_id,
                            )) || writer
                                .inlined_insert_key_exists(
                                    table_id,
                                    schema_version,
                                    effective_row_id,
                                )
                                .await
                                .map_err(RockLakeError::from)?
                            {
                                effective_row_id = effective_row_id.saturating_add(1);
                            }
                        }
                        reserved_inlined_rows.insert((table_id, schema_version, effective_row_id));
                        let payload =
                            serde_json::to_vec(&values.iter().skip(3).cloned().collect::<Vec<_>>())
                                .map_err(|err| RockLakeError::Internal(err.to_string()))?;
                        writer
                            .register_inlined_insert(
                                table_id,
                                schema_version,
                                effective_row_id,
                                payload,
                            )
                            .await
                            .map_err(RockLakeError::from)?;
                    }
                }
            }
            BufferedOp::DeleteInlinedRows {
                table_name,
                row_ids,
            } => {
                needs_snapshot = true;
                if let Some((table_id, schema_version)) = parse_inlined_table_ids(&table_name) {
                    let deleted_count = row_ids.len() as i64;
                    for row_id in row_ids {
                        if replaced_inlined_rows.contains(&(table_id, schema_version, row_id)) {
                            continue;
                        }
                        writer
                            .mark_inlined_insert_deleted(table_id, schema_version, row_id)
                            .await
                            .map_err(RockLakeError::from)?;
                    }
                    writer
                        .adjust_table_record_count(table_id, -deleted_count)
                        .await
                        .map_err(RockLakeError::from)?;
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
                    .map_err(RockLakeError::from)?;
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
                    .map_err(RockLakeError::from)?;
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
                    .map_err(RockLakeError::from)?;
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
                    .map_err(RockLakeError::from)?;
            }
            BufferedOp::InsertTableStats {
                table_id,
                record_count,
                next_row_id,
                file_size_bytes,
            } => {
                writer
                    .update_table_stats(table_id, record_count, next_row_id, file_size_bytes)
                    .await
                    .map_err(RockLakeError::from)?;
            }
        }
    }
    if needs_snapshot {
        let cr = writer
            .create_snapshot(snapshot_author.as_deref(), snapshot_message.as_deref())
            .await
            .map_err(RockLakeError::from)?;
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
/// Build a PgWire response for a single `ducklake_snapshot` row.
///
/// Spec columns (DuckLake v1.0): `snapshot_id, snapshot_time, schema_version,
/// next_catalog_id, next_file_id`.  The `author` and `message` fields are moved
/// to `ducklake_snapshot_changes` per the v1.0 spec.
pub(super) fn make_snapshot_row_response(
    snap: rocklake_core::rows::SnapshotRow,
) -> Response<'static> {
    // Schema derived from the shared schema registry.
    let schema = crate::schema_registry::snapshot_schema();
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
            &Some(snap.snapshot_time.clone()),
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

/// Build a PgWire response for `SELECT * FROM ducklake_snapshot_changes`.
///
/// Spec columns (DuckLake v1.0): `snapshot_id, changes_made, author,
/// commit_message, commit_extra_info`.
///
/// Internally each snapshot may have multiple `SnapshotChangesRow` entries
/// (one per change event).  This builder aggregates them by `snapshot_id`
/// into a single output row, joining `change_type:change_info` tokens into
/// the `changes_made` comma-separated string.
pub(super) fn make_snapshot_changes_response(
    rows: Vec<rocklake_core::rows::SnapshotChangesRow>,
) -> Response<'static> {
    use std::collections::BTreeMap;
    let schema = crate::schema_registry::snapshot_changes_schema();

    // Aggregate per snapshot_id in snapshot order.
    struct Aggregated {
        changes: Vec<String>,
        author: Option<String>,
        commit_message: Option<String>,
        commit_extra_info: Option<String>,
    }
    let mut map: BTreeMap<u64, Aggregated> = BTreeMap::new();
    for r in &rows {
        let entry = map.entry(r.snapshot_id).or_insert(Aggregated {
            changes: Vec::new(),
            author: r.author.clone(),
            commit_message: r.commit_message.clone(),
            commit_extra_info: r.commit_extra_info.clone(),
        });
        // If the row already carries a pre-built changes_made string, use it;
        // otherwise derive one from change_type:change_info.
        if let Some(ref cm) = r.changes_made {
            if !cm.is_empty() {
                entry.changes.push(cm.clone());
                continue;
            }
        }
        let token = match &r.change_info {
            Some(info) if !info.is_empty() => format!("{}:{}", r.change_type, info),
            _ => r.change_type.clone(),
        };
        if !token.is_empty() {
            entry.changes.push(token);
        }
        // Prefer the most-informative value across multiple rows.
        if entry.author.is_none() {
            entry.author = r.author.clone();
        }
        if entry.commit_message.is_none() {
            entry.commit_message = r.commit_message.clone();
        }
        if entry.commit_extra_info.is_none() {
            entry.commit_extra_info = r.commit_extra_info.clone();
        }
    }

    let mut data_rows = Vec::new();
    for (snapshot_id, agg) in &map {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(snapshot_id.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        let changes_made = if agg.changes.is_empty() {
            None
        } else {
            Some(agg.changes.join(","))
        };
        encoder
            .encode_field_with_type_and_format(&changes_made, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&agg.author, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(&agg.commit_message, &Type::TEXT, FieldFormat::Text)
            .expect("pgwire field encoding is infallible");
        encoder
            .encode_field_with_type_and_format(
                &agg.commit_extra_info,
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

pub(super) fn make_latest_snapshot_info_response(
    snap: Option<rocklake_core::rows::SnapshotRow>,
) -> Response<'static> {
    let schema = crate::schema_registry::latest_snapshot_info_schema();

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
    schemas: Vec<rocklake_core::rows::SchemaRow>,
) -> Response<'static> {
    let schema = crate::schema_registry::schema_schema();
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
    tables: Vec<rocklake_core::rows::TableRow>,
) -> Response<'static> {
    let schema = crate::schema_registry::table_schema();
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
    columns: Vec<rocklake_core::rows::ColumnRow>,
) -> Response<'static> {
    let schema = crate::schema_registry::column_schema();
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
    files: Vec<rocklake_core::rows::DataFileRow>,
) -> Response<'static> {
    let schema = crate::schema_registry::data_file_schema();
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

#[derive(Clone)]
enum FileColumnStatsProjectionSource {
    DataFileId,
    TableId,
    ColumnId,
    ColumnSizeBytes,
    ValueCount,
    NullCount,
    MinValue,
    MaxValue,
    ContainsNan,
    ExtraStats,
}

#[derive(Clone)]
struct FileColumnStatsProjection {
    name: String,
    datatype: Type,
    source: FileColumnStatsProjectionSource,
}

pub(super) fn make_file_column_stats_response(
    sql: &str,
    rows: Vec<rocklake_core::rows::FileColumnStatsRow>,
) -> Response<'static> {
    let projections = file_column_stats_projections(sql);
    let schema = Arc::new(
        projections
            .iter()
            .map(|projection| {
                FieldInfo::new(
                    projection.name.clone(),
                    None,
                    None,
                    projection.datatype.clone(),
                    FieldFormat::Text,
                )
            })
            .collect::<Vec<_>>(),
    );

    let mut data_rows = Vec::new();
    for row in &rows {
        let mut encoder = DataRowEncoder::new(schema.clone());
        for projection in &projections {
            match projection.source {
                FileColumnStatsProjectionSource::DataFileId => {
                    encode_text_i64(&mut encoder, row.data_file_id)
                }
                FileColumnStatsProjectionSource::TableId => {
                    encode_text_i64(&mut encoder, row.table_id)
                }
                FileColumnStatsProjectionSource::ColumnId => {
                    encode_text_i64(&mut encoder, row.column_id)
                }
                FileColumnStatsProjectionSource::ColumnSizeBytes => {
                    encode_text_optional_i64(&mut encoder, row.column_size_bytes)
                }
                FileColumnStatsProjectionSource::ValueCount => {
                    encode_text_optional_i64(&mut encoder, row.value_count)
                }
                FileColumnStatsProjectionSource::NullCount => {
                    encode_text_optional_i64(&mut encoder, row.null_count)
                }
                FileColumnStatsProjectionSource::MinValue => {
                    encode_text_value(&mut encoder, &row.min_value)
                }
                FileColumnStatsProjectionSource::MaxValue => {
                    encode_text_value(&mut encoder, &row.max_value)
                }
                FileColumnStatsProjectionSource::ContainsNan => {
                    let value = Some(row.contains_nan.to_string());
                    encode_text_value(&mut encoder, &value);
                }
                FileColumnStatsProjectionSource::ExtraStats => {
                    encode_text_value(&mut encoder, &row.extra_stats)
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

fn encode_text_i64(encoder: &mut DataRowEncoder, value: u64) {
    encode_text_value(encoder, &Some(value.to_string()));
}

fn encode_text_optional_i64(encoder: &mut DataRowEncoder, value: Option<u64>) {
    encode_text_value(encoder, &value.map(|value| value.to_string()));
}

fn encode_text_value(encoder: &mut DataRowEncoder, value: &Option<String>) {
    encoder
        .encode_field_with_type_and_format(value, &Type::TEXT, FieldFormat::Text)
        .expect("pgwire field encoding is infallible");
}

fn text_field(name: &str) -> FieldInfo {
    FieldInfo::new(name.to_string(), None, None, Type::TEXT, FieldFormat::Text)
}

fn file_column_stats_projections(sql: &str) -> Vec<FileColumnStatsProjection> {
    let items = select_projection_items(sql).unwrap_or_default();
    let mut projections = Vec::new();
    for item in items {
        match item {
            SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(_, _) => {
                projections.extend(default_file_column_stats_projections());
            }
            SelectItem::UnnamedExpr(expr) => {
                let source_name = expr_last_identifier(&expr);
                if let Some(projection) =
                    file_column_stats_projection_for_name(&source_name, &source_name)
                {
                    projections.push(projection);
                }
            }
            SelectItem::ExprWithAlias { expr, alias } => {
                let source_name = expr_last_identifier(&expr);
                if let Some(projection) =
                    file_column_stats_projection_for_name(&source_name, &alias.value)
                {
                    projections.push(projection);
                }
            }
        }
    }

    if projections.is_empty() {
        default_file_column_stats_projections()
    } else {
        projections
    }
}

fn table_stats_projections(sql: &str) -> Vec<TableStatsProjection> {
    let items = select_projection_items(sql).unwrap_or_default();
    let mut projections = Vec::new();
    for item in items {
        match item {
            SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(_, _) => {
                projections.extend(default_table_stats_projections());
            }
            SelectItem::UnnamedExpr(expr) => {
                let source_name = expr_last_identifier(&expr);
                if let Some(projection) =
                    table_stats_projection_for_name(&source_name, &source_name)
                {
                    projections.push(projection);
                }
            }
            SelectItem::ExprWithAlias { expr, alias } => {
                let source_name = expr_last_identifier(&expr);
                if let Some(projection) =
                    table_stats_projection_for_name(&source_name, &alias.value)
                {
                    projections.push(projection);
                }
            }
        }
    }

    if projections.is_empty() {
        default_table_stats_projections()
    } else {
        projections
    }
}

fn default_table_stats_projections() -> Vec<TableStatsProjection> {
    vec![
        table_stats_projection("table_id", TableStatsProjectionSource::TableId),
        table_stats_projection("record_count", TableStatsProjectionSource::RecordCount),
        table_stats_projection("next_row_id", TableStatsProjectionSource::NextRowId),
        table_stats_projection("file_size_bytes", TableStatsProjectionSource::FileSizeBytes),
    ]
}

fn table_stats_projection_for_name(
    source_name: &str,
    output_name: &str,
) -> Option<TableStatsProjection> {
    let source_name = source_name.trim_matches('"').to_ascii_lowercase();
    let output_name = output_name.trim_matches('"');
    match source_name.as_str() {
        "table_id" => Some(table_stats_projection(
            output_name,
            TableStatsProjectionSource::TableId,
        )),
        "record_count" => Some(table_stats_projection(
            output_name,
            TableStatsProjectionSource::RecordCount,
        )),
        "next_row_id" => Some(table_stats_projection(
            output_name,
            TableStatsProjectionSource::NextRowId,
        )),
        "file_size_bytes" => Some(table_stats_projection(
            output_name,
            TableStatsProjectionSource::FileSizeBytes,
        )),
        _ => None,
    }
}

fn table_stats_projection(name: &str, source: TableStatsProjectionSource) -> TableStatsProjection {
    TableStatsProjection {
        name: name.to_string(),
        datatype: Type::INT8,
        source,
    }
}

fn default_file_column_stats_projections() -> Vec<FileColumnStatsProjection> {
    vec![
        file_column_stats_projection(
            "data_file_id",
            Type::INT8,
            FileColumnStatsProjectionSource::DataFileId,
        ),
        file_column_stats_projection(
            "table_id",
            Type::INT8,
            FileColumnStatsProjectionSource::TableId,
        ),
        file_column_stats_projection(
            "column_id",
            Type::INT8,
            FileColumnStatsProjectionSource::ColumnId,
        ),
        file_column_stats_projection(
            "column_size_bytes",
            Type::INT8,
            FileColumnStatsProjectionSource::ColumnSizeBytes,
        ),
        file_column_stats_projection(
            "value_count",
            Type::INT8,
            FileColumnStatsProjectionSource::ValueCount,
        ),
        file_column_stats_projection(
            "null_count",
            Type::INT8,
            FileColumnStatsProjectionSource::NullCount,
        ),
        file_column_stats_projection(
            "min_value",
            Type::TEXT,
            FileColumnStatsProjectionSource::MinValue,
        ),
        file_column_stats_projection(
            "max_value",
            Type::TEXT,
            FileColumnStatsProjectionSource::MaxValue,
        ),
        file_column_stats_projection(
            "contains_nan",
            Type::BOOL,
            FileColumnStatsProjectionSource::ContainsNan,
        ),
        file_column_stats_projection(
            "extra_stats",
            Type::TEXT,
            FileColumnStatsProjectionSource::ExtraStats,
        ),
    ]
}

fn file_column_stats_projection_for_name(
    source_name: &str,
    output_name: &str,
) -> Option<FileColumnStatsProjection> {
    let source_name = source_name.trim_matches('"').to_ascii_lowercase();
    let output_name = output_name.trim_matches('"');
    match source_name.as_str() {
        "data_file_id" => Some(file_column_stats_projection(
            output_name,
            Type::INT8,
            FileColumnStatsProjectionSource::DataFileId,
        )),
        "table_id" => Some(file_column_stats_projection(
            output_name,
            Type::INT8,
            FileColumnStatsProjectionSource::TableId,
        )),
        "column_id" => Some(file_column_stats_projection(
            output_name,
            Type::INT8,
            FileColumnStatsProjectionSource::ColumnId,
        )),
        "column_size_bytes" => Some(file_column_stats_projection(
            output_name,
            Type::INT8,
            FileColumnStatsProjectionSource::ColumnSizeBytes,
        )),
        "value_count" => Some(file_column_stats_projection(
            output_name,
            Type::INT8,
            FileColumnStatsProjectionSource::ValueCount,
        )),
        "null_count" => Some(file_column_stats_projection(
            output_name,
            Type::INT8,
            FileColumnStatsProjectionSource::NullCount,
        )),
        "min_value" => Some(file_column_stats_projection(
            output_name,
            Type::TEXT,
            FileColumnStatsProjectionSource::MinValue,
        )),
        "max_value" => Some(file_column_stats_projection(
            output_name,
            Type::TEXT,
            FileColumnStatsProjectionSource::MaxValue,
        )),
        "contains_nan" => Some(file_column_stats_projection(
            output_name,
            Type::BOOL,
            FileColumnStatsProjectionSource::ContainsNan,
        )),
        "extra_stats" => Some(file_column_stats_projection(
            output_name,
            Type::TEXT,
            FileColumnStatsProjectionSource::ExtraStats,
        )),
        _ => None,
    }
}

fn file_column_stats_projection(
    name: &str,
    datatype: Type,
    source: FileColumnStatsProjectionSource,
) -> FileColumnStatsProjection {
    FileColumnStatsProjection {
        name: name.to_string(),
        datatype,
        source,
    }
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
) -> Result<Vec<Response<'a>>, RockLakeError> {
    let store_lock = store.lock().await;

    // Check GC boundary
    let retain_from = rocklake_catalog::gc::read_retain_from(store_lock.db())
        .await
        .map_err(RockLakeError::from)?;

    if start_snapshot < retain_from && retain_from > 0 {
        return Err(RockLakeError::SqlState {
            code: "55000".to_string(),
            message: format!(
                "snapshot {} has been garbage collected (retain_from={})",
                start_snapshot, retain_from
            ),
        });
    }

    // Compute diff using SnapshotDiff
    let from_reader = store_lock
        .read_at(rocklake_core::mvcc::SnapshotId::new(start_snapshot))
        .map_err(RockLakeError::from)?;

    let diff = from_reader
        .snapshot_diff(
            rocklake_core::mvcc::SnapshotId::new(start_snapshot),
            rocklake_core::mvcc::SnapshotId::new(end_snapshot),
        )
        .await
        .map_err(RockLakeError::from)?;

    // v0.27.1: Extract rows from Parquet data files using the catalog's object store.
    // The object store root is shared between catalog metadata and data files
    // when data files are registered with relative paths.
    let object_store = store_lock.object_store();
    drop(store_lock); // release lock before async I/O

    let mut added_rows = Vec::new();
    let mut base_rowid = 0u64;
    for file in &diff.added_data_files {
        let rows = rocklake_sql::table_changes::extract_rows_from_parquet(
            &object_store,
            &file.path,
            base_rowid,
            Some(file.record_count),
            rocklake_sql::DEFAULT_CDC_BATCH_SIZE,
        )
        .await
        .map_err(|e| RockLakeError::SqlState {
            code: e.sqlstate().to_string(),
            message: e.to_string(),
        })?;
        base_rowid += rows.len() as u64;
        added_rows.extend(rows);
    }

    let mut removed_rows = Vec::new();
    base_rowid = 0;
    for file in &diff.retired_data_files {
        let rows = rocklake_sql::table_changes::extract_rows_from_parquet(
            &object_store,
            &file.path,
            base_rowid,
            Some(file.record_count),
            rocklake_sql::DEFAULT_CDC_BATCH_SIZE,
        )
        .await
        .map_err(|e| RockLakeError::SqlState {
            code: e.sqlstate().to_string(),
            message: e.to_string(),
        })?;
        base_rowid += rows.len() as u64;
        removed_rows.extend(rows);
    }

    let cdc_result = rocklake_sql::table_changes::compute_table_changes(
        table_ref,
        start_snapshot,
        end_snapshot,
        retain_from,
        &added_rows,
        &removed_rows,
    )
    .map_err(|e| RockLakeError::SqlState {
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
) -> Result<Vec<Response<'a>>, RockLakeError> {
    let store_lock = store.lock().await;
    let table_id = hash_table_ref(table_ref);

    let db = store_lock.db();
    let (start, end) = rocklake_catalog::next_rowid_range(db, table_id, count)
        .await
        .map_err(RockLakeError::from)?;

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

#[derive(Clone, Copy)]
enum TableStatsProjectionSource {
    TableId,
    RecordCount,
    NextRowId,
    FileSizeBytes,
}

struct TableStatsProjection {
    name: String,
    datatype: Type,
    source: TableStatsProjectionSource,
}

pub(super) fn make_table_stats_rows_response_for_sql(
    sql: &str,
    rows: Vec<rocklake_core::rows::TableStatsRow>,
) -> Response<'static> {
    let projections = table_stats_projections(sql);
    let schema = Arc::new(
        projections
            .iter()
            .map(|projection| {
                FieldInfo::new(
                    projection.name.clone(),
                    None,
                    None,
                    projection.datatype.clone(),
                    FieldFormat::Text,
                )
            })
            .collect::<Vec<_>>(),
    );
    let mut data_rows = Vec::new();
    for row in rows {
        let mut encoder = DataRowEncoder::new(schema.clone());
        for projection in &projections {
            match projection.source {
                TableStatsProjectionSource::TableId => encode_text_i64(&mut encoder, row.table_id),
                TableStatsProjectionSource::RecordCount => {
                    encode_text_i64(&mut encoder, row.record_count)
                }
                TableStatsProjectionSource::NextRowId => {
                    encode_text_optional_i64(&mut encoder, row.next_row_id)
                }
                TableStatsProjectionSource::FileSizeBytes => {
                    encode_text_i64(&mut encoder, row.file_size_bytes)
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

pub(super) fn make_global_table_stats_response(
    stats_rows: Vec<rocklake_core::rows::TableStatsRow>,
    column_stats_rows: Vec<rocklake_core::rows::TableColumnStatsRow>,
) -> Response<'static> {
    let schema = crate::schema_registry::global_table_stats_schema();
    let mut data_rows = Vec::new();
    for stats in &stats_rows {
        let matching_column_stats = column_stats_rows
            .iter()
            .filter(|row| row.table_id == stats.table_id)
            .collect::<Vec<_>>();
        if matching_column_stats.is_empty() {
            data_rows.push(encode_global_table_stats_row(schema.clone(), stats, None));
        } else {
            for column_stats in matching_column_stats {
                data_rows.push(encode_global_table_stats_row(
                    schema.clone(),
                    stats,
                    Some(column_stats),
                ));
            }
        }
    }
    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

fn encode_global_table_stats_row(
    schema: Arc<Vec<FieldInfo>>,
    stats: &rocklake_core::rows::TableStatsRow,
    column_stats: Option<&rocklake_core::rows::TableColumnStatsRow>,
) -> pgwire::error::PgWireResult<pgwire::messages::data::DataRow> {
    let mut encoder = DataRowEncoder::new(schema);
    encode_text_value(&mut encoder, &Some(stats.table_id.to_string()));
    encode_text_value(
        &mut encoder,
        &column_stats.map(|row| row.column_id.to_string()),
    );
    encode_text_value(&mut encoder, &Some(stats.record_count.to_string()));
    encode_text_value(
        &mut encoder,
        &stats.next_row_id.map(|value| value.to_string()),
    );
    encode_text_value(&mut encoder, &Some(stats.file_size_bytes.to_string()));
    encode_text_value(
        &mut encoder,
        &column_stats.map(|row| row.contains_null.to_string()),
    );
    encode_text_value(
        &mut encoder,
        &column_stats.and_then(|row| row.contains_nan.map(|value| value.to_string())),
    );
    encode_text_value(
        &mut encoder,
        &column_stats.and_then(|row| row.min_value.clone()),
    );
    encode_text_value(
        &mut encoder,
        &column_stats.and_then(|row| row.max_value.clone()),
    );
    encode_text_value(
        &mut encoder,
        &column_stats.and_then(|row| row.extra_stats.clone()),
    );
    encoder.finish()
}

pub(super) fn make_snapshot_stats_changes_response(
    snapshot: Option<rocklake_core::rows::SnapshotRow>,
    stats_rows: Vec<rocklake_core::rows::TableStatsRow>,
    column_stats_rows: Vec<rocklake_core::rows::TableColumnStatsRow>,
) -> Response<'static> {
    let schema = Arc::new(vec![
        text_field("snapshot_id"),
        text_field("schema_version"),
        text_field("next_catalog_id"),
        text_field("next_file_id"),
        text_field("changes"),
        text_field("table_id"),
        text_field("column_id"),
        text_field("record_count"),
        text_field("next_row_id"),
        text_field("file_size_bytes"),
        text_field("contains_null"),
        text_field("contains_nan"),
        text_field("min_value"),
        text_field("max_value"),
        text_field("extra_stats"),
    ]);

    let mut data_rows = Vec::new();
    let snapshot = snapshot.unwrap_or(rocklake_core::rows::SnapshotRow {
        snapshot_id: 0,
        schema_version: 0,
        snapshot_time: String::new(),
        author: None,
        message: None,
        next_catalog_id: Some(1),
        next_file_id: Some(1),
    });
    data_rows.push(encode_optional_text_row(
        schema.clone(),
        vec![
            Some(snapshot.snapshot_id.to_string()),
            Some(snapshot.schema_version.to_string()),
            Some(snapshot.next_catalog_id.unwrap_or(1).to_string()),
            Some(snapshot.next_file_id.unwrap_or(1).to_string()),
            Some(String::new()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    ));

    let stats_by_table: BTreeMap<u64, rocklake_core::rows::TableStatsRow> = stats_rows
        .into_iter()
        .map(|stats| (stats.table_id, stats))
        .collect();
    let mut column_stats_by_table: BTreeMap<u64, Vec<rocklake_core::rows::TableColumnStatsRow>> =
        BTreeMap::new();
    for row in column_stats_rows {
        column_stats_by_table
            .entry(row.table_id)
            .or_default()
            .push(row);
    }

    for (table_id, stats) in stats_by_table {
        if let Some(column_rows) = column_stats_by_table.remove(&table_id) {
            for column in column_rows {
                data_rows.push(encode_optional_text_row(
                    schema.clone(),
                    vec![
                        None,
                        None,
                        None,
                        None,
                        None,
                        Some(table_id.to_string()),
                        Some(column.column_id.to_string()),
                        Some(stats.record_count.to_string()),
                        stats.next_row_id.map(|value| value.to_string()),
                        Some(stats.file_size_bytes.to_string()),
                        Some(column.contains_null.to_string()),
                        column.contains_nan.map(|value| value.to_string()),
                        column.min_value,
                        column.max_value,
                        column.extra_stats,
                    ],
                ));
            }
        } else {
            data_rows.push(encode_optional_text_row(
                schema.clone(),
                vec![
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(table_id.to_string()),
                    Some("0".to_string()),
                    Some(stats.record_count.to_string()),
                    stats.next_row_id.map(|value| value.to_string()),
                    Some(stats.file_size_bytes.to_string()),
                    None,
                    None,
                    None,
                    None,
                    None,
                ],
            ));
        }
    }

    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

fn encode_optional_text_row(
    schema: Arc<Vec<FieldInfo>>,
    values: Vec<Option<String>>,
) -> pgwire::error::PgWireResult<pgwire::messages::data::DataRow> {
    let mut encoder = DataRowEncoder::new(schema);
    for value in values {
        encode_text_value(&mut encoder, &value);
    }
    encoder.finish()
}

pub(super) fn make_table_column_stats_response(
    rows: Vec<rocklake_core::rows::TableColumnStatsRow>,
) -> Response<'static> {
    let schema = crate::schema_registry::table_column_stats_schema();
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
    files: Vec<rocklake_core::rows::DeleteFileRow>,
) -> Response<'static> {
    let schema = crate::schema_registry::delete_file_schema();
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
    rows: Vec<rocklake_core::rows::MetadataRow>,
) -> Response<'static> {
    let schema = crate::schema_registry::metadata_schema();
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
pub(super) fn make_views_response(views: Vec<rocklake_core::rows::ViewRow>) -> Response<'static> {
    let schema = crate::schema_registry::view_schema();
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
    macros: Vec<rocklake_core::rows::MacroRow>,
) -> Response<'static> {
    let schema = crate::schema_registry::macro_schema();
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
    rows: Vec<rocklake_core::rows::MacroImplRow>,
) -> Response<'static> {
    let schema = crate::schema_registry::macro_impl_schema();
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
    rows: Vec<rocklake_core::rows::MacroParametersRow>,
) -> Response<'static> {
    let schema = crate::schema_registry::macro_parameters_schema();
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
    let schema = crate::schema_registry::inlined_data_tables_schema();
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
    rows: Vec<rocklake_core::rows::SchemaVersionsRow>,
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
        Expr::Cast { expr, .. } | Expr::Nested(expr) => expr_last_identifier(expr),
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
        "TIMESTAMP" | "TIMESTAMP WITHOUT TIME ZONE" => Type::TIMESTAMP,
        "TIMESTAMP WITH TIME ZONE" | "TIMESTAMPTZ" => Type::TIMESTAMPTZ,
        "DATE" => Type::DATE,
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
        datatype if datatype == &Type::TIMESTAMP || datatype == &Type::TIMESTAMPTZ => {
            // PostgreSQL binary TIMESTAMP = i64 microseconds since 2000-01-01 00:00:00 UTC.
            // This is the same binary layout as INT8, so we encode as INT8.
            let value = value.and_then(timestamp_str_to_pg_micros);
            encoder
                .encode_field_with_type_and_format(&value, &Type::INT8, FieldFormat::Binary)
                .expect("pgwire field encoding is infallible");
        }
        datatype if datatype == &Type::DATE => {
            // PostgreSQL binary DATE = i32 days since 2000-01-01.
            let value = value.and_then(date_str_to_pg_days);
            encoder
                .encode_field_with_type_and_format(&value, &Type::INT4, FieldFormat::Binary)
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

/// Parse a timestamp string such as "2026-05-26 20:53:17.231752" and return
/// the number of microseconds since the PostgreSQL epoch (2000-01-01 00:00:00 UTC).
fn timestamp_str_to_pg_micros(s: &str) -> Option<i64> {
    use chrono::NaiveDateTime;
    // PostgreSQL epoch = 2000-01-01 00:00:00 UTC = Unix timestamp 946 684 800 s
    const PG_EPOCH_MICROS: i64 = 946_684_800 * 1_000_000;
    let s = s.trim();
    let dt = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f")
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok())
        .or_else(|| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f").ok())
        .or_else(|| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").ok())?;
    Some(dt.and_utc().timestamp_micros() - PG_EPOCH_MICROS)
}

/// Parse a date string such as "2026-05-26" and return the number of days
/// since the PostgreSQL epoch (2000-01-01).
fn date_str_to_pg_days(s: &str) -> Option<i32> {
    use chrono::NaiveDate;
    let d = NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()?;
    let pg_epoch = NaiveDate::from_ymd_opt(2000, 1, 1)?;
    Some(d.signed_duration_since(pg_epoch).num_days() as i32)
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
    if value.len().is_multiple_of(2) && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
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
    // Use the schema registry as the single source of truth for all registered
    // DuckLake metadata tables.
    if let Some(schema) = crate::schema_registry::fields_for_table(table_name) {
        let mut resp = QueryResponse::new(schema, futures::stream::iter(vec![]));
        resp.set_command_tag("SELECT 0");
        return Response::Query(resp);
    }

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

    // Fall back to hard-coded schemas for tables not yet in the registry
    // (internal or non-spec tables).
    let schema: Vec<FieldInfo> = match table_name {
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
/// v0.27: Build a PgWire response for `SELECT * FROM ducklake_tag`.
///
/// Spec column names (v1.0 Catalog Version 7): begin_snapshot, end_snapshot,
/// object_id, key, value.  The synthesized `tag_id` column has been removed
/// per spec alignment; `key` and `value` carry the internal `tag_key`/`tag_value`.
pub(super) fn make_tags_response(rows: Vec<rocklake_core::rows::TagRow>) -> Response<'static> {
    let schema = crate::schema_registry::tag_schema();
    let mut data_rows = Vec::new();
    for r in &rows {
        let mut encoder = DataRowEncoder::new(schema.clone());
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
        // `tag_key` exposed as spec column `key`.
        encoder
            .encode_field_with_type_and_format(
                &Some(r.tag_key.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        // `tag_value` exposed as spec column `value`.
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
/// Spec column names (v1.0 Catalog Version 7): begin_snapshot, end_snapshot,
/// column_id, key, value.  The synthesized `tag_id` column has been removed;
/// `key` and `value` carry the internal `tag_key`/`tag_value`.
pub(super) fn make_column_tags_response(
    rows: Vec<rocklake_core::rows::ColumnTagRow>,
) -> Response<'static> {
    let schema = crate::schema_registry::column_tag_schema();
    let mut data_rows = Vec::new();
    for r in &rows {
        let mut encoder = DataRowEncoder::new(schema.clone());
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
        // `tag_key` exposed as spec column `key`.
        encoder
            .encode_field_with_type_and_format(
                &Some(r.tag_key.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("pgwire field encoding is infallible");
        // `tag_value` exposed as spec column `value`.
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
    rows: Vec<rocklake_core::rows::SortInfoRow>,
) -> Response<'static> {
    let schema = crate::schema_registry::sort_info_schema();
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
    let schema = crate::schema_registry::schema_version_schema();
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
