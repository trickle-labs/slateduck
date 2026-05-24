//! v0.9.1 Write Protocol Correctness — regression tests.
//!
//! Covers:
//! - F-01: Sequential `begin_write()` calls on one `CatalogStore` produce
//!   monotonically increasing IDs with no reuse.
//! - F-01: `read_latest()` reflects the just-committed snapshot after every
//!   commit.
//! - F-02: Dropping a `CatalogWriter` without calling `create_snapshot()`
//!   leaves no phantom rows visible in subsequent snapshots.
//! - F-04: `drop_table` and `drop_column` use the correct key when called
//!   through `find_table_schema_id` / `find_column_table_id`.
//! - F-30 (conformance): The writer protocol produces monotonically increasing
//!   IDs and no duplicate facts under simulated mid-write abandonment.

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
    }
}

// ─── F-01: Counter monotonicity ───────────────────────────────────────────────

/// Sequential `begin_write()` → `create_snapshot()` sessions must produce
/// strictly increasing snapshot IDs.  Before v0.9.1, `commit_writer` was
/// missing and the store's counter was never updated, causing the second
/// session to reuse snapshot_id = 1.
#[tokio::test]
async fn sequential_write_sessions_produce_monotonically_increasing_snapshot_ids() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Session 1
    let mut w1 = store.begin_write();
    let schema_id = w1.create_schema("s1").await.unwrap();
    let snap1 = w1.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&w1);

    // Session 2
    let mut w2 = store.begin_write();
    let _table_id = w2.create_table(schema_id, "t1", None).await.unwrap();
    let snap2 = w2.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&w2);

    // Session 3
    let mut w3 = store.begin_write();
    let _table_id2 = w3.create_table(schema_id, "t2", None).await.unwrap();
    let snap3 = w3.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&w3);

    assert!(
        snap1.as_u64() < snap2.as_u64(),
        "snap1={} must be < snap2={}",
        snap1.as_u64(),
        snap2.as_u64()
    );
    assert!(
        snap2.as_u64() < snap3.as_u64(),
        "snap2={} must be < snap3={}",
        snap2.as_u64(),
        snap3.as_u64()
    );

    store.close().await.unwrap();
}

/// `read_latest()` must return the snapshot that was just committed, not a
/// stale value from before the write session.
#[tokio::test]
async fn read_latest_reflects_post_commit_snapshot() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Commit session 1
    let mut w1 = store.begin_write();
    w1.create_schema("main").await.unwrap();
    let snap1 = w1.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&w1);

    let reader = store.read_latest();
    let latest_snap = reader.get_snapshot().await.unwrap();
    assert_eq!(
        latest_snap.map(|s| s.snapshot_id),
        Some(snap1.as_u64()),
        "read_latest() must return snapshot_id {} after commit",
        snap1.as_u64()
    );

    // Commit session 2
    let mut w2 = store.begin_write();
    w2.create_schema("analytics").await.unwrap();
    let snap2 = w2.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&w2);

    let reader2 = store.read_latest();
    let latest_snap2 = reader2.get_snapshot().await.unwrap();
    assert_eq!(
        latest_snap2.map(|s| s.snapshot_id),
        Some(snap2.as_u64()),
        "read_latest() must return snapshot_id {} after second commit",
        snap2.as_u64()
    );

    store.close().await.unwrap();
}

/// Catalog IDs allocated across sessions must be monotonically increasing.
/// Before v0.9.1, the store's catalog counter was never updated, so the
/// second session would reuse the same catalog_id values as the first.
#[tokio::test]
async fn sequential_write_sessions_no_catalog_id_reuse() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Session 1: allocate schema_id and table_id
    let mut w1 = store.begin_write();
    let schema_id1 = w1.create_schema("s1").await.unwrap();
    let table_id1 = w1.create_table(schema_id1, "t1", None).await.unwrap();
    w1.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&w1);

    // Session 2: allocate more IDs — they must be higher than session 1's
    let mut w2 = store.begin_write();
    let schema_id2 = w2.create_schema("s2").await.unwrap();
    let table_id2 = w2.create_table(schema_id2, "t2", None).await.unwrap();
    w2.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&w2);

    assert!(
        schema_id2 > schema_id1,
        "schema_id2={schema_id2} must be > schema_id1={schema_id1}"
    );
    assert!(
        table_id2 > table_id1,
        "table_id2={table_id2} must be > table_id1={table_id1}"
    );

    store.close().await.unwrap();
}

// ─── F-02: Atomic snapshot publication ────────────────────────────────────────

