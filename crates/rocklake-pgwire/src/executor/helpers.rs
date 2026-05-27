//! Shared response-builder helpers and parameter utilities for the executor.

use std::sync::Arc;

use pgwire::api::results::{DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response, Tag};
use pgwire::api::Type;

use rocklake_core::rows::ColumnRow;
use rocklake_sql::ParamValues;

use crate::error::RockLakeError;
use crate::session::SessionState;
use crate::types;

/// F-24: Require a u64 parameter; returns SQLSTATE 22023 if absent or invalid.
pub(super) fn require_param_u64(
    params: &ParamValues,
    idx: usize,
    name: &str,
) -> Result<u64, RockLakeError> {
    params
        .get_u64(idx)
        .map_err(|_| RockLakeError::MissingParam {
            name: name.to_string(),
        })
}

pub(super) fn get_snapshot_param(params: &ParamValues) -> u64 {
    params.get_u64(0).unwrap_or(u64::MAX)
}

pub(super) fn get_show_value(var: &str, session: &SessionState) -> String {
    match var.to_lowercase().as_str() {
        "server_version" => "15.0".to_string(),
        "datestyle" | "date_style" => session.settings.date_style.clone(),
        "timezone" | "time zone" => session.settings.timezone.clone(),
        "client_encoding" => session.settings.client_encoding.clone(),
        "transaction_isolation" => "read committed".to_string(),
        "standard_conforming_strings" => "on".to_string(),
        // Fall back to the generic extra map for any driver-specific variable.
        other => session
            .settings
            .extra
            .get(other)
            .cloned()
            .unwrap_or_default(),
    }
}

pub(super) fn apply_set(var: &str, val: &str, session: &mut SessionState) {
    let clean_val = val.trim_matches('\'').to_string();
    match var.to_lowercase().as_str() {
        "timezone" | "time zone" => session.settings.timezone = clean_val,
        "client_encoding" => session.settings.client_encoding = clean_val,
        "datestyle" | "date_style" => session.settings.date_style = clean_val,
        "application_name" => session.settings.application_name = clean_val,
        // Store any unknown setting in the generic extra map so that driver-
        // specific SET commands (e.g. SET synchronize_seqscans = off) are
        // accepted and retrievable via SHOW without crashing or returning errors.
        other => {
            session.settings.extra.insert(other.to_string(), clean_val);
        }
    }
}

