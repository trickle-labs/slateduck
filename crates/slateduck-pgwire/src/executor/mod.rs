//! SQL executor: translates classified SQL into CatalogStore operations.
//!
//! This module is decomposed into sub-modules by feature family:
//! - `helpers`: shared response builders and parameter utilities
//! - `catalog`: catalog read/write operations and execute_commit
//! - `extension`: extension schema operations
//! - `session`: snapshot lease operations
//! - `meta`: VirtualCatalogScan and info_schema operations

mod catalog;
mod extension;
mod helpers;
mod meta;
mod session;

use std::sync::Arc;

use pgwire::api::results::{Response, Tag};

use slateduck_catalog::CatalogStore;
use slateduck_sql::{classify_statement, ParamValues, StatementKind};

use crate::error::SlateDuckError;
use crate::notify::NotifyManager;
use crate::session::{BufferedOp, SessionState};

use catalog::{
    execute_commit, execute_next_rowid_range, execute_table_changes, make_columns_response,
    make_data_files_response, make_delete_files_response, make_file_ids_response,
    make_macros_response, make_metadata_response, make_schemas_response,
    make_snapshot_row_response, make_table_stats_response, make_tables_response,
    make_views_response,
};
use extension::{
    execute_create_extension_table, execute_delete_extension_rows, execute_insert_extension_row,
    execute_select_extension_table,
};
use helpers::{
    apply_set, get_show_value, get_snapshot_param, make_empty_response, make_null_int_response,
    make_pg_type_response, make_single_int_response, make_single_text_response, require_param_u64,
};
use meta::execute_virtual_catalog_scan;
use session::{execute_hold_snapshot, execute_release_snapshot};

/// Execute a SQL statement against the catalog, returning PG wire responses.
pub async fn execute_sql<'a>(
    sql: &'a str,
    params: &ParamValues,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
    session: &mut SessionState,
    notify_manager: &Arc<NotifyManager>,
    extension_schemas: &Arc<Vec<String>>,
) -> Result<Vec<Response<'a>>, SlateDuckError> {
    let kind = classify_statement(sql)?;
    execute_classified(
        kind,
        sql,
        params,
        store,
        session,
        notify_manager,
        extension_schemas,
    )
    .await
}

