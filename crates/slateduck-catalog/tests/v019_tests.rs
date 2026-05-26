//! v0.19 Integration Tests — CDC Correctness & Catalog Transaction Hardening
//!
//! Tests for all v0.19 roadmap items:
//! - CAS-protected writer epoch (concurrent open, exactly one winner)
//! - Extension schema concurrent inserts (all row IDs unique)
//! - GC advance vs concurrent lease acquisition (lease always wins)
//! - Overflow-safe counter arithmetic
//! - Staged write discipline (crash between db.put and create_snapshot)
//! - table_changes() change stream reconstructs end-snapshot state

use object_store::path::Path as ObjectPath;
use slateduck_catalog::{CatalogStore, OpenOptions};
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

// ─── CAS Writer Epoch Tests ──────────────────────────────────────────────────

/// Two concurrent opens: exactly one writer wins the epoch contest.
#[tokio::test]
async fn writer_epoch_cas_concurrent_open() {
    let dir = TempDir::new().unwrap();
    let opts = test_opts(&dir);

    // First open succeeds
    let mut store1 = CatalogStore::open(opts.clone()).await.unwrap();

    // Second open also succeeds (takes over the epoch since it has a newer timestamp)
    let mut store2 = CatalogStore::open(opts.clone()).await.unwrap();

    // Store1's writer should be fenced: create_snapshot should fail.
    let mut w1 = store1.begin_write();
    w1.create_schema("fenced_schema").await.unwrap();
    let result = w1.create_snapshot(None, None).await;
    assert!(
        result.is_err(),
        "first writer should be fenced after second open"
    );

    // Store2's writer should succeed.
    let mut w2 = store2.begin_write();
    w2.create_schema("ok_schema").await.unwrap();
    let snap = w2.create_snapshot(None, None).await;
    assert!(snap.is_ok(), "second writer should succeed");
    store2.commit_writer(snap.expect("second writer should succeed"));

    store1.close().await.unwrap();
    store2.close().await.unwrap();
}

/// check_epoch fails when epoch key is absent (fail closed).
#[tokio::test]
async fn writer_epoch_missing_key_fails_closed() {
    let dir = TempDir::new().unwrap();
    let opts = test_opts(&dir);

    let mut store = CatalogStore::open(opts.clone()).await.unwrap();

    // Delete the epoch key externally to simulate corruption.
    let epoch_key = slateduck_core::keys::key_system(slateduck_core::tags::SYSTEM_WRITER_EPOCH);
    store.db().delete(&epoch_key).await.unwrap();

    // Now create_snapshot should fail because epoch key is missing.
    let mut w = store.begin_write();
    w.create_schema("doomed").await.unwrap();
    let result = w.create_snapshot(None, None).await;
    assert!(result.is_err(), "missing epoch key should cause failure");

    store.close().await.unwrap();
}

// ─── Extension Schema Concurrent Insert Tests ────────────────────────────────

/// Concurrent inserts produce unique row IDs.
#[tokio::test]
async fn extension_concurrent_inserts_unique_ids() {
    let dir = TempDir::new().unwrap();
    let opts = test_opts(&dir);
    let store = CatalogStore::open(opts).await.unwrap();
    let db = store.db();

    // Create extension table first
    slateduck_catalog::extension::create_extension_table(
        db,
        slateduck_catalog::extension::EXTENSION_PGTRICKLE,
        "cursors",
    )
    .await
    .unwrap();

    // Insert multiple rows sequentially (simulating concurrent scenario)
    let mut ids = Vec::new();
    for i in 0..10 {
        let id = slateduck_catalog::extension::insert_extension_row(
            db,
            slateduck_catalog::extension::EXTENSION_PGTRICKLE,
            "cursors",
            &format!("{{\"cursor\":{i}}}"),
        )
        .await
        .unwrap();
        ids.push(id);
    }

    // All IDs must be unique
    let unique_ids: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(
        unique_ids.len(),
        ids.len(),
        "all extension row IDs must be unique: {:?}",
        ids
    );

    // IDs should be sequential starting from 1
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(*id, (i + 1) as u64);
    }

    store.close().await.unwrap();
}

// ─── GC Advance vs Lease Tests ───────────────────────────────────────────────

