//! v0.27.8 atomicity, snapshot-changes, and type-aware stats tests.
//!
//! Covers all tasks from the v0.27.8 roadmap:
//!   § Transaction Atomicity
//!     1. disconnect_mid_batch_leaves_catalog_unchanged
//!     2. concurrent_snapshot_ids_only_winner_visible
//!     3. writer_fencing_no_partial_artifacts
//!   § Spec-Complete Snapshot Changes
//!     4. snapshot_changes_spec_fields_after_workload
//!     5. snapshot_changes_author_not_in_snapshot_row
//!     6. losing_writer_snapshot_not_in_snapshot_changes
//!   § Type-Aware Column Stats
//!     7. stats_merge_handles_unsigned_integer_correctly
//!     8. stats_merge_handles_decimal_correctly
//!     9. stats_merge_handles_date_correctly
//!    10. stats_merge_handles_timestamp_correctly
//!    11. stats_merge_handles_uuid_correctly
//!    12. stats_merge_handles_boolean_correctly

use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use rocklake_catalog::{CatalogError, CatalogStore, OpenOptions};
use rocklake_core::mvcc::SnapshotId;
use std::sync::Arc;
use tempfile::TempDir;

fn test_opts(dir: &TempDir) -> OpenOptions {
    let path = dir.path().to_str().unwrap().to_string();
    let store = Arc::new(LocalFileSystem::new_with_prefix(&path).unwrap());
    OpenOptions {
        object_store: store,
        path: ObjectPath::from("catalog"),
        encryption: None,
    }
}

// ─── § Transaction Atomicity ────────────────────────────────────────────────

/// Dropping a writer without calling `create_snapshot()` must leave the catalog
/// in its pre-batch state.  No staged writes must reach SlateDB.
#[tokio::test]
async fn disconnect_mid_batch_leaves_catalog_unchanged() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Initial committed state: one schema.
    let snap0 = {
        let mut w = store.begin_write();
        w.create_schema("committed_schema").await.unwrap();
        let cr = w.create_snapshot(Some("setup"), None).await.unwrap();
        store.commit_writer(cr);
        cr.snapshot_id.as_u64()
    };

    // Begin a second batch that creates another schema and a table — but do NOT
    // call create_snapshot().  Dropping the writer simulates a mid-batch disconnect.
    {
        let mut w = store.begin_write();
        w.create_schema("uncommitted_schema").await.unwrap();
        w.create_table(1, "lost_table", None).await.unwrap();
        // `w` is dropped here — no commit.
    }

    // The catalog must still be at the committed state.
    let reader = store.read_at(SnapshotId::new(snap0)).unwrap();

    // Committed schema is visible.
    let schemas = reader.list_schemas().await.unwrap();
    assert_eq!(schemas.len(), 1, "only committed schema should be visible");
    assert_eq!(schemas[0].schema_name, "committed_schema");

    // Uncommitted schema must not be visible.
    assert!(
        schemas
            .iter()
            .all(|s| s.schema_name != "uncommitted_schema"),
        "uncommitted schema must not be visible"
    );
}