async fn execute_classified<'a>(
    kind: StatementKind,
    _sql: &'a str,
    params: &ParamValues,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
    session: &mut SessionState,
    notify_manager: &Arc<NotifyManager>,
    extension_schemas: &Arc<Vec<String>>,
) -> Result<Vec<Response<'a>>, SlateDuckError> {
    match kind {
        // ─── Session / Introspection ───────────────────────────────────
        StatementKind::SelectVersion => Ok(vec![make_single_text_response(
            "version",
            "PostgreSQL 15.0 on x86_64-pc-linux-gnu",
        )]),
        StatementKind::SelectCurrentSchema => {
            Ok(vec![make_single_text_response("current_schema", "public")])
        }
        StatementKind::SelectCurrentDatabase => Ok(vec![make_single_text_response(
            "current_database",
            "ducklake",
        )]),
        StatementKind::SelectPgType => Ok(vec![make_pg_type_response()]),
        StatementKind::ShowVariable(ref var) => {
            let val = get_show_value(var, session);
            Ok(vec![make_single_text_response(var, &val)])
        }
        StatementKind::SetVariable(ref var, ref val) => {
            apply_set(var, val, session);
            Ok(vec![Response::Execution(Tag::new("SET"))])
        }

        // ─── Transaction Control ───────────────────────────────────────
        StatementKind::Begin => {
            session.in_transaction = true;
            Ok(vec![Response::TransactionStart(Tag::new("BEGIN"))])
        }
        StatementKind::Commit => {
            let ops = session.pending_txn.take();
            session.in_transaction = false;
            execute_commit(ops, store, notify_manager).await?;
            Ok(vec![Response::TransactionEnd(Tag::new("COMMIT"))])
        }
        StatementKind::Rollback => {
            session.pending_txn.clear();
            session.in_transaction = false;
            Ok(vec![Response::TransactionEnd(Tag::new("ROLLBACK"))])
        }

        // ─── Read Operations ───────────────────────────────────────────
        StatementKind::SelectMaxSnapshot => {
            // F-11: clone reader out of mutex, drop lock before async I/O.
            let reader = { store.lock().await.read_latest() };
            let snap = reader.get_snapshot().await.map_err(SlateDuckError::from)?;
            let id = snap.map(|s| s.snapshot_id).unwrap_or(0);
            Ok(vec![make_single_int_response("max", id as i64)])
        }
        StatementKind::SelectSchemas => {
            let snap_id = get_snapshot_param(params);
            let reader = {
                let s = store.lock().await;
                s.read_at(slateduck_core::mvcc::SnapshotId::new(snap_id))
                    .map_err(SlateDuckError::from)?
            };
            let schemas = reader.list_schemas().await.map_err(SlateDuckError::from)?;
            Ok(vec![make_schemas_response(schemas)])
        }
        StatementKind::SelectTables => {
            let schema_id = require_param_u64(params, 0, "schema_id")?;
            let snap_id = params.get_u64(1).unwrap_or(u64::MAX);
            let reader = {
                let s = store.lock().await;
                s.read_at(slateduck_core::mvcc::SnapshotId::new(snap_id))
                    .map_err(SlateDuckError::from)?
            };
            let tables = reader
                .list_tables(schema_id)
                .await
                .map_err(SlateDuckError::from)?;
            Ok(vec![make_tables_response(tables)])
        }
        StatementKind::SelectColumns => {
            let table_id = require_param_u64(params, 0, "table_id")?;
            let snap_id = params.get_u64(1).unwrap_or(u64::MAX);
            let reader = {
                let s = store.lock().await;
                s.read_at(slateduck_core::mvcc::SnapshotId::new(snap_id))
                    .map_err(SlateDuckError::from)?
            };
            let result = reader
                .describe_table(table_id)
                .await
                .map_err(SlateDuckError::from)?;
            let columns = result.map(|(_, cols)| cols).unwrap_or_default();
            Ok(vec![make_columns_response(columns)])
        }
        StatementKind::SelectDataFiles => {
            let table_id = require_param_u64(params, 0, "table_id")?;
            let snap_id = params.get_u64(1).unwrap_or(u64::MAX);
            let reader = {
                let s = store.lock().await;
                s.read_at(slateduck_core::mvcc::SnapshotId::new(snap_id))
                    .map_err(SlateDuckError::from)?
            };
            let files = reader
                .list_data_files(table_id)
                .await
                .map_err(SlateDuckError::from)?;
            Ok(vec![make_data_files_response(files)])
        }
        StatementKind::SelectFileColumnStats => {
            let table_id = require_param_u64(params, 0, "table_id")?;
            let column_id = require_param_u64(params, 1, "column_id")?;
            let snap_id = params.get_u64(2).unwrap_or(u64::MAX);
            let reader = {
                let s = store.lock().await;
                s.read_at(slateduck_core::mvcc::SnapshotId::new(snap_id))
                    .map_err(SlateDuckError::from)?
            };
            let predicate = params.get(3).unwrap_or("");
            // v0.26: look up the actual column type for type-aware pruning.
            let col_type = reader
                .get_column_type(table_id, column_id)
                .await
                .map_err(SlateDuckError::from)?
                .as_deref()
                .map(slateduck_core::types::DuckLakeType::parse)
                .unwrap_or(slateduck_core::types::DuckLakeType::Varchar);
            let file_ids = reader
                .prune_files(table_id, column_id, predicate, &col_type)
                .await
                .map_err(SlateDuckError::from)?;
            Ok(vec![make_file_ids_response(file_ids)])
        }
        StatementKind::SelectTableStats => {
            let table_id = require_param_u64(params, 0, "table_id")?;
            let snap_id = params.get_u64(1).unwrap_or(u64::MAX);
            let reader = {
                let s = store.lock().await;
                s.read_at(slateduck_core::mvcc::SnapshotId::new(snap_id))
                    .map_err(SlateDuckError::from)?
            };
            let stats = reader
                .get_table_stats(table_id)
                .await
                .map_err(SlateDuckError::from)?;
            Ok(vec![make_table_stats_response(stats)])
        }
        StatementKind::SelectDeleteFiles => {
            let table_id = require_param_u64(params, 0, "table_id")?;
            let snap_id = params.get_u64(1).unwrap_or(u64::MAX);
            let reader = {
                let s = store.lock().await;
                s.read_at(slateduck_core::mvcc::SnapshotId::new(snap_id))
                    .map_err(SlateDuckError::from)?
            };
            let files = reader
                .list_delete_files(table_id)
                .await
                .map_err(SlateDuckError::from)?;
            Ok(vec![make_delete_files_response(files)])
        }
        StatementKind::SelectSnapshot => {
            let snap_id = params.get_u64(0).unwrap_or(u64::MAX);
            let reader = {
                let s = store.lock().await;
                s.read_at(slateduck_core::mvcc::SnapshotId::new(snap_id))
                    .map_err(SlateDuckError::from)?
            };
            let snap = reader.get_snapshot().await.map_err(SlateDuckError::from)?;
            if let Some(snap) = snap {
                Ok(vec![make_snapshot_row_response(snap)])
            } else {
                Ok(vec![make_empty_response()])
            }
        }
        StatementKind::SelectMetadata => {
            let snap_id = params.get_u64(0).unwrap_or(u64::MAX);
            let reader = {
                let s = store.lock().await;
                s.read_at(slateduck_core::mvcc::SnapshotId::new(snap_id))
                    .map_err(SlateDuckError::from)?
            };
            let rows = reader
                .list_all_metadata()
                .await
                .map_err(SlateDuckError::from)?;
            Ok(vec![make_metadata_response(rows)])
        }
        StatementKind::SelectViews => {
            let snap_id = params.get_u64(0).unwrap_or(u64::MAX);
            let reader = {
                let s = store.lock().await;
                s.read_at(slateduck_core::mvcc::SnapshotId::new(snap_id))
                    .map_err(SlateDuckError::from)?
            };
            let rows = reader
                .list_all_views()
                .await
                .map_err(SlateDuckError::from)?;
            Ok(vec![make_views_response(rows)])
        }
        StatementKind::SelectMacros => {
            let snap_id = params.get_u64(0).unwrap_or(u64::MAX);
            let reader = {
                let s = store.lock().await;
                s.read_at(slateduck_core::mvcc::SnapshotId::new(snap_id))
                    .map_err(SlateDuckError::from)?
            };
            let rows = reader
                .list_all_macros()
                .await
                .map_err(SlateDuckError::from)?;
            Ok(vec![make_macros_response(rows)])
        }
        StatementKind::SelectInlinedData | StatementKind::SelectInlinedRows => {
            // Return empty result set for inlined data (read-only introspection)
            Ok(vec![make_empty_response()])
        }

        // ─── pg-tide-relay extensions ──────────────────────────────────
        StatementKind::SelectMaxSnapshotAfter => {
            let after_id = params.get_u64(0).unwrap_or(0);
            let reader = { store.lock().await.read_latest() };
            let snap = reader.get_snapshot().await.map_err(SlateDuckError::from)?;
            let id = snap.map(|s| s.snapshot_id).unwrap_or(0);
            if id > after_id {
                Ok(vec![make_single_int_response("max", id as i64)])
            } else {
                Ok(vec![make_null_int_response("max")])
            }
        }
        StatementKind::SelectFirstSnapshot => {
            let reader = {
                let s = store.lock().await;
                s.read_at(slateduck_core::mvcc::SnapshotId::new(1))
                    .map_err(SlateDuckError::from)?
            };
            let snap = reader.get_snapshot().await.map_err(SlateDuckError::from)?;
            if let Some(snap) = snap {
                Ok(vec![make_snapshot_row_response(snap)])
            } else {
                Ok(vec![make_empty_response()])
            }
        }
        StatementKind::SelectDataFilesWithLimit => {
            let table_id = require_param_u64(params, 0, "table_id")?;
            let limit = params.get_u64(1).unwrap_or(u64::MAX);
            let snap_id = params.get_u64(2).unwrap_or(u64::MAX);
            let reader = {
                let s = store.lock().await;
                s.read_at(slateduck_core::mvcc::SnapshotId::new(snap_id))
                    .map_err(SlateDuckError::from)?
            };
            let mut files = reader
                .list_data_files(table_id)
                .await
                .map_err(SlateDuckError::from)?;
            files.truncate(limit as usize);
            Ok(vec![make_data_files_response(files)])
        }
        StatementKind::SelectGenRandomUuid => {
            let uuid_val = uuid::Uuid::new_v4().to_string();
            Ok(vec![make_single_text_response(
                "gen_random_uuid",
                &uuid_val,
            )])
        }

        // ─── Write Operations (buffered in transaction) ────────────────
        StatementKind::InsertSnapshot => {
            let op = BufferedOp::InsertSnapshot {
                author: params.get_optional_string(0),
                message: params.get_optional_string(1),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store, notify_manager).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertSnapshotChanges => {
            let op = BufferedOp::InsertSnapshotChanges {
                change_type: params.get_string(0).unwrap_or_default(),
                change_info: params.get_optional_string(1),
                schema_id: params.get_u64(2).ok(),
                table_id: params.get_u64(3).ok(),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store, notify_manager).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertSchema => {
            let op = BufferedOp::InsertSchema {
                schema_name: params.get_string(0).unwrap_or_default(),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store, notify_manager).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertTable => {
            let op = BufferedOp::InsertTable {
                schema_id: params.get_u64(0).unwrap_or(1),
                table_name: params.get_string(1).unwrap_or_default(),
                data_path: params.get_optional_string(2),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store, notify_manager).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertColumn => {
            let op = BufferedOp::InsertColumn {
                table_id: params.get_u64(0).unwrap_or(0),
                column_name: params.get_string(1).unwrap_or_default(),
                data_type: params.get_string(2).unwrap_or_default(),
                column_index: params.get_u64(3).unwrap_or(0),
                is_nullable: params.get_bool(4).unwrap_or(true),
                default_value: params.get_optional_string(5),
                initial_default: params.get_optional_string(6),
                default_value_type: params.get_optional_string(7),
                default_value_dialect: params.get_optional_string(8),
                parent_column: params.get_u64(9).ok(),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store, notify_manager).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertDataFile => {
            let op = BufferedOp::InsertDataFile {
                table_id: params.get_u64(0).unwrap_or(0),
                path: params.get_string(1).unwrap_or_default(),
                file_format: params
                    .get_string(2)
                    .unwrap_or_else(|_| "parquet".to_string()),
                row_count: params.get_u64(3).unwrap_or(0),
                file_size_bytes: params.get_u64(4).unwrap_or(0),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store, notify_manager).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertDeleteFile => {
            let op = BufferedOp::InsertDeleteFile {
                data_file_id: params.get_u64(0).unwrap_or(0),
                path: params.get_string(1).unwrap_or_default(),
                delete_count: params.get_u64(2).unwrap_or(0),
                file_size_bytes: params.get_u64(3).unwrap_or(0),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store, notify_manager).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertTableStats => {
            let op = BufferedOp::InsertTableStats {
                table_id: params.get_u64(0).unwrap_or(0),
                record_count: params.get_u64(1).unwrap_or(0),
                file_count: params.get_u64(2).unwrap_or(0),
                file_size_bytes: params.get_u64(3).unwrap_or(0),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store, notify_manager).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertFileColumnStats => {
            let op = BufferedOp::InsertFileColumnStats {
                table_id: params.get_u64(0).unwrap_or(0),
                column_id: params.get_u64(1).unwrap_or(0),
                data_file_id: params.get_u64(2).unwrap_or(0),
                contains_null: params.get_bool(3).unwrap_or(false),
                min_value: params.get_optional_string(4),
                max_value: params.get_optional_string(5),
                contains_nan: params.get_bool(6).unwrap_or(false),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store, notify_manager).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertMetadata => {
            let op = BufferedOp::InsertMetadata {
                key: params.get_string(0).unwrap_or_default(),
                value: params.get_string(1).unwrap_or_default(),
                scope: params.get_optional_string(2),
                scope_id: params.get_u64(3).ok(),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store, notify_manager).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertInlinedDataTables => {
            let op = BufferedOp::InsertInlinedDataTables {
                table_id: params.get_u64(0).unwrap_or(0),
                schema_version: params.get_u64(1).unwrap_or(0),
                sql: params.get_string(2).unwrap_or_default(),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store, notify_manager).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertView => {
            let op = BufferedOp::InsertView {
                schema_id: params.get_u64(0).unwrap_or(0),
                view_name: params.get_string(1).unwrap_or_default(),
                sql: params.get_string(2).unwrap_or_default(),
                view_uuid: params.get_optional_string(3),
                dialect: params.get_optional_string(4),
                column_aliases: params.get_optional_string(5),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store, notify_manager).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertMacro => {
            let op = BufferedOp::InsertMacro {
                schema_id: params.get_u64(0).unwrap_or(0),
                macro_name: params.get_string(1).unwrap_or_default(),
                macro_type: params.get_string(2).unwrap_or_default(),
                macro_uuid: params.get_optional_string(3),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store, notify_manager).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertMacroImpl => {
            let op = BufferedOp::InsertMacroImpl {
                macro_id: params.get_u64(0).unwrap_or(0),
                sql: params.get_string(1).unwrap_or_default(),
                dialect: params.get_optional_string(2),
                impl_type: params.get_optional_string(3),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store, notify_manager).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertMacroParameters => {
            let op = BufferedOp::InsertMacroParams {
                macro_id: params.get_u64(0).unwrap_or(0),
                impl_id: params.get_u64(1).unwrap_or(0),
                column_id: params.get_u64(2).unwrap_or(0),
                parameter_name: params.get_string(3).unwrap_or_default(),
                parameter_type: params.get_string(4).unwrap_or_default(),
                default_value: params.get_optional_string(5),
                default_value_type: params.get_optional_string(6),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store, notify_manager).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }

        StatementKind::UpdateEndSnapshot(ref table_name) => {
            let op = BufferedOp::UpdateEndSnapshot {
                table_name: table_name.clone(),
                entity_id: params.get_u64(1).unwrap_or(0),
                begin_snapshot: params.get_u64(2).unwrap_or(0),
                end_snapshot: params.get_u64(0).unwrap_or(0),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store, notify_manager).await?;
            }
            Ok(vec![Response::Execution(Tag::new("UPDATE 1"))])
        }
        StatementKind::UpdateTableStats => {
            let op = BufferedOp::UpdateTableStats {
                table_id: params.get_u64(1).unwrap_or(0),
                row_count_delta: params.get_i64(0).unwrap_or(0),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store, notify_manager).await?;
            }
            Ok(vec![Response::Execution(Tag::new("UPDATE 1"))])
        }

        // ─── Inlined Data DDL/DML ──────────────────────────────────────
        StatementKind::CreateInlinedTable => {
            // Accept CREATE TABLE for inlined tables (no-op, tracked in catalog)
            Ok(vec![Response::Execution(Tag::new("CREATE TABLE"))])
        }
        StatementKind::InsertInlinedRow => Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))]),
        StatementKind::UpdateInlinedRowEndSnapshot => {
            Ok(vec![Response::Execution(Tag::new("UPDATE 1"))])
        }

        // ─── Virtual Catalog SQL Tables ────────────────────────────────
        // SELECT * FROM slateduck_catalog.{table_name}: read-only introspection.
        // Mutations are rejected with SQLSTATE 25006.
        StatementKind::VirtualCatalogScan { ref table_name } => {
            execute_virtual_catalog_scan(table_name, store).await
        }

        // ─── v0.18: DuckLake Standard Interface ────────────────────────────
        StatementKind::TableChanges {
            ref table_ref,
            start_snapshot,
            end_snapshot,
        } => execute_table_changes(table_ref, start_snapshot, end_snapshot, store).await,
        StatementKind::NextRowidRange {
            ref table_ref,
            count,
        } => execute_next_rowid_range(table_ref, count, store).await,
        StatementKind::HoldSnapshot {
            min_snapshot_id,
            ref consumer_id,
            ttl_seconds,
        } => execute_hold_snapshot(min_snapshot_id, consumer_id, ttl_seconds, store).await,
        StatementKind::ReleaseSnapshot { ref consumer_id } => {
            execute_release_snapshot(consumer_id, store).await
        }
        StatementKind::Listen { ref channel } => {
            session.subscriptions.listen(channel, notify_manager).await;
            Ok(vec![Response::Execution(Tag::new("LISTEN"))])
        }
        StatementKind::Unlisten { ref channel } => {
            session.subscriptions.unlisten(channel);
            Ok(vec![Response::Execution(Tag::new("UNLISTEN"))])
        }
        StatementKind::CreateExtensionTable {
            ref schema_name,
            ref table_name,
        } => {
            execute_create_extension_table(schema_name, table_name, store, extension_schemas).await
        }
        StatementKind::InsertExtensionRow {
            ref schema_name,
            ref table_name,
            ref columns,
            ..
        } => {
            execute_insert_extension_row(
                schema_name,
                table_name,
                columns,
                params,
                store,
                extension_schemas,
            )
            .await
        }
        StatementKind::SelectExtensionTable {
            ref schema_name,
            ref table_name,
        } => {
            execute_select_extension_table(schema_name, table_name, store, extension_schemas).await
        }
        StatementKind::DeleteExtensionRows {
            ref schema_name,
            ref table_name,
        } => execute_delete_extension_rows(schema_name, table_name, store, extension_schemas).await,

        StatementKind::Unsupported(ref desc) => Err(SlateDuckError::Unsupported(desc.clone())),
    }
}
