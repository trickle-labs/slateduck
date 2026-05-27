//! v0.27.5 conformance tests — DuckLake v1.0 Spec Gap Closure.
//!
//! Covers:
//!   Phase 1 — Snapshot schema: denormalized next_catalog_id/next_file_id; author retained internally.
//!   Phase 2 — Snapshot changes accumulation: multiple changes → single row with changes_made.
//!   Phase 3 — DROP TABLE cascade: delete files and inlined rows retired correctly.
//!   Phase 4 — Type-aware stats comparison: booleans, negative integers, multi-digit ints, ISO dates.
//!   Phase 5 — Integration: list_all_snapshot_changes round-trip.

use object_store::path::Path as ObjectPath;
use rocklake_catalog::{CatalogStore, OpenOptions};
use rocklake_core::mvcc::SnapshotId;
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

// ─── Phase 1: Snapshot schema ─────────────────────────────────────────────────

/// Snapshot row must carry denormalized next_catalog_id and next_file_id.
#[tokio::test]
async fn snapshot_row_contains_denormalized_counters() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let snap = {
        let mut w = store.begin_write();
        w.create_schema("myschema").await.unwrap();
        w.create_snapshot(Some("alice"), Some("initial"))
            .await
            .unwrap()
    };
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let row = reader.get_snapshot().await.unwrap().expect("snapshot row");

    assert!(
        row.next_catalog_id.unwrap_or(0) > 0,
        "next_catalog_id should be denormalized into snapshot row"
    );
    assert!(
        row.next_file_id.is_some(),
        "next_file_id should always be present in snapshot row"
    );
}

/// v0.27.8: author/message are no longer stored in ducklake_snapshot (SnapshotRow);
/// they live exclusively in ducklake_snapshot_changes (SnapshotChangesRow).
/// SnapshotRow still has the fields for backward-compatible decoding of existing
/// rows but new snapshots set them to None.
#[tokio::test]
async fn snapshot_row_retains_author_internally() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let snap = {
        let mut w = store.begin_write();
        w.create_schema("s").await.unwrap();
        w.create_snapshot(Some("bob"), Some("initial commit"))
            .await
            .unwrap()
    };
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let row = reader.get_snapshot().await.unwrap().expect("snapshot row");
    // v0.27.8: SnapshotRow.author and .message are no longer populated.
    assert_eq!(row.author, None, "author must not be stored in SnapshotRow");
    assert_eq!(
        row.message, None,
        "message must not be stored in SnapshotRow"
    );

    // Author/message are stored exclusively in ducklake_snapshot_changes.
    let changes = reader.list_all_snapshot_changes().await.unwrap();
    assert_eq!(changes.len(), 1, "one changes row per snapshot");
    assert_eq!(changes[0].author.as_deref(), Some("bob"));
    assert_eq!(changes[0].commit_message.as_deref(), Some("initial commit"));
}

// ─── Phase 2: Snapshot changes accumulation ────────────────────────────────────

/// Multiple add_snapshot_changes calls must produce one SnapshotChangesRow with
/// a comma-separated changes_made string, not just the last call's value.
#[tokio::test]
async fn snapshot_changes_accumulate_into_single_row() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let snap = {
        let mut w = store.begin_write();
        w.create_schema("s").await.unwrap();
        w.add_snapshot_changes(
            "created_schema".to_string(),
            Some("s".to_string()),
            None,
            None,
        )
        .await
        .unwrap();
        w.add_snapshot_changes(
            "created_table".to_string(),
            Some("t1".to_string()),
            None,
            None,
        )
        .await
        .unwrap();
        w.add_snapshot_changes(
            "created_table".to_string(),
            Some("t2".to_string()),
            None,
            None,
        )
        .await
        .unwrap();
        w.create_snapshot(Some("charlie"), Some("big bang"))
            .await
            .unwrap()
    };
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let rows = reader.list_all_snapshot_changes().await.unwrap();
    assert_eq!(rows.len(), 1, "exactly one row per snapshot");
    let row = &rows[0];
    assert_eq!(row.snapshot_id, snap.snapshot_id.as_u64());
    let cm = row.changes_made.as_deref().unwrap_or("");
    assert!(
        cm.contains("created_schema:s"),
        "missing created_schema:s in changes_made: {cm}"
    );
    assert!(
        cm.contains("created_table:t1"),
        "missing created_table:t1 in changes_made: {cm}"
    );
    assert!(
        cm.contains("created_table:t2"),
        "missing created_table:t2 in changes_made: {cm}"
    );
    assert_eq!(row.author.as_deref(), Some("charlie"));
    assert_eq!(row.commit_message.as_deref(), Some("big bang"));
}

