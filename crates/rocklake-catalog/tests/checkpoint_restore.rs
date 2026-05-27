//! Checkpoint restore snapshot-ID safety tests (F-07, v0.27.3).
//!
//! Verifies that `restore_checkpoint()` correctly advances the snapshot counter
//! so that no previously-issued snapshot ID is ever re-allocated after a restore.
//!
//! # Scenario
//!
//! 1. Write N snapshots (IDs 1..=N).
//! 2. Create a checkpoint.
//! 3. Restore the checkpoint.
//! 4. Re-open the catalog.
//! 5. Assert the first new snapshot ID > N.
//! 6. Assert none of 1..=N appear again in subsequent allocations.

use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use rocklake_catalog::{CatalogStore, OpenOptions};
use std::sync::Arc;
use tempfile::TempDir;

fn test_opts(dir: &TempDir) -> OpenOptions {
    let store = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    OpenOptions {
        object_store: store,
        path: ObjectPath::from("catalog"),
        encryption: None,
    }
}

async fn open_raw_db(dir: &TempDir) -> slatedb::Db {
    let store = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    slatedb::Db::open(ObjectPath::from("catalog"), store)
        .await
        .unwrap()
}

/// Write `n` snapshots, creating one schema per snapshot.
async fn write_n_snapshots(store: &mut CatalogStore, n: u32) -> Vec<u64> {
    let mut ids = Vec::with_capacity(n as usize);
    let mut writer = store.begin_write();
    for i in 0..n {
        writer.create_schema(&format!("schema_{i}")).await.unwrap();
        let commit = writer.create_snapshot(Some("setup"), None).await.unwrap();
        ids.push(commit.snapshot_id.as_u64());
        store.commit_writer(commit);
        writer = store.begin_write();
    }
    ids
}

/// The next allocated snapshot ID must be strictly greater than all IDs
/// written before the checkpoint.
#[tokio::test]
async fn next_snapshot_id_after_restore_is_fresh() {
    let dir = TempDir::new().unwrap();

    // Phase 1: write 5 snapshots.
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let pre_ids = write_n_snapshots(&mut store, 5).await;
    let highest_pre = *pre_ids.iter().max().unwrap();
    assert_eq!(highest_pre, 5, "expected snapshot IDs 1..=5");
    store.close().await.unwrap();

    // Phase 2: create a checkpoint.
    let db = open_raw_db(&dir).await;
    let cp = rocklake_catalog::checkpoint::create_checkpoint(&db, Some("test"))
        .await
        .unwrap();
    assert_eq!(cp.snapshot_id, 5);
    db.close().await.unwrap();

    // Phase 3: restore the checkpoint.
    let db = open_raw_db(&dir).await;
    rocklake_catalog::checkpoint::restore_checkpoint(&db, cp.id)
        .await
        .unwrap();
    db.close().await.unwrap();

    // Phase 4: re-open the catalog.
    let mut store2 = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Phase 5: write a new snapshot and verify the ID is fresh (> 5).
    let mut writer2 = store2.begin_write();
    writer2.create_schema("post_restore").await.unwrap();
    let new_commit = writer2
        .create_snapshot(Some("after-restore"), None)
        .await
        .unwrap();
    let new_id = new_commit.snapshot_id.as_u64();
    store2.commit_writer(new_commit);

    assert!(
        new_id > highest_pre,
        "post-restore snapshot ID {new_id} must be > highest pre-restore ID {highest_pre}"
    );

    // Phase 6: verify none of the pre-restore IDs are reused.
    for &old_id in &pre_ids {
        assert_ne!(
            new_id, old_id,
            "snapshot ID {new_id} must not reuse historical ID {old_id}"
        );
    }

    store2.close().await.unwrap();
}

/// Multiple writes after restore all get IDs beyond the pre-restore maximum.
#[tokio::test]
async fn multiple_writes_after_restore_stay_fresh() {
    let dir = TempDir::new().unwrap();

    // Write 3 snapshots.
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let pre_ids = write_n_snapshots(&mut store, 3).await;
    let highest_pre = *pre_ids.iter().max().unwrap();
    store.close().await.unwrap();

    // Create and restore checkpoint.
    let db = open_raw_db(&dir).await;
    let cp = rocklake_catalog::checkpoint::create_checkpoint(&db, None)
        .await
        .unwrap();
    db.close().await.unwrap();

    let db = open_raw_db(&dir).await;
    rocklake_catalog::checkpoint::restore_checkpoint(&db, cp.id)
        .await
        .unwrap();
    db.close().await.unwrap();

    // Write 3 more snapshots after restore.
    let mut store2 = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let post_ids = write_n_snapshots(&mut store2, 3).await;
    store2.close().await.unwrap();

    // All post-restore IDs must be strictly greater than all pre-restore IDs.
    for &post_id in &post_ids {
        assert!(
            post_id > highest_pre,
            "post-restore ID {post_id} must be > highest pre-restore ID {highest_pre}"
        );
    }

    // No post-restore ID should match any pre-restore ID.
    for &post_id in &post_ids {
        for &pre_id in &pre_ids {
            assert_ne!(
                post_id, pre_id,
                "ID {post_id} was reused from pre-restore history"
            );
        }
    }
}
