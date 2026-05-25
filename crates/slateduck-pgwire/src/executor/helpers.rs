//! Shared response-builder helpers and parameter utilities for the executor.

use std::sync::Arc;

use pgwire::api::results::{DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response};
use pgwire::api::Type;

use slateduck_sql::ParamValues;

use crate::error::SlateDuckError;
use crate::session::SessionState;
use crate::types;

/// F-24: Require a u64 parameter; returns SQLSTATE 22023 if absent or invalid.
pub(super) fn require_param_u64(
    params: &ParamValues,
    idx: usize,
    name: &str,
) -> Result<u64, SlateDuckError> {
    params
        .get_u64(idx)
        .map_err(|_| SlateDuckError::MissingParam {
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
        _ => String::new(),
    }
}

pub(super) fn apply_set(var: &str, val: &str, session: &mut SessionState) {
    let clean_val = val.trim_matches('\'').to_string();
    match var.to_lowercase().as_str() {
        "timezone" | "time zone" => session.settings.timezone = clean_val,
        "client_encoding" => session.settings.client_encoding = clean_val,
        "datestyle" | "date_style" => session.settings.date_style = clean_val,
        "application_name" => session.settings.application_name = clean_val,
        _ => {} // Accept and ignore unknown settings
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

pub(super) fn make_null_int_response<'a>(col_name: &str) -> Response<'a> {
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
