//! v0.20 catalog key-encoding migration.
//!
//! Snapshot lease keys and extension schema keys were previously encoded as
//! `[tag][DefaultHasher-64-bit-hash]`. `DefaultHasher` is not stable across
//! Rust versions and distinct strings can collide. v0.20 replaces both with
//! length-prefixed UTF-8: `[tag][u16-len-BE][utf-8-bytes]`.
//!
//! This module detects whether the migration has already been applied (by
//! checking a system marker key) and, if not, rewrites any old-format keys
//! found in the catalog in a single pass before setting the marker.
//!
//! The migration is idempotent: running it on an already-migrated catalog
//! is a no-op (the marker is present → skip scan entirely).

use prost::Message;
use slatedb::Db;
use slateduck_core::keys;
use slateduck_core::rows::{ExtensionSchemaRow, SnapshotLeaseRow};
use slateduck_core::tags::{
    TAG_EXTENSION_SCHEMA, TAG_SNAPSHOT_LEASE, SYSTEM_KEY_ENCODING_V020_MIGRATED,
};

use crate::error::{CatalogError, CatalogResult};

/// Run the v0.20 key-encoding migration if it has not yet been applied.
///
/// This is called automatically from `CatalogStore::open()`. The full scan
/// is skipped on subsequent opens because the system marker is already set.
pub async fn migrate_key_encoding_if_needed(db: &Db) -> CatalogResult<()> {
    let marker_key = keys::key_system(SYSTEM_KEY_ENCODING_V020_MIGRATED);
    if db.get(&marker_key).await?.is_some() {
        return Ok(()); // Already migrated.
    }

    // Migrate snapshot lease keys.
    migrate_lease_keys(db).await?;

    // Migrate extension schema keys.
    migrate_extension_keys(db).await?;

    // Write migration marker to skip future scans.
    db.put(&marker_key, b"1").await?;
    Ok(())
}

/// Rewrite any hash-encoded snapshot lease keys to length-prefixed encoding.
async fn migrate_lease_keys(db: &Db) -> CatalogResult<()> {
    let prefix = vec![TAG_SNAPSHOT_LEASE];
    let mut iter = db.scan_prefix(&prefix).await?;

    let mut to_migrate: Vec<(Vec<u8>, Vec<u8>)> = Vec::new(); // (old_key, value)
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row = match SnapshotLeaseRow::decode(kv.value.as_ref()) {
            Ok(r) => r,
            Err(_) => continue, // Skip corrupt rows; repair handles those.
        };
        let expected_key = keys::key_snapshot_lease(&row.consumer_id);
        if kv.key.as_ref() != expected_key.as_slice() {
            to_migrate.push((kv.key.to_vec(), kv.value.to_vec()));
        }
    }

    for (old_key, value) in to_migrate {
        // Decode again to get consumer_id for new key.
        if let Ok(row) = SnapshotLeaseRow::decode(value.as_ref()) {
            let new_key = keys::key_snapshot_lease(&row.consumer_id);
            db.put(&new_key, &value).await?;
            db.delete(&old_key).await?;
        }
    }
    Ok(())
}

/// Rewrite any hash-encoded extension schema keys to length-prefixed encoding.
async fn migrate_extension_keys(db: &Db) -> CatalogResult<()> {
    let prefix = vec![TAG_EXTENSION_SCHEMA];
    let mut iter = db.scan_prefix(&prefix).await?;

    let mut to_migrate: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row = match ExtensionSchemaRow::decode(kv.value.as_ref()) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let expected_key = keys::key_extension_schema(
            row.extension_id as u8,
            &row.table_name,
            row.row_id,
        );
        if kv.key.as_ref() != expected_key.as_slice() {
            to_migrate.push((kv.key.to_vec(), kv.value.to_vec()));
        }
    }

    for (old_key, value) in to_migrate {
        if let Ok(row) = ExtensionSchemaRow::decode(value.as_ref()) {
            let new_key = keys::key_extension_schema(
                row.extension_id as u8,
                &row.table_name,
                row.row_id,
            );
            db.put(&new_key, &value).await?;
            db.delete(&old_key).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use slateduck_core::rows::SnapshotLeaseRow;
    use tempfile::TempDir;

    async fn open_test_db(dir: &TempDir) -> Db {
        let object_store = std::sync::Arc::new(
            object_store::local::LocalFileSystem::new_with_prefix(dir.path()).unwrap(),
        );
        let path = object_store::path::Path::from("test");
        slatedb::Db::open(path, object_store).await.unwrap()
    }

    #[tokio::test]
    async fn test_migration_marker_prevents_rescan() {
        let dir = TempDir::new().unwrap();
        let db = open_test_db(&dir).await;

        // First call: no marker → runs scan (no old keys to migrate)
        migrate_key_encoding_if_needed(&db).await.unwrap();

        // Verify marker is set
        let marker_key = keys::key_system(SYSTEM_KEY_ENCODING_V020_MIGRATED);
        assert!(db.get(&marker_key).await.unwrap().is_some());

        // Second call: marker present → skip
        migrate_key_encoding_if_needed(&db).await.unwrap();
    }

    #[tokio::test]
    async fn test_new_format_lease_keys_are_not_migrated() {
        let dir = TempDir::new().unwrap();
        let db = open_test_db(&dir).await;

        // Write a new-format lease key directly.
        let consumer_id = "my-consumer";
        let key = keys::key_snapshot_lease(consumer_id);
        let row = SnapshotLeaseRow {
            consumer_id: consumer_id.to_string(),
            min_snapshot_id: 42,
            expires_at_unix_ms: u64::MAX,
        };
        db.put(&key, &row.encode_to_vec()).await.unwrap();

        migrate_key_encoding_if_needed(&db).await.unwrap();

        // Key must still exist (was not deleted as an old-format key).
        assert!(db.get(&key).await.unwrap().is_some());
    }
}