pub(super) fn make_single_text_response<'a>(col_name: &str, value: &str) -> Response<'a> {
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

pub(super) fn make_single_int_response<'a>(col_name: &str, value: i64) -> Response<'a> {
    let schema = Arc::new(vec![FieldInfo::new(
        col_name.to_string(),
        None,
        None,
        Type::INT8,
        FieldFormat::Binary,
    )]);
    let mut encoder = DataRowEncoder::new(schema.clone());
    encoder
        .encode_field_with_type_and_format(&Some(value), &Type::INT8, FieldFormat::Binary)
        .unwrap();
    let row = encoder.finish();
    let data_rows = vec![row];
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag("SELECT 1");
    Response::Query(resp)
}

pub(super) fn make_null_int_response<'a>(col_name: &str) -> Response<'a> {
    let schema = Arc::new(vec![FieldInfo::new(
        col_name.to_string(),
        None,
        None,
        Type::INT8,
        FieldFormat::Binary,
    )]);
    let mut encoder = DataRowEncoder::new(schema.clone());
    encoder
        .encode_field_with_type_and_format(&None::<i64>, &Type::INT8, FieldFormat::Binary)
        .unwrap();
    let row = encoder.finish();
    let data_rows = vec![row];
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag("SELECT 1");
    Response::Query(resp)
}

pub(super) fn make_pg_type_response<'a>() -> Response<'a> {
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

pub(super) fn make_empty_response<'a>() -> Response<'a> {
    let schema = Arc::new(vec![]);
    let resp = QueryResponse::new(schema, futures::stream::iter(Vec::new()));
    Response::Query(resp)
}

pub(super) fn make_version_with_rds_check_response<'a>() -> Response<'a> {
    let schema = Arc::new(vec![
        FieldInfo::new(
            "version".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "count".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
    ]);
    let mut encoder = DataRowEncoder::new(schema.clone());
    encoder
        .encode_field_with_type_and_format(
            &Some("PostgreSQL 15.0 on x86_64-pc-linux-gnu".to_string()),
            &Type::TEXT,
            FieldFormat::Text,
        )
        .expect("pgwire field encoding is infallible");
    // RDS check: 0 means not on RDS
    encoder
        .encode_field_with_type_and_format(&Some("0".to_string()), &Type::INT8, FieldFormat::Text)
        .expect("pgwire field encoding is infallible");
    let row = encoder.finish();
    let data_rows = vec![row];
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag("SELECT 1");
    Response::Query(resp)
}

/// `SELECT to_regclass('...')` — returns a single NULL TEXT row.
/// NULL tells DuckDB the named relation does not exist.
pub(super) fn make_null_text_response<'a>(col_name: &str) -> Response<'a> {
    let schema = Arc::new(vec![FieldInfo::new(
        col_name.to_string(),
        None,
        None,
        Type::TEXT,
        FieldFormat::Text,
    )]);
    let mut encoder = DataRowEncoder::new(schema.clone());
    encoder
        .encode_field_with_type_and_format(&None::<String>, &Type::TEXT, FieldFormat::Text)
        .expect("pgwire field encoding is infallible");
    let row = encoder.finish();
    let data_rows = vec![row];
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag("SELECT 1");
    Response::Query(resp)
}

/// `SELECT EXISTS(...)` — returns a single `false` BOOL row.
pub(super) fn make_false_bool_response<'a>(col_name: &str) -> Response<'a> {
    let schema = Arc::new(vec![FieldInfo::new(
        col_name.to_string(),
        None,
        None,
        Type::BOOL,
        FieldFormat::Text,
    )]);
    let mut encoder = DataRowEncoder::new(schema.clone());
    // PostgreSQL text format for boolean false is "f"
    encoder
        .encode_field_with_type_and_format(&Some("f".to_string()), &Type::TEXT, FieldFormat::Text)
        .expect("pgwire field encoding is infallible");
    let row = encoder.finish();
    let data_rows = vec![row];
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag("SELECT 1");
    Response::Query(resp)
}

/// Build one result-set row for the `pg_namespace` response.
/// Columns: `(oid INT8, nspname TEXT)`
fn make_pg_namespace_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        FieldInfo::new("oid".to_string(), None, None, Type::INT8, FieldFormat::Text),
        FieldInfo::new(
            "nspname".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
    ])
}

fn make_pg_namespace_rows(
    schema: Arc<Vec<FieldInfo>>,
    namespaces: &[(i64, &str)],
) -> Vec<pgwire::error::PgWireResult<pgwire::messages::data::DataRow>> {
    namespaces
        .iter()
        .map(|(oid, name)| {
            let mut enc = DataRowEncoder::new(schema.clone());
            enc.encode_field_with_type_and_format(
                &Some(oid.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("infallible");
            enc.encode_field_with_type_and_format(
                &Some(name.to_string()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .expect("infallible");
            enc.finish()
        })
        .collect()
}

/// Build the `pg_class` UNION result-set schema.
/// Columns: `(namespace_id INT8, relname TEXT, relpages INT8, attname TEXT,
///  type_name TEXT, type_modifier INT8, ndim INT8, attnum INT8,
///  notnull BOOL, constraint_id INT8, constraint_type TEXT, constraint_key TEXT)`
fn make_pg_class_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        FieldInfo::new(
            "namespace_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "relname".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "relpages".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "attname".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "type_name".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "type_modifier".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "ndim".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "attnum".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "notnull".to_string(),
            None,
            None,
            Type::BOOL,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "constraint_id".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "constraint_type".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "constraint_key".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
    ])
}

pub(super) fn make_pg_catalog_inlined_table_response<'a>(
    table_name: &str,
    columns: Vec<ColumnRow>,
) -> Response<'a> {
    let schema = make_pg_class_schema();
    let mut rows = Vec::new();
    let mut attnum = 1i64;

    for (name, type_name) in [
        ("row_id".to_string(), "int8".to_string()),
        ("begin_snapshot".to_string(), "int8".to_string()),
        ("end_snapshot".to_string(), "int8".to_string()),
    ] {
        rows.push(make_pg_class_attribute_row(
            schema.clone(),
            table_name,
            &name,
            &type_name,
            attnum,
        ));
        attnum += 1;
    }

    for column in columns {
        rows.push(make_pg_class_attribute_row(
            schema.clone(),
            table_name,
            &column.column_name,
            &pg_type_name_for_inlined_column(&column.data_type),
            attnum,
        ));
        attnum += 1;
    }

    let count = rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Response::Query(resp)
}

fn make_pg_class_attribute_row(
    schema: Arc<Vec<FieldInfo>>,
    table_name: &str,
    attname: &str,
    type_name: &str,
    attnum: i64,
) -> pgwire::error::PgWireResult<pgwire::messages::data::DataRow> {
    let mut enc = DataRowEncoder::new(schema);
    let values = [
        Some("1".to_string()),
        Some(table_name.to_string()),
        None,
        Some(attname.to_string()),
        Some(type_name.to_string()),
        None,
        Some("0".to_string()),
        Some(attnum.to_string()),
        Some("false".to_string()),
        None,
        None,
        None,
    ];
    for value in values {
        enc.encode_field_with_type_and_format(&value, &Type::TEXT, FieldFormat::Text)
            .expect("infallible");
    }
    enc.finish()
}

fn pg_type_name_for_inlined_column(logical_type: &str) -> String {
    match logical_type.to_ascii_lowercase().as_str() {
        "boolean" | "bool" => "bool".to_string(),
        "tinyint" | "int8" => "int2".to_string(),
        "smallint" | "int16" => "int2".to_string(),
        "integer" | "int" | "int4" | "int32" => "int4".to_string(),
        "bigint" | "int64" => "int8".to_string(),
        "varchar" | "text" | "string" | "blob" | "bytea" => "bytea".to_string(),
        "timestamp" | "timestamp without time zone" => "timestamp".to_string(),
        "timestamp with time zone" | "timestamptz" => "timestamptz".to_string(),
        "date" => "date".to_string(),
        _ => "text".to_string(),
    }
}

/// Build the `pg_enum` result-set schema.
/// Columns: `(oid INT8, enumtypid INT8, typname TEXT, enumlabel TEXT)`
fn make_pg_enum_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        FieldInfo::new("oid".to_string(), None, None, Type::INT8, FieldFormat::Text),
        FieldInfo::new(
            "enumtypid".to_string(),
            None,
            None,
            Type::INT8,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "typname".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "enumlabel".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
    ])
}

