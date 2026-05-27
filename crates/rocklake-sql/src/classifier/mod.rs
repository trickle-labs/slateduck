//! SQL statement classifier: pattern-match on AST to identify DuckLake operations.
//!
//! This module is decomposed into sub-modules by concern:
//! - `normalize`: pre-parser AST normalizer (schema prefix stripping, subquery lifting)
//! - `prefix`: pre-parser classifiers for LISTEN/UNLISTEN
//! - `table_selects`: table select classifiers and identifier string helpers
//! - `ast`: AST-based SQL statement classifiers

mod ast;
pub(crate) mod normalize;
mod prefix;
mod table_selects;

use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

use crate::error::SqlDispatchError;

use ast::classify_ast;
use prefix::classify_listen_prefix;

/// Fast-path detection for COPY ... FROM STDIN.
/// Returns Some(StatementKind::CopyFromStdin) if the SQL matches.
fn classify_copy_from_stdin(sql: &str) -> Option<StatementKind> {
    let upper = sql.to_uppercase();
    // Match: COPY "public"."ducklake_*" FROM STDIN (FORMAT binary)
    // Also match: COPY public.ducklake_* FROM STDIN
    // Also match: COPY "public"."ducklake_*" (col1, col2) FROM STDIN ...
    if !upper.contains("COPY") || !upper.contains("FROM STDIN") {
        return None;
    }

    // Extract table name between COPY and FROM
    let copy_idx = upper.find("COPY")?;
    let from_idx = upper.find("FROM STDIN")?;
    if from_idx <= copy_idx + 4 {
        return None;
    }

    let table_part = sql[copy_idx + 4..from_idx].trim();
    // Normalize: strip quotes, schema prefix, and column list
    let table = normalize_copy_table(table_part);

    // Only accept ducklake_* tables
    if table.starts_with("ducklake_") {
        Some(StatementKind::CopyFromStdin { table })
    } else {
        // Unknown table — return None to fall through to sqlparser
        None
    }
}

/// Strip column list from COPY table reference.
/// `"public"."table" (col1, col2)` -> `"public"."table"`
fn strip_column_list(s: &str) -> &str {
    // Find the first '(' that's not inside quotes
    let mut in_quote = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            '(' if !in_quote => return s[..i].trim(),
            _ => {}
        }
    }
    s
}

/// Fast-path detection for COPY (SELECT ...) TO STDOUT.
/// Returns Some(StatementKind::CopyToStdout) if the SQL matches.
fn classify_copy_to_stdout(sql: &str) -> Option<StatementKind> {
    let upper = sql.to_uppercase();
    // Match: COPY (SELECT ...) TO STDOUT (FORMAT binary)
    if !upper.contains("COPY") || !upper.contains("TO STDOUT") {
        return None;
    }

    // Check for COPY (...) pattern
    let copy_idx = upper.find("COPY")?;
    let stdout_idx = upper.find("TO STDOUT")?;
    if stdout_idx <= copy_idx + 4 {
        return None;
    }

    // Extract the query between COPY ( and ) TO STDOUT
    let after_copy = &sql[copy_idx + 4..].trim_start();
    if !after_copy.starts_with('(') {
        return None;
    }

    // Find matching closing paren (simple approach: first ')' before TO STDOUT)
    let query_part = &sql[copy_idx + 4..stdout_idx];
    let query_part = query_part.trim();
    if !query_part.starts_with('(') || !query_part.ends_with(')') {
        return None;
    }

    // Extract the inner query
    let inner_query = &query_part[1..query_part.len() - 1].trim();
    if !inner_query.to_uppercase().starts_with("SELECT") {
        return None;
    }

    Some(StatementKind::CopyToStdout {
        query: inner_query.to_string(),
    })
}

/// Normalize a table name from COPY statement.
/// Strips quotes, `public.` schema prefix, and any column list `(col1, col2, ...)`.
fn normalize_copy_table(s: &str) -> String {
    let s = s.trim();

    // Strip column list: find the first '(' that's not inside quotes
    // COPY "public"."table" (col1, col2) -> "public"."table"
    let s = strip_column_list(s);

    // Remove surrounding quotes if present
    let s = s.trim_matches('"');
    // Handle schema-qualified: "public"."table" or public.table
    let s = if let Some(rest) = s.strip_prefix("public.") {
        rest.trim_matches('"')
    } else if let Some(rest) = s.strip_prefix("\"public\".") {
        rest.trim_matches('"')
    } else if s.contains(".") {
        // General case: schema.table
        let parts: Vec<&str> = s.splitn(2, '.').collect();
        if parts.len() == 2 {
            parts[1].trim_matches('"')
        } else {
            s
        }
    } else {
        s
    };
    s.to_lowercase()
}

