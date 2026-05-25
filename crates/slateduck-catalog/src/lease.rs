//! Snapshot lease management: prevents GC from advancing past leased snapshots.
//!
//! Consumers (e.g., pg-trickle) acquire leases via `hold_snapshot()` to guarantee
//! that `table_changes(start_snapshot=...)` will not return SQLSTATE 55000.
//! Leases have a TTL to prevent leaked leases from blocking GC indefinitely.

use prost::Message;
use slatedb::Db;
use slateduck_core::keys;
use slateduck_core::rows::SnapshotLeaseRow;

use crate::error::{CatalogError, CatalogResult};

/// Hold a snapshot lease: prevents GC from advancing past `min_snapshot_id`.
///
/// If the consumer already holds a lease, it is updated in place.
pub async fn hold_snapshot(
    db: &Db,
    consumer_id: &str,
    min_snapshot_id: u64,
    ttl_seconds: u64,
) -> CatalogResult<SnapshotLeaseRow> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let expires_at_unix_ms = now_ms + (ttl_seconds * 1000);

    let row = SnapshotLeaseRow {
        consumer_id: consumer_id.to_string(),
        min_snapshot_id,
        expires_at_unix_ms,
    };

    let key = keys::key_snapshot_lease(consumer_id);
    let value = row.encode_to_vec();
    db.put(&key, &value).await?;

    Ok(row)
}

/// Release a snapshot lease by consumer_id.
pub async fn release_snapshot(db: &Db, consumer_id: &str) -> CatalogResult<bool> {
    let key = keys::key_snapshot_lease(consumer_id);
    let existed = db.get(&key).await?.is_some();
    db.delete(&key).await?;
    Ok(existed)
}

/// Read all active (non-expired) snapshot leases.
pub async fn list_active_leases(db: &Db) -> CatalogResult<Vec<SnapshotLeaseRow>> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let prefix = keys::prefix_snapshot_leases();
    let mut leases = Vec::new();
    let mut iter = db.scan_prefix(&prefix).await?;

    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        if let Ok(row) = SnapshotLeaseRow::decode(kv.value.as_ref()) {
            if row.expires_at_unix_ms > now_ms {
                leases.push(row);
            }
        }
    }

    Ok(leases)
}

/// Get the minimum snapshot ID that is currently leased (active, non-expired).
/// Returns `None` if no active leases exist.
pub async fn minimum_leased_snapshot(db: &Db) -> CatalogResult<Option<u64>> {
    let leases = list_active_leases(db).await?;
    Ok(leases.iter().map(|l| l.min_snapshot_id).min())
}

/// Compute the end key for a prefix scan (increment last byte).
#[allow(dead_code)]
fn scan_end_for_prefix(prefix: &[u8]) -> Vec<u8> {
    let mut end = prefix.to_vec();
    if let Some(last) = end.last_mut() {
        *last = last.wrapping_add(1);
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;
    use slateduck_core::tags::TAG_SNAPSHOT_LEASE;

    #[test]
    fn test_scan_end_for_prefix() {
        let prefix = vec![TAG_SNAPSHOT_LEASE];
        let end = scan_end_for_prefix(&prefix);
        assert_eq!(end, vec![TAG_SNAPSHOT_LEASE + 1]);
    }
}
