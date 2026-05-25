//! SQL statement classifier: pattern-match on AST to identify DuckLake operations.
//!
//! All classification is done on `sqlparser-rs` AST nodes, never on raw SQL strings.

use sqlparser::ast::{Expr, ObjectName, SelectItem, SetExpr, Statement, TableFactor, TableObject};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

use crate::error::SqlDispatchError;

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

    // ─── Unsupported ───────────────────────────────────────────────────
    Unsupported(String),
}

/// Classify a SQL string into a `StatementKind`.
pub fn classify_statement(sql: &str) -> Result<StatementKind, SqlDispatchError> {
    // Pre-parse fast path for IVM custom syntax not supported by sqlparser-rs.
    if let Some(kind) = classify_ivm_prefix(sql) {
        return Ok(kind);
    }

    let dialect = PostgreSqlDialect {};
    let statements = Parser::parse_sql(&dialect, sql)
        .map_err(|e| SqlDispatchError::ParseError(e.to_string()))?;

    if statements.is_empty() {
        return Err(SqlDispatchError::ParseError("empty statement".to_string()));
    }

    Ok(classify_ast(&statements[0]))
}

/// Fast string-based pre-classifier for IVM DDL statements that sqlparser-rs
/// cannot parse (non-standard keyword combinations like INCREMENTAL).
fn classify_ivm_prefix(sql: &str) -> Option<StatementKind> {
    let upper = sql.trim().to_uppercase();
    let trimmed = sql.trim();

    if upper.starts_with("CREATE INCREMENTAL MATERIALIZED VIEW") {
        // Extract "[[schema.]name] AS ..."
        let rest = &trimmed["CREATE INCREMENTAL MATERIALIZED VIEW".len()..].trim_start();
        // Split off the AS clause.
        let (name_part, select_sql) = if let Some(pos) = find_as_keyword(rest) {
            (&rest[..pos].trim(), rest[pos + 2..].trim().to_string())
        } else {
            return Some(StatementKind::Unsupported(
                "CREATE INCREMENTAL MATERIALIZED VIEW missing AS clause".to_string(),
            ));
        };
        let (schema, name) = split_qualified_name(name_part);
        return Some(StatementKind::CreateIncrementalMatview {
            name,
            schema,
            select_sql,
            with_options: Vec::new(),
        });
    }

    if upper.starts_with("DROP INCREMENTAL MATERIALIZED VIEW") {
        let rest = trimmed["DROP INCREMENTAL MATERIALIZED VIEW".len()..].trim_start();
        let (if_exists, name_part) = if rest.to_uppercase().starts_with("IF EXISTS") {
            (true, rest["IF EXISTS".len()..].trim_start())
        } else {
            (false, rest)
        };
        let (schema, name) = split_qualified_name(name_part);
        return Some(StatementKind::DropIncrementalMatview {
            name,
            schema,
            if_exists,
        });
    }

    if upper.starts_with("ALTER INCREMENTAL MATERIALIZED VIEW") {
        let rest = trimmed["ALTER INCREMENTAL MATERIALIZED VIEW".len()..].trim_start();
        // Just capture the name; options parsing is a v0.12 concern.
        let (schema, name) = split_qualified_name(rest);
        return Some(StatementKind::AlterIncrementalMatview {
            name,
            schema,
            options: Vec::new(),
        });
    }

    if upper.starts_with("REFRESH INCREMENTAL MATERIALIZED VIEW") {
        let rest = trimmed["REFRESH INCREMENTAL MATERIALIZED VIEW".len()..].trim_start();
        let name_part = rest
            .trim_end_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '_')
            .trim();
        let (schema, name) = split_qualified_name(name_part);
        return Some(StatementKind::RefreshIncrementalMatviewFull { name, schema });
    }

    if upper.starts_with("SHOW MATERIALIZED VIEWS") {
        return Some(StatementKind::ShowMaterializedViews);
    }

    if upper.starts_with("SHOW MATVIEW SHARDS") {
        let rest = trimmed["SHOW MATVIEW SHARDS".len()..].trim_start();
        let (schema, name) = split_qualified_name(rest);
        return Some(StatementKind::ShowMatviewShards {
            view_name: name,
            schema,
        });
    }

    if upper.starts_with("EXPLAIN MATVIEW") {
        let rest = trimmed["EXPLAIN MATVIEW".len()..].trim_start();
        let (schema, name) = split_qualified_name(rest);
        return Some(StatementKind::ExplainMatview {
            view_name: name,
            schema,
        });
    }

    None
}