/// The bounded set of SQL statement shapes supported by RockLake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatementKind {
    // ─── Session / Introspection ───────────────────────────────────────
    SelectVersion,
    /// `SELECT version(), (SELECT COUNT(*) FROM pg_settings WHERE name LIKE 'rds%')`
    /// — DuckDB PostgreSQL connector checks for AWS RDS
    SelectVersionWithRdsCheck,
    /// `SELECT 1` — Health check query
    SelectOne,
    SelectCurrentSchema,
    SelectCurrentDatabase,
    SelectPgType,
    ShowVariable(String),
    SetVariable(String, String),

    // ─── Session / Connection Management ──────────────────────────────
    /// `DISCARD ALL` / `DISCARD SEQUENCES` / `DISCARD PLANS` / `DISCARD TEMP`
    /// — DuckDB sends this when returning a connection to the pool.
    DiscardAll,
    /// `SELECT to_regclass('duckdb_secrets')` — DuckDB secret storage fast-path
    /// check. Returns `NULL` because RockLake has no `duckdb_secrets` table.
    SelectToRegclass,
    /// `SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE ...)`
    /// — DuckDB secret storage fallback check. Returns `false`.
    SelectExistsInfoSchema,
    /// `SELECT pg_database_size(current_database())` — informational size query.
    SelectPgDatabaseSize,
    /// Multi-statement catalog scan sent by the DuckDB postgres scanner as a
    /// single `PQsendQuery` call:
    /// `BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ;
    ///  SELECT oid, nspname FROM pg_namespace ...;
    ///  SELECT ... FROM pg_class JOIN ... UNION ALL ...;
    ///  SELECT ... FROM pg_enum JOIN ...;
    ///  SELECT ... FROM pg_type JOIN ...;
    ///  SELECT ... FROM pg_indexes JOIN ...;
    ///  ROLLBACK;`
    /// Returns five result sets in sequence.
    PgCatalogScan,

    // ─── Transaction Control ───────────────────────────────────────────
    Begin,
    Commit,
    Rollback,

    // ─── DuckLake Read Operations ──────────────────────────────────────
    SelectMaxSnapshot,
    /// Full latest-snapshot tuple used by DuckLake metadata manager:
    /// `SELECT snapshot_id, schema_version, next_catalog_id, next_file_id FROM ducklake_snapshot ...`
    SelectLatestSnapshotInfo,
    SelectSnapshotStatsAndChanges,
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
    SelectTableColumnStats,
    SelectMetadata,
    SelectSnapshot,
    /// `SELECT ... FROM ducklake_snapshot ORDER BY snapshot_id ASC LIMIT 1`
    SelectFirstSnapshot,
    /// `SELECT * FROM ducklake_snapshot_changes`
    SelectSnapshotChanges,
    SelectInlinedData,
    SelectViews,
    SelectMacros,
    SelectMacroImpls,
    SelectMacroParameters,
    /// `SELECT gen_random_uuid()` — pg-tide-relay generates UUIDs
    SelectGenRandomUuid,
    /// `SELECT ducklake_latest_snapshot_id($1::regclass)` — pg-trickle CDC
    /// startup handshake: resolves the latest snapshot boundary for a named
    /// table before calling `table_changes()`.  The table argument is the
    /// qualified table name cast to `regclass` (e.g. `'lake.events'::regclass`).
    ///
    /// Returns a single BIGINT column `ducklake_latest_snapshot_id` containing
    /// the `snapshot_id` of the latest visible snapshot, or NULL if no snapshot
    /// has been committed yet.
    SelectLatestSnapshotId,

    // ─── v0.27 DuckLake Facade Tables ─────────────────────────────────
    /// `SELECT * FROM ducklake_tag`
    SelectTags,
    /// `SELECT * FROM ducklake_column_tag`
    SelectColumnTags,
    /// `SELECT * FROM ducklake_sort_info`
    SelectSortInfo,
    /// `SELECT * FROM ducklake_schema_version`
    SelectSchemaVersion,
    SelectDuckLakeMetadataTable {
        table_name: String,
    },

    // ─── DuckLake Write Operations ─────────────────────────────────────
    InsertSnapshot,
    InsertSnapshotChanges,
    InsertSchema,
    InsertTable,
    InsertColumn,
    InsertDataFile,
    InsertDeleteFile,
    InsertTableStats,
    InsertTableColumnStats,
    InsertFileColumnStats,
    InsertMetadata,
    InsertInlinedDataTables,
    InsertSchemaVersions,
    InsertView,
    InsertMacro,
    InsertMacroImpl,
    InsertMacroParameters,

    UpdateEndSnapshot(String),
    UpdateTableStats,
    UpdateTableColumnStats,

    // ─── Inlined Data DDL/DML ──────────────────────────────────────────
    CreateInlinedTable,
    InsertInlinedRow,
    UpdateInlinedRowEndSnapshot,
    SelectInlinedRows,

    // ─── Virtual Catalog SQL Tables ────────────────────────────────────
    /// `SELECT * FROM rocklake_catalog.{table_name}` — read-only catalog introspection.
    /// Mutations against `rocklake_catalog.*` return SQLSTATE 25006.
    VirtualCatalogScan {
        table_name: String,
    },

    // ─── v0.18 DuckLake Standard Interface ─────────────────────────────
    /// `SELECT * FROM table_changes('schema.table', start_snapshot := N, end_snapshot := M)`
    TableChanges {
        table_ref: String,
        start_snapshot: u64,
        end_snapshot: u64,
    },
    /// `SELECT rocklake.next_rowid_range('schema.table', count := N)`
    NextRowidRange {
        table_ref: String,
        count: u64,
    },
    /// `SELECT rocklake.hold_snapshot(min_snapshot_id := N, consumer_id := '...', ttl_seconds := N)`
    HoldSnapshot {
        min_snapshot_id: u64,
        consumer_id: String,
        ttl_seconds: u64,
    },
    /// `SELECT rocklake.release_snapshot(consumer_id := '...')`
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

    // ─── COPY Protocol ─────────────────────────────────────────────────
    /// `COPY "public"."ducklake_*" FROM STDIN (FORMAT binary)`
    /// DuckDB 1.5+ uses binary COPY for catalog initialization.
    /// RockLake accepts and discards the payload (no-op).
    CopyFromStdin {
        table: String,
    },
    /// `COPY (SELECT ...) TO STDOUT (FORMAT binary)`
    /// DuckDB 1.5+ uses binary COPY to read catalog data.
    CopyToStdout {
        /// The embedded SQL query to execute.
        query: String,
    },

    // ─── Unsupported ───────────────────────────────────────────────────
    Unsupported(String),
}