/// When only author/message are given (no change events), a snapshot changes row
/// must still be created carrying author/commit_message.
#[tokio::test]
async fn snapshot_changes_row_created_for_author_only_snapshot() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let snap = {
        let mut w = store.begin_write();
        w.create_schema("s").await.unwrap();
        w.create_snapshot(Some("diane"), Some("solo commit"))
            .await
            .unwrap()
    };
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let rows = reader.list_all_snapshot_changes().await.unwrap();
    assert_eq!(
        rows.len(),
        1,
        "snapshot changes row must be created when author is given"
    );
    let row = &rows[0];
    assert_eq!(row.author.as_deref(), Some("diane"));
    assert_eq!(row.commit_message.as_deref(), Some("solo commit"));
}

/// Snapshots with no author/message and no changes produce no snapshot-changes row.
#[tokio::test]
async fn snapshot_without_author_produces_no_changes_row() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let snap = {
        let mut w = store.begin_write();
        w.create_schema("s").await.unwrap();
        w.create_snapshot(None, None).await.unwrap()
    };
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let rows = reader.list_all_snapshot_changes().await.unwrap();
    assert!(
        rows.is_empty(),
        "no snapshot changes row expected when no author/message/changes are given"
    );
}

// ─── Phase 3: DROP TABLE cascade ──────────────────────────────────────────────

/// Dropping a table must retire the data files registered for it and all
/// associated delete files (matched by data_file_id even when table_id is None).
#[tokio::test]
async fn drop_table_cascades_to_delete_files() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (schema_id, table_id, snap1) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        let fid = w
            .register_data_file(tid, "s3://b/t/f1.parquet", "parquet", 100, 1000)
            .await
            .unwrap();
        w.register_delete_file(fid, "s3://b/t/del1.parquet", 10, 200)
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        let s = cr.snapshot_id.as_u64();
        store.commit_writer(cr);
        (sid, tid, s)
    };

    // Drop the table.
    let drop_snap = {
        let mut w = store.begin_write();
        let tables = store
            .read_at(SnapshotId::new(snap1))
            .unwrap()
            .list_tables(schema_id)
            .await
            .unwrap();
        let trow = tables
            .iter()
            .find(|t| t.table_id == table_id)
            .expect("table should exist");
        w.drop_table(schema_id, table_id, trow.begin_snapshot)
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        let s = cr.snapshot_id.as_u64();
        store.commit_writer(cr);
        s
    };

    // After drop: data files must be invisible.
    let reader = store.read_at(SnapshotId::new(drop_snap)).unwrap();
    let files = reader.list_data_files(table_id).await.unwrap();
    assert!(
        files.is_empty(),
        "data files must not be visible after table drop"
    );

    // After drop: delete files must be invisible (retired via data_file_id association).
    let dfs = reader.list_delete_files(table_id).await.unwrap();
    assert!(
        dfs.is_empty(),
        "delete files must not be visible after table drop (cascade via data_file_id)"
    );
}

