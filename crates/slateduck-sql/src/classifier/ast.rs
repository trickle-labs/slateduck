//! AST-based SQL statement classifiers.

use sqlparser::ast::{Expr, ObjectName, SelectItem, SetExpr, Statement, TableFactor, TableObject};

use super::table_selects::classify_table_select_with_query;
use super::StatementKind;

pub(super) fn classify_ast(stmt: &Statement) -> StatementKind {
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
            TableObject::TableName(name) => {
                let columns: Vec<String> = insert.columns.iter().map(|c| c.to_string()).collect();
                classify_insert(name, &columns)
            }
            _ => StatementKind::Unsupported("INSERT into function".to_string()),
        },

        // UPDATE statements
        Statement::Update { table, .. } => classify_update(table),

        // CREATE TABLE for inlined tables
        Statement::CreateTable(ct) => {
            let name = ct.name.to_string().to_lowercase();
            if name.contains("ducklake_inlined") {
                StatementKind::CreateInlinedTable
            } else if name.contains('.') {
                // Extension schema DDL: CREATE TABLE IF NOT EXISTS pgtrickle.table_name
                let parts: Vec<&str> = name.splitn(2, '.').collect();
                if parts.len() == 2 {
                    StatementKind::CreateExtensionTable {
                        schema_name: parts[0].to_string(),
                        table_name: parts[1].to_string(),
                    }
                } else {
                    StatementKind::Unsupported(format!("CREATE TABLE {name}"))
                }
            } else {
                StatementKind::Unsupported(format!("CREATE TABLE {name}"))
            }
        }

        // DELETE for extension schema tables
        Statement::Delete(del) => {
            let tables = match &del.from {
                sqlparser::ast::FromTable::WithFromKeyword(t) => t,
                sqlparser::ast::FromTable::WithoutKeyword(t) => t,
            };
            if let Some(from) = tables.first() {
                let table_name = extract_table_name(&from.relation)
                    .unwrap_or_default()
                    .to_lowercase();
                if table_name.contains('.') {
                    let parts: Vec<&str> = table_name.splitn(2, '.').collect();
                    if parts.len() == 2 {
                        return StatementKind::DeleteExtensionRows {
                            schema_name: parts[0].to_string(),
                            table_name: parts[1].to_string(),
                        };
                    }
                }
            }
            StatementKind::Unsupported("DELETE".to_string())
        }

        _ => StatementKind::Unsupported(format!("{stmt}")),
    }
}

pub(super) fn classify_query(query: &sqlparser::ast::Query) -> StatementKind {
    let body = query.body.as_ref();
    match body {
        SetExpr::Select(select) => {
            // Check for function calls: version(), current_schema(), current_database(), gen_random_uuid()
            if select.from.is_empty() {
                return classify_no_from_select(select);
            }

            // Check FROM table
            if let Some(from) = select.from.first() {
                // Check for table_changes() function call in FROM
                if let Some(kind) = classify_table_function_from(&from.relation) {
                    return kind;
                }
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

pub(super) fn classify_no_from_select(select: &sqlparser::ast::Select) -> StatementKind {
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
                "slateduck.next_rowid_range" => {
                    return classify_next_rowid_range_call(func);
                }
                "slateduck.hold_snapshot" => {
                    return classify_hold_snapshot_call(func);
                }
                "slateduck.release_snapshot" => {
                    return classify_release_snapshot_call(func);
                }
                _ => {}
            }
        }
    }
    StatementKind::Unsupported("SELECT without FROM".to_string())
}

pub(super) fn classify_insert(table_name: &ObjectName, columns: &[String]) -> StatementKind {
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
        s if s.contains('.') => {
            // Extension schema INSERT: pgtrickle.pgt_ducklake_provenance
            let parts: Vec<&str> = s.splitn(2, '.').collect();
            if parts.len() == 2 {
                StatementKind::InsertExtensionRow {
                    schema_name: parts[0].to_string(),
                    table_name: parts[1].to_string(),
                    columns: columns.to_vec(),
                    values_json: String::new(),
                }
            } else {
                StatementKind::Unsupported(format!("INSERT INTO {name}"))
            }
        }
        _ => StatementKind::Unsupported(format!("INSERT INTO {name}")),
    }
}