/// Classify a SQL string into a `StatementKind`.
pub fn classify_statement(sql: &str) -> Result<StatementKind, SqlDispatchError> {
    // Apply AST normalization first: strip schema prefixes etc.
    let sql_cow = normalize::normalize_sql(sql);
    let sql = sql_cow.as_ref();
    let lower = sql.to_ascii_lowercase();

    // Pre-parse fast path for LISTEN/UNLISTEN.
    if let Some(result) = classify_listen_prefix(sql) {
        return result;
    }

    if lower.contains("update") && lower.contains("ducklake_table_column_stats") {
        return Ok(StatementKind::UpdateTableColumnStats);
    }

    if lower.contains("update") && lower.contains("ducklake_inlined_data_") {
        return Ok(StatementKind::UpdateInlinedRowEndSnapshot);
    }

    // `SET TIME ZONE <value>` is a PostgreSQL-specific syntax that sqlparser
    // may emit as a non-SetVariable AST node.  Map it to SetVariable so
    // drivers that send this form (e.g. pgcli, psycopg3) are accepted.
    if lower.starts_with("set time zone") {
        let tz = sql["set time zone".len()..]
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        return Ok(StatementKind::SetVariable("TimeZone".to_string(), tz));
    }

    if lower.contains("union all")
        && lower.contains("ducklake_snapshot")
        && lower.contains("ducklake_snapshot_changes")
        && lower.contains("ducklake_table_stats")
        && lower.contains("ducklake_table_column_stats")
    {
        return Ok(StatementKind::SelectSnapshotStatsAndChanges);
    }

    // Pre-parse detection for the multi-statement pg_namespace catalog scan.
    // DuckDB postgres scanner sends this as a single PQsendQuery call; sqlparser
    // would only see the leading BEGIN. Detect by characteristic tokens.
    if sql.contains("pg_namespace") && sql.contains("pg_class") {
        return Ok(StatementKind::PgCatalogScan);
    }

    // Pre-parse fast path for COPY ... FROM STDIN (binary).
    // DuckDB 1.5+ uses binary COPY for catalog initialization.
    // sqlparser doesn't handle the protocol-level COPY syntax, so we detect it here.
    if let Some(kind) = classify_copy_from_stdin(sql) {
        return Ok(kind);
    }

    // Pre-parse fast path for COPY (SELECT ...) TO STDOUT (binary).
    // DuckDB 1.5+ uses binary COPY to read catalog data.
    if let Some(kind) = classify_copy_to_stdout(sql) {
        return Ok(kind);
    }

    let dialect = PostgreSqlDialect {};
    let statements = Parser::parse_sql(&dialect, sql)
        .map_err(|e| SqlDispatchError::ParseError(e.to_string()))?;

    if statements.is_empty() {
        return Err(SqlDispatchError::ParseError("empty statement".to_string()));
    }

    // Attempt to lift trivial subquery wrappers added by some client drivers.
    let stmt = &statements[0];
    if let Some(lifted) = normalize::try_lift_trivial_subquery(stmt) {
        return Ok(classify_ast(&lifted));
    }

    Ok(classify_ast(stmt))
}