/// Dropping a table must retire its live inlined insert rows.
#[tokio::test]
async fn drop_table_cascades_to_inlined_rows() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (schema_id, table_id, snap1) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        w.register_inlined_data_table(tid, "s_t", 1).await.unwrap();
        w.register_inlined_insert(tid, 1, 1, b"row-data-bytes".to_vec())
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        let s = cr.snapshot_id.as_u64();
        store.commit_writer(cr);
        (sid, tid, s)
    };

    // Verify inlined insert is visible before drop.
    {
        let reader = store.read_at(SnapshotId::new(snap1)).unwrap();
        let rows = reader.list_inlined_inserts(table_id).await.unwrap();
        assert!(
            !rows.is_empty(),
            "inlined insert should be visible before drop"
        );
    }

    // Drop the table.
    let drop_snap = {
        let mut w = store.begin_write();
        let tables = store
            .read_at(SnapshotId::new(snap1))
            .unwrap()
            .list_tables(schema_id)
            .await
            .unwrap();
        let trow = tables
            .iter()
            .find(|t| t.table_id == table_id)
            .expect("table should exist");
        w.drop_table(schema_id, table_id, trow.begin_snapshot)
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        let s = cr.snapshot_id.as_u64();
        store.commit_writer(cr);
        s
    };

    // After drop: inlined rows must be invisible.
    let reader = store.read_at(SnapshotId::new(drop_snap)).unwrap();
    let rows = reader.list_inlined_inserts(table_id).await.unwrap();
    assert!(
        rows.is_empty(),
        "inlined insert rows must not be visible after table drop"
    );
}

// ─── Phase 4: Type-aware stats comparison ──────────────────────────────────────
//
// `upsert_table_column_stats` reads existing table-level stats and merges the
// incoming values using `merge_min`/`merge_max`, which call the type-aware
// `stats_value_less_or_equal` helper.  We test it directly here.

/// Negative integers must compare numerically, not lexicographically.
#[tokio::test]
async fn stats_merge_handles_negative_integers_correctly() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (table_id, column_id) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        let cid = w
            .add_column(tid, "v", "INTEGER", 0, true, None)
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        (tid, cid)
    };

    // Batch 1: min = -10, max = -2
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("-10"),
            Some("-2"),
            None,
            None,
        )
        .await
        .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    // Batch 2: min = -5, max = 3 — expands range to [-10, 3]
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("-5"),
            Some("3"),
            None,
            None,
        )
        .await
        .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    let reader = store.read_at(SnapshotId::new(3)).unwrap();
    let tcs = reader.list_all_table_column_stats().await.unwrap();
    let stats = tcs
        .iter()
        .find(|s| s.table_id == table_id && s.column_id == column_id)
        .expect("table column stats must exist");

    assert_eq!(
        stats.min_value.as_deref(),
        Some("-10"),
        "numeric minimum should be -10"
    );
    assert_eq!(
        stats.max_value.as_deref(),
        Some("3"),
        "numeric maximum should be 3"
    );
}

/// Multi-digit integer: 10 > 2 numerically; "10" < "2" lexicographically.
#[tokio::test]
async fn stats_merge_handles_integer_digit_count_correctly() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (table_id, column_id) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        let cid = w
            .add_column(tid, "v", "BIGINT", 0, true, None)
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        (tid, cid)
    };

    // Batch 1: only value = 2
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(table_id, column_id, false, Some("2"), Some("2"), None, None)
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    // Batch 2: only value = 10
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("10"),
            Some("10"),
            None,
            None,
        )
        .await
        .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    let reader = store.read_at(SnapshotId::new(3)).unwrap();
    let tcs = reader.list_all_table_column_stats().await.unwrap();
    let stats = tcs
        .iter()
        .find(|s| s.table_id == table_id && s.column_id == column_id)
        .expect("table column stats");

    assert_eq!(
        stats.min_value.as_deref(),
        Some("2"),
        "numeric minimum of 2 and 10 is 2"
    );
    assert_eq!(
        stats.max_value.as_deref(),
        Some("10"),
        "numeric maximum of 2 and 10 is 10, not '2' (lexicographic)"
    );
}