/// Find the byte position of a standalone ` AS ` keyword (case-insensitive).
fn find_as_keyword(s: &str) -> Option<usize> {
    let upper = s.to_uppercase();
    // Search for " AS " with word boundaries (space on both sides).
    let mut i = 0;
    while i + 4 <= upper.len() {
        if &upper[i..i + 4] == " AS " {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Split "schema.name" or just "name" from a name fragment.
/// Returns `(schema, name)`.
fn split_qualified_name(s: &str) -> (Option<String>, String) {
    // Take just the first "word" (identifiers, dots, underscores).
    let token: String = s
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.' || *c == '"')
        .collect();
    if let Some(dot) = token.find('.') {
        (Some(token[..dot].to_string()), token[dot + 1..].to_string())
    } else {
        (None, token)
    }
}

fn classify_ast(stmt: &Statement) -> StatementKind {
    match stmt {
        // Transaction control
        Statement::StartTransaction { .. } => StatementKind::Begin,
        Statement::Commit { .. } => StatementKind::Commit,
        Statement::Rollback { .. } => StatementKind::Rollback,

        // SET variable
        Statement::SetVariable {
            variables, value, ..
        } => {
            let var_name = variables
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(".");
            let val = value
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            StatementKind::SetVariable(var_name, val)
        }

        // SHOW variable
        Statement::ShowVariable { variable } => {
            let var_name = variable
                .iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(".");
            StatementKind::ShowVariable(var_name)
        }

        // SELECT queries
        Statement::Query(query) => classify_query(query),

        // INSERT statements
        Statement::Insert(insert) => match &insert.table {
            TableObject::TableName(name) => classify_insert(name),
            _ => StatementKind::Unsupported("INSERT into function".to_string()),
        },

        // UPDATE statements
        Statement::Update { table, .. } => classify_update(table),

        // CREATE TABLE for inlined tables
        Statement::CreateTable(ct) => {
            let name = ct.name.to_string().to_lowercase();
            if name.contains("ducklake_inlined") {
                StatementKind::CreateInlinedTable
            } else {
                StatementKind::Unsupported(format!("CREATE TABLE {name}"))
            }
        }

        _ => StatementKind::Unsupported(format!("{stmt}")),
    }
}

fn classify_query(query: &sqlparser::ast::Query) -> StatementKind {
    let body = query.body.as_ref();
    match body {
        SetExpr::Select(select) => {
            // Check for function calls: version(), current_schema(), current_database(), gen_random_uuid()
            if select.from.is_empty() {
                return classify_no_from_select(select);
            }

            // Check FROM table
            if let Some(from) = select.from.first() {
                let table_name = extract_table_name(&from.relation);
                if let Some(name) = table_name {
                    return classify_table_select_with_query(&name, query, select);
                }
            }

            StatementKind::Unsupported("unrecognized SELECT".to_string())
        }
        _ => StatementKind::Unsupported("non-SELECT query body".to_string()),
    }
}

fn classify_no_from_select(select: &sqlparser::ast::Select) -> StatementKind {
    if let Some(SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. }) =
        select.projection.first()
    {
        if let Expr::Function(func) = expr {
            let func_name = func.name.to_string().to_lowercase();
            match func_name.as_str() {
                "version" => return StatementKind::SelectVersion,
                "current_schema" => return StatementKind::SelectCurrentSchema,
                "current_database" => return StatementKind::SelectCurrentDatabase,
                "gen_random_uuid" => return StatementKind::SelectGenRandomUuid,
                _ => {}
            }
        }
    }
    StatementKind::Unsupported("SELECT without FROM".to_string())
}

/// Classify a SELECT from a known table, considering ORDER BY / LIMIT / WHERE patterns.
fn classify_table_select_with_query(
    table_name: &str,
    query: &sqlparser::ast::Query,
    select: &sqlparser::ast::Select,
) -> StatementKind {
    let lower = table_name.to_lowercase();
    match lower.as_str() {
        "ducklake_snapshot" => classify_snapshot_select(query, select),
        "ducklake_schema" => StatementKind::SelectSchemas,
        "ducklake_table" => StatementKind::SelectTables,
        "ducklake_column" => StatementKind::SelectColumns,
        "ducklake_data_file" => classify_data_file_select(query),
        "ducklake_delete_file" => StatementKind::SelectDeleteFiles,
        "ducklake_file_column_stats" => StatementKind::SelectFileColumnStats,
        "ducklake_table_stats" => StatementKind::SelectTableStats,
        "ducklake_metadata" => StatementKind::SelectMetadata,
        "ducklake_inlined_data_tables" => StatementKind::SelectInlinedData,
        "ducklake_view" => StatementKind::SelectViews,
        "ducklake_macro" => StatementKind::SelectMacros,
        s if s.starts_with("pg_catalog.pg_type") || s == "pg_type" => StatementKind::SelectPgType,
        s if s.starts_with("ducklake_inlined_") => StatementKind::SelectInlinedRows,
        // Virtual catalog schema: slateduck_catalog.{table}
        s if s.starts_with("slateduck_catalog.") => {
            let table_name = s
                .strip_prefix("slateduck_catalog.")
                .unwrap_or(s)
                .to_string();
            StatementKind::VirtualCatalogScan { table_name }
        }
        _ => StatementKind::Unsupported(format!("SELECT from {table_name}")),
    }
}

/// Classify SELECT on ducklake_snapshot — detect ASC LIMIT 1 and WHERE snapshot_id > $1 patterns.
fn classify_snapshot_select(
    query: &sqlparser::ast::Query,
    select: &sqlparser::ast::Select,
) -> StatementKind {
    // Check for ORDER BY snapshot_id ASC LIMIT 1 → SelectFirstSnapshot
    if has_order_by_asc_limit_1(query) {
        return StatementKind::SelectFirstSnapshot;
    }

    // Check for max(snapshot_id) ... WHERE snapshot_id > $1 → SelectMaxSnapshotAfter
    if has_where_snapshot_gt(select) {
        return StatementKind::SelectMaxSnapshotAfter;
    }

    StatementKind::SelectMaxSnapshot
}

/// Classify SELECT on ducklake_data_file — detect parameterized LIMIT.
fn classify_data_file_select(query: &sqlparser::ast::Query) -> StatementKind {
    if has_parameterized_limit(query) {
        return StatementKind::SelectDataFilesWithLimit;
    }
    StatementKind::SelectDataFiles
}

/// Check if query has ORDER BY ... ASC LIMIT 1.
fn has_order_by_asc_limit_1(query: &sqlparser::ast::Query) -> bool {
    if query.order_by.is_some() {
        if let Some(ref limit) = query.limit {
            let limit_str = limit.to_string();
            if limit_str == "1" {
                return true;
            }
        }
    }
    false
}

/// Check if the SELECT has a WHERE clause with `snapshot_id > $N`.
fn has_where_snapshot_gt(select: &sqlparser::ast::Select) -> bool {
    if let Some(ref selection) = select.selection {
        let sel_str = selection.to_string().to_lowercase();
        if sel_str.contains("snapshot_id") && sel_str.contains(">") {
            return true;
        }
    }
    false
}

/// Check if query has a parameterized LIMIT ($N).
fn has_parameterized_limit(query: &sqlparser::ast::Query) -> bool {
    if let Some(ref limit) = query.limit {
        let limit_str = limit.to_string();
        if limit_str.starts_with('$') {
            return true;
        }
    }
    false
}

fn classify_insert(table_name: &ObjectName) -> StatementKind {
    let name = table_name.to_string().to_lowercase();
    match name.as_str() {
        "ducklake_snapshot" => StatementKind::InsertSnapshot,
        "ducklake_snapshot_changes" => StatementKind::InsertSnapshotChanges,
        "ducklake_schema" => StatementKind::InsertSchema,
        "ducklake_table" => StatementKind::InsertTable,
        "ducklake_column" => StatementKind::InsertColumn,
        "ducklake_data_file" => StatementKind::InsertDataFile,
        "ducklake_delete_file" => StatementKind::InsertDeleteFile,
        "ducklake_table_stats" => StatementKind::InsertTableStats,
        "ducklake_file_column_stats" => StatementKind::InsertFileColumnStats,
        "ducklake_metadata" => StatementKind::InsertMetadata,
        "ducklake_inlined_data_tables" => StatementKind::InsertInlinedDataTables,
        "ducklake_view" => StatementKind::InsertView,
        "ducklake_macro" => StatementKind::InsertMacro,
        "ducklake_macro_impl" => StatementKind::InsertMacroImpl,
        "ducklake_macro_parameters" => StatementKind::InsertMacroParameters,
        s if s.starts_with("ducklake_inlined_") => StatementKind::InsertInlinedRow,
        _ => StatementKind::Unsupported(format!("INSERT INTO {name}")),
    }
}

fn classify_update(table: &sqlparser::ast::TableWithJoins) -> StatementKind {
    let table_name = extract_table_name(&table.relation)
        .unwrap_or_default()
        .to_lowercase();
    match table_name.as_str() {
        "ducklake_table_stats" => StatementKind::UpdateTableStats,
        "ducklake_table" | "ducklake_column" | "ducklake_data_file" | "ducklake_view"
        | "ducklake_macro" | "ducklake_schema" => {
            StatementKind::UpdateEndSnapshot(table_name.clone())
        }
        s if s.starts_with("ducklake_inlined_") => StatementKind::UpdateInlinedRowEndSnapshot,
        _ => StatementKind::Unsupported(format!("UPDATE {table_name}")),
    }
}

fn extract_table_name(factor: &TableFactor) -> Option<String> {
    match factor {
        TableFactor::Table { name, .. } => Some(name.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_version() {
        let kind = classify_statement("SELECT version()").unwrap();
        assert_eq!(kind, StatementKind::SelectVersion);
    }

    #[test]
    fn test_classify_current_schema() {
        let kind = classify_statement("SELECT current_schema()").unwrap();
        assert_eq!(kind, StatementKind::SelectCurrentSchema);
    }

    #[test]
    fn test_classify_pg_type() {
        let kind = classify_statement(
            "SELECT oid, typname FROM pg_catalog.pg_type WHERE typname IN ('bool','int4')",
        )
        .unwrap();
        assert_eq!(kind, StatementKind::SelectPgType);
    }

    #[test]
    fn test_classify_begin_commit_rollback() {
        assert_eq!(classify_statement("BEGIN").unwrap(), StatementKind::Begin);
        assert_eq!(classify_statement("COMMIT").unwrap(), StatementKind::Commit);
        assert_eq!(
            classify_statement("ROLLBACK").unwrap(),
            StatementKind::Rollback
        );
    }

    #[test]
    fn test_classify_set() {
        let kind = classify_statement("SET timezone = 'UTC'").unwrap();
        assert!(matches!(kind, StatementKind::SetVariable(_, _)));
    }

    #[test]
    fn test_classify_show() {
        let kind = classify_statement("SHOW server_version").unwrap();
        assert!(matches!(kind, StatementKind::ShowVariable(_)));
    }

    #[test]
    fn test_classify_insert_snapshot() {
        let kind = classify_statement(
            "INSERT INTO ducklake_snapshot (snapshot_id, schema_version) VALUES (1, 1)",
        )
        .unwrap();
        assert_eq!(kind, StatementKind::InsertSnapshot);
    }

    #[test]
    fn test_classify_select_max_snapshot() {
        let kind = classify_statement("SELECT max(snapshot_id) FROM ducklake_snapshot").unwrap();
        assert_eq!(kind, StatementKind::SelectMaxSnapshot);
    }

    #[test]
    fn test_classify_select_tables() {
        let kind = classify_statement(
            "SELECT * FROM ducklake_table WHERE schema_id = 1 AND begin_snapshot <= 5 AND (end_snapshot IS NULL OR 5 < end_snapshot)",
        )
        .unwrap();
        assert_eq!(kind, StatementKind::SelectTables);
    }

    #[test]
    fn test_classify_update_end_snapshot() {
        let kind = classify_statement(
            "UPDATE ducklake_table SET end_snapshot = 3 WHERE table_id = 1 AND end_snapshot IS NULL",
        )
        .unwrap();
        assert_eq!(
            kind,
            StatementKind::UpdateEndSnapshot("ducklake_table".to_string())
        );
    }

    #[test]
    fn test_classify_unsupported() {
        let kind = classify_statement("DROP TABLE foo").unwrap();
        assert!(matches!(kind, StatementKind::Unsupported(_)));
    }

    #[test]
    fn test_classify_select_current_database() {
        let kind = classify_statement("SELECT current_database()").unwrap();
        assert_eq!(kind, StatementKind::SelectCurrentDatabase);
    }

    #[test]
    fn test_classify_insert_data_file() {
        let kind = classify_statement(
            "INSERT INTO ducklake_data_file (data_file_id, table_id, path) VALUES (1, 2, 'test.parquet')",
        )
        .unwrap();
        assert_eq!(kind, StatementKind::InsertDataFile);
    }

    #[test]
    fn test_classify_select_file_column_stats() {
        let kind = classify_statement(
            "SELECT data_file_id FROM ducklake_file_column_stats WHERE table_id = 1 AND column_id = 2",
        )
        .unwrap();
        assert_eq!(kind, StatementKind::SelectFileColumnStats);
    }

    // ─── pg-tide-relay extension patterns ──────────────────────────────

    #[test]
    fn test_classify_select_first_snapshot() {
        let kind =
            classify_statement("SELECT * FROM ducklake_snapshot ORDER BY snapshot_id ASC LIMIT 1")
                .unwrap();
        assert_eq!(kind, StatementKind::SelectFirstSnapshot);
    }

    #[test]
    fn test_classify_select_max_snapshot_after() {
        let kind = classify_statement(
            "SELECT max(snapshot_id) FROM ducklake_snapshot WHERE snapshot_id > $1",
        )
        .unwrap();
        assert_eq!(kind, StatementKind::SelectMaxSnapshotAfter);
    }

    #[test]
    fn test_classify_select_data_files_with_limit() {
        let kind =
            classify_statement("SELECT * FROM ducklake_data_file WHERE table_id = $1 LIMIT $2")
                .unwrap();
        assert_eq!(kind, StatementKind::SelectDataFilesWithLimit);
    }

    #[test]
    fn test_classify_gen_random_uuid() {
        let kind = classify_statement("SELECT gen_random_uuid()").unwrap();
        assert_eq!(kind, StatementKind::SelectGenRandomUuid);
    }

    #[test]
    fn test_classify_insert_metadata() {
        let kind = classify_statement(
            "INSERT INTO ducklake_metadata (metadata_key, metadata_value) VALUES ($1, $2)",
        )
        .unwrap();
        assert_eq!(kind, StatementKind::InsertMetadata);
    }

    #[test]
    fn test_classify_select_metadata() {
        let kind =
            classify_statement("SELECT value FROM ducklake_metadata WHERE metadata_key = $1")
                .unwrap();
        assert_eq!(kind, StatementKind::SelectMetadata);
    }

    // ─── Virtual Catalog SQL Tables ────────────────────────────────────

    #[test]
    fn test_classify_virtual_catalog_scan_snapshot() {
        let kind = classify_statement("SELECT * FROM slateduck_catalog.ducklake_snapshot").unwrap();
        assert_eq!(
            kind,
            StatementKind::VirtualCatalogScan {
                table_name: "ducklake_snapshot".to_string()
            }
        );
    }

    #[test]
    fn test_classify_virtual_catalog_scan_counters() {
        let kind =
            classify_statement("SELECT * FROM slateduck_catalog.slateduck_counters").unwrap();
        assert_eq!(
            kind,
            StatementKind::VirtualCatalogScan {
                table_name: "slateduck_counters".to_string()
            }
        );
    }

    #[test]
    fn test_classify_virtual_catalog_scan_data_file() {
        let kind = classify_statement(
            "SELECT data_file_id, path, begin_snapshot FROM slateduck_catalog.ducklake_data_file WHERE table_id = 42 ORDER BY begin_snapshot DESC LIMIT 20",
        )
        .unwrap();
        assert_eq!(
            kind,
            StatementKind::VirtualCatalogScan {
                table_name: "ducklake_data_file".to_string()
            }
        );
    }
}