/// Build the `pg_type` composites result-set schema.
/// Columns: `(oid INT8, id INT8, type TEXT, attname TEXT, typname TEXT)`
fn make_pg_type_composites_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        FieldInfo::new("oid".to_string(), None, None, Type::INT8, FieldFormat::Text),
        FieldInfo::new("id".to_string(), None, None, Type::INT8, FieldFormat::Text),
        FieldInfo::new(
            "type".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "attname".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "typname".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
    ])
}

/// Build the `pg_indexes` result-set schema.
/// Columns: `(oid INT8, tablename TEXT, indexname TEXT)`
fn make_pg_indexes_schema() -> Arc<Vec<FieldInfo>> {
    Arc::new(vec![
        FieldInfo::new("oid".to_string(), None, None, Type::INT8, FieldFormat::Text),
        FieldInfo::new(
            "tablename".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "indexname".to_string(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        ),
    ])
}

/// Return the five result sets for the DuckDB postgres-scanner catalog scan,
/// plus a ROLLBACK command-complete.
///
/// The batch is:
///   1. `pg_namespace` — `(1, 'public')` and `(2, 'main')`
///   2. `pg_class` UNION — empty (no tables to advertise over postgres wire)
///   3. `pg_enum` — empty
///   4. `pg_type` composites — empty
///   5. `pg_indexes` — empty
///   +  ROLLBACK command-complete tag
pub(super) fn make_pg_catalog_scan_responses<'a>() -> Vec<Response<'a>> {
    // Result set 1: pg_namespace
    let ns_schema = make_pg_namespace_schema();
    let ns_rows = make_pg_namespace_rows(ns_schema.clone(), &[(1, "public"), (2, "main")]);
    let mut ns_resp = QueryResponse::new(ns_schema, futures::stream::iter(ns_rows));
    ns_resp.set_command_tag("SELECT 2");

    // Result set 2: pg_class UNION — empty
    let cls_schema = make_pg_class_schema();
    let cls_rows: Vec<pgwire::error::PgWireResult<pgwire::messages::data::DataRow>> = Vec::new();
    let mut cls_resp = QueryResponse::new(cls_schema, futures::stream::iter(cls_rows));
    cls_resp.set_command_tag("SELECT 0");

    // Result set 3: pg_enum — empty
    let enum_schema = make_pg_enum_schema();
    let enum_rows: Vec<pgwire::error::PgWireResult<pgwire::messages::data::DataRow>> = Vec::new();
    let mut enum_resp = QueryResponse::new(enum_schema, futures::stream::iter(enum_rows));
    enum_resp.set_command_tag("SELECT 0");

    // Result set 4: pg_type composites — empty
    let typ_schema = make_pg_type_composites_schema();
    let typ_rows: Vec<pgwire::error::PgWireResult<pgwire::messages::data::DataRow>> = Vec::new();
    let mut typ_resp = QueryResponse::new(typ_schema, futures::stream::iter(typ_rows));
    typ_resp.set_command_tag("SELECT 0");

    // Result set 5: pg_indexes — empty
    let idx_schema = make_pg_indexes_schema();
    let idx_rows: Vec<pgwire::error::PgWireResult<pgwire::messages::data::DataRow>> = Vec::new();
    let mut idx_resp = QueryResponse::new(idx_schema, futures::stream::iter(idx_rows));
    idx_resp.set_command_tag("SELECT 0");

    vec![
        Response::Query(ns_resp),
        Response::Query(cls_resp),
        Response::Query(enum_resp),
        Response::Query(typ_resp),
        Response::Query(idx_resp),
        Response::TransactionEnd(Tag::new("ROLLBACK")),
    ]
}
