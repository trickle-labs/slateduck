//! v0.47.0 Integration Tests — Read-Only Catalog Access
//!
//! Tests for RFC-01 roadmap items:
//! - 16 simultaneous ReadOnlyCatalog opens produce zero CAS conflicts
//! - ReadOnlyCatalog::refresh() advances to the latest committed snapshot
//! - Concurrent writer + N readers do not interfere
//! - open_without_epoch skips the CAS writer-epoch

use object_store::path::Path as ObjectPath;
use rocklake_catalog::{CatalogStore, OpenOptions, ReadOnlyCatalog};
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

// ─── RFC-01: ReadOnlyCatalog — zero CAS conflicts ─────────────────────────

/// 16 simultaneous ReadOnlyCatalog opens must produce exactly zero
/// writer-epoch CAS transaction conflicts and must all succeed.
#[tokio::test]
async fn test_16_simultaneous_readonly_opens_zero_conflicts() {
    let dir = TempDir::new().unwrap();

    // Bootstrap a writer so the catalog has some initial state.
    {
        let mut w = CatalogStore::open(test_opts(&dir)).await.unwrap();
        let mut writer = w.begin_write();
        writer.create_schema("analytics").await.unwrap();
        let result = writer.create_snapshot(None, None).await.unwrap();
        w.commit_writer(result);
        w.close().await.unwrap();
    }

    // Open 16 read-only handles concurrently.
    let dir_path = dir.path().to_str().unwrap().to_string();
    let mut handles = Vec::with_capacity(16);
    for _ in 0..16 {
        let dp = dir_path.clone();
        handles.push(tokio::spawn(async move {
            let store =
                Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&dp).unwrap());
            let opts = OpenOptions {
                object_store: store,
                path: ObjectPath::from("catalog"),
                encryption: None,
            };
            ReadOnlyCatalog::open(opts).await
        }));
    }

    let mut success_count = 0usize;
    for h in handles {
        let cat = h
            .await
            .expect("task panicked")
            .expect("open_readonly failed");
        // Each reader must see the schema we committed.
        let reader = cat.reader().expect("reader() failed");
        let schemas = reader.list_schemas().await.expect("list_schemas failed");
        assert!(
            schemas.iter().any(|s| s.schema_name == "analytics"),
            "reader must see the committed schema"
        );
        cat.close().await.expect("close failed");
        success_count += 1;
    }
    assert_eq!(success_count, 16, "all 16 readers must succeed");
}

/// ReadOnlyCatalog::refresh() advances to the latest committed snapshot.
#[tokio::test]
async fn test_readonly_refresh_sees_new_snapshots() {
    let dir = TempDir::new().unwrap();

    // Open a read-only handle first (empty catalog).
    let mut cat = ReadOnlyCatalog::open(test_opts(&dir)).await.unwrap();
    let initial_snap = cat.current_snapshot_id();

    // Now write a snapshot via the writer path.
    let mut w = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = w.begin_write();
    writer.create_schema("new_schema").await.unwrap();
    let result = writer.create_snapshot(None, None).await.unwrap();
    w.commit_writer(result);
    w.close().await.unwrap();

    // Before refresh the reader still sees the old snapshot.
    assert_eq!(
        cat.current_snapshot_id(),
        initial_snap,
        "snapshot should not change without refresh"
    );

    // After refresh the reader must see the new snapshot.
    let refreshed = cat.refresh().await.expect("refresh failed");
    assert!(
        refreshed.as_u64() > initial_snap.as_u64(),
        "refreshed snapshot ({}) must be newer than initial ({})",
        refreshed.as_u64(),
        initial_snap.as_u64()
    );

    let reader = cat.reader().expect("reader() after refresh failed");
    let schemas = reader.list_schemas().await.expect("list_schemas failed");
    assert!(
        schemas.iter().any(|s| s.schema_name == "new_schema"),
        "reader must see the newly committed schema after refresh"
    );

    cat.close().await.unwrap();
}