/// Boolean stats must order false < true.
#[tokio::test]
async fn stats_merge_handles_booleans_correctly() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (table_id, column_id) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        let cid = w
            .add_column(tid, "flag", "BOOLEAN", 0, true, None)
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        (tid, cid)
    };

    // Batch 1: only true values
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("true"),
            Some("true"),
            None,
            None,
        )
        .await
        .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    // Batch 2: mixed batch — min = false, max = true
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
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    let reader = store.read_at(SnapshotId::new(3)).unwrap();
    let tcs = reader.list_all_table_column_stats().await.unwrap();
    let stats = tcs
        .iter()
        .find(|s| s.table_id == table_id && s.column_id == column_id)
        .expect("table column stats");

    assert_eq!(
        stats.min_value.as_deref(),
        Some("false"),
        "boolean min should be false"
    );
    assert_eq!(
        stats.max_value.as_deref(),
        Some("true"),
        "boolean max should be true"
    );
}

/// ISO-8601 DATE values sort correctly lexicographically.
#[tokio::test]
async fn stats_merge_handles_dates_correctly() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (table_id, column_id) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        let cid = w.add_column(tid, "d", "DATE", 0, true, None).await.unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        (tid, cid)
    };

    // Batch 1: 2024-01-01 .. 2024-06-30
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("2024-01-01"),
            Some("2024-06-30"),
            None,
            None,
        )
        .await
        .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    // Batch 2: 2023-12-01 .. 2024-09-15 — expands range
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("2023-12-01"),
            Some("2024-09-15"),
            None,
            None,
        )
        .await
        .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    let reader = store.read_at(SnapshotId::new(3)).unwrap();
    let tcs = reader.list_all_table_column_stats().await.unwrap();
    let stats = tcs
        .iter()
        .find(|s| s.table_id == table_id && s.column_id == column_id)
        .expect("table column stats");

    assert_eq!(
        stats.min_value.as_deref(),
        Some("2023-12-01"),
        "date minimum should be 2023-12-01"
    );
    assert_eq!(
        stats.max_value.as_deref(),
        Some("2024-09-15"),
        "date maximum should be 2024-09-15"
    );
}

// ─── Phase 5: Integration — list_all_snapshot_changes round-trip ───────────────

/// Multiple snapshots each generate their own SnapshotChangesRow; all are
/// returned by list_all_snapshot_changes.
#[tokio::test]
async fn list_all_snapshot_changes_returns_all_snapshots() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let mut last_snap_id = SnapshotId::new(0);
    for i in 0u32..3 {
        let snap = {
            let mut w = store.begin_write();
            w.create_schema(&format!("s{i}")).await.unwrap();
            w.add_snapshot_changes(
                "created_schema".to_string(),
                Some(format!("s{i}")),
                None,
                None,
            )
            .await
            .unwrap();
            w.create_snapshot(Some(&format!("user{i}")), Some(&format!("commit {i}")))
                .await
                .unwrap()
        };
        last_snap_id = snap.snapshot_id;
        store.commit_writer(snap);
    }

    let reader = store.read_at(last_snap_id).unwrap();
    let rows = reader.list_all_snapshot_changes().await.unwrap();
    assert_eq!(rows.len(), 3, "should have one changes row per snapshot");
    for row in &rows {
        assert!(
            row.author.is_some(),
            "author should be stored in snapshot changes row"
        );
        assert!(
            row.changes_made
                .as_deref()
                .unwrap_or("")
                .contains("created_schema:"),
            "changes_made should contain the created_schema token"
        );
    }
}

// ─── Phase 6: DROP TABLE cascade completeness ─────────────────────────────────

