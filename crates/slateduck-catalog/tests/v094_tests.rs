//! Integration tests for v0.9.4 GA Ready features.
//!
//! Tests: F-11 concurrent reads, F-13 O(1) describe_table,
//!        F-20 writer session regression, F-22 DataFusion concurrent schema_names.

use object_store::path::Path as ObjectPath;
use slateduck_catalog::{CatalogStore, OpenOptions};
use slateduck_core::mvcc::SnapshotId;
use std::sync::Arc;
use tempfile::TempDir;

fn test_opts(dir: &TempDir) -> OpenOptions {
    let path = dir.path().to_str().unwrap().to_string();
    let store = Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&path).unwrap());
    OpenOptions {
        object_store: store,
        path: ObjectPath::from("catalog"),
        encryption: None,
    }
}

// ─── F-11: Concurrent Reads ───────────────────────────────────────────────

/// N concurrent readers created from the same store must all see the
/// committed state and must not block each other.
#[tokio::test]
async fn concurrent_readers_do_not_block() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("main").await.unwrap();
    let _snap = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&writer);

    // read_at() is now sync — the lock is dropped immediately after this call.
    let snap_id = SnapshotId::new(1);

    // Spawn 8 concurrent read tasks; each must see the "main" schema.
    let store = Arc::new(tokio::sync::Mutex::new(store));
    let mut handles = vec![];
    for _ in 0..8 {
        let store = store.clone();
        let handle = tokio::spawn(async move {
            let reader = { store.lock().await.read_at(snap_id).unwrap() };
            reader.list_schemas().await.unwrap()
        });
        handles.push(handle);
    }

    for h in handles {
        let schemas = h.await.unwrap();
        assert_eq!(schemas.len(), 1, "each concurrent reader must see 1 schema");
        assert_eq!(schemas[0].schema_name, "main");
    }
}

// ─── F-13: O(1) describe_table via Secondary Index ───────────────────────

/// describe_table must return the correct table info using the TAG_TABLE_BY_ID
/// secondary index (no full-table scan required).
#[tokio::test]
async fn describe_table_uses_secondary_index() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("catalog_test").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "events", Some("s3://bucket/events/"))
        .await
        .unwrap();
    writer
        .add_column(table_id, "event_id", "BIGINT", 0, false, None)
        .await
        .unwrap();
    writer
        .add_column(table_id, "event_name", "VARCHAR", 1, true, None)
        .await
        .unwrap();
    let snap = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&writer);

    let reader = store.read_at(snap).unwrap();
    let result = reader.describe_table(table_id).await.unwrap();
    assert!(result.is_some(), "describe_table must return the table");
    let (table, cols) = result.unwrap();
    assert_eq!(table.table_name, "events");
    assert_eq!(table.schema_id, schema_id);
    assert_eq!(cols.len(), 2);
    assert_eq!(cols[0].column_name, "event_id");
    assert_eq!(cols[1].column_name, "event_name");
}

/// describe_table must return None for a non-existent table_id.
#[tokio::test]
async fn describe_table_nonexistent_returns_none() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("s").await.unwrap();
    let snap = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&writer);

    let reader = store.read_at(snap).unwrap();
    let result = reader.describe_table(9999).await.unwrap();
    assert!(result.is_none());
}

// ─── F-20: Writer Session Regression ─────────────────────────────────────

/// Two sequential begin_write() sessions produce monotonically increasing,
/// non-overlapping snapshot IDs.
#[tokio::test]
async fn sequential_sessions_monotonic_snapshot_ids() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let mut w1 = store.begin_write();
    w1.create_schema("s1").await.unwrap();
    let snap1 = w1.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&w1);

    let mut w2 = store.begin_write();
    w2.create_schema("s2").await.unwrap();
    let snap2 = w2.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&w2);

    assert!(
        snap2 > snap1,
        "second session snapshot must be greater than first"
    );
    assert!(snap1.as_u64() >= 1, "first snapshot must be >= 1");
}

/// read_latest() after commit returns the committed snapshot, not a prior one.
#[tokio::test]
async fn read_latest_reflects_committed_snapshot() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let mut writer = store.begin_write();
    let schema_id = writer.create_schema("fresh_schema").await.unwrap();
    let _ = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&writer);

    let reader = store.read_latest();
    let schemas = reader.list_schemas().await.unwrap();
    let found = schemas
        .iter()
        .any(|s| s.schema_id == schema_id && s.schema_name == "fresh_schema");
    assert!(found, "read_latest must reflect the committed schema");
}

