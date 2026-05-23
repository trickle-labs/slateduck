//! Bounded SQL dispatcher — pattern matches on sqlparser-rs AST nodes.
//!
//! Implements exactly the statement shapes present in the Phase 0 wire corpus.
//! Anything outside this bounded set returns `SQLSTATE 0A000` (feature not supported).

use sqlparser::ast::{
    AssignmentTarget, Expr, Query, SelectItem, SetExpr, Statement, TableFactor, Value,
};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

/// A classified catalog operation extracted from SQL.
#[derive(Debug, Clone, PartialEq)]
pub enum CatalogOp {
    // -- Transaction control --
    Begin,
    Commit,
    Rollback,

    // -- Session commands --
    Set { variable: String, value: String },
    Show { variable: String },

    // -- Introspection queries --
    SelectCurrentSchema,
    SelectCurrentDatabase,
    SelectVersion,
    SelectPgType { type_names: Vec<String> },

    // -- DuckLake read operations --
    SelectMaxSnapshot,
    SelectLatestSnapshot,
    SelectSchemas { dl_snapshot_id: u64 },
    SelectTables { schema_id: u64, dl_snapshot_id: u64 },
    SelectColumns { table_id: u64, dl_snapshot_id: u64 },
    SelectDataFiles { table_id: u64 },
    SelectDataFilesWithDeletes { table_id: u64 },
    SelectFileColumnStats { table_id: u64, column_id: u64 },
    SelectTableStats { table_id: u64 },
    SelectMetadata { key: String },
    SelectSnapshot { snapshot_id: u64 },
    SelectSnapshotChanges { snapshot_id: u64 },
    SelectInlinedInserts { table_id: u64 },
    SelectInlinedDeletes { table_id: u64 },

    // -- DuckLake write operations --
    InsertSnapshot(InsertSnapshotOp),
    InsertSnapshotChanges(InsertSnapshotChangesOp),
    InsertSchema(InsertSchemaOp),
    InsertTable(InsertTableOp),
    InsertColumn(InsertColumnOp),
    InsertDataFile(InsertDataFileOp),
    InsertDeleteFile(InsertDeleteFileOp),
    InsertInlinedInsert(InsertInlinedInsertOp),
    InsertInlinedDelete(InsertInlinedDeleteOp),
    InsertTableStats(InsertTableStatsOp),
    InsertFileColumnStats(InsertFileColumnStatsOp),

    UpdateEndSnapshot(UpdateEndSnapshotOp),
    UpdateTableStats(UpdateTableStatsOp),