/// DROP TABLE must retire all live tags attached to the table.
#[tokio::test]
async fn drop_table_cascades_to_tags() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Create schema, table, and add tags.
    let (schema_id, table_id) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        w.set_tag(tid, "owner", "data-team").await.unwrap();
        w.set_tag(tid, "env", "prod").await.unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        (sid, tid)
    };
    let _ = schema_id;

    // Verify tags are visible before drop.
    {
        let reader = store.read_at(SnapshotId::new(1)).unwrap();
        let tags = reader.list_all_tags().await.unwrap();
        let live: Vec<_> = tags
            .iter()
            .filter(|t| t.object_id == table_id && t.end_snapshot.is_none())
            .collect();
        assert_eq!(live.len(), 2, "both tags must be live before drop");
    }

    // Drop the table.
    {
        let mut w = store.begin_write();
        w.drop_table(schema_id, table_id, 1).await.unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    // After drop all tags must have end_snapshot set.
    let reader = store.read_at(SnapshotId::new(2)).unwrap();
    let tags = reader.list_all_tags().await.unwrap();
    let still_live: Vec<_> = tags
        .iter()
        .filter(|t| t.object_id == table_id && t.end_snapshot.is_none())
        .collect();
    assert!(
        still_live.is_empty(),
        "all tags should be retired after DROP TABLE, but found: {still_live:?}"
    );
}

/// DROP TABLE must retire all live column tags attached to columns of that table.
#[tokio::test]
async fn drop_table_cascades_to_column_tags() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (schema_id, table_id, col_id) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        let cid = w
            .add_column(tid, "amount", "BIGINT", 0, false, None)
            .await
            .unwrap();
        w.set_column_tag(tid, cid, "pii", "false").await.unwrap();
        w.set_column_tag(tid, cid, "sensitive", "low")
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        (sid, tid, cid)
    };
    let _ = col_id;

    // Drop the table.
    {
        let mut w = store.begin_write();
        w.drop_table(schema_id, table_id, 1).await.unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    // After drop all column tags must have end_snapshot set.
    let reader = store.read_at(SnapshotId::new(2)).unwrap();
    let col_tags = reader.list_all_column_tags().await.unwrap();
    let still_live: Vec<_> = col_tags
        .iter()
        .filter(|ct| ct.table_id == table_id && ct.end_snapshot.is_none())
        .collect();
    assert!(
        still_live.is_empty(),
        "all column tags should be retired after DROP TABLE, found: {still_live:?}"
    );
}

/// DROP TABLE must retire all live sort info rows attached to that table.
#[tokio::test]
async fn drop_table_cascades_to_sort_info() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (schema_id, table_id) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        w.add_sort_info(tid, 1).await.unwrap();
        w.add_sort_info(tid, 2).await.unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        (sid, tid)
    };

    // Verify sort info is live before drop.
    {
        let reader = store.read_at(SnapshotId::new(1)).unwrap();
        let sort_rows = reader.list_all_sort_info().await.unwrap();
        let live: Vec<_> = sort_rows
            .iter()
            .filter(|r| r.table_id == table_id && r.end_snapshot.is_none())
            .collect();
        assert_eq!(
            live.len(),
            2,
            "both sort_info rows should be live before drop"
        );
    }

    // Drop the table.
    {
        let mut w = store.begin_write();
        w.drop_table(schema_id, table_id, 1).await.unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    // After drop all sort info must have end_snapshot set.
    let reader = store.read_at(SnapshotId::new(2)).unwrap();
    let sort_rows = reader.list_all_sort_info().await.unwrap();
    let still_live: Vec<_> = sort_rows
        .iter()
        .filter(|r| r.table_id == table_id && r.end_snapshot.is_none())
        .collect();
    assert!(
        still_live.is_empty(),
        "all sort_info rows should be retired after DROP TABLE, found: {still_live:?}"
    );
}

// ─── Phase 7: ALTER TABLE time-travel ─────────────────────────────────────────

