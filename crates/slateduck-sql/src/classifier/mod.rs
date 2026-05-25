//! SQL statement classifier: pattern-match on AST to identify DuckLake operations.
//!
//! This module is decomposed into sub-modules by concern:
//! - `prefix`: pre-parser classifiers for IVM DDL and LISTEN/UNLISTEN
//! - `table_selects`: table select classifiers and identifier string helpers
//! - `ast`: AST-based SQL statement classifiers

mod ast;
mod prefix;
mod table_selects;

use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

use crate::error::SqlDispatchError;

use ast::classify_ast;
use prefix::{classify_ivm_prefix, classify_listen_prefix};

/// The bounded set of SQL statement shapes supported by SlateDuck.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatementKind {
    // ─── Session / Introspection ───────────────────────────────────────
    SelectVersion,
    SelectCurrentSchema,
    SelectCurrentDatabase,
    SelectPgType,
    ShowVariable(String),
    SetVariable(String, String),

    // ─── Transaction Control ───────────────────────────────────────────
    Begin,
    Commit,
    Rollback,

    // ─── DuckLake Read Operations ──────────────────────────────────────
    SelectMaxSnapshot,
    /// `SELECT max(snapshot_id) FROM ducklake_snapshot WHERE snapshot_id > $1`
    SelectMaxSnapshotAfter,
    SelectSchemas,
    SelectTables,
    SelectColumns,
    SelectDataFiles,
    /// `SELECT ... FROM ducklake_data_file ... LIMIT $N` (parameterized limit)
    SelectDataFilesWithLimit,
    SelectDeleteFiles,
    SelectFileColumnStats,
    SelectTableStats,
    SelectMetadata,
    SelectSnapshot,
    /// `SELECT ... FROM ducklake_snapshot ORDER BY snapshot_id ASC LIMIT 1`
    SelectFirstSnapshot,
    SelectInlinedData,
    SelectViews,
    SelectMacros,
    /// `SELECT gen_random_uuid()` — pg-tide-relay generates UUIDs
    SelectGenRandomUuid,

    // ─── DuckLake Write Operations ─────────────────────────────────────
    InsertSnapshot,
    InsertSnapshotChanges,
    InsertSchema,
    InsertTable,
    InsertColumn,
    InsertDataFile,
    InsertDeleteFile,
    InsertTableStats,
    InsertFileColumnStats,
    InsertMetadata,
    InsertInlinedDataTables,
    InsertView,
    InsertMacro,
    InsertMacroImpl,
    InsertMacroParameters,

    UpdateEndSnapshot(String),
    UpdateTableStats,

    // ─── Inlined Data DDL/DML ──────────────────────────────────────────
    CreateInlinedTable,
    InsertInlinedRow,
    UpdateInlinedRowEndSnapshot,
    SelectInlinedRows,

    // ─── Virtual Catalog SQL Tables ────────────────────────────────────
    /// `SELECT * FROM slateduck_catalog.{table_name}` — read-only catalog introspection.
    /// Mutations against `slateduck_catalog.*` return SQLSTATE 25006.
    VirtualCatalogScan {
        table_name: String,
    },

    // ─── v0.11 IVM Statements ───────────────────────────────────────────
    /// `CREATE INCREMENTAL MATERIALIZED VIEW [schema.]name AS <select>`
    CreateIncrementalMatview {
        name: String,
        schema: Option<String>,
        select_sql: String,
        with_options: Vec<(String, String)>,
    },
    /// `DROP INCREMENTAL MATERIALIZED VIEW [IF EXISTS] [schema.]name`
    DropIncrementalMatview {
        name: String,
        schema: Option<String>,
        if_exists: bool,
    },
    /// `ALTER INCREMENTAL MATERIALIZED VIEW [schema.]name SET (option=value, ...)`
    AlterIncrementalMatview {
        name: String,
        schema: Option<String>,
        options: Vec<(String, String)>,
    },
    /// `REFRESH INCREMENTAL MATERIALIZED VIEW [schema.]name FULL`
    RefreshIncrementalMatviewFull {
        name: String,
        schema: Option<String>,
    },
    /// `SHOW MATERIALIZED VIEWS`
    ShowMaterializedViews,
    /// `SHOW MATVIEW SHARDS [schema.]name`
    ShowMatviewShards {
        view_name: String,
        schema: Option<String>,
    },
    /// `EXPLAIN MATVIEW [schema.]name`
    ExplainMatview {
        view_name: String,
        schema: Option<String>,
    },

    // ─── v0.18 DuckLake Standard Interface ─────────────────────────────
    /// `SELECT * FROM table_changes('schema.table', start_snapshot := N, end_snapshot := M)`
    TableChanges {
        table_ref: String,
        start_snapshot: u64,
        end_snapshot: u64,
    },
    /// `SELECT slateduck.next_rowid_range('schema.table', count := N)`
    NextRowidRange {
        table_ref: String,
        count: u64,
    },
    /// `SELECT slateduck.hold_snapshot(min_snapshot_id := N, consumer_id := '...', ttl_seconds := N)`
    HoldSnapshot {
        min_snapshot_id: u64,
        consumer_id: String,
        ttl_seconds: u64,
    },
    /// `SELECT slateduck.release_snapshot(consumer_id := '...')`
    ReleaseSnapshot {
        consumer_id: String,
    },
    /// `LISTEN channel`
    Listen {
        channel: String,
    },
    /// `UNLISTEN channel`
    Unlisten {
        channel: String,
    },
    /// `CREATE TABLE IF NOT EXISTS <extension_schema>.<table> (...)`
    CreateExtensionTable {
        schema_name: String,
        table_name: String,
    },
    /// `INSERT INTO <extension_schema>.<table> (...) VALUES (...)`
    InsertExtensionRow {
        schema_name: String,
        table_name: String,
        columns: Vec<String>,
        values_json: String,
    },
    /// `SELECT ... FROM <extension_schema>.<table>`
    SelectExtensionTable {
        schema_name: String,
        table_name: String,
    },
    /// `DELETE FROM <extension_schema>.<table> WHERE ...`
    DeleteExtensionRows {
        schema_name: String,
        table_name: String,
    },

    // ─── Unsupported ───────────────────────────────────────────────────
    Unsupported(String),
}

/// Classify a SQL string into a `StatementKind`.
pub fn classify_statement(sql: &str) -> Result<StatementKind, SqlDispatchError> {
    // Pre-parse fast path for IVM custom syntax not supported by sqlparser-rs.
    if let Some(kind) = classify_ivm_prefix(sql) {
        return Ok(kind);
    }

    // Pre-parse fast path for LISTEN/UNLISTEN.
    if let Some(result) = classify_listen_prefix(sql) {
        return result;
    }

    let dialect = PostgreSqlDialect {};
    let statements = Parser::parse_sql(&dialect, sql)
        .map_err(|e| SqlDispatchError::ParseError(e.to_string()))?;

    if statements.is_empty() {
        return Err(SqlDispatchError::ParseError("empty statement".to_string()));
    }

    Ok(classify_ast(&statements[0]))
}