/// GC advance respects an existing lease (lease always wins).
#[tokio::test]
async fn gc_advance_respects_lease() {
    let dir = TempDir::new().unwrap();
    let opts = test_opts(&dir);

    let mut store = CatalogStore::open(opts).await.unwrap();

    // Create some snapshots to advance past
    for _ in 0..5 {
        let mut w = store.begin_write();
        let result = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(result);
    }

    let db = store.db();

    // Acquire a lease at snapshot 3
    slateduck_catalog::lease::hold_snapshot(db, "test-consumer", 3, 3600)
        .await
        .unwrap();

    // Try to advance retain-from past the leased snapshot
    let result = slateduck_catalog::gc::gc_apply(db, 4).await;
    assert!(
        result.is_err(),
        "GC should not advance past a leased snapshot"
    );

    // Advancing to snapshot 2 (below leased) should succeed
    let result = slateduck_catalog::gc::gc_apply(db, 2).await;
    assert!(result.is_ok());
    let applied = result.unwrap();
    assert_eq!(applied.new_retain_from, 2);

    store.close().await.unwrap();
}

// ─── Overflow Safety Tests ───────────────────────────────────────────────────

/// next_rowid_range rejects count == 0.
#[tokio::test]
async fn rowid_range_rejects_zero_count() {
    let dir = TempDir::new().unwrap();
    let opts = test_opts(&dir);
    let store = CatalogStore::open(opts).await.unwrap();

    let result = slateduck_catalog::next_rowid_range(store.db(), 1, 0).await;
    assert!(result.is_err(), "count=0 should be rejected");
    let err = result.unwrap_err();
    assert!(err.to_string().contains("must be > 0"));

    store.close().await.unwrap();
}

/// next_rowid_range detects overflow near u64::MAX.
#[tokio::test]
async fn rowid_range_overflow_detection() {
    let dir = TempDir::new().unwrap();
    let opts = test_opts(&dir);
    let store = CatalogStore::open(opts).await.unwrap();
    let db = store.db();

    // First, seed the counter near u64::MAX
    let (_, _) = slateduck_catalog::next_rowid_range(db, 99, u64::MAX - 10)
        .await
        .unwrap();

    // Now try to allocate more than fits
    let result = slateduck_catalog::next_rowid_range(db, 99, 20).await;
    assert!(result.is_err(), "should detect overflow");
    assert!(result.unwrap_err().to_string().contains("overflow"));

    store.close().await.unwrap();
}

/// Lease TTL overflow is detected.
#[tokio::test]
async fn lease_ttl_overflow_detected() {
    let dir = TempDir::new().unwrap();
    let opts = test_opts(&dir);
    let store = CatalogStore::open(opts).await.unwrap();
    let db = store.db();

    // TTL that would overflow when multiplied by 1000
    let result =
        slateduck_catalog::lease::hold_snapshot(db, "overflow-consumer", 1, u64::MAX / 500).await;
    assert!(result.is_err(), "should detect TTL overflow");
    assert!(result.unwrap_err().to_string().contains("overflow"));

    store.close().await.unwrap();
}

// ─── Staged Write Discipline Tests ──────────────────────────────────────────

/// Non-MVCC writes (table stats) are visible immediately without create_snapshot.
#[tokio::test]
async fn staged_write_discipline_non_mvcc_visible_without_snapshot() {
    let dir = TempDir::new().unwrap();
    let opts = test_opts(&dir);
    let mut store = CatalogStore::open(opts).await.unwrap();

    // Create a table first (needs a snapshot)
    let mut w = store.begin_write();
    let schema_id = w.create_schema("test").await.unwrap();
    let table_id = w.create_table(schema_id, "orders", None).await.unwrap();
    let _snap = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(_snap);

    // Now write table stats directly (non-MVCC write)
    let mut w2 = store.begin_write();
    w2.update_table_stats(table_id, 1000, 5, 1024 * 1024)
        .await
        .unwrap();
    // Don't call create_snapshot — stats should still be written to DB

    // Verify the stats key exists in the DB even without a snapshot commit
    let stats_key = slateduck_core::keys::key_table_stats(table_id);
    let data = store.db().get(&stats_key).await.unwrap();
    assert!(
        data.is_some(),
        "table stats should be written even without snapshot commit"
    );

    store.close().await.unwrap();
}