/// Only the snapshot-changes row of the winning writer must be visible; the
/// losing (fenced) writer must leave no artifacts in ducklake_snapshot_changes.
#[tokio::test]
async fn concurrent_snapshot_ids_only_winner_visible() {
    let dir = TempDir::new().unwrap();

    // Writer A opens first.
    let mut store_a = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer_a = store_a.begin_write();
    writer_a.create_schema("schema_a").await.unwrap();
    writer_a
        .add_snapshot_changes(
            "created_schema".to_string(),
            Some("schema_a".to_string()),
            None,
            None,
        )
        .await
        .unwrap();

    // Ensure clock advances so writer B gets a strictly newer epoch.
    tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;

    // Writer B opens second — it now owns the epoch.
    let mut store_b = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer_b = store_b.begin_write();
    writer_b.create_schema("schema_b").await.unwrap();
    writer_b
        .add_snapshot_changes(
            "created_schema".to_string(),
            Some("schema_b".to_string()),
            None,
            None,
        )
        .await
        .unwrap();

    // Both attempt to commit.  B holds the winning epoch; A is stale.
    let result_b = writer_b
        .create_snapshot(Some("winner"), Some("b wins"))
        .await;
    let result_a = writer_a
        .create_snapshot(Some("loser"), Some("a loses"))
        .await;

    assert!(
        result_b.is_ok(),
        "writer B (latest epoch) must commit; got: {result_b:?}"
    );
    let is_fenced = matches!(
        &result_a,
        Err(CatalogError::WriterEpochMismatch) | Err(CatalogError::TransactionConflict(_))
    );
    assert!(is_fenced, "writer A must be fenced; got: {result_a:?}");

    let commit_b = result_b.unwrap();
    store_b.commit_writer(commit_b);

    // After the race, only writer B's snapshot_changes row must be present.
    let reader = store_b
        .read_at(SnapshotId::new(commit_b.snapshot_id.as_u64()))
        .unwrap();
    let changes = reader.list_all_snapshot_changes().await.unwrap();

    // Exactly one snapshot_changes row — for the winning writer.
    assert_eq!(
        changes.len(),
        1,
        "only winner's snapshot_changes row should be persisted; found {} rows",
        changes.len()
    );
    let row = &changes[0];
    assert_eq!(row.author.as_deref(), Some("winner"));
    assert_eq!(row.commit_message.as_deref(), Some("b wins"));
    let cm = row.changes_made.as_deref().unwrap_or("");
    assert!(
        cm.contains("created_schema:schema_b"),
        "changes_made must reference schema_b, got: {cm}"
    );
    // Loser's data must not appear.
    assert!(
        !cm.contains("schema_a"),
        "loser's schema_a must not appear in changes_made, got: {cm}"
    );
}

/// A writer that is fenced mid-batch (epoch superseded) and then attempts to
/// commit must not leave any partial catalog rows visible to a new writer.
#[tokio::test]
async fn writer_fencing_no_partial_artifacts() {
    let dir = TempDir::new().unwrap();

    // Writer A opens and stages several mutations.
    let mut store_a = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer_a = store_a.begin_write();
    writer_a.create_schema("partial_schema").await.unwrap();
    writer_a.create_schema("another_partial").await.unwrap();

    // Ensure clock advances.
    tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;

    // Writer B opens, taking ownership of the epoch.
    let mut store_b = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer_b = store_b.begin_write();
    writer_b.create_schema("real_schema").await.unwrap();
    let commit_b = writer_b
        .create_snapshot(Some("new_writer"), None)
        .await
        .expect("writer B must succeed");
    store_b.commit_writer(commit_b);

    // Writer A's commit attempt must be rejected.
    let result_a = writer_a.create_snapshot(Some("fenced"), None).await;
    assert!(
        matches!(
            result_a,
            Err(CatalogError::WriterEpochMismatch) | Err(CatalogError::TransactionConflict(_))
        ),
        "writer A must be fenced; got: {result_a:?}"
    );

    // A new reader at the winning snapshot must see only writer B's schema.
    let reader = store_b
        .read_at(SnapshotId::new(commit_b.snapshot_id.as_u64()))
        .unwrap();
    let schemas = reader.list_schemas().await.unwrap();
    let names: Vec<&str> = schemas.iter().map(|s| s.schema_name.as_str()).collect();
    assert_eq!(names, vec!["real_schema"], "only writer B's schema visible");
    assert!(
        !names.contains(&"partial_schema"),
        "partial_schema must not be visible after fencing"
    );
    assert!(
        !names.contains(&"another_partial"),
        "another_partial must not be visible after fencing"
    );
}

// ─── § Spec-Complete Snapshot Changes ───────────────────────────────────────

/// After a workload that creates a schema, creates a table, and inserts rows,
/// ducklake_snapshot_changes must have spec-correct `changes_made`, `author`,
/// and `commit_message` fields.
#[tokio::test]
async fn snapshot_changes_spec_fields_after_workload() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let snap = {
        let mut w = store.begin_write();
        let sid = w.create_schema("myschema").await.unwrap();
        w.add_snapshot_changes(
            "created_schema".to_string(),
            Some("myschema".to_string()),
            Some(sid),
            None,
        )
        .await
        .unwrap();
        let tid = w.create_table(sid, "mytable", None).await.unwrap();
        w.add_snapshot_changes(
            "created_table".to_string(),
            Some(tid.to_string()),
            Some(sid),
            Some(tid),
        )
        .await
        .unwrap();
        w.add_snapshot_changes(
            "inserted_rows".to_string(),
            Some(format!("{tid}:5")),
            None,
            Some(tid),
        )
        .await
        .unwrap();
        w.create_snapshot(Some("alice"), Some("initial workload"))
            .await
            .unwrap()
    };
    store.commit_writer(snap);

    let reader = store
        .read_at(SnapshotId::new(snap.snapshot_id.as_u64()))
        .unwrap();
    let changes = reader.list_all_snapshot_changes().await.unwrap();

    assert_eq!(changes.len(), 1, "one changes row per snapshot");
    let row = &changes[0];

    // Spec fields must be present.
    assert_eq!(row.author.as_deref(), Some("alice"));
    assert_eq!(row.commit_message.as_deref(), Some("initial workload"));

    let cm = row.changes_made.as_deref().unwrap_or("");
    assert!(
        cm.contains("created_schema:myschema"),
        "changes_made must contain created_schema token, got: {cm}"
    );
    assert!(
        cm.contains("created_table:"),
        "changes_made must contain created_table token, got: {cm}"
    );
    assert!(
        cm.contains("inserted_rows:"),
        "changes_made must contain inserted_rows token, got: {cm}"
    );
}

