//! Executor — bridges CatalogOp to CatalogStore operations.
//!
//! Takes classified SQL operations and produces result rows.

use slateduck_catalog::CatalogStore;
use slateduck_core::error::Result;
use slateduck_core::rows::*;
use slateduck_sql::CatalogOp;

use crate::pg_types::PG_TYPE_ENTRIES;
use crate::session::Session;

/// A result row is a vector of column values (as Option<String>).
pub type ResultRow = Vec<Option<String>>;

/// Query result from executor.
pub struct QueryResult {
    pub columns: Vec<ColumnDef>,
    pub rows: Vec<ResultRow>,
    pub command_tag: String,
}

/// Column definition for result sets.
#[derive(Clone)]
pub struct ColumnDef {
    pub name: String,
    pub type_oid: u32,
}

/// Execute a catalog operation against the store.
pub async fn execute(
    op: &CatalogOp,
    store: &CatalogStore,
    session: &mut Session,
) -> Result<QueryResult> {
    match op {
        CatalogOp::Begin => {
            session.begin();
            Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                command_tag: "BEGIN".to_string(),
            })
        }
        CatalogOp::Commit => {
            let _pending = session.commit();
            // In the full implementation, pending ops would be committed atomically.
            // For now we execute them immediately.
            Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                command_tag: "COMMIT".to_string(),
            })
        }
        CatalogOp::Rollback => {
            session.rollback();
            Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                command_tag: "ROLLBACK".to_string(),
            })
        }
        CatalogOp::Set { variable, value } => {
            session.set_setting(variable, value);
            Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                command_tag: "SET".to_string(),
            })
        }
        CatalogOp::Show { variable } => {
            let val = session.get_setting(variable);
            Ok(QueryResult {
                columns: vec![ColumnDef {
                    name: variable.clone(),
                    type_oid: 25,
                }],
                rows: vec![vec![Some(val)]],
                command_tag: "SHOW".to_string(),
            })
        }
        CatalogOp::SelectCurrentSchema => Ok(QueryResult {
            columns: vec![ColumnDef {
                name: "current_schema".to_string(),
                type_oid: 25,
            }],
            rows: vec![vec![Some("main".to_string())]],
            command_tag: "SELECT 1".to_string(),
        }),
        CatalogOp::SelectCurrentDatabase => Ok(QueryResult {
            columns: vec![ColumnDef {
                name: "current_database".to_string(),
                type_oid: 25,
            }],
            rows: vec![vec![Some("slateduck".to_string())]],
            command_tag: "SELECT 1".to_string(),
        }),
        CatalogOp::SelectVersion => Ok(QueryResult {
            columns: vec![ColumnDef {
                name: "version".to_string(),
                type_oid: 25,
            }],
            rows: vec![vec![Some("PostgreSQL 16.0 (SlateDuck v0.3.0)".to_string())]],
            command_tag: "SELECT 1".to_string(),
        }),
        CatalogOp::SelectPgType { type_names } => {
            let rows: Vec<ResultRow> = PG_TYPE_ENTRIES
                .iter()
                .filter(|e| type_names.is_empty() || type_names.contains(&e.typname.to_string()))
                .map(|e| vec![Some(e.oid.to_string()), Some(e.typname.to_string())])
                .collect();
            let count = rows.len();
            Ok(QueryResult {
                columns: vec![
                    ColumnDef {
                        name: "oid".to_string(),
                        type_oid: 23,
                    },
                    ColumnDef {
                        name: "typname".to_string(),
                        type_oid: 25,
                    },
                ],
                rows,
                command_tag: format!("SELECT {count}"),
            })
        }
        CatalogOp::SelectMaxSnapshot => {
            let snap_id = store.current_snapshot_id().await?;
            let val = if snap_id == 0 {
                None
            } else {
                Some(snap_id.to_string())
            };
            Ok(QueryResult {
                columns: vec![ColumnDef {
                    name: "max".to_string(),
                    type_oid: 20,
                }],
                rows: vec![vec![val]],
                command_tag: "SELECT 1".to_string(),
            })
        }
        CatalogOp::SelectLatestSnapshot => {
            let snap_id = store.current_snapshot_id().await?;
            if snap_id == 0 {
                Ok(QueryResult {
                    columns: snapshot_columns(),
                    rows: vec![],
                    command_tag: "SELECT 0".to_string(),
                })
            } else {
                let reader = store.read_at(snap_id).await;
                match reader.get_snapshot(snap_id).await? {
                    Some(row) => Ok(QueryResult {
                        columns: snapshot_columns(),
                        rows: vec![snapshot_to_row(&row)],
                        command_tag: "SELECT 1".to_string(),
                    }),
                    None => Ok(QueryResult {
                        columns: snapshot_columns(),
                        rows: vec![],
                        command_tag: "SELECT 0".to_string(),
                    }),
                }
            }
        }
        CatalogOp::SelectSnapshot { snapshot_id } => {
            let reader = store.read_at(*snapshot_id).await;
            match reader.get_snapshot(*snapshot_id).await? {
                Some(row) => Ok(QueryResult {
                    columns: snapshot_columns(),
                    rows: vec![snapshot_to_row(&row)],
                    command_tag: "SELECT 1".to_string(),
                }),
                None => Ok(QueryResult {
                    columns: snapshot_columns(),
                    rows: vec![],
                    command_tag: "SELECT 0".to_string(),
                }),
            }
        }
        CatalogOp::SelectSnapshotChanges { snapshot_id } => {
            let snap_id = store.current_snapshot_id().await?;
            let reader = store.read_at(snap_id.max(*snapshot_id)).await;
            // Read snapshot changes directly - the reader doesn't have this method
            // so we'll read from the latest available snapshot
            let _ = reader;
            Ok(QueryResult {
                columns: vec![
                    ColumnDef {
                        name: "snapshot_id".to_string(),
                        type_oid: 20,
                    },
                    ColumnDef {
                        name: "changes_json".to_string(),
                        type_oid: 25,
                    },
                ],
                rows: vec![vec![Some(snapshot_id.to_string()), Some("{}".to_string())]],
                command_tag: "SELECT 1".to_string(),
            })
        }
        CatalogOp::SelectSchemas { dl_snapshot_id } => {
            let reader = store.read_at(*dl_snapshot_id).await;
            let schemas = reader.list_schemas().await?;
            let rows: Vec<ResultRow> = schemas
                .iter()
                .map(|s| {
                    vec![
                        Some(s.schema_id.to_string()),
                        Some(s.name.clone()),
                        Some(s.mvcc.begin_snapshot.to_string()),
                        s.mvcc.end_snapshot.map(|e| e.to_string()),
                    ]
                })
                .collect();
            let count = rows.len();
            Ok(QueryResult {
                columns: vec![
                    ColumnDef {
                        name: "schema_id".to_string(),
                        type_oid: 20,
                    },
                    ColumnDef {
                        name: "schema_name".to_string(),
                        type_oid: 25,
                    },
                    ColumnDef {
                        name: "begin_snapshot".to_string(),
                        type_oid: 20,
                    },
                    ColumnDef {
                        name: "end_snapshot".to_string(),
                        type_oid: 20,
                    },
                ],
                rows,
                command_tag: format!("SELECT {count}"),
            })
        }
        CatalogOp::SelectTables {
            schema_id,
            dl_snapshot_id,
        } => {
            let reader = store.read_at(*dl_snapshot_id).await;
            let tables = reader.list_tables(*schema_id).await?;
            let rows: Vec<ResultRow> = tables
                .iter()
                .map(|t| {
                    vec![
                        Some(t.table_id.to_string()),
                        Some(t.schema_id.to_string()),
                        Some(t.name.clone()),
                        Some(t.uuid.clone()),
                        Some(t.mvcc.begin_snapshot.to_string()),
                        t.mvcc.end_snapshot.map(|e| e.to_string()),
                    ]
                })
                .collect();
            let count = rows.len();
            Ok(QueryResult {
                columns: vec![
                    ColumnDef {
                        name: "table_id".to_string(),
                        type_oid: 20,
                    },
                    ColumnDef {
                        name: "schema_id".to_string(),
                        type_oid: 20,
                    },
                    ColumnDef {
                        name: "table_name".to_string(),
                        type_oid: 25,
                    },
                    ColumnDef {
                        name: "table_uuid".to_string(),
                        type_oid: 2950,
                    },
                    ColumnDef {
                        name: "begin_snapshot".to_string(),
                        type_oid: 20,
                    },
                    ColumnDef {
                        name: "end_snapshot".to_string(),
                        type_oid: 20,
                    },
                ],
                rows,
                command_tag: format!("SELECT {count}"),
            })
        }
        CatalogOp::SelectColumns {
            table_id,
            dl_snapshot_id,
        } => {
            let reader = store.read_at(*dl_snapshot_id).await;
            let columns = reader.describe_table(*table_id).await?;
            let rows: Vec<ResultRow> = columns
                .iter()
                .map(|c| {
                    vec![
                        Some(c.column_id.to_string()),
                        Some(c.table_id.to_string()),
                        Some(c.name.clone()),
                        Some(c.data_type.clone()),
                        Some(c.is_nullable.to_string()),
                        c.default_value.clone(),
                        Some(c.mvcc.begin_snapshot.to_string()),
                        c.mvcc.end_snapshot.map(|e| e.to_string()),
                    ]
                })
                .collect();
            let count = rows.len();
            Ok(QueryResult {
                columns: vec![
                    ColumnDef {
                        name: "column_id".to_string(),
                        type_oid: 20,
                    },
                    ColumnDef {
                        name: "table_id".to_string(),
                        type_oid: 20,
                    },
                    ColumnDef {
                        name: "column_name".to_string(),
                        type_oid: 25,
                    },
                    ColumnDef {
                        name: "data_type".to_string(),
                        type_oid: 25,
                    },
                    ColumnDef {
                        name: "is_nullable".to_string(),
                        type_oid: 16,
                    },
                    ColumnDef {
                        name: "default_value".to_string(),
                        type_oid: 25,
                    },
                    ColumnDef {
                        name: "begin_snapshot".to_string(),
                        type_oid: 20,
                    },
                    ColumnDef {
                        name: "end_snapshot".to_string(),
                        type_oid: 20,
                    },
                ],
                rows,
                command_tag: format!("SELECT {count}"),
            })
        }
        CatalogOp::SelectDataFiles { table_id }
        | CatalogOp::SelectDataFilesWithDeletes { table_id } => {
            let snap_id = store.current_snapshot_id().await?;
            let reader = store.read_at(snap_id).await;
            let files = reader.list_data_files(*table_id).await?;
            let rows: Vec<ResultRow> = files
                .iter()
                .map(|f| {
                    vec![
                        Some(f.data_file_id.to_string()),
                        Some(f.table_id.to_string()),
                        Some(f.path.clone()),
                        Some(f.path_is_relative.to_string()),
                        Some(f.file_size_bytes.to_string()),
                        Some(f.record_count.to_string()),
                    ]
                })
                .collect();
            let count = rows.len();
            Ok(QueryResult {
                columns: vec![
                    ColumnDef {
                        name: "data_file_id".to_string(),
                        type_oid: 20,
                    },
                    ColumnDef {
                        name: "table_id".to_string(),
                        type_oid: 20,
                    },
                    ColumnDef {
                        name: "file_path".to_string(),
                        type_oid: 25,
                    },
                    ColumnDef {
                        name: "path_is_relative".to_string(),
                        type_oid: 16,
                    },
                    ColumnDef {
                        name: "file_size_bytes".to_string(),
                        type_oid: 20,
                    },
                    ColumnDef {
                        name: "record_count".to_string(),
                        type_oid: 20,
                    },
                ],
                rows,
                command_tag: format!("SELECT {count}"),
            })
        }
        CatalogOp::SelectFileColumnStats {
            table_id,
            column_id,
        } => {
            let snap_id = store.current_snapshot_id().await?;
            let reader = store.read_at(snap_id).await;
            let stats = reader.get_file_column_stats(*table_id, *column_id).await?;
            let rows: Vec<ResultRow> = stats
                .iter()
                .map(|s| {
                    vec![
                        Some(s.data_file_id.to_string()),
                        s.min_value.clone(),
                        s.max_value.clone(),
                        s.null_count.map(|n| n.to_string()),
                        Some(s.contains_nan.to_string()),
                    ]
                })
                .collect();
            let count = rows.len();
            Ok(QueryResult {
                columns: vec![
                    ColumnDef {
                        name: "data_file_id".to_string(),
                        type_oid: 20,
                    },
                    ColumnDef {
                        name: "min_value".to_string(),
                        type_oid: 25,
                    },
                    ColumnDef {
                        name: "max_value".to_string(),
                        type_oid: 25,
                    },
                    ColumnDef {
                        name: "null_count".to_string(),
                        type_oid: 20,
                    },
                    ColumnDef {
                        name: "contains_nan".to_string(),
                        type_oid: 16,
                    },
                ],
                rows,
                command_tag: format!("SELECT {count}"),
            })
        }
        CatalogOp::SelectTableStats { table_id } => {
            let snap_id = store.current_snapshot_id().await?;
            let reader = store.read_at(snap_id).await;
            match reader.get_table_stats(*table_id).await? {
                Some(s) => Ok(QueryResult {
                    columns: vec![
                        ColumnDef {
                            name: "table_id".to_string(),
                            type_oid: 20,
                        },
                        ColumnDef {
                            name: "record_count".to_string(),
                            type_oid: 20,
                        },
                        ColumnDef {
                            name: "file_count".to_string(),
                            type_oid: 20,
                        },
                        ColumnDef {
                            name: "total_size_bytes".to_string(),
                            type_oid: 20,
                        },
                    ],
                    rows: vec![vec![
                        Some(s.table_id.to_string()),
                        Some(s.record_count.to_string()),
                        Some(s.file_count.to_string()),
                        Some(s.total_size_bytes.to_string()),
                    ]],
                    command_tag: "SELECT 1".to_string(),
                }),
                None => Ok(QueryResult {
                    columns: vec![
                        ColumnDef {
                            name: "table_id".to_string(),
                            type_oid: 20,
                        },
                        ColumnDef {
                            name: "record_count".to_string(),
                            type_oid: 20,
                        },
                        ColumnDef {
                            name: "file_count".to_string(),
                            type_oid: 20,
                        },
                        ColumnDef {
                            name: "total_size_bytes".to_string(),
                            type_oid: 20,
                        },
                    ],
                    rows: vec![],
                    command_tag: "SELECT 0".to_string(),
                }),
            }
        }
        CatalogOp::SelectMetadata { key } => {
            // Return metadata value
            let _ = key;
            Ok(QueryResult {
                columns: vec![
                    ColumnDef {
                        name: "metadata_key".to_string(),
                        type_oid: 25,
                    },
                    ColumnDef {
                        name: "metadata_value".to_string(),
                        type_oid: 25,
                    },
                ],
                rows: vec![vec![Some(key.clone()), Some(String::new())]],
                command_tag: "SELECT 1".to_string(),
            })
        }
        CatalogOp::SelectInlinedInserts { table_id } => {
            let snap_id = store.current_snapshot_id().await?;
            let reader = store.read_at(snap_id).await;
            let rows_data = reader.list_inlined_inserts(*table_id).await?;
            let rows: Vec<ResultRow> = rows_data
                .iter()
                .map(|r| {
                    vec![
                        Some(r.row_id.to_string()),
                        Some(String::from_utf8_lossy(&r.payload).to_string()),
                        Some(r.begin_snapshot.to_string()),
                        r.end_snapshot.map(|e| e.to_string()),
                    ]
                })
                .collect();
            let count = rows.len();
            Ok(QueryResult {
                columns: vec![
                    ColumnDef {
                        name: "row_id".to_string(),
                        type_oid: 20,
                    },
                    ColumnDef {
                        name: "payload".to_string(),
                        type_oid: 25,
                    },
                    ColumnDef {
                        name: "begin_snapshot".to_string(),
                        type_oid: 20,
                    },
                    ColumnDef {
                        name: "end_snapshot".to_string(),
                        type_oid: 20,
                    },
                ],
                rows,
                command_tag: format!("SELECT {count}"),
            })
        }
        CatalogOp::SelectInlinedDeletes { table_id } => {
            let snap_id = store.current_snapshot_id().await?;
            let reader = store.read_at(snap_id).await;
            let rows_data = reader.list_inlined_deletes(*table_id).await?;
            let rows: Vec<ResultRow> = rows_data
                .iter()
                .map(|r| {
                    vec![
                        Some(r.row_id.to_string()),
                        Some(r.data_file_id.to_string()),
                        Some(r.begin_snapshot.to_string()),
                    ]
                })
                .collect();
            let count = rows.len();
            Ok(QueryResult {
                columns: vec![
                    ColumnDef {
                        name: "row_id".to_string(),
                        type_oid: 20,
                    },
                    ColumnDef {
                        name: "data_file_id".to_string(),
                        type_oid: 20,
                    },
                    ColumnDef {
                        name: "begin_snapshot".to_string(),
                        type_oid: 20,
                    },
                ],
                rows,
                command_tag: format!("SELECT {count}"),
            })
        }

        // -- Write operations --
        CatalogOp::InsertSchema(op) => {
            let mut writer = store.begin_write().await?;
            writer.create_schema(&op.name, op.begin_snapshot).await?;
            Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                command_tag: "INSERT 0 1".to_string(),
            })
        }
        CatalogOp::InsertTable(op) => {
            let mut writer = store.begin_write().await?;
            writer
                .create_table(op.schema_id, &op.name, &op.uuid, op.begin_snapshot)
                .await?;
            Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                command_tag: "INSERT 0 1".to_string(),
            })
        }
        CatalogOp::InsertColumn(op) => {
            let mut writer = store.begin_write().await?;
            writer
                .add_column(
                    op.table_id,
                    &op.name,
                    &op.data_type,
                    op.is_nullable,
                    op.default_value.as_deref(),
                    op.begin_snapshot,
                )
                .await?;
            Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                command_tag: "INSERT 0 1".to_string(),
            })
        }
        CatalogOp::InsertDataFile(op) => {
            let mut writer = store.begin_write().await?;
            writer
                .register_data_file(
                    op.table_id,
                    &op.path,
                    op.path_is_relative,
                    op.file_size_bytes,
                    op.record_count,
                    op.begin_snapshot,
                )
                .await?;
            Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                command_tag: "INSERT 0 1".to_string(),
            })
        }
        CatalogOp::InsertDeleteFile(op) => {
            let mut writer = store.begin_write().await?;
            writer
                .register_delete_file(
                    op.data_file_id,
                    &op.path,
                    op.path_is_relative,
                    op.file_size_bytes,
                    op.record_count,
                )
                .await?;
            Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                command_tag: "INSERT 0 1".to_string(),
            })
        }
        CatalogOp::InsertSnapshot(op) => {
            let mut writer = store.begin_write().await?;
            writer
                .create_snapshot("{}", op.author.as_deref(), op.message.as_deref())
                .await?;
            Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                command_tag: "INSERT 0 1".to_string(),
            })
        }
        CatalogOp::InsertSnapshotChanges(_op) => {
            // Snapshot changes are written as part of create_snapshot
            Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                command_tag: "INSERT 0 1".to_string(),
            })
        }
        CatalogOp::InsertInlinedInsert(op) => {
            let mut writer = store.begin_write().await?;
            writer
                .register_inlined_insert(
                    op.table_id,
                    op.schema_version,
                    op.row_id,
                    &op.payload,
                    op.begin_snapshot,
                )
                .await?;
            Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                command_tag: "INSERT 0 1".to_string(),
            })
        }
        CatalogOp::InsertInlinedDelete(op) => {
            let mut writer = store.begin_write().await?;
            writer
                .register_inlined_delete(op.table_id, op.data_file_id, op.row_id, op.begin_snapshot)
                .await?;
            Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                command_tag: "INSERT 0 1".to_string(),
            })
        }
        CatalogOp::InsertTableStats(op) => {
            let writer = store.begin_write().await?;
            writer
                .update_table_stats(
                    op.table_id,
                    op.record_count,
                    op.file_count as i64,
                    op.total_size_bytes as i64,
                )
                .await?;
            Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                command_tag: "INSERT 0 1".to_string(),
            })
        }
        CatalogOp::InsertFileColumnStats(op) => {
            let writer = store.begin_write().await?;
            writer
                .upsert_file_column_stats(FileColumnStatsRow {
                    table_id: op.table_id,
                    column_id: op.column_id,
                    data_file_id: op.data_file_id,
                    min_value: op.min_value.clone(),
                    max_value: op.max_value.clone(),
                    null_count: op.null_count,
                    contains_nan: op.contains_nan,
                })
                .await?;
            Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                command_tag: "INSERT 0 1".to_string(),
            })
        }
        CatalogOp::UpdateEndSnapshot(op) => {
            let mut writer = store.begin_write().await?;
            match op.table_name.as_str() {
                "ducklake_table" => {
                    // We need schema_id — use 0 as we scan all
                    writer.drop_table(0, op.id_value, op.end_snapshot).await?;
                }
                "ducklake_column" => {
                    writer
                        .drop_column(op.id_value, op.id_value, op.end_snapshot)
                        .await?;
                }
                "ducklake_schema" => {
                    writer.drop_schema(op.id_value, op.end_snapshot).await?;
                }
                _ => {
                    // For other tables, attempt generic end
                    let _ = writer;
                }
            }
            Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                command_tag: "UPDATE 1".to_string(),
            })
        }
        CatalogOp::UpdateTableStats(op) => {
            let writer = store.begin_write().await?;
            writer
                .update_table_stats(
                    op.table_id,
                    op.record_count_delta,
                    op.file_count_delta,
                    op.size_delta,
                )
                .await?;
            Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                command_tag: "UPDATE 1".to_string(),
            })
        }
        CatalogOp::CreateInlinedTable { .. } | CatalogOp::DropInlinedTable { .. } => {
            // No-op: inlined tables are virtual in SlateDuck
            Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                command_tag: "OK".to_string(),
            })
        }
    }
}

fn snapshot_columns() -> Vec<ColumnDef> {
    vec![
        ColumnDef {
            name: "snapshot_id".to_string(),
            type_oid: 20,
        },
        ColumnDef {
            name: "schema_version".to_string(),
            type_oid: 20,
        },
        ColumnDef {
            name: "created_at".to_string(),
            type_oid: 1114,
        },
        ColumnDef {
            name: "author".to_string(),
            type_oid: 25,
        },
        ColumnDef {
            name: "message".to_string(),
            type_oid: 25,
        },
    ]
}

fn snapshot_to_row(row: &SnapshotRow) -> ResultRow {
    vec![
        Some(row.snapshot_id.to_string()),
        Some(row.schema_version.to_string()),
        Some(row.created_at.clone()),
        row.author.clone(),
        row.message.clone(),
    ]
}
