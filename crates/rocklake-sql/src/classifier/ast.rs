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

        // DISCARD: DuckDB sends this when returning a connection to the pool.
        Statement::Discard { .. } => StatementKind::DiscardAll,

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
            // Strip surrounding quotes from bare table names (DuckDB 1.5+ sends
            // `CREATE TABLE IF NOT EXISTS "ducklake_metadata"` with quoted identifiers).
            let name_unquoted = name.trim_matches('"');
            if name_unquoted.contains("ducklake_inlined") {
                StatementKind::CreateInlinedTable
            } else if name_unquoted.starts_with("ducklake_") {
                // Bare quoted DuckLake core table — treat as no-op.
                StatementKind::CreateInlinedTable
            } else if name.contains('.') {
                // Extension schema DDL: CREATE TABLE IF NOT EXISTS pgtrickle.table_name
                let parts: Vec<&str> = name.splitn(2, '.').collect();
                if parts.len() == 2 {
                    let schema = parts[0].trim_matches('"');
                    let table = parts[1].trim_matches('"');
                    // DuckDB 1.5+ sends schema-qualified "public"."ducklake_*" DDL.
                    // These are core DuckLake tables managed by rocklake — treat as no-op.
                    if schema == "public" && table.starts_with("ducklake_") {
                        StatementKind::CreateInlinedTable
                    } else {
                        StatementKind::CreateExtensionTable {
                            schema_name: schema.to_string(),
                            table_name: table.to_string(),
                        }
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
                            schema_name: parts[0].trim_matches('"').to_string(),
                            table_name: parts[1].trim_matches('"').to_string(),
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

            // DuckDB postgres_scanner wraps ducklake_snapshot reads in a derived subquery
            // for projection pushdown via postgres_query():
            //   SELECT "col" FROM (SELECT ... FROM ducklake_snapshot ...) AS __unnamed_subquery
            // Handle these specifically, then fall through to direct table classification.
            if let Some(kind) = classify_derived_snapshot_select(select) {
                return kind;
            }
            // Also handle projection-pushdown derived subqueries for all other ducklake tables
            // (e.g. SELECT "col1", "col2" FROM (SELECT ... FROM ducklake_X ...) AS __sub).
            if let Some(kind) = classify_derived_ducklake_select(select) {
                return kind;
            }

            // Check FROM table
            if let Some(from) = select.from.first() {
                // Check for table_changes() function call in FROM
                if let Some(kind) = classify_table_function_from(&from.relation) {
                    return kind;
                }
                let table_name = extract_table_name(&from.relation);
                if let Some(name) = table_name {
                    let lower = name.to_lowercase();
                    let normalized = strip_public_schema(&lower);
                    return classify_table_select_with_query(normalized, query, select);
                }
            }

            StatementKind::Unsupported("unrecognized SELECT".to_string())
        }
        _ => StatementKind::Unsupported("non-SELECT query body".to_string()),
    }
}

fn classify_derived_snapshot_select(select: &sqlparser::ast::Select) -> Option<StatementKind> {
    let from = select.from.first()?;
    let TableFactor::Derived { subquery, .. } = &from.relation else {
        return None;
    };

    let SetExpr::Select(inner_select) = subquery.body.as_ref() else {
        return None;
    };

    let inner_from = inner_select.from.first()?;
    let inner_table = extract_table_name(&inner_from.relation)?;
    let inner_lower = inner_table.to_lowercase();
    let inner_normalized = strip_public_schema(&inner_lower);
    if inner_normalized == "ducklake_snapshot" {
        if outer_projection_looks_like_max(select) {
            Some(StatementKind::SelectMaxSnapshot)
        } else if outer_projection_looks_like_snapshot_tuple(select) {
            Some(StatementKind::SelectLatestSnapshotInfo)
        } else {
            None
        }
    } else {
        None
    }
}

/// Handle DuckDB projection-pushdown pattern for ALL ducklake tables (except snapshot,
/// which is handled by `classify_derived_snapshot_select`):
///   SELECT "col1", "col2" FROM (SELECT ... FROM ducklake_X WHERE ...) AS __unnamed_subquery
///
/// Classifies as the same StatementKind as a direct SELECT from `ducklake_X`.
fn classify_derived_ducklake_select(select: &sqlparser::ast::Select) -> Option<StatementKind> {
    let from = select.from.first()?;
    let TableFactor::Derived { subquery, .. } = &from.relation else {
        return None;
    };
    let SetExpr::Select(inner_select) = subquery.body.as_ref() else {
        return None;
    };
    let inner_from = inner_select.from.first()?;
    // Handle multi-level nesting by walking to the innermost Table reference.
    let table_name = extract_innermost_table_name(&inner_from.relation)?;
    let lower = table_name.to_lowercase();
    let normalized = strip_public_schema(&lower);
    // Skip ducklake_snapshot — handled by classify_derived_snapshot_select.
    if normalized == "ducklake_snapshot" {
        return None;
    }
    if !normalized.starts_with("ducklake_") {
        return None;
    }
    Some(classify_table_select_with_query(
        normalized,
        subquery.as_ref(),
        inner_select,
    ))
}

