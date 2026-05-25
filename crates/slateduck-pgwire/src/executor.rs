//! SQL executor: translates classified SQL into CatalogStore operations.

use std::sync::Arc;

use pgwire::api::results::{DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response, Tag};
use pgwire::api::Type;

use slateduck_catalog::CatalogStore;
use slateduck_sql::{classify_statement, ParamValues, StatementKind};

use crate::error::SlateDuckError;
use crate::session::{BufferedOp, SessionState};
use crate::types;

/// Execute a SQL statement against the catalog, returning PG wire responses.
pub async fn execute_sql<'a>(
    sql: &'a str,
    params: &ParamValues,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
    session: &mut SessionState,
) -> Result<Vec<Response<'a>>, SlateDuckError> {
    let kind = classify_statement(sql)?;
    execute_classified(kind, sql, params, store, session).await
}

async fn execute_classified<'a>(
    kind: StatementKind,
    _sql: &'a str,
    params: &ParamValues,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
    session: &mut SessionState,
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
            execute_commit(ops, store).await?;
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
            let col_type = slateduck_core::types::DuckLakeType::Varchar;
            let file_ids = reader
                .prune_files(table_id, column_id, predicate, &col_type)
                .await
                .map_err(SlateDuckError::from)?;
            Ok(vec![make_file_ids_response(file_ids)])
        }
        StatementKind::SelectSnapshot
        | StatementKind::SelectTableStats
        | StatementKind::SelectMetadata
        | StatementKind::SelectInlinedData
        | StatementKind::SelectViews
        | StatementKind::SelectMacros
        | StatementKind::SelectDeleteFiles
        | StatementKind::SelectInlinedRows => {
            // Return empty result set for these less commonly used reads
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
                execute_commit(vec![op], store).await?;
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
                execute_commit(vec![op], store).await?;
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
                execute_commit(vec![op], store).await?;
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
                execute_commit(vec![op], store).await?;
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
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store).await?;
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
                execute_commit(vec![op], store).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertDeleteFile => {
            let op = BufferedOp::InsertDeleteFile {
                data_file_id: params.get_u64(0).unwrap_or(0),
                path: params.get_string(1).unwrap_or_default(),
                row_count: params.get_u64(2).unwrap_or(0),
                file_size_bytes: params.get_u64(3).unwrap_or(0),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertTableStats => {
            let op = BufferedOp::InsertTableStats {
                table_id: params.get_u64(0).unwrap_or(0),
                row_count: params.get_u64(1).unwrap_or(0),
                file_count: params.get_u64(2).unwrap_or(0),
                total_size_bytes: params.get_u64(3).unwrap_or(0),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertFileColumnStats => {
            let op = BufferedOp::InsertFileColumnStats {
                table_id: params.get_u64(0).unwrap_or(0),
                column_id: params.get_u64(1).unwrap_or(0),
                data_file_id: params.get_u64(2).unwrap_or(0),
                has_null: params.get_bool(3).unwrap_or(false),
                min_value: params.get_optional_string(4),
                max_value: params.get_optional_string(5),
                contains_nan: params.get_bool(6).unwrap_or(false),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertMetadata => {
            let op = BufferedOp::InsertMetadata {
                key: params.get_string(0).unwrap_or_default(),
                value: params.get_string(1).unwrap_or_default(),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store).await?;
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
                execute_commit(vec![op], store).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertView => {
            let op = BufferedOp::InsertView {
                schema_id: params.get_u64(0).unwrap_or(0),
                view_name: params.get_string(1).unwrap_or_default(),
                sql: params.get_string(2).unwrap_or_default(),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertMacro => {
            let op = BufferedOp::InsertMacro {
                schema_id: params.get_u64(0).unwrap_or(0),
                macro_name: params.get_string(1).unwrap_or_default(),
                macro_type: params.get_string(2).unwrap_or_default(),
            };
            if session.in_transaction {
                session.pending_txn.push(op)?;
            } else {
                execute_commit(vec![op], store).await?;
            }
            Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
        }
        StatementKind::InsertMacroImpl | StatementKind::InsertMacroParameters => {
            // Accept but no-op for now (macros deferred)
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
                execute_commit(vec![op], store).await?;
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
                execute_commit(vec![op], store).await?;
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
            let reader = { store.lock().await.read_latest() };
            match table_name.as_str() {
                "ducklake_snapshot" => {
                    // Return all snapshots without MVCC filtering.
                    let snap = reader.get_snapshot().await.map_err(SlateDuckError::from)?;
                    let id = snap.as_ref().map(|s| s.snapshot_id).unwrap_or(0);
                    Ok(vec![make_single_int_response("snapshot_id", id as i64)])
                }
                "ducklake_schema" => {
                    let schemas = reader.list_schemas().await.map_err(SlateDuckError::from)?;
                    Ok(vec![make_schemas_response(schemas)])
                }
                "ducklake_table" => {
                    // All tables across all schemas without MVCC filtering.
                    let schemas = reader.list_schemas().await.map_err(SlateDuckError::from)?;
                    let mut all_tables = vec![];
                    for schema in schemas {
                        let tables = reader
                            .list_tables(schema.schema_id)
                            .await
                            .map_err(SlateDuckError::from)?;
                        all_tables.extend(tables);
                    }
                    Ok(vec![make_tables_response(all_tables)])
                }
                "ducklake_column" => Ok(vec![make_empty_response()]),
                "ducklake_data_file" => Ok(vec![make_empty_response()]),
                "ducklake_delete_file" => Ok(vec![make_empty_response()]),
                "ducklake_file_column_stats" => Ok(vec![make_empty_response()]),
                "ducklake_table_stats" => Ok(vec![make_empty_response()]),
                "ducklake_metadata" => Ok(vec![make_empty_response()]),
                "slateduck_counters" => Ok(vec![make_empty_response()]),
                "slateduck_system" => Ok(vec![make_empty_response()]),
                _ => Ok(vec![make_empty_response()]),
            }
        }

        // IVM statements — routing to the IVM worker is out of scope for the
        // pg-wire layer in v0.11; return an unsupported error with a clear message.
        StatementKind::CreateIncrementalMatview { .. }
        | StatementKind::DropIncrementalMatview { .. }
        | StatementKind::AlterIncrementalMatview { .. }
        | StatementKind::RefreshIncrementalMatviewFull { .. }
        | StatementKind::ShowMaterializedViews
        | StatementKind::ShowMatviewShards { .. }
        | StatementKind::ExplainMatview { .. } => Err(SlateDuckError::Unsupported(
            "IVM DDL is processed by the slateduck-ivm worker, not the pg-wire layer".into(),
        )),

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
        StatementKind::Listen { channel: _ } => Ok(vec![Response::Execution(Tag::new("LISTEN"))]),
        StatementKind::Unlisten { channel: _ } => {
            Ok(vec![Response::Execution(Tag::new("UNLISTEN"))])
        }
        StatementKind::CreateExtensionTable {
            ref schema_name,
            ref table_name,
        } => execute_create_extension_table(schema_name, table_name, store).await,
        StatementKind::InsertExtensionRow {
            ref schema_name,
            ref table_name,
            ..
        } => execute_insert_extension_row(schema_name, table_name, params, store).await,
        StatementKind::SelectExtensionTable {
            ref schema_name,
            ref table_name,
        } => execute_select_extension_table(schema_name, table_name, store).await,
        StatementKind::DeleteExtensionRows {
            ref schema_name,
            ref table_name,
        } => execute_delete_extension_rows(schema_name, table_name, store).await,

        StatementKind::Unsupported(ref desc) => Err(SlateDuckError::Unsupported(desc.clone())),
    }
}

/// Execute a committed batch of operations against the catalog.
#[tracing::instrument(skip(ops, store), fields(op_count = ops.len()))]
async fn execute_commit(
    ops: Vec<BufferedOp>,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
) -> Result<(), SlateDuckError> {
    if ops.is_empty() {
        return Ok(());
    }

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
                    .upsert_file_column_stats(
                        table_id,
                        column_id,
                        data_file_id,
                        has_null,
                        min_value.as_deref(),
                        max_value.as_deref(),
                        contains_nan,
                    )
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
    Ok(())
}

// ─── Response Builders ─────────────────────────────────────────────────────

/// F-24: Require a u64 parameter; returns SQLSTATE 22023 if absent or invalid.
fn require_param_u64(params: &ParamValues, idx: usize, name: &str) -> Result<u64, SlateDuckError> {
    params
        .get_u64(idx)
        .map_err(|_| SlateDuckError::MissingParam {
            name: name.to_string(),
        })
}

fn get_snapshot_param(params: &ParamValues) -> u64 {
    params.get_u64(0).unwrap_or(u64::MAX)
}

fn get_show_value(var: &str, session: &SessionState) -> String {
    match var.to_lowercase().as_str() {
        "server_version" => "15.0".to_string(),
        "datestyle" | "date_style" => session.settings.date_style.clone(),
        "timezone" | "time zone" => session.settings.timezone.clone(),
        "client_encoding" => session.settings.client_encoding.clone(),
        "transaction_isolation" => "read committed".to_string(),
        "standard_conforming_strings" => "on".to_string(),
        _ => String::new(),
    }
}

fn apply_set(var: &str, val: &str, session: &mut SessionState) {
    let clean_val = val.trim_matches('\'').to_string();
    match var.to_lowercase().as_str() {
        "timezone" | "time zone" => session.settings.timezone = clean_val,
        "client_encoding" => session.settings.client_encoding = clean_val,
        "datestyle" | "date_style" => session.settings.date_style = clean_val,
        "application_name" => session.settings.application_name = clean_val,
        _ => {} // Accept and ignore unknown settings
    }
}

fn make_single_text_response<'a>(col_name: &str, value: &str) -> Response<'a> {
    let schema = Arc::new(vec![FieldInfo::new(
        col_name.to_string(),
        None,
        None,
        Type::TEXT,
        FieldFormat::Text,
    )]);
    let mut encoder = DataRowEncoder::new(schema.clone());
    encoder
        .encode_field_with_type_and_format(&Some(value.to_string()), &Type::TEXT, FieldFormat::Text)
        .unwrap();
    let row = encoder.finish();
    let data_rows = vec![row];
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag("SELECT 1");
    Response::Query(resp)
}

fn make_single_int_response<'a>(col_name: &str, value: i64) -> Response<'a> {
    let schema = Arc::new(vec![FieldInfo::new(
        col_name.to_string(),
        None,
        None,
        Type::INT8,
        FieldFormat::Text,
    )]);
    let mut encoder = DataRowEncoder::new(schema.clone());
    encoder
        .encode_field_with_type_and_format(&Some(value.to_string()), &Type::TEXT, FieldFormat::Text)
        .unwrap();
    let row = encoder.finish();
    let data_rows = vec![row];
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag("SELECT 1");
    Response::Query(resp)
}

fn make_null_int_response<'a>(col_name: &str) -> Response<'a> {
    let schema = Arc::new(vec![FieldInfo::new(
        col_name.to_string(),
        None,
        None,
        Type::INT8,
        FieldFormat::Text,
    )]);
    let mut encoder = DataRowEncoder::new(schema.clone());
    encoder
        .encode_field_with_type_and_format(&None::<String>, &Type::TEXT, FieldFormat::Text)
        .unwrap();
    let row = encoder.finish();
    let data_rows = vec![row];
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag("SELECT 1");
    Response::Query(resp)
}

fn make_snapshot_row_response(snap: slateduck_core::rows::SnapshotRow) -> Response<'static> {
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

fn make_pg_type_response<'a>() -> Response<'a> {
    let schema = Arc::new(vec![
        FieldInfo::new("oid".to_string(), None, None, Type::INT4, FieldFormat::Text),
        FieldInfo::new(
            "typname".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
    ]);
    let mut data_rows = Vec::new();
    for (name, oid) in types::PG_TYPE_MAP {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(oid.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(name.to_string()),
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

fn make_empty_response<'a>() -> Response<'a> {
    let schema = Arc::new(vec![]);
    let resp = QueryResponse::new(schema, futures::stream::iter(Vec::new()));
    Response::Query(resp)
}

fn make_schemas_response(schemas: Vec<slateduck_core::rows::SchemaRow>) -> Response<'static> {
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

fn make_tables_response(tables: Vec<slateduck_core::rows::TableRow>) -> Response<'static> {
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

fn make_columns_response(columns: Vec<slateduck_core::rows::ColumnRow>) -> Response<'static> {
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

fn make_data_files_response(files: Vec<slateduck_core::rows::DataFileRow>) -> Response<'static> {
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

fn make_file_ids_response(file_ids: Vec<u64>) -> Response<'static> {
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

async fn execute_table_changes<'a>(
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
            "table_ref".into(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
    ]);

    let mut data_rows = Vec::new();

    // Added data files → INSERT records
    for _file in &diff.added_data_files {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some("0".to_string()),
                &Type::INT8,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some("insert".to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(table_ref.to_string()),
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

async fn execute_next_rowid_range<'a>(
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

async fn execute_hold_snapshot<'a>(
    min_snapshot_id: u64,
    consumer_id: &str,
    ttl_seconds: u64,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
) -> Result<Vec<Response<'a>>, SlateDuckError> {
    let store_lock = store.lock().await;
    let db = store_lock.db();
    slateduck_catalog::hold_snapshot(db, consumer_id, min_snapshot_id, ttl_seconds)
        .await
        .map_err(SlateDuckError::from)?;

    Ok(vec![make_single_text_response("hold_snapshot", "OK")])
}

async fn execute_release_snapshot<'a>(
    consumer_id: &str,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
) -> Result<Vec<Response<'a>>, SlateDuckError> {
    let store_lock = store.lock().await;
    let db = store_lock.db();
    let released = slateduck_catalog::release_snapshot(db, consumer_id)
        .await
        .map_err(SlateDuckError::from)?;

    Ok(vec![make_single_text_response(
        "release_snapshot",
        if released { "OK" } else { "NOT_FOUND" },
    )])
}

async fn execute_create_extension_table<'a>(
    schema_name: &str,
    table_name: &str,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
) -> Result<Vec<Response<'a>>, SlateDuckError> {
    let extension_id = slateduck_catalog::resolve_extension_id(schema_name).ok_or_else(|| {
        SlateDuckError::Unsupported(format!(
            "extension schema '{schema_name}' is not registered"
        ))
    })?;
    let store_lock = store.lock().await;
    let db = store_lock.db();
    slateduck_catalog::create_extension_table(db, extension_id, table_name)
        .await
        .map_err(SlateDuckError::from)?;
    Ok(vec![Response::Execution(Tag::new("CREATE TABLE"))])
}

async fn execute_insert_extension_row<'a>(
    schema_name: &str,
    table_name: &str,
    params: &ParamValues,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
) -> Result<Vec<Response<'a>>, SlateDuckError> {
    let extension_id = slateduck_catalog::resolve_extension_id(schema_name).ok_or_else(|| {
        SlateDuckError::Unsupported(format!(
            "extension schema '{schema_name}' is not registered"
        ))
    })?;
    let store_lock = store.lock().await;
    let db = store_lock.db();

    let data_json = params.to_json_string();
    slateduck_catalog::insert_extension_row(db, extension_id, table_name, &data_json)
        .await
        .map_err(SlateDuckError::from)?;
    Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
}

async fn execute_select_extension_table<'a>(
    schema_name: &str,
    table_name: &str,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
) -> Result<Vec<Response<'a>>, SlateDuckError> {
    let extension_id = slateduck_catalog::resolve_extension_id(schema_name).ok_or_else(|| {
        SlateDuckError::Unsupported(format!(
            "extension schema '{schema_name}' is not registered"
        ))
    })?;
    let store_lock = store.lock().await;
    let db = store_lock.db();
    let rows = slateduck_catalog::select_extension_rows(db, extension_id, table_name)
        .await
        .map_err(SlateDuckError::from)?;

    let schema = Arc::new(vec![
        FieldInfo::new("row_id".into(), None, None, Type::INT8, FieldFormat::Text),
        FieldInfo::new("data".into(), None, None, Type::TEXT, FieldFormat::Text),
    ]);
    let mut data_rows = Vec::new();
    for row in &rows {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(row.row_id.to_string()),
                &Type::INT8,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(row.data_json.clone()),
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

async fn execute_delete_extension_rows<'a>(
    schema_name: &str,
    table_name: &str,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
) -> Result<Vec<Response<'a>>, SlateDuckError> {
    let extension_id = slateduck_catalog::resolve_extension_id(schema_name).ok_or_else(|| {
        SlateDuckError::Unsupported(format!(
            "extension schema '{schema_name}' is not registered"
        ))
    })?;
    let store_lock = store.lock().await;
    let db = store_lock.db();
    let deleted = slateduck_catalog::delete_extension_rows(db, extension_id, table_name)
        .await
        .map_err(SlateDuckError::from)?;
    Ok(vec![Response::Execution(Tag::new(&format!(
        "DELETE {deleted}"
    )))])
}

fn hash_table_ref(table_ref: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    table_ref.hash(&mut hasher);
    hasher.finish()
}