/// open_without_epoch should not increment the writer epoch counter.
#[tokio::test]
async fn test_open_without_epoch_does_not_increment_epoch() {
    use rocklake_core::keys;
    use rocklake_core::tags::SYSTEM_WRITER_EPOCH;
    use rocklake_core::values;

    let dir = TempDir::new().unwrap();

    // First open via open() — this sets epoch to 1.
    let w1 = CatalogStore::open(test_opts(&dir)).await.unwrap();
    drop(w1);

    // Record the epoch after the writer open.
    let epoch_after_writer = {
        let db = slatedb::Db::open(
            ObjectPath::from("catalog"),
            Arc::new(object_store::local::LocalFileSystem::new_with_prefix(dir.path()).unwrap()),
        )
        .await
        .unwrap();
        let key = keys::key_system(SYSTEM_WRITER_EPOCH);
        let val = db
            .get(&key)
            .await
            .unwrap()
            .map(|d| values::decode_counter(&d).unwrap())
            .unwrap_or(0);
        db.close().await.unwrap();
        val
    };

    // Now open three readers via open_without_epoch — epoch must not change.
    for _ in 0..3 {
        let r = CatalogStore::open_without_epoch(test_opts(&dir))
            .await
            .unwrap();
        drop(r);
    }

    let epoch_after_readers = {
        let db = slatedb::Db::open(
            ObjectPath::from("catalog"),
            Arc::new(object_store::local::LocalFileSystem::new_with_prefix(dir.path()).unwrap()),
        )
        .await
        .unwrap();
        let key = keys::key_system(SYSTEM_WRITER_EPOCH);
        let val = db
            .get(&key)
            .await
            .unwrap()
            .map(|d| values::decode_counter(&d).unwrap())
            .unwrap_or(0);
        db.close().await.unwrap();
        val
    };

    assert_eq!(
        epoch_after_writer, epoch_after_readers,
        "open_without_epoch must not increment the writer epoch"
    );
}

/// Writer and concurrent readers do not interfere with each other.
#[tokio::test]
async fn test_concurrent_writer_and_readers() {
    let dir = TempDir::new().unwrap();

    // Bootstrap the catalog.
    {
        let mut w = CatalogStore::open(test_opts(&dir)).await.unwrap();
        let mut writer = w.begin_write();
        writer.create_schema("base").await.unwrap();
        let result = writer.create_snapshot(None, None).await.unwrap();
        w.commit_writer(result);
        w.close().await.unwrap();
    }

    let dir_path = dir.path().to_str().unwrap().to_string();

    // Spawn 8 reader tasks and 1 writer task, all running concurrently.
    let writer_task = {
        let dp = dir_path.clone();
        tokio::spawn(async move {
            let store =
                Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&dp).unwrap());
            let opts = OpenOptions {
                object_store: store,
                path: ObjectPath::from("catalog"),
                encryption: None,
            };
            let mut w = CatalogStore::open(opts).await.unwrap();
            for i in 0..5u64 {
                let mut writer = w.begin_write();
                writer.create_schema(&format!("schema_{i}")).await.unwrap();
                let result = writer.create_snapshot(None, None).await.unwrap();
                w.commit_writer(result);
            }
            w.close().await.unwrap();
        })
    };

    let mut reader_tasks = Vec::with_capacity(8);
    for _ in 0..8 {
        let dp = dir_path.clone();
        reader_tasks.push(tokio::spawn(async move {
            let store =
                Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&dp).unwrap());
            let opts = OpenOptions {
                object_store: store,
                path: ObjectPath::from("catalog"),
                encryption: None,
            };
            let cat = ReadOnlyCatalog::open(opts).await.unwrap();
            // Refresh a few times to pick up writer changes.
            let mut refreshed = cat;
            for _ in 0..3 {
                let _ = refreshed.refresh().await;
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            let reader = refreshed.reader().unwrap();
            let schemas = reader.list_schemas().await.unwrap();
            assert!(
                !schemas.is_empty(),
                "must always see at least the base schema"
            );
            refreshed.close().await.unwrap();
        }));
    }

    writer_task.await.expect("writer task panicked");
    for rt in reader_tasks {
        rt.await.expect("reader task panicked");
    }
}