/// Walk TableFactor levels (including nested Derived subqueries) to find the
/// innermost concrete Table reference and return its name.
fn extract_innermost_table_name(factor: &TableFactor) -> Option<String> {
    match factor {
        TableFactor::Table { name, .. } => Some(name.to_string()),
        TableFactor::Derived { subquery, .. } => {
            let SetExpr::Select(inner_select) = subquery.body.as_ref() else {
                return None;
            };
            let inner_from = inner_select.from.first()?;
            extract_innermost_table_name(&inner_from.relation)
        }
        _ => None,
    }
}

fn outer_projection_looks_like_snapshot_tuple(select: &sqlparser::ast::Select) -> bool {
    [
        "snapshot_id",
        "schema_version",
        "next_catalog_id",
        "next_file_id",
    ]
    .iter()
    .all(|name| {
        select
            .projection
            .iter()
            .any(|item| projection_item_name(item) == *name)
    })
}

fn projection_item_name(item: &SelectItem) -> String {
    match item {
        SelectItem::UnnamedExpr(expr) => expr_last_identifier(expr),
        SelectItem::ExprWithAlias { alias, .. } => alias.value.to_lowercase(),
        SelectItem::QualifiedWildcard(_, _) | SelectItem::Wildcard(_) => "*".to_string(),
    }
}

fn expr_last_identifier(expr: &Expr) -> String {
    match expr {
        Expr::Identifier(id) => id.value.to_lowercase(),
        Expr::CompoundIdentifier(parts) => parts
            .last()
            .map(|id| id.value.to_lowercase())
            .unwrap_or_default(),
        _ => expr.to_string().to_lowercase(),
    }
}

fn outer_projection_looks_like_max(select: &sqlparser::ast::Select) -> bool {
    let Some(item) = select.projection.first() else {
        return false;
    };

    match item {
        SelectItem::UnnamedExpr(Expr::Identifier(id)) => id.value.eq_ignore_ascii_case("max"),
        SelectItem::UnnamedExpr(Expr::CompoundIdentifier(parts)) => parts
            .last()
            .map(|id| id.value.eq_ignore_ascii_case("max"))
            .unwrap_or(false),
        SelectItem::ExprWithAlias { expr, .. } => match expr {
            Expr::Identifier(id) => id.value.eq_ignore_ascii_case("max"),
            Expr::CompoundIdentifier(parts) => parts
                .last()
                .map(|id| id.value.eq_ignore_ascii_case("max"))
                .unwrap_or(false),
            _ => false,
        },
        _ => false,
    }
}

pub(super) fn classify_no_from_select(select: &sqlparser::ast::Select) -> StatementKind {
    // Check for multi-item SELECT (e.g., version() + RDS check)
    if select.projection.len() >= 2 {
        // Check if first item is version() and second is a subquery
        if let Some(SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. }) =
            select.projection.first()
        {
            if let Expr::Function(func) = expr {
                if func.name.to_string().to_lowercase() == "version" {
                    // Check second item for RDS pattern
                    if let Some(
                        SelectItem::UnnamedExpr(Expr::Subquery(_))
                        | SelectItem::ExprWithAlias {
                            expr: Expr::Subquery(_),
                            ..
                        },
                    ) = select.projection.get(1)
                    {
                        return StatementKind::SelectVersionWithRdsCheck;
                    }
                }
            }
        }
    }

    // Single-item SELECT functions
    if let Some(SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. }) =
        select.projection.first()
    {
        // Check for literal 1 (SELECT 1)
        if let Expr::Value(val) = expr {
            if let sqlparser::ast::Value::Number(n, _) = &val.value {
                if n == "1" {
                    return StatementKind::SelectOne;
                }
            }
        }

        if let Expr::Function(func) = expr {
            let func_name = func.name.to_string().to_lowercase();
            match func_name.as_str() {
                "version" => return StatementKind::SelectVersion,
                "current_schema" => return StatementKind::SelectCurrentSchema,
                "current_database" => return StatementKind::SelectCurrentDatabase,
                "gen_random_uuid" => return StatementKind::SelectGenRandomUuid,
                // pg-trickle CDC startup: SELECT ducklake_latest_snapshot_id($1::regclass)
                "ducklake_latest_snapshot_id" => {
                    return StatementKind::SelectLatestSnapshotId;
                }
                // DuckDB postgres scanner: secret storage fast-path check.
                "to_regclass" => return StatementKind::SelectToRegclass,
                // DuckDB postgres scanner: database size query.
                "pg_database_size" => return StatementKind::SelectPgDatabaseSize,
                "rocklake.next_rowid_range" => {
                    return classify_next_rowid_range_call(func);
                }
                "rocklake.hold_snapshot" => {
                    return classify_hold_snapshot_call(func);
                }
                "rocklake.release_snapshot" => {
                    return classify_release_snapshot_call(func);
                }
                _ => {}
            }
        }

        // SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE ...)
        // DuckDB secret storage fallback check.
        if let Expr::Exists { subquery, .. } = expr {
            if subquery
                .to_string()
                .to_lowercase()
                .contains("information_schema")
            {
                return StatementKind::SelectExistsInfoSchema;
            }
        }
    }
    StatementKind::Unsupported("SELECT without FROM".to_string())
}