pub(super) fn classify_update(table: &sqlparser::ast::TableWithJoins) -> StatementKind {
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

pub(super) fn extract_table_name(factor: &TableFactor) -> Option<String> {
    match factor {
        TableFactor::Table { name, .. } => Some(name.to_string()),
        _ => None,
    }
}

/// Check if a FROM clause references `table_changes(...)` as a table function.
pub(super) fn classify_table_function_from(factor: &TableFactor) -> Option<StatementKind> {
    match factor {
        TableFactor::Table { name, args, .. } => {
            let func_name = name.to_string().to_lowercase();
            if func_name == "table_changes" {
                if let Some(args) = args {
                    return Some(parse_table_changes_args(args));
                }
            }
            None
        }
        _ => None,
    }
}

/// Parse args from `table_changes('schema.table', start_snapshot := N, end_snapshot := M)`.
/// Also accepts positional form: `table_changes('schema.table', N, M)`.
pub(super) fn parse_table_changes_args(args: &sqlparser::ast::TableFunctionArgs) -> StatementKind {
    let arg_list = &args.args;
    let mut table_ref = String::new();
    let mut start_snapshot = 0u64;
    let mut end_snapshot = u64::MAX;

    for (i, arg) in arg_list.iter().enumerate() {
        match arg {
            sqlparser::ast::FunctionArg::Named { name, arg, .. } => {
                let name_str = name.to_string().to_lowercase();
                let val = extract_arg_value(arg);
                match name_str.as_str() {
                    "start_snapshot" => start_snapshot = val.parse().unwrap_or(0),
                    "end_snapshot" => end_snapshot = val.parse().unwrap_or(u64::MAX),
                    _ => {}
                }
            }
            sqlparser::ast::FunctionArg::Unnamed(arg) => match i {
                0 => table_ref = extract_arg_value(arg).trim_matches('\'').to_string(),
                1 => start_snapshot = extract_arg_value(arg).parse().unwrap_or(0),
                2 => end_snapshot = extract_arg_value(arg).parse().unwrap_or(u64::MAX),
                _ => {}
            },
            sqlparser::ast::FunctionArg::ExprNamed { name, arg, .. } => {
                let name_str = name.to_string().to_lowercase();
                let val = extract_arg_value(arg);
                match name_str.as_str() {
                    "start_snapshot" => start_snapshot = val.parse().unwrap_or(0),
                    "end_snapshot" => end_snapshot = val.parse().unwrap_or(u64::MAX),
                    _ => {}
                }
            }
        }
    }

    StatementKind::TableChanges {
        table_ref,
        start_snapshot,
        end_snapshot,
    }
}

/// Extract a string value from a FunctionArgExpr.
pub(super) fn extract_arg_value(arg: &sqlparser::ast::FunctionArgExpr) -> String {
    match arg {
        sqlparser::ast::FunctionArgExpr::Expr(expr) => expr.to_string(),
        sqlparser::ast::FunctionArgExpr::QualifiedWildcard(_) => String::new(),
        sqlparser::ast::FunctionArgExpr::Wildcard => String::new(),
    }
}

/// Classify `slateduck.next_rowid_range(table_ref, count := N)`.
pub(super) fn classify_next_rowid_range_call(func: &sqlparser::ast::Function) -> StatementKind {
    let args = &func.args;
    let mut table_ref = String::new();
    let mut count = 1u64;

    if let sqlparser::ast::FunctionArguments::List(arg_list) = args {
        for (i, arg) in arg_list.args.iter().enumerate() {
            match arg {
                sqlparser::ast::FunctionArg::Named { name, arg, .. } => {
                    let name_str = name.to_string().to_lowercase();
                    let val = extract_arg_value(arg);
                    if name_str == "count" {
                        count = val.parse().unwrap_or(1);
                    }
                }
                sqlparser::ast::FunctionArg::Unnamed(arg) if i == 0 => {
                    table_ref = extract_arg_value(arg).trim_matches('\'').to_string();
                }
                _ => {}
            }
        }
    }

    StatementKind::NextRowidRange { table_ref, count }
}

/// Classify `slateduck.hold_snapshot(min_snapshot_id := N, consumer_id := '...', ttl_seconds := N)`.
pub(super) fn classify_hold_snapshot_call(func: &sqlparser::ast::Function) -> StatementKind {
    let args = &func.args;
    let mut min_snapshot_id = 0u64;
    let mut consumer_id = String::new();
    let mut ttl_seconds = 300u64;

    if let sqlparser::ast::FunctionArguments::List(arg_list) = args {
        for arg in arg_list.args.iter() {
            if let sqlparser::ast::FunctionArg::Named { name, arg, .. } = arg {
                let name_str = name.to_string().to_lowercase();
                let val = extract_arg_value(arg);
                match name_str.as_str() {
                    "min_snapshot_id" => min_snapshot_id = val.parse().unwrap_or(0),
                    "consumer_id" => consumer_id = val.trim_matches('\'').to_string(),
                    "ttl_seconds" => ttl_seconds = val.parse().unwrap_or(300),
                    _ => {}
                }
            }
        }
    }

    StatementKind::HoldSnapshot {
        min_snapshot_id,
        consumer_id,
        ttl_seconds,
    }
}

/// Classify `slateduck.release_snapshot(consumer_id := '...')`.
pub(super) fn classify_release_snapshot_call(func: &sqlparser::ast::Function) -> StatementKind {
    let args = &func.args;
    let mut consumer_id = String::new();

    if let sqlparser::ast::FunctionArguments::List(arg_list) = args {
        for arg in arg_list.args.iter() {
            if let sqlparser::ast::FunctionArg::Named { name, arg, .. } = arg {
                let name_str = name.to_string().to_lowercase();
                let val = extract_arg_value(arg);
                if name_str == "consumer_id" {
                    consumer_id = val.trim_matches('\'').to_string();
                }
            }
        }
    }

    StatementKind::ReleaseSnapshot { consumer_id }
}
