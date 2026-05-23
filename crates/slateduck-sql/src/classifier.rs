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
    SelectSchemas,
    SelectTables,
    SelectColumns,
    SelectDataFiles,
    SelectDeleteFiles,
    SelectFileColumnStats,
    SelectTableStats,
    SelectMetadata,
    SelectSnapshot,
    SelectInlinedData,
    SelectViews,
    SelectMacros,

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

    // ─── Unsupported ───────────────────────────────────────────────────
    Unsupported(String),
}

/// Classify a SQL string into a `StatementKind`.
pub fn classify_statement(sql: &str) -> Result<StatementKind, SqlDispatchError> {
    let dialect = PostgreSqlDialect {};
    let statements = Parser::parse_sql(&dialect, sql)
        .map_err(|e| SqlDispatchError::ParseError(e.to_string()))?;

    if statements.is_empty() {
        return Err(SqlDispatchError::ParseError("empty statement".to_string()));
    }

    Ok(classify_ast(&statements[0]))
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
            // Check for function calls: version(), current_schema(), current_database()
            if select.from.is_empty() {
                return classify_no_from_select(select);
            }

            // Check FROM table
            if let Some(from) = select.from.first() {
                let table_name = extract_table_name(&from.relation);
                if let Some(name) = table_name {
                    return classify_table_select(&name);
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
                _ => {}
            }
        }
    }
    StatementKind::Unsupported("SELECT without FROM".to_string())
}

fn classify_table_select(table_name: &str) -> StatementKind {
    let lower = table_name.to_lowercase();
    match lower.as_str() {
        "ducklake_snapshot" => StatementKind::SelectMaxSnapshot,
        "ducklake_schema" => StatementKind::SelectSchemas,
        "ducklake_table" => StatementKind::SelectTables,
        "ducklake_column" => StatementKind::SelectColumns,
        "ducklake_data_file" => StatementKind::SelectDataFiles,
        "ducklake_delete_file" => StatementKind::SelectDeleteFiles,
        "ducklake_file_column_stats" => StatementKind::SelectFileColumnStats,
        "ducklake_table_stats" => StatementKind::SelectTableStats,
        "ducklake_metadata" => StatementKind::SelectMetadata,
        "ducklake_inlined_data_tables" => StatementKind::SelectInlinedData,
        "ducklake_view" => StatementKind::SelectViews,
        "ducklake_macro" => StatementKind::SelectMacros,
        s if s.starts_with("pg_catalog.pg_type") || s == "pg_type" => StatementKind::SelectPgType,
        s if s.starts_with("ducklake_inlined_") => StatementKind::SelectInlinedRows,
        _ => StatementKind::Unsupported(format!("SELECT from {table_name}")),
    }
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
}