/// ADD COLUMN → time-travel read at old snapshot shows original columns only.
#[tokio::test]
async fn alter_table_add_column_time_travel() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Snapshot 1: table with one column.
    let (table_id, snap1) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        w.add_column(tid, "id", "BIGINT", 0, false, None)
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        let sn = cr.snapshot_id;
        store.commit_writer(cr);
        (tid, sn)
    };

    // Snapshot 2: add second column.
    {
        let mut w = store.begin_write();
        w.add_column(table_id, "name", "VARCHAR", 1, true, None)
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    // At snap1: only "id" visible.
    let r1 = store.read_at(snap1).unwrap();
    let (_, cols1) = r1
        .describe_table(table_id)
        .await
        .unwrap()
        .expect("table must exist at snap1");
    assert_eq!(
        cols1.len(),
        1,
        "at snap1 should see only original column, got: {cols1:?}"
    );
    assert_eq!(cols1[0].column_name, "id");

    // At snap2: both columns visible.
    let r2 = store.read_at(SnapshotId::new(2)).unwrap();
    let (_, cols2) = r2
        .describe_table(table_id)
        .await
        .unwrap()
        .expect("table must exist at snap2");
    assert_eq!(
        cols2.len(),
        2,
        "at snap2 should see both columns, got: {cols2:?}"
    );
    let names: Vec<_> = cols2.iter().map(|c| c.column_name.as_str()).collect();
    assert!(names.contains(&"id"), "id column must be visible at snap2");
    assert!(
        names.contains(&"name"),
        "name column must be visible at snap2"
    );
}

/// DROP COLUMN → time-travel read at old snapshot still shows the dropped column.
#[tokio::test]
async fn alter_table_drop_column_time_travel() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Snapshot 1: two columns.
    let (table_id, col_id, snap1) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        w.add_column(tid, "id", "BIGINT", 0, false, None)
            .await
            .unwrap();
        let cid = w
            .add_column(tid, "tmp", "VARCHAR", 1, true, None)
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        let sn = cr.snapshot_id;
        store.commit_writer(cr);
        (tid, cid, sn)
    };

    // Snapshot 2: drop the "tmp" column.
    {
        let mut w = store.begin_write();
        w.drop_column(table_id, col_id, snap1.as_u64())
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    // At snap1: both columns visible.
    let r1 = store.read_at(snap1).unwrap();
    let (_, cols1) = r1
        .describe_table(table_id)
        .await
        .unwrap()
        .expect("table must exist at snap1");
    assert_eq!(cols1.len(), 2, "at snap1 both columns should be visible");

    // At snap2: only "id" visible.
    let r2 = store.read_at(SnapshotId::new(2)).unwrap();
    let (_, cols2) = r2
        .describe_table(table_id)
        .await
        .unwrap()
        .expect("table must exist at snap2");
    assert_eq!(
        cols2.len(),
        1,
        "at snap2 only id should remain, got: {cols2:?}"
    );
    assert_eq!(cols2[0].column_name, "id");
}

// ─── Phase 8: Stats type coverage — timestamps ─────────────────────────────────

/// Timestamp stats comparison: later timestamps sort greater.
#[tokio::test]
async fn stats_merge_handles_timestamps_correctly() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (table_id, column_id) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        let cid = w
            .add_column(tid, "ts", "TIMESTAMP", 0, false, None)
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        (tid, cid)
    };

    // Batch 1: 2024-01-01T00:00:00 .. 2024-06-01T12:00:00
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("2024-01-01T00:00:00"),
            Some("2024-06-01T12:00:00"),
            None,
            None,
        )
        .await
        .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    // Batch 2: 2023-12-15T08:30:00 .. 2024-12-31T23:59:59
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("2023-12-15T08:30:00"),
            Some("2024-12-31T23:59:59"),
            None,
            None,
        )
        .await
        .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    let reader = store.read_at(SnapshotId::new(3)).unwrap();
    let tcs = reader.list_all_table_column_stats().await.unwrap();
    let stats = tcs
        .iter()
        .find(|s| s.table_id == table_id && s.column_id == column_id)
        .expect("table column stats");

    assert_eq!(
        stats.min_value.as_deref(),
        Some("2023-12-15T08:30:00"),
        "timestamp minimum should be 2023-12-15T08:30:00"
    );
    assert_eq!(
        stats.max_value.as_deref(),
        Some("2024-12-31T23:59:59"),
        "timestamp maximum should be 2024-12-31T23:59:59"
    );
}

