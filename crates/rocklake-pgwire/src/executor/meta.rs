//! Meta-query executor operations: VirtualCatalogScan.

use std::sync::Arc;

use pgwire::api::results::Response;

use rocklake_catalog::CatalogStore;

use crate::error::RocklakeError;

use super::catalog::{make_schemas_response, make_tables_response};
use super::helpers::{make_empty_response, make_single_int_response};

pub(super) async fn execute_virtual_catalog_scan<'a>(
    table_name: &str,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
) -> Result<Vec<Response<'a>>, RocklakeError> {
    let reader = { store.lock().await.read_latest() };
    match table_name {
        "ducklake_snapshot" => {
            let snap = reader.get_snapshot().await.map_err(RocklakeError::from)?;
            let id = snap.as_ref().map(|s| s.snapshot_id).unwrap_or(0);
            Ok(vec![make_single_int_response("snapshot_id", id as i64)])
        }
        "ducklake_schema" => {
            let schemas = reader.list_schemas().await.map_err(RocklakeError::from)?;
            Ok(vec![make_schemas_response(schemas)])
        }
        "ducklake_table" => {
            let schemas = reader.list_schemas().await.map_err(RocklakeError::from)?;
            let mut all_tables = vec![];
            for schema in schemas {
                let tables = reader
                    .list_tables(schema.schema_id)
                    .await
                    .map_err(RocklakeError::from)?;
                all_tables.extend(tables);
            }
            Ok(vec![make_tables_response(all_tables)])
        }
        _ => Ok(vec![make_empty_response()]),
    }
}
