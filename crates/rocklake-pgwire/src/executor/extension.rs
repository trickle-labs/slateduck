//! Extension schema executor operations: CREATE/INSERT/SELECT/DELETE on extension tables.

use std::sync::Arc;

use pgwire::api::results::{DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response, Tag};
use pgwire::api::Type;

use rocklake_catalog::CatalogStore;
use rocklake_sql::ParamValues;

use crate::error::RocklakeError;

pub(super) fn hash_table_ref(table_ref: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    table_ref.hash(&mut hasher);
    hasher.finish()
}

pub(super) async fn execute_create_extension_table<'a>(
    schema_name: &str,
    table_name: &str,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
    extension_schemas: &Arc<Vec<String>>,
) -> Result<Vec<Response<'a>>, RocklakeError> {
    if !rocklake_catalog::is_registered_extension(schema_name, extension_schemas) {
        return Err(RocklakeError::PermissionDenied(format!(
            "extension schema '{schema_name}' is not in the allowed list"
        )));
    }
    let extension_id = rocklake_catalog::resolve_extension_id(schema_name).ok_or_else(|| {
        RocklakeError::Unsupported(format!(
            "extension schema '{schema_name}' is not registered"
        ))
    })?;
    let store_lock = store.lock().await;
    let db = store_lock.db();
    rocklake_catalog::create_extension_table(db, extension_id, table_name)
        .await
        .map_err(RocklakeError::from)?;
    Ok(vec![Response::Execution(Tag::new("CREATE TABLE"))])
}

pub(super) async fn execute_insert_extension_row<'a>(
    schema_name: &str,
    table_name: &str,
    columns: &[String],
    params: &ParamValues,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
    extension_schemas: &Arc<Vec<String>>,
) -> Result<Vec<Response<'a>>, RocklakeError> {
    if !rocklake_catalog::is_registered_extension(schema_name, extension_schemas) {
        return Err(RocklakeError::PermissionDenied(format!(
            "extension schema '{schema_name}' is not in the allowed list"
        )));
    }
    let extension_id = rocklake_catalog::resolve_extension_id(schema_name).ok_or_else(|| {
        RocklakeError::Unsupported(format!(
            "extension schema '{schema_name}' is not registered"
        ))
    })?;
    let store_lock = store.lock().await;
    let db = store_lock.db();

    let data_json = params.to_json_string_with_columns(columns);
    rocklake_catalog::insert_extension_row(db, extension_id, table_name, &data_json)
        .await
        .map_err(RocklakeError::from)?;
    Ok(vec![Response::Execution(Tag::new("INSERT 0 1"))])
}

pub(super) async fn execute_select_extension_table<'a>(
    schema_name: &str,
    table_name: &str,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
    extension_schemas: &Arc<Vec<String>>,
) -> Result<Vec<Response<'a>>, RocklakeError> {
    if !rocklake_catalog::is_registered_extension(schema_name, extension_schemas) {
        return Err(RocklakeError::PermissionDenied(format!(
            "extension schema '{schema_name}' is not in the allowed list"
        )));
    }
    let extension_id = rocklake_catalog::resolve_extension_id(schema_name).ok_or_else(|| {
        RocklakeError::Unsupported(format!(
            "extension schema '{schema_name}' is not registered"
        ))
    })?;
    let store_lock = store.lock().await;
    let db = store_lock.db();
    let rows = rocklake_catalog::select_extension_rows(db, extension_id, table_name)
        .await
        .map_err(RocklakeError::from)?;

    let schema = Arc::new(vec![
        FieldInfo::new("row_id".into(), None, None, Type::INT8, FieldFormat::Text),
        FieldInfo::new("data".into(), None, None, Type::TEXT, FieldFormat::Text),
    ]);
    let mut data_rows = Vec::new();
    for row in &rows {
        let mut encoder = DataRowEncoder::new(schema.clone());
        encoder
            .encode_field_with_type_and_format(
                &Some(row.row_id.to_string()),
                &Type::INT8,
                FieldFormat::Text,
            )
            .unwrap();
        encoder
            .encode_field_with_type_and_format(
                &Some(row.data_json.clone()),
                &Type::TEXT,
                FieldFormat::Text,
            )
            .unwrap();
        data_rows.push(encoder.finish());
    }
    let count = data_rows.len();
    let mut resp = QueryResponse::new(schema, futures::stream::iter(data_rows));
    resp.set_command_tag(&format!("SELECT {count}"));
    Ok(vec![Response::Query(resp)])
}

pub(super) async fn execute_delete_extension_rows<'a>(
    schema_name: &str,
    table_name: &str,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
    extension_schemas: &Arc<Vec<String>>,
) -> Result<Vec<Response<'a>>, RocklakeError> {
    if !rocklake_catalog::is_registered_extension(schema_name, extension_schemas) {
        return Err(RocklakeError::PermissionDenied(format!(
            "extension schema '{schema_name}' is not in the allowed list"
        )));
    }
    let extension_id = rocklake_catalog::resolve_extension_id(schema_name).ok_or_else(|| {
        RocklakeError::Unsupported(format!(
            "extension schema '{schema_name}' is not registered"
        ))
    })?;
    let store_lock = store.lock().await;
    let db = store_lock.db();
    let deleted = rocklake_catalog::delete_extension_rows(db, extension_id, table_name)
        .await
        .map_err(RocklakeError::from)?;
    Ok(vec![Response::Execution(Tag::new(&format!(
        "DELETE {deleted}"
    )))])
}