    // -- DDL for inlined data tables (no-op in our implementation) --
    CreateInlinedTable { table_name: String },
    DropInlinedTable { table_name: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertSnapshotOp {
    pub snapshot_id: u64,
    pub schema_version: u64,
    pub created_at: String,
    pub author: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertSnapshotChangesOp {
    pub snapshot_id: u64,
    pub changes_json: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertSchemaOp {
    pub schema_id: u64,
    pub name: String,
    pub begin_snapshot: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertTableOp {
    pub schema_id: u64,
    pub table_id: u64,
    pub name: String,
    pub uuid: String,
    pub begin_snapshot: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertColumnOp {
    pub table_id: u64,
    pub column_id: u64,
    pub name: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub default_value: Option<String>,
    pub begin_snapshot: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertDataFileOp {
    pub table_id: u64,
    pub data_file_id: u64,
    pub path: String,
    pub path_is_relative: bool,
    pub file_size_bytes: u64,
    pub record_count: u64,
    pub begin_snapshot: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertDeleteFileOp {
    pub data_file_id: u64,
    pub delete_file_id: u64,
    pub path: String,
    pub path_is_relative: bool,
    pub file_size_bytes: u64,
    pub record_count: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertInlinedInsertOp {
    pub table_id: u64,
    pub schema_version: u64,
    pub row_id: u64,
    pub payload: Vec<u8>,
    pub begin_snapshot: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertInlinedDeleteOp {
    pub table_id: u64,
    pub data_file_id: u64,
    pub row_id: u64,
    pub begin_snapshot: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertTableStatsOp {
    pub table_id: u64,
    pub record_count: i64,
    pub file_count: u64,
    pub total_size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertFileColumnStatsOp {
    pub table_id: u64,
    pub column_id: u64,
    pub data_file_id: u64,
    pub min_value: Option<String>,
    pub max_value: Option<String>,
    pub null_count: Option<u64>,
    pub contains_nan: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateEndSnapshotOp {
    pub table_name: String,
    pub end_snapshot: u64,
    pub id_column: String,
    pub id_value: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateTableStatsOp {
    pub table_id: u64,
    pub record_count_delta: i64,
    pub file_count_delta: i64,
    pub size_delta: i64,
}

/// Error from the SQL dispatcher.
#[derive(Debug, Clone, PartialEq)]
pub enum DispatchError {
    /// SQL that we don't support.
    Unsupported(String),
    /// Parse error.
    ParseError(String),
    /// Invalid parameter value.
    InvalidValue(String),
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(msg) => write!(f, "unsupported: {msg}"),
            Self::ParseError(msg) => write!(f, "parse error: {msg}"),
            Self::InvalidValue(msg) => write!(f, "invalid value: {msg}"),
        }
    }
}

impl std::error::Error for DispatchError {}

/// Parse and classify a SQL statement into a `CatalogOp`.
pub fn dispatch(sql: &str) -> Result<CatalogOp, DispatchError> {
    let trimmed = sql.trim();

    // Fast path for transaction control (case insensitive)
    let upper = trimmed.to_uppercase();
    if upper == "BEGIN" || upper == "BEGIN TRANSACTION" || upper == "START TRANSACTION" {
        return Ok(CatalogOp::Begin);
    }
    if upper == "COMMIT" || upper == "END" {
        return Ok(CatalogOp::Commit);
    }
    if upper == "ROLLBACK" || upper == "ABORT" {
        return Ok(CatalogOp::Rollback);
    }

    let stmts = Parser::parse_sql(&PostgreSqlDialect {}, trimmed)
        .map_err(|e| DispatchError::ParseError(e.to_string()))?;

    if stmts.is_empty() {
        return Err(DispatchError::ParseError("empty statement".to_string()));
    }

    classify_statement(&stmts[0])
}

fn classify_statement(stmt: &Statement) -> Result<CatalogOp, DispatchError> {
    match stmt {
        Statement::Query(query) => classify_query(query),
        Statement::Insert(insert) => classify_insert(insert),
        Statement::Update { .. } => classify_update(stmt),
        Statement::SetVariable { .. } => classify_set(stmt),
        Statement::ShowVariable { .. } => classify_show(stmt),
        Statement::CreateTable(ct) => {
            let name = ct.name.to_string();
            if name.contains("ducklake_inlined") {
                Ok(CatalogOp::CreateInlinedTable { table_name: name })
            } else {
                Err(DispatchError::Unsupported(format!("CREATE TABLE {name}")))
            }
        }
        Statement::Drop { names, .. } => {
            if let Some(name) = names.first() {
                let n = name.to_string();
                if n.contains("ducklake_inlined") {
                    Ok(CatalogOp::DropInlinedTable { table_name: n })
                } else {
                    Err(DispatchError::Unsupported(format!("DROP {n}")))
                }
            } else {
                Err(DispatchError::Unsupported("DROP without name".to_string()))
            }
        }
        _ => Err(DispatchError::Unsupported(format!("{stmt:?}"))),
    }
}

fn classify_query(query: &Query) -> Result<CatalogOp, DispatchError> {
    let Query { body, .. } = query;

    match body.as_ref() {
        SetExpr::Select(select) => {
            // Check for function calls: current_schema(), version(), current_database()
            if select.projection.len() == 1 {
                if let Some(op) = check_scalar_function(&select.projection[0])? {
                    return Ok(op);
                }
            }

            // Check for pg_type query
            if is_pg_type_query(select) {
                return classify_pg_type_query(select);
            }

            // Check for max(snapshot_id) FROM ducklake_snapshot
            if is_max_snapshot_query(select) {
                return Ok(CatalogOp::SelectMaxSnapshot);
            }

            // Check FROM clause for table name
            let table_name = extract_from_table_name(select)?;

            match table_name.as_str() {
                "ducklake_snapshot" => classify_snapshot_select(select),
                "ducklake_snapshot_changes" => classify_snapshot_changes_select(select),
                "ducklake_schema" => classify_schema_select(select),
                "ducklake_table" => classify_table_select(select),
                "ducklake_column" => classify_column_select(select),
                "ducklake_data_file" => classify_data_file_select(select),
                "ducklake_file_column_stats" => classify_file_column_stats_select(select),
                "ducklake_table_stats" => classify_table_stats_select(select),
                "ducklake_metadata" => classify_metadata_select(select),
                t if t.starts_with("ducklake_inlined") => {
                    // Inlined table queries
                    if t.contains("insert") {
                        let table_id = extract_table_id_from_inlined_name(t)?;
                        Ok(CatalogOp::SelectInlinedInserts { table_id })
                    } else if t.contains("delete") {
                        let table_id = extract_table_id_from_inlined_name(t)?;
                        Ok(CatalogOp::SelectInlinedDeletes { table_id })
                    } else {
                        Err(DispatchError::Unsupported(format!("SELECT FROM {t}")))
                    }
                }
                other => Err(DispatchError::Unsupported(format!("SELECT FROM {other}"))),
            }
        }
        _ => Err(DispatchError::Unsupported("non-SELECT query".to_string())),
    }
}

fn check_scalar_function(item: &SelectItem) -> Result<Option<CatalogOp>, DispatchError> {
    if let SelectItem::UnnamedExpr(Expr::Function(func)) = item {
        let name = func.name.to_string().to_lowercase();
        match name.as_str() {
            "current_schema" => return Ok(Some(CatalogOp::SelectCurrentSchema)),
            "current_database" => return Ok(Some(CatalogOp::SelectCurrentDatabase)),
            "version" => return Ok(Some(CatalogOp::SelectVersion)),
            _ => {}
        }
    }
    Ok(None)
}

fn is_pg_type_query(select: &sqlparser::ast::Select) -> bool {
    for item in &select.from {
        if let TableFactor::Table { name, .. } = &item.relation {
            let n = name.to_string().to_lowercase();
            if n.contains("pg_type") || n.contains("pg_catalog.pg_type") {
                return true;
            }
        }
    }
    false
}

fn classify_pg_type_query(select: &sqlparser::ast::Select) -> Result<CatalogOp, DispatchError> {
    // Extract type names from WHERE typname IN (...)
    let mut type_names = Vec::new();
    if let Some(ref selection) = select.selection {
        extract_in_list_values(selection, &mut type_names);
    }
    Ok(CatalogOp::SelectPgType { type_names })
}

fn extract_in_list_values(expr: &Expr, values: &mut Vec<String>) {
    match expr {
        Expr::InList { list, .. } => {
            for item in list {
                if let Expr::Value(v) = item {
                    if let Some(s) = value_to_string(&v.value) {
                        values.push(s);
                    }
                }
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            extract_in_list_values(left, values);
            extract_in_list_values(right, values);
        }
        _ => {}
    }
}

fn is_max_snapshot_query(select: &sqlparser::ast::Select) -> bool {
    if select.projection.len() == 1 {
        if let SelectItem::UnnamedExpr(Expr::Function(func)) = &select.projection[0] {
            let name = func.name.to_string().to_lowercase();
            if name == "max" {
                // Check FROM ducklake_snapshot
                if let Some(from) = select.from.first() {
                    if let TableFactor::Table { name, .. } = &from.relation {
                        return name.to_string().to_lowercase() == "ducklake_snapshot";
                    }
                }
            }
        }
    }
    false
}

fn extract_from_table_name(select: &sqlparser::ast::Select) -> Result<String, DispatchError> {
    if let Some(from) = select.from.first() {
        match &from.relation {
            TableFactor::Table { name, alias, .. } => {
                let _ = alias; // ignore alias
                Ok(name.to_string().to_lowercase())
            }
            _ => Err(DispatchError::Unsupported(
                "non-table FROM clause".to_string(),
            )),
        }
    } else {
        Err(DispatchError::Unsupported("no FROM clause".to_string()))
    }
}

fn classify_snapshot_select(select: &sqlparser::ast::Select) -> Result<CatalogOp, DispatchError> {
    // Could be: SELECT ... ORDER BY snapshot_id DESC LIMIT 1 (latest)
    // or SELECT ... WHERE snapshot_id = $1
    if let Some(ref selection) = select.selection {
        if let Some(id) = extract_eq_value(selection, "snapshot_id") {
            return Ok(CatalogOp::SelectSnapshot { snapshot_id: id });
        }
    }
    Ok(CatalogOp::SelectLatestSnapshot)
}

fn classify_snapshot_changes_select(
    select: &sqlparser::ast::Select,
) -> Result<CatalogOp, DispatchError> {
    if let Some(ref selection) = select.selection {
        if let Some(id) = extract_eq_value(selection, "snapshot_id") {
            return Ok(CatalogOp::SelectSnapshotChanges { snapshot_id: id });
        }
    }
    Err(DispatchError::Unsupported(
        "snapshot_changes without snapshot_id filter".to_string(),
    ))
}

fn classify_schema_select(select: &sqlparser::ast::Select) -> Result<CatalogOp, DispatchError> {
    // Extract dl_snapshot_id from MVCC filter
    if let Some(ref selection) = select.selection {
        if let Some(snap_id) = extract_mvcc_snapshot_id(selection) {
            return Ok(CatalogOp::SelectSchemas {
                dl_snapshot_id: snap_id,
            });
        }
    }
    Err(DispatchError::Unsupported(
        "schema query without MVCC filter".to_string(),
    ))
}

fn classify_table_select(select: &sqlparser::ast::Select) -> Result<CatalogOp, DispatchError> {
    if let Some(ref selection) = select.selection {
        let schema_id = extract_eq_value(selection, "schema_id").unwrap_or(0);
        if let Some(snap_id) = extract_mvcc_snapshot_id(selection) {
            return Ok(CatalogOp::SelectTables {
                schema_id,
                dl_snapshot_id: snap_id,
            });
        }
    }
    Err(DispatchError::Unsupported(
        "table query without MVCC filter".to_string(),
    ))
}

fn classify_column_select(select: &sqlparser::ast::Select) -> Result<CatalogOp, DispatchError> {
    if let Some(ref selection) = select.selection {
        let table_id = extract_eq_value(selection, "table_id").unwrap_or(0);
        if let Some(snap_id) = extract_mvcc_snapshot_id(selection) {
            return Ok(CatalogOp::SelectColumns {
                table_id,
                dl_snapshot_id: snap_id,
            });
        }
    }
    Err(DispatchError::Unsupported(
        "column query without MVCC filter".to_string(),
    ))
}

fn classify_data_file_select(select: &sqlparser::ast::Select) -> Result<CatalogOp, DispatchError> {
    if let Some(ref selection) = select.selection {
        if let Some(table_id) = extract_eq_value(selection, "table_id") {
            // Check if there's a JOIN with delete files
            if !select.from.is_empty() && !select.from[0].joins.is_empty() {
                return Ok(CatalogOp::SelectDataFilesWithDeletes { table_id });
            }
            return Ok(CatalogOp::SelectDataFiles { table_id });
        }
    }
    Err(DispatchError::Unsupported(
        "data_file query without table_id".to_string(),
    ))
}

fn classify_file_column_stats_select(
    select: &sqlparser::ast::Select,
) -> Result<CatalogOp, DispatchError> {
    if let Some(ref selection) = select.selection {
        let table_id = extract_eq_value(selection, "table_id").unwrap_or(0);
        let column_id = extract_eq_value(selection, "column_id").unwrap_or(0);
        return Ok(CatalogOp::SelectFileColumnStats {
            table_id,
            column_id,
        });
    }
    Err(DispatchError::Unsupported(
        "file_column_stats without filter".to_string(),
    ))
}

fn classify_table_stats_select(
    select: &sqlparser::ast::Select,
) -> Result<CatalogOp, DispatchError> {
    if let Some(ref selection) = select.selection {
        if let Some(table_id) = extract_eq_value(selection, "table_id") {
            return Ok(CatalogOp::SelectTableStats { table_id });
        }
    }
    Err(DispatchError::Unsupported(
        "table_stats without table_id".to_string(),
    ))
}

fn classify_metadata_select(select: &sqlparser::ast::Select) -> Result<CatalogOp, DispatchError> {
    if let Some(ref selection) = select.selection {
        if let Some(key) = extract_eq_string_value(selection, "metadata_key") {
            return Ok(CatalogOp::SelectMetadata { key });
        }
    }
    Err(DispatchError::Unsupported(
        "metadata without key filter".to_string(),
    ))
}

fn classify_insert(insert: &sqlparser::ast::Insert) -> Result<CatalogOp, DispatchError> {
    let table_name = insert.table.to_string().to_lowercase();

    match table_name.as_str() {
        "ducklake_snapshot" => classify_snapshot_insert(insert),
        "ducklake_snapshot_changes" => classify_snapshot_changes_insert(insert),
        "ducklake_schema" => classify_schema_insert(insert),
        "ducklake_table" => classify_table_insert(insert),
        "ducklake_column" => classify_column_insert(insert),
        "ducklake_data_file" => classify_data_file_insert(insert),
        "ducklake_delete_file" => classify_delete_file_insert(insert),
        "ducklake_table_stats" => classify_table_stats_insert(insert),
        "ducklake_file_column_stats" => classify_file_column_stats_insert(insert),
        t if t.contains("ducklake_inlined") && t.contains("insert") => {
            classify_inlined_insert_insert(insert)
        }
        t if t.contains("ducklake_inlined") && t.contains("delete") => {
            classify_inlined_delete_insert(insert)
        }
        other => Err(DispatchError::Unsupported(format!("INSERT INTO {other}"))),
    }
}

fn classify_update(stmt: &Statement) -> Result<CatalogOp, DispatchError> {
    if let Statement::Update {
        table,
        assignments,
        selection,
        ..
    } = stmt
    {
        let table_name = match &table.relation {
            TableFactor::Table { name, .. } => name.to_string().to_lowercase(),
            _ => return Err(DispatchError::Unsupported("complex UPDATE".to_string())),
        };

        // Check if it's an UPDATE ... SET end_snapshot = $1
        let is_end_snapshot_update = assignments
            .iter()
            .any(|a| assignment_target_name(&a.target).to_lowercase() == "end_snapshot");

        if is_end_snapshot_update {
            let end_snapshot = assignments
                .iter()
                .find(|a| assignment_target_name(&a.target).to_lowercase() == "end_snapshot")
                .and_then(|a| expr_to_u64(&a.value))
                .ok_or_else(|| DispatchError::InvalidValue("end_snapshot value".to_string()))?;

            // Extract ID from WHERE clause
            let (id_column, id_value) = if let Some(sel) = selection {
                extract_id_from_where(sel)?
            } else {
                return Err(DispatchError::Unsupported(
                    "UPDATE without WHERE".to_string(),
                ));
            };

            return Ok(CatalogOp::UpdateEndSnapshot(UpdateEndSnapshotOp {
                table_name,
                end_snapshot,
                id_column,
                id_value,
            }));
        }

        // Check if it's UPDATE ducklake_table_stats SET record_count = record_count + $1
        if table_name == "ducklake_table_stats" {
            let table_id = selection
                .as_ref()
                .and_then(|s| extract_eq_value(s, "table_id"))
                .unwrap_or(0);

            let mut record_count_delta: i64 = 0;
            let mut file_count_delta: i64 = 0;
            let mut size_delta: i64 = 0;

            for assignment in assignments {
                let col = assignment_target_name(&assignment.target).to_lowercase();

                match col.as_str() {
                    "record_count" => {
                        record_count_delta = extract_delta_value(&assignment.value);
                    }
                    "file_count" => {
                        file_count_delta = extract_delta_value(&assignment.value);
                    }
                    "total_size_bytes" => {
                        size_delta = extract_delta_value(&assignment.value);
                    }
                    _ => {}
                }
            }

            return Ok(CatalogOp::UpdateTableStats(UpdateTableStatsOp {
                table_id,
                record_count_delta,
                file_count_delta,
                size_delta,
            }));
        }

        Err(DispatchError::Unsupported(format!(
            "UPDATE {table_name} with unsupported pattern"
        )))
    } else {
        Err(DispatchError::Unsupported("not an UPDATE".to_string()))
    }
}

fn classify_set(stmt: &Statement) -> Result<CatalogOp, DispatchError> {
    if let Statement::SetVariable {
        variables, value, ..
    } = stmt
    {
        let var_name = variables
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(".");
        let val_str = value
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        Ok(CatalogOp::Set {
            variable: var_name,
            value: val_str.trim_matches('\'').to_string(),
        })
    } else {
        Err(DispatchError::Unsupported("not a SET".to_string()))
    }
}

fn classify_show(stmt: &Statement) -> Result<CatalogOp, DispatchError> {
    if let Statement::ShowVariable { variable, .. } = stmt {
        let var_name = variable
            .iter()
            .map(|i| i.value.clone())
            .collect::<Vec<_>>()
            .join(".");
        Ok(CatalogOp::Show { variable: var_name })
    } else {
        Err(DispatchError::Unsupported("not a SHOW".to_string()))
    }
}

// -- INSERT classifiers --

fn classify_snapshot_insert(insert: &sqlparser::ast::Insert) -> Result<CatalogOp, DispatchError> {
    let values = extract_insert_values(insert)?;
    Ok(CatalogOp::InsertSnapshot(InsertSnapshotOp {
        snapshot_id: values.first().and_then(|v| v.parse().ok()).unwrap_or(0),
        schema_version: values.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
        created_at: values.get(2).cloned().unwrap_or_default(),
        author: values.get(3).cloned().filter(|s| !s.is_empty()),
        message: values.get(4).cloned().filter(|s| !s.is_empty()),
    }))
}

fn classify_snapshot_changes_insert(
    insert: &sqlparser::ast::Insert,
) -> Result<CatalogOp, DispatchError> {
    let values = extract_insert_values(insert)?;
    Ok(CatalogOp::InsertSnapshotChanges(InsertSnapshotChangesOp {
        snapshot_id: values.first().and_then(|v| v.parse().ok()).unwrap_or(0),
        changes_json: values.get(1).cloned().unwrap_or_default(),
    }))
}

fn classify_schema_insert(insert: &sqlparser::ast::Insert) -> Result<CatalogOp, DispatchError> {
    let values = extract_insert_values(insert)?;
    Ok(CatalogOp::InsertSchema(InsertSchemaOp {
        schema_id: values.first().and_then(|v| v.parse().ok()).unwrap_or(0),
        name: values.get(1).cloned().unwrap_or_default(),
        begin_snapshot: values.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
    }))
}

fn classify_table_insert(insert: &sqlparser::ast::Insert) -> Result<CatalogOp, DispatchError> {
    let values = extract_insert_values(insert)?;
    Ok(CatalogOp::InsertTable(InsertTableOp {
        schema_id: values.first().and_then(|v| v.parse().ok()).unwrap_or(0),
        table_id: values.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
        name: values.get(2).cloned().unwrap_or_default(),
        uuid: values.get(3).cloned().unwrap_or_default(),
        begin_snapshot: values.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
    }))
}

fn classify_column_insert(insert: &sqlparser::ast::Insert) -> Result<CatalogOp, DispatchError> {
    let values = extract_insert_values(insert)?;
    Ok(CatalogOp::InsertColumn(InsertColumnOp {
        table_id: values.first().and_then(|v| v.parse().ok()).unwrap_or(0),
        column_id: values.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
        name: values.get(2).cloned().unwrap_or_default(),
        data_type: values.get(3).cloned().unwrap_or_default(),
        is_nullable: values
            .get(4)
            .map(|v| v == "true" || v == "t" || v == "1")
            .unwrap_or(true),
        default_value: values.get(5).cloned().filter(|s| !s.is_empty()),
        begin_snapshot: values.get(6).and_then(|v| v.parse().ok()).unwrap_or(0),
    }))
}

fn classify_data_file_insert(insert: &sqlparser::ast::Insert) -> Result<CatalogOp, DispatchError> {
    let values = extract_insert_values(insert)?;
    Ok(CatalogOp::InsertDataFile(InsertDataFileOp {
        table_id: values.first().and_then(|v| v.parse().ok()).unwrap_or(0),
        data_file_id: values.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
        path: values.get(2).cloned().unwrap_or_default(),
        path_is_relative: values
            .get(3)
            .map(|v| v == "true" || v == "t" || v == "1")
            .unwrap_or(false),
        file_size_bytes: values.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
        record_count: values.get(5).and_then(|v| v.parse().ok()).unwrap_or(0),
        begin_snapshot: values.get(6).and_then(|v| v.parse().ok()).unwrap_or(0),
    }))
}

fn classify_delete_file_insert(
    insert: &sqlparser::ast::Insert,
) -> Result<CatalogOp, DispatchError> {
    let values = extract_insert_values(insert)?;
    Ok(CatalogOp::InsertDeleteFile(InsertDeleteFileOp {
        data_file_id: values.first().and_then(|v| v.parse().ok()).unwrap_or(0),
        delete_file_id: values.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
        path: values.get(2).cloned().unwrap_or_default(),
        path_is_relative: values
            .get(3)
            .map(|v| v == "true" || v == "t" || v == "1")
            .unwrap_or(false),
        file_size_bytes: values.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
        record_count: values.get(5).and_then(|v| v.parse().ok()).unwrap_or(0),
    }))
}

fn classify_inlined_insert_insert(
    insert: &sqlparser::ast::Insert,
) -> Result<CatalogOp, DispatchError> {
    let values = extract_insert_values(insert)?;
    Ok(CatalogOp::InsertInlinedInsert(InsertInlinedInsertOp {
        table_id: values.first().and_then(|v| v.parse().ok()).unwrap_or(0),
        schema_version: values.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
        row_id: values.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
        payload: values
            .get(3)
            .map(|v| v.as_bytes().to_vec())
            .unwrap_or_default(),
        begin_snapshot: values.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
    }))
}

fn classify_inlined_delete_insert(
    insert: &sqlparser::ast::Insert,
) -> Result<CatalogOp, DispatchError> {
    let values = extract_insert_values(insert)?;
    Ok(CatalogOp::InsertInlinedDelete(InsertInlinedDeleteOp {
        table_id: values.first().and_then(|v| v.parse().ok()).unwrap_or(0),
        data_file_id: values.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
        row_id: values.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
        begin_snapshot: values.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
    }))
}

fn classify_table_stats_insert(
    insert: &sqlparser::ast::Insert,
) -> Result<CatalogOp, DispatchError> {
    let values = extract_insert_values(insert)?;
    Ok(CatalogOp::InsertTableStats(InsertTableStatsOp {
        table_id: values.first().and_then(|v| v.parse().ok()).unwrap_or(0),
        record_count: values.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
        file_count: values.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
        total_size_bytes: values.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
    }))
}

fn classify_file_column_stats_insert(
    insert: &sqlparser::ast::Insert,
) -> Result<CatalogOp, DispatchError> {
    let values = extract_insert_values(insert)?;
    Ok(CatalogOp::InsertFileColumnStats(InsertFileColumnStatsOp {
        table_id: values.first().and_then(|v| v.parse().ok()).unwrap_or(0),
        column_id: values.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
        data_file_id: values.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
        min_value: values.get(3).cloned().filter(|s| !s.is_empty()),
        max_value: values.get(4).cloned().filter(|s| !s.is_empty()),
        null_count: values.get(5).and_then(|v| v.parse().ok()),
        contains_nan: values
            .get(6)
            .map(|v| v == "true" || v == "t" || v == "1")
            .unwrap_or(false),
    }))
}

// -- Helper functions --

fn assignment_target_name(target: &AssignmentTarget) -> String {
    match target {
        AssignmentTarget::ColumnName(name) => name.to_string(),
        AssignmentTarget::Tuple(names) => names
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", "),
    }
}

fn extract_insert_values(insert: &sqlparser::ast::Insert) -> Result<Vec<String>, DispatchError> {
    let body = insert
        .source
        .as_ref()
        .ok_or_else(|| DispatchError::Unsupported("INSERT without VALUES".to_string()))?;

    if let SetExpr::Values(values) = body.body.as_ref() {
        if let Some(row) = values.rows.first() {
            return Ok(row.iter().map(expr_to_string).collect());
        }
    }
    Err(DispatchError::Unsupported(
        "INSERT without simple VALUES".to_string(),
    ))
}

fn expr_to_string(expr: &Expr) -> String {
    match expr {
        Expr::Value(v) => value_to_string(&v.value).unwrap_or_default(),
        Expr::UnaryOp {
            op: sqlparser::ast::UnaryOperator::Minus,
            expr,
        } => {
            let inner = expr_to_string(expr);
            format!("-{inner}")
        }
        Expr::Identifier(ident) => ident.value.clone(),
        _ => expr.to_string(),
    }
}

fn expr_to_u64(expr: &Expr) -> Option<u64> {
    match expr {
        Expr::Value(v) => value_to_string(&v.value)?.parse().ok(),
        _ => expr.to_string().parse().ok(),
    }
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Number(n, _) => Some(n.clone()),
        Value::SingleQuotedString(s) => Some(s.clone()),
        Value::DoubleQuotedString(s) => Some(s.clone()),
        Value::Boolean(b) => Some(b.to_string()),
        Value::Null => Some(String::new()),
        _ => Some(value.to_string()),
    }
}

fn extract_eq_value(expr: &Expr, column_name: &str) -> Option<u64> {
    match expr {
        Expr::BinaryOp { left, op, right } => {
            if matches!(op, sqlparser::ast::BinaryOperator::Eq) {
                if let Expr::Identifier(ident) = left.as_ref() {
                    if ident.value.to_lowercase() == column_name {
                        return expr_to_u64(right);
                    }
                }
                if let Expr::Identifier(ident) = right.as_ref() {
                    if ident.value.to_lowercase() == column_name {
                        return expr_to_u64(left);
                    }
                }
            }
            if matches!(
                op,
                sqlparser::ast::BinaryOperator::And | sqlparser::ast::BinaryOperator::Or
            ) {
                let l = extract_eq_value(left, column_name);
                if l.is_some() {
                    return l;
                }
                return extract_eq_value(right, column_name);
            }
            None
        }
        _ => None,
    }
}

fn extract_eq_string_value(expr: &Expr, column_name: &str) -> Option<String> {
    match expr {
        Expr::BinaryOp { left, op, right } => {
            if matches!(op, sqlparser::ast::BinaryOperator::Eq) {
                if let Expr::Identifier(ident) = left.as_ref() {
                    if ident.value.to_lowercase() == column_name {
                        if let Expr::Value(v) = right.as_ref() {
                            return value_to_string(&v.value);
                        }
                    }
                }
            }
            if matches!(
                op,
                sqlparser::ast::BinaryOperator::And | sqlparser::ast::BinaryOperator::Or
            ) {
                let l = extract_eq_string_value(left, column_name);
                if l.is_some() {
                    return l;
                }
                return extract_eq_string_value(right, column_name);
            }
            None
        }
        _ => None,
    }
}

fn extract_mvcc_snapshot_id(expr: &Expr) -> Option<u64> {
    // Look for: begin_snapshot <= N AND (end_snapshot IS NULL OR N < end_snapshot)
    // We extract N from the `begin_snapshot <= N` part
    match expr {
        Expr::BinaryOp { left, op, right } => {
            if matches!(op, sqlparser::ast::BinaryOperator::LtEq) {
                if let Expr::Identifier(ident) = left.as_ref() {
                    if ident.value.to_lowercase() == "begin_snapshot" {
                        return expr_to_u64(right);
                    }
                }
            }
            if matches!(op, sqlparser::ast::BinaryOperator::GtEq) {
                if let Expr::Identifier(ident) = right.as_ref() {
                    if ident.value.to_lowercase() == "begin_snapshot" {
                        return expr_to_u64(left);
                    }
                }
            }
            if matches!(
                op,
                sqlparser::ast::BinaryOperator::And | sqlparser::ast::BinaryOperator::Or
            ) {
                let l = extract_mvcc_snapshot_id(left);
                if l.is_some() {
                    return l;
                }
                return extract_mvcc_snapshot_id(right);
            }
            None
        }
        Expr::Nested(inner) => extract_mvcc_snapshot_id(inner),
        _ => None,
    }
}

fn extract_id_from_where(expr: &Expr) -> Result<(String, u64), DispatchError> {
    match expr {
        Expr::BinaryOp { left, op, right } => {
            if matches!(op, sqlparser::ast::BinaryOperator::Eq) {
                if let Expr::Identifier(ident) = left.as_ref() {
                    let col = ident.value.to_lowercase();
                    if col.ends_with("_id") {
                        if let Some(val) = expr_to_u64(right) {
                            return Ok((col, val));
                        }
                    }
                }
            }
            if matches!(op, sqlparser::ast::BinaryOperator::And) {
                let l = extract_id_from_where(left);
                if l.is_ok() {
                    return l;
                }
                return extract_id_from_where(right);
            }
            Err(DispatchError::InvalidValue(
                "cannot extract ID from WHERE".to_string(),
            ))
        }
        _ => Err(DispatchError::InvalidValue(
            "cannot extract ID from WHERE".to_string(),
        )),
    }
}

fn extract_delta_value(expr: &Expr) -> i64 {
    match expr {
        Expr::BinaryOp { left, op, right } => {
            if matches!(op, sqlparser::ast::BinaryOperator::Plus) {
                // record_count + N => delta is N
                if let Some(val) = expr_to_u64(right) {
                    return val as i64;
                }
                if let Some(val) = expr_to_u64(left) {
                    return val as i64;
                }
            }
            if matches!(op, sqlparser::ast::BinaryOperator::Minus) {
                if let Some(val) = expr_to_u64(right) {
                    return -(val as i64);
                }
            }
            0
        }
        Expr::Value(_) => expr_to_u64(expr).map(|v| v as i64).unwrap_or(0),
        _ => 0,
    }
}

fn extract_table_id_from_inlined_name(name: &str) -> Result<u64, DispatchError> {
    // Format: ducklake_inlined_insert_t{table_id}_v{version} or similar
    // Try to extract a numeric table_id
    let parts: Vec<&str> = name.split('_').collect();
    for (i, part) in parts.iter().enumerate() {
        if let Some(stripped) = part.strip_prefix('t') {
            if let Ok(id) = stripped.parse::<u64>() {
                return Ok(id);
            }
        }
        // Also try just numeric parts after "insert" or "delete"
        if (*part == "insert" || *part == "delete") && i + 1 < parts.len() {
            if let Ok(id) = parts[i + 1].parse::<u64>() {
                return Ok(id);
            }
        }
    }
    // Fallback: try the last numeric part
    for part in parts.iter().rev() {
        if let Ok(id) = part.parse::<u64>() {
            return Ok(id);
        }
    }
    Err(DispatchError::InvalidValue(format!(
        "cannot extract table_id from inlined table name: {name}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_begin_commit_rollback() {
        assert_eq!(dispatch("BEGIN").unwrap(), CatalogOp::Begin);
        assert_eq!(dispatch("COMMIT").unwrap(), CatalogOp::Commit);
        assert_eq!(dispatch("ROLLBACK").unwrap(), CatalogOp::Rollback);
        assert_eq!(dispatch("begin").unwrap(), CatalogOp::Begin);
        assert_eq!(dispatch("BEGIN TRANSACTION").unwrap(), CatalogOp::Begin);
        assert_eq!(dispatch("START TRANSACTION").unwrap(), CatalogOp::Begin);
        assert_eq!(dispatch("END").unwrap(), CatalogOp::Commit);
        assert_eq!(dispatch("ABORT").unwrap(), CatalogOp::Rollback);
    }

    #[test]
    fn dispatch_set_show() {
        let op = dispatch("SET timezone = 'UTC'").unwrap();
        assert_eq!(
            op,
            CatalogOp::Set {
                variable: "timezone".to_string(),
                value: "UTC".to_string()
            }
        );

        let op = dispatch("SHOW server_version").unwrap();
        assert_eq!(
            op,
            CatalogOp::Show {
                variable: "server_version".to_string()
            }
        );
    }

    #[test]
    fn dispatch_current_schema() {
        assert_eq!(
            dispatch("SELECT current_schema()").unwrap(),
            CatalogOp::SelectCurrentSchema
        );
    }

    #[test]
    fn dispatch_version() {
        assert_eq!(
            dispatch("SELECT version()").unwrap(),
            CatalogOp::SelectVersion
        );
    }

    #[test]
    fn dispatch_max_snapshot() {
        assert_eq!(
            dispatch("SELECT max(snapshot_id) FROM ducklake_snapshot").unwrap(),
            CatalogOp::SelectMaxSnapshot
        );
    }

    #[test]
    fn dispatch_schema_mvcc_select() {
        let sql = "SELECT schema_id, schema_name FROM ducklake_schema \
                   WHERE begin_snapshot <= 5 AND (end_snapshot IS NULL OR 5 < end_snapshot)";
        let op = dispatch(sql).unwrap();
        assert_eq!(op, CatalogOp::SelectSchemas { dl_snapshot_id: 5 });
    }

    #[test]
    fn dispatch_table_mvcc_select() {
        let sql = "SELECT * FROM ducklake_table \
                   WHERE schema_id = 1 AND begin_snapshot <= 3 AND (end_snapshot IS NULL OR 3 < end_snapshot)";
        let op = dispatch(sql).unwrap();
        assert_eq!(
            op,
            CatalogOp::SelectTables {
                schema_id: 1,
                dl_snapshot_id: 3,
            }
        );
    }

    #[test]
    fn dispatch_data_file_select() {
        let sql = "SELECT * FROM ducklake_data_file WHERE table_id = 42";
        let op = dispatch(sql).unwrap();
        assert_eq!(op, CatalogOp::SelectDataFiles { table_id: 42 });
    }

    #[test]
    fn dispatch_data_file_with_join() {
        let sql = "SELECT d.data_file_id, d.file_path FROM ducklake_data_file d \
                   LEFT JOIN ducklake_delete_file del ON d.data_file_id = del.data_file_id \
                   WHERE table_id = 7";
        let op = dispatch(sql).unwrap();
        assert_eq!(op, CatalogOp::SelectDataFilesWithDeletes { table_id: 7 });
    }

    #[test]
    fn dispatch_file_column_stats() {
        let sql = "SELECT data_file_id FROM ducklake_file_column_stats \
                   WHERE table_id = 1 AND column_id = 2 AND min_value > '10'";
        let op = dispatch(sql).unwrap();
        assert_eq!(
            op,
            CatalogOp::SelectFileColumnStats {
                table_id: 1,
                column_id: 2
            }
        );
    }

    #[test]
    fn dispatch_pg_type() {
        let sql = "SELECT oid, typname FROM pg_catalog.pg_type WHERE typname IN ('bool','int4','int8','text')";
        let op = dispatch(sql).unwrap();
        if let CatalogOp::SelectPgType { type_names } = op {
            assert!(type_names.contains(&"bool".to_string()));
            assert!(type_names.contains(&"int4".to_string()));
        } else {
            panic!("expected SelectPgType");
        }
    }

    #[test]
    fn dispatch_insert_snapshot() {
        let sql = "INSERT INTO ducklake_snapshot (snapshot_id, schema_version, created_at) VALUES (1, 0, '2024-01-01')";
        let op = dispatch(sql).unwrap();
        assert!(matches!(op, CatalogOp::InsertSnapshot(_)));
    }

    #[test]
    fn dispatch_update_end_snapshot() {
        let sql = "UPDATE ducklake_table SET end_snapshot = 5 WHERE table_id = 1 AND end_snapshot IS NULL";
        let op = dispatch(sql).unwrap();
        if let CatalogOp::UpdateEndSnapshot(u) = op {
            assert_eq!(u.table_name, "ducklake_table");
            assert_eq!(u.end_snapshot, 5);
            assert_eq!(u.id_value, 1);
        } else {
            panic!("expected UpdateEndSnapshot");
        }
    }

    #[test]
    fn dispatch_update_table_stats() {
        let sql =
            "UPDATE ducklake_table_stats SET record_count = record_count + 100 WHERE table_id = 2";
        let op = dispatch(sql).unwrap();
        if let CatalogOp::UpdateTableStats(u) = op {
            assert_eq!(u.table_id, 2);
            assert_eq!(u.record_count_delta, 100);
        } else {
            panic!("expected UpdateTableStats");
        }
    }

    #[test]
    fn dispatch_create_inlined_table() {
        let sql = "CREATE TABLE ducklake_inlined_insert_t1_v1 (row_id BIGINT, payload BYTEA)";
        let op = dispatch(sql).unwrap();
        assert!(matches!(op, CatalogOp::CreateInlinedTable { .. }));
    }

    #[test]
    fn dispatch_unsupported_returns_error() {
        let result = dispatch("CREATE TABLE users (id INT)");
        assert!(result.is_err());
    }

    #[test]
    fn dispatch_select_metadata() {
        let sql = "SELECT * FROM ducklake_metadata WHERE metadata_key = 'data_path'";
        let op = dispatch(sql).unwrap();
        assert_eq!(
            op,
            CatalogOp::SelectMetadata {
                key: "data_path".to_string()
            }
        );
    }
}