/// After v0.27.8, SnapshotRow.author and .message must be None for newly
/// committed snapshots.  Author/message live in SnapshotChangesRow only.
#[tokio::test]
async fn snapshot_changes_author_not_in_snapshot_row() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let snap = {
        let mut w = store.begin_write();
        w.create_schema("s").await.unwrap();
        w.create_snapshot(Some("dave"), Some("v0.27.8 spec test"))
            .await
            .unwrap()
    };
    store.commit_writer(snap);

    let reader = store
        .read_at(SnapshotId::new(snap.snapshot_id.as_u64()))
        .unwrap();

    // SnapshotRow must NOT carry author/message (v0.27.8 change).
    let snap_row = reader.get_snapshot().await.unwrap().expect("snapshot row");
    assert_eq!(
        snap_row.author, None,
        "SnapshotRow.author must be None after v0.27.8"
    );
    assert_eq!(
        snap_row.message, None,
        "SnapshotRow.message must be None after v0.27.8"
    );

    // SnapshotChangesRow MUST carry them.
    let changes = reader.list_all_snapshot_changes().await.unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].author.as_deref(), Some("dave"));
    assert_eq!(
        changes[0].commit_message.as_deref(),
        Some("v0.27.8 spec test")
    );
}

/// The losing writer's snapshot_id must NOT appear in ducklake_snapshot_changes.
#[tokio::test]
async fn losing_writer_snapshot_not_in_snapshot_changes() {
    let dir = TempDir::new().unwrap();

    // Writer A opens and stages changes.
    let mut store_a = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer_a = store_a.begin_write();
    writer_a.create_schema("loser_schema").await.unwrap();
    writer_a
        .add_snapshot_changes(
            "created_schema".to_string(),
            Some("loser_schema".to_string()),
            None,
            None,
        )
        .await
        .unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;

    // Writer B opens, wins the epoch.
    let mut store_b = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer_b = store_b.begin_write();
    writer_b.create_schema("winner_schema").await.unwrap();
    writer_b
        .add_snapshot_changes(
            "created_schema".to_string(),
            Some("winner_schema".to_string()),
            None,
            None,
        )
        .await
        .unwrap();
    let commit_b = writer_b
        .create_snapshot(Some("winner"), None)
        .await
        .expect("B must commit");
    store_b.commit_writer(commit_b);

    // A attempts to commit — must fail.
    let _ = writer_a.create_snapshot(Some("loser"), None).await;

    // Only one snapshot_changes row must exist (winner's).
    let reader = store_b
        .read_at(SnapshotId::new(commit_b.snapshot_id.as_u64()))
        .unwrap();
    let changes = reader.list_all_snapshot_changes().await.unwrap();
    assert_eq!(
        changes.len(),
        1,
        "only winner's snapshot_changes must be persisted; found {} rows",
        changes.len()
    );
    let cm = changes[0].changes_made.as_deref().unwrap_or("");
    assert!(
        cm.contains("winner_schema"),
        "changes_made must reference winner_schema, got: {cm}"
    );
    assert!(
        !cm.contains("loser_schema"),
        "loser_schema must not appear in changes_made, got: {cm}"
    );
}

// ─── § Type-Aware Column Stats ───────────────────────────────────────────────