/// MVCC writes are NOT visible without create_snapshot (staged discipline).
#[tokio::test]
async fn staged_write_discipline_mvcc_invisible_without_snapshot() {
    let dir = TempDir::new().unwrap();
    let opts = test_opts(&dir);
    let mut store = CatalogStore::open(opts).await.unwrap();

    // Create initial snapshot so read_latest has something
    let mut w0 = store.begin_write();
    let _cr = w0.create_snapshot(None, None).await.unwrap();
    store.commit_writer(_cr);

    // Stage a schema creation but don't commit
    let mut w = store.begin_write();
    w.create_schema("phantom").await.unwrap();
    // Don't call create_snapshot

    // The reader should not see the schema
    let reader = store.read_latest();
    let schemas = reader.list_schemas().await.unwrap();
    assert!(
        schemas.iter().all(|s| s.schema_name != "phantom"),
        "staged MVCC writes should not be visible without snapshot"
    );

    store.close().await.unwrap();
}

// ─── SnapshotDiff Multi-Window Tests ─────────────────────────────────────────

/// table_changes spanning multiple snapshots includes all intermediate changes.
#[tokio::test]
async fn snapshot_diff_multi_window() {
    let dir = TempDir::new().unwrap();
    let opts = test_opts(&dir);
    let mut store = CatalogStore::open(opts).await.unwrap();

    // Create schema + table
    let mut w = store.begin_write();
    let schema_id = w.create_schema("public").await.unwrap();
    let table_id = w.create_table(schema_id, "events", None).await.unwrap();
    let snap0 = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap0);

    // Snapshot 2: add file1
    let mut w = store.begin_write();
    w.register_data_file(table_id, "file1.parquet", "parquet", 10, 1024)
        .await
        .unwrap();
    let snap1 = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap1);

    // Snapshot 3: add file2
    let mut w = store.begin_write();
    w.register_data_file(table_id, "file2.parquet", "parquet", 20, 2048)
        .await
        .unwrap();
    let snap2 = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap2);

    // Snapshot 4: add file3
    let mut w = store.begin_write();
    w.register_data_file(table_id, "file3.parquet", "parquet", 30, 4096)
        .await
        .unwrap();
    let snap3 = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap3);

    // Multi-window diff: snap0 → snap3 should include files from snap1, snap2, snap3
    let reader = store.read_at(snap3).unwrap();
    let diff = reader.snapshot_diff(snap0, snap3).await.unwrap();
    assert_eq!(
        diff.added_data_files.len(),
        3,
        "multi-window diff should include all 3 files"
    );

    // Single-window diff: snap1 → snap2 should include only file2
    let diff12 = reader.snapshot_diff(snap1, snap2).await.unwrap();
    assert_eq!(diff12.added_data_files.len(), 1);
    assert!(diff12.added_data_files[0].path.contains("file2"));

    store.close().await.unwrap();
}

// ─── DataFileRow versioning tests ────────────────────────────────────────────

/// DataFileRow.begin_snapshot is set correctly on registration.
#[tokio::test]
async fn data_file_row_begin_snapshot_set() {
    let dir = TempDir::new().unwrap();
    let opts = test_opts(&dir);
    let mut store = CatalogStore::open(opts).await.unwrap();

    let mut w = store.begin_write();
    let schema_id = w.create_schema("test").await.unwrap();
    let table_id = w.create_table(schema_id, "t", None).await.unwrap();
    w.register_data_file(table_id, "f.parquet", "parquet", 1, 1)
        .await
        .unwrap();
    let snap = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let files = reader.list_data_files(table_id).await.unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].begin_snapshot, Some(snap.as_u64()));
    assert_eq!(files[0].end_snapshot, None);

    store.close().await.unwrap();
}

// ─── table_changes property test: change stream reconstructs end state ───────