/// Float stats comparison: negative floats and multi-digit floats sort correctly.
#[tokio::test]
async fn stats_merge_handles_floats_correctly() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (table_id, column_id) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        let cid = w
            .add_column(tid, "price", "DOUBLE", 0, false, None)
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        (tid, cid)
    };

    // Batch 1: -3.14 .. 100.5
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("-3.14"),
            Some("100.5"),
            None,
            None,
        )
        .await
        .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    // Batch 2: -1000.0 .. 50.0 — expands min, shrinks max
    {
        let mut w = store.begin_write();
        w.upsert_table_column_stats(
            table_id,
            column_id,
            false,
            Some("-1000.0"),
            Some("50.0"),
            None,
            None,
        )
        .await
        .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    let reader = store.read_at(SnapshotId::new(3)).unwrap();
    let tcs = reader.list_all_table_column_stats().await.unwrap();
    let stats = tcs
        .iter()
        .find(|s| s.table_id == table_id && s.column_id == column_id)
        .expect("table column stats");

    assert_eq!(
        stats.min_value.as_deref(),
        Some("-1000.0"),
        "float minimum should be -1000.0, not -3.14 (lexicographic would be wrong)"
    );
    assert_eq!(
        stats.max_value.as_deref(),
        Some("100.5"),
        "float maximum should be 100.5"
    );
}

// ─── Phase 9: Transaction atomicity ───────────────────────────────────────────

/// A partial batch (writer dropped without create_snapshot) leaves the catalog
/// state unchanged — no partially written rows are visible.
#[tokio::test]
async fn partial_batch_is_not_visible_after_drop() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Commit snapshot 1 with one schema.
    {
        let mut w = store.begin_write();
        w.create_schema("committed").await.unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    // Start a write, do mutations, but drop the writer WITHOUT calling create_snapshot.
    {
        let mut w = store.begin_write();
        w.create_schema("abandoned").await.unwrap();
        // Drop w without create_snapshot — the staged mutations are lost.
        drop(w);
    }

    // The catalog should still be at snapshot 1 with only "committed" schema.
    let reader = store.read_at(SnapshotId::new(1)).unwrap();
    let schemas = reader.list_schemas().await.unwrap();
    let names: Vec<_> = schemas.iter().map(|s| s.schema_name.as_str()).collect();
    assert!(
        names.contains(&"committed"),
        "committed schema must be visible"
    );
    assert!(
        !names.contains(&"abandoned"),
        "abandoned schema must NOT be visible, got schemas: {names:?}"
    );
}

/// Committed writer advances snapshot id; a new writer starting after sees the
/// updated state.
#[tokio::test]
async fn committed_writer_state_visible_to_next_writer() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Writer 1 creates a schema.
    let snap = {
        let mut w = store.begin_write();
        w.create_schema("alpha").await.unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        let sn = cr.snapshot_id;
        store.commit_writer(cr);
        sn
    };

    // Writer 2 creates another schema (sees writer 1's committed state).
    {
        let mut w = store.begin_write();
        w.create_schema("beta").await.unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
    }

    // At final snapshot both schemas visible.
    let reader = store.read_at(SnapshotId::new(2)).unwrap();
    let schemas = reader.list_schemas().await.unwrap();
    let names: Vec<_> = schemas.iter().map(|s| s.schema_name.as_str()).collect();
    assert!(names.contains(&"alpha"), "alpha must be visible");
    assert!(names.contains(&"beta"), "beta must be visible");
    let _ = snap;
}