/// Dropping a `CatalogWriter` before calling `create_snapshot()` must not
/// leave any rows visible in subsequent snapshots (phantom-row test).
#[tokio::test]
async fn abandoned_writer_leaves_no_phantom_rows() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Commit a baseline snapshot
    let mut w_base = store.begin_write();
    let schema_id = w_base.create_schema("base").await.unwrap();
    let base_snap = w_base.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&w_base);

    // Start a writer that stages a table but never calls create_snapshot()
    {
        let mut w_abandoned = store.begin_write();
        w_abandoned
            .create_table(schema_id, "ghost_table", None)
            .await
            .unwrap();
        // w_abandoned is dropped here without create_snapshot()
    }

    // A new snapshot must not see the ghost table
    let mut w_final = store.begin_write();
    let final_snap = w_final.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&w_final);

    let reader = store.read_latest();
    let tables = reader.list_tables(schema_id).await.unwrap();
    assert!(
        tables.is_empty(),
        "No tables should be visible at snapshot {} — ghost_table must not appear (phantom row test). Found: {:?}",
        final_snap.as_u64(),
        tables.iter().map(|t| &t.table_name).collect::<Vec<_>>()
    );

    // Base snapshot (read_at) must still be readable and show no tables
    let reader_base = store.read_at(base_snap);
    let base_tables = reader_base.list_tables(schema_id).await.unwrap();
    assert!(
        base_tables.is_empty(),
        "Base snapshot must also have no tables"
    );

    store.close().await.unwrap();
}

/// All mutations staged in a single writer must be atomically visible in the
/// snapshot produced by `create_snapshot()` — no partial commits.
#[tokio::test]
async fn all_staged_mutations_visible_after_create_snapshot() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let mut writer = store.begin_write();
    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "events", None).await.unwrap();
    writer
        .add_column(table_id, "id", "BIGINT", 0, false, None)
        .await
        .unwrap();
    writer
        .add_column(table_id, "ts", "TIMESTAMP", 1, true, None)
        .await
        .unwrap();
    writer
        .add_column(table_id, "payload", "VARCHAR", 2, true, None)
        .await
        .unwrap();
    let snap = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&writer);

    let reader = store.read_at(snap);
    let schemas = reader.list_schemas().await.unwrap();
    assert_eq!(schemas.len(), 1, "schema must be visible");

    let tables = reader.list_tables(schema_id).await.unwrap();
    assert_eq!(tables.len(), 1, "table must be visible");

    let desc = reader.describe_table(table_id).await.unwrap();
    let (_, cols) = desc.unwrap();
    assert_eq!(cols.len(), 3, "all 3 columns must be visible atomically");

    store.close().await.unwrap();
}

// ─── F-04: UPDATE end_snapshot key resolution ─────────────────────────────────

/// `find_table_schema_id` must return the correct schema_id for a live table,
/// enabling `drop_table` to mark the right key.
#[tokio::test]
async fn find_table_schema_id_returns_correct_schema() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let mut writer = store.begin_write();
    let schema_id = writer.create_schema("data").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "orders", None)
        .await
        .unwrap();
    let snap = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&writer);

    // Verify find_table_schema_id returns the correct schema
    let reader_writer = store.begin_write();
    let found_schema = reader_writer
        .find_table_schema_id(table_id)
        .await
        .unwrap();
    assert_eq!(
        found_schema,
        Some(schema_id),
        "find_table_schema_id({table_id}) must return schema_id={schema_id}"
    );
    drop(reader_writer); // no commit needed

    // Now drop the table using the correct schema_id
    let mut w2 = store.begin_write();
    let resolved_schema = w2
        .find_table_schema_id(table_id)
        .await
        .unwrap()
        .unwrap();
    w2.drop_table(resolved_schema, table_id, snap.as_u64())
        .await
        .unwrap();
    let snap2 = w2.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&w2);

    // After drop, table must not be visible at the new snapshot
    let reader2 = store.read_at(snap2);
    let tables = reader2.list_tables(schema_id).await.unwrap();
    assert!(
        tables.is_empty(),
        "dropped table must not be visible at snap2"
    );

    // But it IS visible at the original snapshot (MVCC immutability)
    let reader_old = store.read_at(snap);
    let old_tables = reader_old.list_tables(schema_id).await.unwrap();
    assert_eq!(
        old_tables.len(),
        1,
        "table must still be visible at original snapshot"
    );

    store.close().await.unwrap();
}

/// `find_column_table_id` must return the correct table_id for a live column,
/// enabling `drop_column` to mark the right key.
#[tokio::test]
async fn find_column_table_id_returns_correct_table() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let mut writer = store.begin_write();
    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "users", None).await.unwrap();
    let column_id = writer
        .add_column(table_id, "email", "VARCHAR", 0, true, None)
        .await
        .unwrap();
    let snap = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&writer);

    // Verify find_column_table_id returns the correct table
    let lookup_writer = store.begin_write();
    let found_table = lookup_writer
        .find_column_table_id(column_id)
        .await
        .unwrap();
    assert_eq!(
        found_table,
        Some(table_id),
        "find_column_table_id({column_id}) must return table_id={table_id}"
    );
    drop(lookup_writer);

    // Drop the column using the correctly resolved table_id
    let mut w2 = store.begin_write();
    let resolved_table = w2
        .find_column_table_id(column_id)
        .await
        .unwrap()
        .unwrap();
    w2.drop_column(resolved_table, column_id, snap.as_u64())
        .await
        .unwrap();
    let snap2 = w2.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&w2);

    // After drop, column must not be visible at the new snapshot
    let reader2 = store.read_at(snap2);
    let desc = reader2.describe_table(table_id).await.unwrap();
    let (_, cols) = desc.unwrap();
    assert!(cols.is_empty(), "dropped column must not be visible");

    // But it IS visible at the original snapshot
    let reader_old = store.read_at(snap);
    let desc_old = reader_old.describe_table(table_id).await.unwrap();
    let (_, old_cols) = desc_old.unwrap();
    assert_eq!(old_cols.len(), 1, "column visible at original snapshot");

    store.close().await.unwrap();
}