/// Strip a `"public".` or `public.` schema prefix, and strip surrounding
/// double-quote delimiters from a bare single-identifier table name.
/// DuckDB 1.5+ sends schema-qualified names (e.g. `"public"."ducklake_metadata"`);
/// rocklake matches against unqualified names, so we normalize here.
fn strip_public_schema(s: &str) -> &str {
    if let Some(rest) = s.strip_prefix("\"public\".") {
        rest.trim_matches('"')
    } else if let Some(rest) = s.strip_prefix("public.") {
        rest.trim_matches('"')
    } else if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        // A single bare quoted identifier like `"ducklake_metadata"`.
        // Only strip quotes when there is no interior unescaped quote (which
        // would indicate a multi-part name such as `"My Schema".my_table`).
        let inner = &s[1..s.len() - 1];
        if !inner.contains('"') {
            inner
        } else {
            s
        }
    } else {
        s
    }
}

pub(super) fn classify_insert(table_name: &ObjectName, columns: &[String]) -> StatementKind {
    let raw = table_name.to_string().to_lowercase();
    let name = strip_public_schema(&raw);
    match name {
        "ducklake_snapshot" => StatementKind::InsertSnapshot,
        "ducklake_snapshot_changes" => StatementKind::InsertSnapshotChanges,
        "ducklake_schema" => StatementKind::InsertSchema,
        "ducklake_table" => StatementKind::InsertTable,
        "ducklake_column" => StatementKind::InsertColumn,
        "ducklake_data_file" => StatementKind::InsertDataFile,
        "ducklake_delete_file" => StatementKind::InsertDeleteFile,
        "ducklake_table_stats" => StatementKind::InsertTableStats,
        "ducklake_table_column_stats" => StatementKind::InsertTableColumnStats,
        "ducklake_file_column_stats" => StatementKind::InsertFileColumnStats,
        "ducklake_metadata" => StatementKind::InsertMetadata,
        "ducklake_inlined_data_tables" => StatementKind::InsertInlinedDataTables,
        "ducklake_schema_versions" => StatementKind::InsertSchemaVersions,
        "ducklake_view" => StatementKind::InsertView,
        "ducklake_macro" => StatementKind::InsertMacro,
        "ducklake_macro_impl" => StatementKind::InsertMacroImpl,
        "ducklake_macro_parameters" => StatementKind::InsertMacroParameters,
        s if s.starts_with("ducklake_inlined_") => StatementKind::InsertInlinedRow,
        // Catch-all for any unrecognized ducklake_* table (future-proofing)
        s if s.starts_with("ducklake_") => StatementKind::InsertInlinedRow,
        s if s.contains('.') => {
            // Extension schema INSERT: pgtrickle.pgt_ducklake_provenance
            let parts: Vec<&str> = s.splitn(2, '.').collect();
            if parts.len() == 2 {
                StatementKind::InsertExtensionRow {
                    schema_name: parts[0].trim_matches('"').to_string(),
                    table_name: parts[1].trim_matches('"').to_string(),
                    columns: columns.to_vec(),
                    values_json: String::new(),
                }
            } else {
                StatementKind::Unsupported(format!("INSERT INTO {raw}"))
            }
        }
        _ => StatementKind::Unsupported(format!("INSERT INTO {raw}")),
    }
}

pub(super) fn classify_update(table: &sqlparser::ast::TableWithJoins) -> StatementKind {
    let raw = extract_table_name(&table.relation)
        .unwrap_or_default()
        .to_lowercase();
    let table_name = strip_public_schema(&raw);
    match table_name {
        "ducklake_table_stats" => StatementKind::UpdateTableStats,
        "ducklake_table" | "ducklake_column" | "ducklake_data_file" | "ducklake_view"
        | "ducklake_macro" | "ducklake_schema" => {
            StatementKind::UpdateEndSnapshot(table_name.to_string())
        }
        s if s.starts_with("ducklake_inlined_") => StatementKind::UpdateInlinedRowEndSnapshot,
        // Catch-all for any unrecognized ducklake_* table (future-proofing)
        s if s.starts_with("ducklake_") => StatementKind::UpdateEndSnapshot(table_name.to_string()),
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

/// Classify `rocklake.next_rowid_range(table_ref, count := N)`.
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

/// Classify `rocklake.hold_snapshot(min_snapshot_id := N, consumer_id := '...', ttl_seconds := N)`.
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

/// Classify `rocklake.release_snapshot(consumer_id := '...')`.
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