/// Helper: open a store, create a table with one column, return (store, table_id, column_id).
async fn setup_single_column_table(dir: &TempDir, col_type: &str) -> (CatalogStore, u64, u64) {
    let mut store = CatalogStore::open(test_opts(dir)).await.unwrap();
    let cr = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        let _cid = w
            .add_column(tid, "v", col_type, 0, true, None)
            .await
            .unwrap();
        w.create_snapshot(None, None).await.unwrap()
    };
    let table_id = 2; // first table gets catalog_id 2 (schema=1, table=2)
    let column_id = 3; // first column gets catalog_id 3
    store.commit_writer(cr);
    (store, table_id, column_id)
}

/// UBIGINT / unsigned integer: stats for u64::MAX must compare correctly.
/// u64::MAX = 18446744073709551615, well within i128 range.
#[tokio::test]
async fn stats_merge_handles_unsigned_integer_correctly() {
    let dir = TempDir::new().unwrap();
    let (mut store, table_id, column_id) = setup_single_column_table(&dir, "UBIGINT").await;

    // Batch 1: range [0, 100]
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("0"),
            Some("100"),
            None,
            None,
        )
        .await
        .unwrap();
        store.commit_writer(w.create_snapshot(None, None).await.unwrap());
    }

    // Batch 2: range [50, 18446744073709551615] (u64::MAX)
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("50"),
            Some("18446744073709551615"),
            None,
            None,
        )
        .await
        .unwrap();
        store.commit_writer(w.create_snapshot(None, None).await.unwrap());
    }

    let reader = store.read_at(SnapshotId::new(3)).unwrap();
    let tcs = reader.list_all_table_column_stats().await.unwrap();
    let stats = tcs
        .iter()
        .find(|s| s.table_id == table_id && s.column_id == column_id)
        .expect("stats must exist");

    assert_eq!(stats.min_value.as_deref(), Some("0"), "min must be 0");
    assert_eq!(
        stats.max_value.as_deref(),
        Some("18446744073709551615"),
        "max must be u64::MAX"
    );
}

/// DECIMAL: exact comparison must not lose precision via f64 for large exact decimals.
#[tokio::test]
async fn stats_merge_handles_decimal_correctly() {
    let dir = TempDir::new().unwrap();
    let (mut store, table_id, column_id) = setup_single_column_table(&dir, "DECIMAL(20,3)").await;

    // Batch 1: range [1.000, 999999999999999999.999]
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("1.000"),
            Some("999999999999999999.999"),
            None,
            None,
        )
        .await
        .unwrap();
        store.commit_writer(w.create_snapshot(None, None).await.unwrap());
    }

    // Batch 2: range [0.001, 1000000000000000000.000] — should expand max
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("0.001"),
            Some("1000000000000000000.000"),
            None,
            None,
        )
        .await
        .unwrap();
        store.commit_writer(w.create_snapshot(None, None).await.unwrap());
    }

    let reader = store.read_at(SnapshotId::new(3)).unwrap();
    let tcs = reader.list_all_table_column_stats().await.unwrap();
    let stats = tcs
        .iter()
        .find(|s| s.table_id == table_id && s.column_id == column_id)
        .expect("stats must exist");

    assert_eq!(
        stats.min_value.as_deref(),
        Some("0.001"),
        "min must be 0.001"
    );
    assert_eq!(
        stats.max_value.as_deref(),
        Some("1000000000000000000.000"),
        "max must be the larger value"
    );
}

/// DATE: ISO-8601 date strings must sort correctly (lexicographic is correct).
#[tokio::test]
async fn stats_merge_handles_date_correctly() {
    let dir = TempDir::new().unwrap();
    let (mut store, table_id, column_id) = setup_single_column_table(&dir, "DATE").await;

    // Batch 1
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("2024-06-01"),
            Some("2024-12-31"),
            None,
            None,
        )
        .await
        .unwrap();
        store.commit_writer(w.create_snapshot(None, None).await.unwrap());
    }

    // Batch 2: extends range in both directions
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("2024-01-01"),
            Some("2025-03-15"),
            None,
            None,
        )
        .await
        .unwrap();
        store.commit_writer(w.create_snapshot(None, None).await.unwrap());
    }

    let reader = store.read_at(SnapshotId::new(3)).unwrap();
    let tcs = reader.list_all_table_column_stats().await.unwrap();
    let stats = tcs
        .iter()
        .find(|s| s.table_id == table_id && s.column_id == column_id)
        .expect("stats must exist");

    assert_eq!(
        stats.min_value.as_deref(),
        Some("2024-01-01"),
        "min date must be 2024-01-01"
    );
    assert_eq!(
        stats.max_value.as_deref(),
        Some("2025-03-15"),
        "max date must be 2025-03-15"
    );
}