// ─── F-30: Writer protocol conformance ────────────────────────────────────────

/// Conformance test: simulated mid-write failure (writer abandoned before
/// snapshot) followed by a normal write must produce no duplicate IDs and no
/// unpublished facts.
#[tokio::test]
async fn writer_protocol_conformance_no_duplicate_ids_under_simulated_failure() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Step 1: Successful write
    let mut w1 = store.begin_write();
    let schema_id = w1.create_schema("prod").await.unwrap();
    let _t1 = w1.create_table(schema_id, "t1", None).await.unwrap();
    let snap1 = w1.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&w1);

    // Step 2: Simulated failure — writer abandoned mid-write
    {
        let mut w_fail = store.begin_write();
        let _ghost = w_fail.create_table(schema_id, "ghost", None).await.unwrap();
        // w_fail dropped without create_snapshot() — simulates a crash
    }

    // Step 3: Normal write after simulated failure
    let mut w2 = store.begin_write();
    let t2 = w2.create_table(schema_id, "t2", None).await.unwrap();
    let snap2 = w2.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&w2);

    // Assertions:
    // 1. snap2 > snap1 (monotonically increasing)
    assert!(
        snap2.as_u64() > snap1.as_u64(),
        "snap2={} must be > snap1={}",
        snap2.as_u64(),
        snap1.as_u64()
    );

    // 2. t2 is visible at snap2
    let reader = store.read_at(snap2);
    let tables = reader.list_tables(schema_id).await.unwrap();
    assert_eq!(tables.len(), 2, "t1 and t2 must be visible at snap2");
    assert!(
        tables.iter().any(|t| t.table_id == t2),
        "t2 must be in the table list"
    );

    // 3. No ghost table
    assert!(
        !tables.iter().any(|t| t.table_name == "ghost"),
        "ghost table must not be visible after abandoned writer"
    );

    // 4. No table ID collisions
    let ids: std::collections::HashSet<u64> = tables.iter().map(|t| t.table_id).collect();
    assert_eq!(
        ids.len(),
        tables.len(),
        "all table IDs must be unique (no reuse)"
    );

    store.close().await.unwrap();
}

/// After a store is closed and reopened, counters must be loaded from SlateDB
/// (not from stale in-memory state), preventing ID reuse across process restarts.
#[tokio::test]
async fn reopen_catalog_loads_counters_from_persistent_storage() {
    let dir = TempDir::new().unwrap();
    let opts = test_opts(&dir);

    // First open: write two schemas
    let (schema_id1, schema_id2, snap1) = {
        let mut store = CatalogStore::open(opts.clone()).await.unwrap();
        let mut w = store.begin_write();
        let s1 = w.create_schema("alpha").await.unwrap();
        let s2 = w.create_schema("beta").await.unwrap();
        let snap = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(&w);
        store.close().await.unwrap();
        (s1, s2, snap)
    };

    // Second open: IDs must be higher than those allocated in the first open
    let mut store2 = CatalogStore::open(opts).await.unwrap();
    let mut w2 = store2.begin_write();
    let schema_id3 = w2.create_schema("gamma").await.unwrap();
    let snap2 = w2.create_snapshot(None, None).await.unwrap();
    store2.commit_writer(&w2);

    assert!(
        schema_id3 > schema_id2,
        "schema_id3={schema_id3} after reopen must be > schema_id2={schema_id2}"
    );
    assert!(
        snap2.as_u64() > snap1.as_u64(),
        "snap2={} after reopen must be > snap1={}",
        snap2.as_u64(),
        snap1.as_u64()
    );

    // All three schemas visible at latest
    let reader = store2.read_latest();
    let schemas = reader.list_schemas().await.unwrap();
    assert_eq!(
        schemas.len(),
        3,
        "all three schemas must be visible after reopen"
    );
    let ids: Vec<u64> = schemas.iter().map(|s| s.schema_id).collect();
    let unique: std::collections::HashSet<u64> = ids.iter().cloned().collect();
    assert_eq!(ids.len(), unique.len(), "no schema_id reuse after reopen");
    let _ = schema_id1; // used above for ordering check

    store2.close().await.unwrap();
}