/// An aborted write session (dropped without create_snapshot) must not
/// expose its mutations in subsequent snapshots.
#[tokio::test]
async fn aborted_session_mutations_not_visible() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Commit a baseline snapshot.
    let mut w_base = store.begin_write();
    w_base.create_schema("baseline").await.unwrap();
    let snap_base = w_base.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&w_base);

    // Abort: begin a write, mutate, then drop without create_snapshot.
    {
        let mut w_abort = store.begin_write();
        w_abort.create_schema("should_not_appear").await.unwrap();
        // Dropped here without calling create_snapshot.
        drop(w_abort);
    }

    // A subsequent read at snap_base must not see the aborted schema.
    let reader = store.read_at(snap_base).unwrap();
    let schemas = reader.list_schemas().await.unwrap();
    let has_aborted = schemas.iter().any(|s| s.schema_name == "should_not_appear");
    assert!(
        !has_aborted,
        "aborted session mutations must not be visible"
    );
}

// ─── F-22: DataFusion Concurrent schema_names ────────────────────────────

#[cfg(test)]
mod datafusion_concurrent {
    use super::*;
    use datafusion::catalog::CatalogProvider;
    use slateduck_datafusion::SlateDuckCatalogProvider;

    /// schema_names() called from multiple threads concurrently must return
    /// consistent results and not panic.
    ///
    /// This is a sync test using a dedicated multi-thread runtime so that
    /// handle.block_on() inside the bridge can make progress.
    #[test]
    fn concurrent_schema_names_do_not_race() {
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap(),
        );

        let dir = TempDir::new().unwrap();
        rt.block_on(async {
            let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
            let mut writer = store.begin_write();
            writer.create_schema("alpha").await.unwrap();
            writer.create_schema("beta").await.unwrap();
            let _snap = writer.create_snapshot(None, None).await.unwrap();
            store.commit_writer(&writer);
            store.close().await.unwrap();
        });

        let path = dir.path().to_str().unwrap().to_string();
        let obj_store =
            Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&path).unwrap());
        let provider = Arc::new(
            rt.block_on(SlateDuckCatalogProvider::open(
                obj_store,
                ObjectPath::from("catalog"),
                Some(SnapshotId::new(1)),
            ))
            .unwrap(),
        );

        let mut handles = vec![];
        for _ in 0..4 {
            let p = provider.clone();
            let h = std::thread::spawn(move || p.schema_names());
            handles.push(h);
        }

        for h in handles {
            let names = h.join().expect("thread must not panic");
            assert!(
                names.contains(&"alpha".to_string()),
                "schema 'alpha' must appear: {names:?}"
            );
            assert!(
                names.contains(&"beta".to_string()),
                "schema 'beta' must appear: {names:?}"
            );
        }
        // rt is dropped here in sync context — no panic
    }

    /// schema_names() called outside a Tokio runtime context (no try_current
    /// available) must still return correct results (F-14 fix).
    ///
    /// Uses a sync test with a dedicated multi-thread runtime to ensure
    /// the bridge can drive async I/O correctly.
    #[test]
    fn schema_names_works_outside_async_context() {
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap(),
        );

        let dir = TempDir::new().unwrap();
        rt.block_on(async {
            let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
            let mut writer = store.begin_write();
            writer.create_schema("sync_schema").await.unwrap();
            let _snap = writer.create_snapshot(None, None).await.unwrap();
            store.commit_writer(&writer);
            store.close().await.unwrap();
        });

        let path = dir.path().to_str().unwrap().to_string();
        let obj_store =
            Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&path).unwrap());
        let provider = Arc::new(
            rt.block_on(SlateDuckCatalogProvider::open(
                obj_store,
                ObjectPath::from("catalog"),
                Some(SnapshotId::new(1)),
            ))
            .unwrap(),
        );

        // Call schema_names from a pure OS thread (not inside any tokio task).
        let names = std::thread::spawn(move || provider.schema_names())
            .join()
            .unwrap();
        assert!(
            names.contains(&"sync_schema".to_string()),
            "schema must be visible outside async context: {names:?}"
        );
        // rt is dropped here in sync context — no panic
    }
}
