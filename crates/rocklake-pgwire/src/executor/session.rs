//! Session-scoped executor operations: snapshot leases.

use std::sync::Arc;

use pgwire::api::results::Response;

use rocklake_catalog::CatalogStore;

use crate::error::RocklakeError;

use super::helpers::make_single_text_response;

pub(super) async fn execute_hold_snapshot<'a>(
    min_snapshot_id: u64,
    consumer_id: &str,
    ttl_seconds: u64,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
) -> Result<Vec<Response<'a>>, RocklakeError> {
    let store_lock = store.lock().await;
    let db = store_lock.db();
    rocklake_catalog::hold_snapshot(db, consumer_id, min_snapshot_id, ttl_seconds)
        .await
        .map_err(RocklakeError::from)?;

    Ok(vec![make_single_text_response("hold_snapshot", "OK")])
}

pub(super) async fn execute_release_snapshot<'a>(
    consumer_id: &str,
    store: &Arc<tokio::sync::Mutex<CatalogStore>>,
) -> Result<Vec<Response<'a>>, RocklakeError> {
    let store_lock = store.lock().await;
    let db = store_lock.db();
    let released = rocklake_catalog::release_snapshot(db, consumer_id)
        .await
        .map_err(RocklakeError::from)?;

    Ok(vec![make_single_text_response(
        "release_snapshot",
        if released { "OK" } else { "NOT_FOUND" },
    )])
}