/// TIMESTAMP: ISO-8601 timestamp strings must sort correctly.
#[tokio::test]
async fn stats_merge_handles_timestamp_correctly() {
    let dir = TempDir::new().unwrap();
    let (mut store, table_id, column_id) = setup_single_column_table(&dir, "TIMESTAMP").await;

    // Batch 1
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("2024-01-01 10:00:00"),
            Some("2024-06-30 23:59:59"),
            None,
            None,
        )
        .await
        .unwrap();
        store.commit_writer(w.create_snapshot(None, None).await.unwrap());
    }

    // Batch 2: extends range
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("2023-12-31 00:00:00"),
            Some("2025-01-01 00:00:00"),
            None,
            None,
        )
        .await
        .unwrap();
        store.commit_writer(w.create_snapshot(None, None).await.unwrap());
    }

    let reader = store.read_at(SnapshotId::new(3)).unwrap();
    let tcs = reader.list_all_table_column_stats().await.unwrap();
    let stats = tcs
        .iter()
        .find(|s| s.table_id == table_id && s.column_id == column_id)
        .expect("stats must exist");

    assert_eq!(
        stats.min_value.as_deref(),
        Some("2023-12-31 00:00:00"),
        "min timestamp wrong"
    );
    assert_eq!(
        stats.max_value.as_deref(),
        Some("2025-01-01 00:00:00"),
        "max timestamp wrong"
    );
}

/// UUID: lexicographic ordering is correct for RFC-4122 UUIDs.
#[tokio::test]
async fn stats_merge_handles_uuid_correctly() {
    let dir = TempDir::new().unwrap();
    let (mut store, table_id, column_id) = setup_single_column_table(&dir, "UUID").await;

    let uuid_low = "00000000-0000-0000-0000-000000000001";
    let uuid_mid = "7fffffff-ffff-ffff-ffff-ffffffffffff";
    let uuid_high = "ffffffff-ffff-ffff-ffff-ffffffffffff";

    // Batch 1: [uuid_mid, uuid_mid]
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some(uuid_mid),
            Some(uuid_mid),
            None,
            None,
        )
        .await
        .unwrap();
        store.commit_writer(w.create_snapshot(None, None).await.unwrap());
    }

    // Batch 2: [uuid_low, uuid_high] — expands range
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some(uuid_low),
            Some(uuid_high),
            None,
            None,
        )
        .await
        .unwrap();
        store.commit_writer(w.create_snapshot(None, None).await.unwrap());
    }

    let reader = store.read_at(SnapshotId::new(3)).unwrap();
    let tcs = reader.list_all_table_column_stats().await.unwrap();
    let stats = tcs
        .iter()
        .find(|s| s.table_id == table_id && s.column_id == column_id)
        .expect("stats must exist");

    assert_eq!(stats.min_value.as_deref(), Some(uuid_low), "min UUID wrong");
    assert_eq!(
        stats.max_value.as_deref(),
        Some(uuid_high),
        "max UUID wrong"
    );
}

/// BOOLEAN: false < true must be maintained across merges.
#[tokio::test]
async fn stats_merge_handles_boolean_correctly() {
    let dir = TempDir::new().unwrap();
    let (mut store, table_id, column_id) = setup_single_column_table(&dir, "BOOLEAN").await;

    // Batch 1: only false values
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("false"),
            Some("false"),
            None,
            None,
        )
        .await
        .unwrap();
        store.commit_writer(w.create_snapshot(None, None).await.unwrap());
    }

    // Batch 2: mixed true and false — max should become true
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("false"),
            Some("true"),
            None,
            None,
        )
        .await
        .unwrap();
        store.commit_writer(w.create_snapshot(None, None).await.unwrap());
    }

    let reader = store.read_at(SnapshotId::new(3)).unwrap();
    let tcs = reader.list_all_table_column_stats().await.unwrap();
    let stats = tcs
        .iter()
        .find(|s| s.table_id == table_id && s.column_id == column_id)
        .expect("stats must exist");

    assert_eq!(
        stats.min_value.as_deref(),
        Some("false"),
        "min boolean must be false"
    );
    assert_eq!(
        stats.max_value.as_deref(),
        Some("true"),
        "max boolean must be true"
    );
}