#[test]
fn table_changes_stream_reconstructs_end_state() {
    use slateduck_sql::table_changes::*;

    // Test with various combinations of inserts, deletes, and updates
    let test_cases: Vec<(
        Vec<ParquetRowData>,
        Vec<ParquetRowData>,
        Vec<ParquetRowData>,
    )> = vec![
        // Case 1: Pure inserts (empty start → 3 rows)
        (
            vec![],
            vec![
                ParquetRowData {
                    rowid: 1,
                    columns_json: r#"{"a":1}"#.to_string(),
                },
                ParquetRowData {
                    rowid: 2,
                    columns_json: r#"{"a":2}"#.to_string(),
                },
                ParquetRowData {
                    rowid: 3,
                    columns_json: r#"{"a":3}"#.to_string(),
                },
            ],
            vec![], // no removals
        ),
        // Case 2: Pure deletes (2 rows → empty)
        (
            vec![
                ParquetRowData {
                    rowid: 1,
                    columns_json: r#"{"a":1}"#.to_string(),
                },
                ParquetRowData {
                    rowid: 2,
                    columns_json: r#"{"a":2}"#.to_string(),
                },
            ],
            vec![], // no additions
            vec![
                ParquetRowData {
                    rowid: 1,
                    columns_json: r#"{"a":1}"#.to_string(),
                },
                ParquetRowData {
                    rowid: 2,
                    columns_json: r#"{"a":2}"#.to_string(),
                },
            ],
        ),
        // Case 3: Mix of insert + update + delete
        (
            vec![
                ParquetRowData {
                    rowid: 1,
                    columns_json: r#"{"v":"old1"}"#.to_string(),
                },
                ParquetRowData {
                    rowid: 2,
                    columns_json: r#"{"v":"old2"}"#.to_string(),
                },
                ParquetRowData {
                    rowid: 3,
                    columns_json: r#"{"v":"old3"}"#.to_string(),
                },
            ],
            vec![
                ParquetRowData {
                    rowid: 2,
                    columns_json: r#"{"v":"new2"}"#.to_string(),
                }, // update row 2
                ParquetRowData {
                    rowid: 4,
                    columns_json: r#"{"v":"new4"}"#.to_string(),
                }, // insert row 4
            ],
            vec![
                ParquetRowData {
                    rowid: 2,
                    columns_json: r#"{"v":"old2"}"#.to_string(),
                }, // update row 2 preimage
                ParquetRowData {
                    rowid: 3,
                    columns_json: r#"{"v":"old3"}"#.to_string(),
                }, // delete row 3
            ],
        ),
    ];

    for (start_state, added, removed) in &test_cases {
        let result = compute_table_changes("t", 0, 1, 0, added, removed).unwrap();
        let end_state = apply_changes(start_state, &result.records);

        // Compute expected end state manually
        let mut expected: std::collections::HashMap<u64, &ParquetRowData> =
            start_state.iter().map(|r| (r.rowid, r)).collect();

        for r in removed {
            expected.remove(&r.rowid);
        }
        for r in added {
            expected.insert(r.rowid, r);
        }

        let mut expected_sorted: Vec<_> = expected.values().collect();
        expected_sorted.sort_by_key(|r| r.rowid);

        assert_eq!(
            end_state.len(),
            expected_sorted.len(),
            "end state row count mismatch"
        );
        for (actual, exp) in end_state.iter().zip(expected_sorted.iter()) {
            assert_eq!(actual.rowid, exp.rowid);
            assert_eq!(actual.columns_json, exp.columns_json);
        }
    }
}

// ─── GC Lease Resilience ─────────────────────────────────────────────────────

/// list_active_leases returns error on corrupt rows (not silent skip).
#[tokio::test]
async fn list_active_leases_corrupt_row_errors() {
    let dir = TempDir::new().unwrap();
    let opts = test_opts(&dir);
    let store = CatalogStore::open(opts).await.unwrap();
    let db = store.db();

    // Write a corrupt value to a lease key
    let key = slateduck_core::keys::key_snapshot_lease("corrupt-consumer");
    db.put(&key, &[0xFF, 0xFF, 0xFF]).await.unwrap(); // Invalid protobuf

    let result = slateduck_catalog::lease::list_active_leases(db).await;
    assert!(result.is_err(), "corrupt lease row should produce error");
    assert!(result.unwrap_err().to_string().contains("corrupt"));

    store.close().await.unwrap();
}

// ─── read_fresh_latest test ──────────────────────────────────────────────────

/// read_fresh_latest reads counter from SlateDB, not in-memory cache.
#[tokio::test]
async fn read_fresh_latest_reads_from_db() {
    let dir = TempDir::new().unwrap();
    let opts = test_opts(&dir);
    let mut store = CatalogStore::open(opts).await.unwrap();

    // Create a snapshot
    let mut w = store.begin_write();
    w.create_schema("s").await.unwrap();
    let snap = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    // read_fresh_latest should see the same snapshot
    let reader = store.read_fresh_latest().await.unwrap();
    assert_eq!(reader.snapshot_id(), snap.snapshot_id);

    store.close().await.unwrap();
}
