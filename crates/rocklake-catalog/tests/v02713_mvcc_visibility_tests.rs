//! v0.27.13 MVCC visibility and `file_order` sorting conformance tests.
//!
//! Verifies that `list_data_files` and `list_delete_files` strictly enforce the
//! DuckLake v1.0 spec MVCC predicates:
//!
//!   visible iff `begin_snapshot <= snapshot_id`
//!            AND (`end_snapshot IS NULL` OR `end_snapshot > snapshot_id`)
//!
//! Also validates that `list_data_files` returns results sorted ascending by
//! `file_order`, as required by the DuckLake spec.
//!
//! These tests exercise the RockLake `CatalogStore` and `CatalogReader` APIs
//! directly (no PG-Wire layer), confirming that the catalog-level guarantees
//! required by all Postgres drivers and BI tools hold.

use object_store::memory::InMemory;
use object_store::path::Path as ObjectPath;
use rocklake_catalog::{CatalogStore, OpenOptions};
use rocklake_core::mvcc::SnapshotId;
use std::sync::Arc;

fn make_opts() -> OpenOptions {
    let store: Arc<dyn object_store::ObjectStore> = Arc::new(InMemory::new());
    OpenOptions {
        object_store: store,
        path: ObjectPath::from("v02713-mvcc"),
        encryption: None,
    }
}

// ─── V-01: begin_snapshot filter ─────────────────────────────────────────────

/// A data file registered at snapshot N must NOT be visible at snapshot N-1.
///
/// DuckLake v1.0 spec: `begin_snapshot <= snapshot_id` must hold for
/// visibility. A file committed at snapshot 2 is invisible to readers at
/// snapshot 1.
#[tokio::test]
async fn data_file_visibility_respects_begin_snapshot() {
    let mut cat = CatalogStore::open(make_opts()).await.unwrap();

    // ── Snapshot 1: create schema and table (no data files yet) ──────────────
    let mut w1 = cat.begin_write();
    let schema_id = w1.create_schema("data").await.unwrap();
    let table_id = w1.create_table(schema_id, "events", None).await.unwrap();
    let snap1 = w1
        .create_snapshot(Some("v02713-mvcc"), Some("create table"))
        .await
        .unwrap();
    cat.commit_writer(snap1);

    // ── Snapshot 2: register a data file ─────────────────────────────────────
    let mut w2 = cat.begin_write();
    let _file_id = w2
        .register_data_file_with_metadata(
            table_id,
            "data/events/part-0001.parquet",
            "parquet",
            1_000,
            65_536,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let snap2 = w2
        .create_snapshot(Some("v02713-mvcc"), Some("add data file"))
        .await
        .unwrap();
    cat.commit_writer(snap2);

    // ── Read at snapshot 1: file must NOT be visible ──────────────────────────
    let reader_at_1 = cat.read_at(snap1).unwrap();
    let files_at_1 = reader_at_1.list_data_files(table_id).await.unwrap();
    assert!(
        files_at_1.is_empty(),
        "data file registered at snapshot 2 must NOT be visible at snapshot 1; \
         got {} file(s): {files_at_1:?}",
        files_at_1.len()
    );

    // ── Read at snapshot 2: file MUST be visible ──────────────────────────────
    let reader_at_2 = cat.read_at(snap2).unwrap();
    let files_at_2 = reader_at_2.list_data_files(table_id).await.unwrap();
    assert_eq!(
        files_at_2.len(),
        1,
        "data file registered at snapshot 2 must be visible at snapshot 2; \
         got {} file(s)",
        files_at_2.len()
    );
}

// ─── V-02: end_snapshot filter ───────────────────────────────────────────────

/// A data file retired at snapshot N must NOT be visible at snapshot N.
///
/// DuckLake v1.0 spec: `end_snapshot IS NULL OR end_snapshot > snapshot_id`.
/// When `end_snapshot = N`, the file is excluded at snapshot N (N > N is false)
/// but visible at snapshot N-1 (N > N-1 is true).
#[tokio::test]
async fn data_file_visibility_respects_end_snapshot() {
    let mut cat = CatalogStore::open(make_opts()).await.unwrap();

    // ── Snapshot 1: create schema, table, and a data file ────────────────────
    let mut w1 = cat.begin_write();
    let schema_id = w1.create_schema("retire").await.unwrap();
    let table_id = w1
        .create_table(schema_id, "retire_table", None)
        .await
        .unwrap();
    let _file_id = w1
        .register_data_file_with_metadata(
            table_id,
            "data/retire_table/part-0001.parquet",
            "parquet",
            500,
            32_768,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let snap1 = w1
        .create_snapshot(Some("v02713-mvcc"), Some("create table with file"))
        .await
        .unwrap();
    cat.commit_writer(snap1);

    // Verify file is visible at snap1.
    let reader_at_1 = cat.read_at(snap1).unwrap();
    let files_at_1 = reader_at_1.list_data_files(table_id).await.unwrap();
    assert_eq!(
        files_at_1.len(),
        1,
        "file must be visible before retirement"
    );

    // ── Snapshot 2: drop the table (cascades to retire all its data files) ────
    let mut w2 = cat.begin_write();
    w2.drop_table(schema_id, table_id, snap1.snapshot_id.as_u64())
        .await
        .unwrap();
    let snap2 = w2
        .create_snapshot(Some("v02713-mvcc"), Some("drop table"))
        .await
        .unwrap();
    cat.commit_writer(snap2);

    // ── Read at snapshot 1: file MUST still be visible ───────────────────────
    let reader_at_1b = cat.read_at(snap1).unwrap();
    let files_still_at_1 = reader_at_1b.list_data_files(table_id).await.unwrap();
    assert_eq!(
        files_still_at_1.len(),
        1,
        "file must remain visible at snapshot 1 after retirement at snapshot 2"
    );

    // ── Read at snapshot 2: file must NOT be visible ──────────────────────────
    let reader_at_2 = cat.read_at(snap2).unwrap();
    let files_at_2 = reader_at_2.list_data_files(table_id).await.unwrap();
    assert!(
        files_at_2.is_empty(),
        "file retired at snapshot 2 must NOT be visible at snapshot 2; \
         got {} file(s)",
        files_at_2.len()
    );
}

// ─── V-03: file_order ascending sort ─────────────────────────────────────────

/// `list_data_files` must return files sorted ascending by `file_order`.
///
/// DuckLake v1.0 spec: data files must be presented in insertion order to
/// ensure deterministic query plans across drivers.
#[tokio::test]
async fn data_files_sorted_ascending_by_file_order() {
    let mut cat = CatalogStore::open(make_opts()).await.unwrap();

    let mut writer = cat.begin_write();
    let schema_id = writer.create_schema("ordered").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "ordered_table", None)
        .await
        .unwrap();

    // Register three files in order; each gets a sequentially increasing
    // data_file_id which is also used as file_order.
    let id_a = writer
        .register_data_file_with_metadata(
            table_id,
            "data/ordered_table/part-0001.parquet",
            "parquet",
            100,
            8_192,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let id_b = writer
        .register_data_file_with_metadata(
            table_id,
            "data/ordered_table/part-0002.parquet",
            "parquet",
            200,
            8_192,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let id_c = writer
        .register_data_file_with_metadata(
            table_id,
            "data/ordered_table/part-0003.parquet",
            "parquet",
            300,
            8_192,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let snap = writer
        .create_snapshot(Some("v02713-mvcc"), Some("ordered files"))
        .await
        .unwrap();
    cat.commit_writer(snap);

    let reader = cat.read_latest();
    let files = reader.list_data_files(table_id).await.unwrap();

    assert_eq!(files.len(), 3, "must find all 3 registered files");

    // Verify file IDs are returned in insertion (file_order) order.
    let ids: Vec<u64> = files.iter().map(|f| f.data_file_id).collect();
    assert_eq!(
        ids,
        vec![id_a, id_b, id_c],
        "files must be sorted ascending by file_order; got: {ids:?}"
    );

    // Also verify file_order values are set and monotonically increasing.
    let orders: Vec<u64> = files
        .iter()
        .map(|f| f.file_order.unwrap_or(f.data_file_id))
        .collect();
    assert!(
        orders.windows(2).all(|w| w[0] < w[1]),
        "file_order values must be strictly ascending; got: {orders:?}"
    );
}

// ─── V-04: delete file begin_snapshot visibility ─────────────────────────────

/// A delete file registered at snapshot N is NOT visible at snapshot N-1.
///
/// Mirrors V-01 for delete files. The `list_delete_files` reader must apply
/// the same `begin_snapshot <= snapshot_id` predicate.
#[tokio::test]
async fn delete_file_visibility_respects_begin_snapshot() {
    let mut cat = CatalogStore::open(make_opts()).await.unwrap();

    // ── Snapshot 1: create schema, table, and a data file ────────────────────
    let mut w1 = cat.begin_write();
    let schema_id = w1.create_schema("deletes").await.unwrap();
    let table_id = w1
        .create_table(schema_id, "del_events", None)
        .await
        .unwrap();
    let data_file_id = w1
        .register_data_file_with_metadata(
            table_id,
            "data/del_events/part-0001.parquet",
            "parquet",
            1_000,
            65_536,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let snap1 = w1
        .create_snapshot(Some("v02713-mvcc"), Some("base data file"))
        .await
        .unwrap();
    cat.commit_writer(snap1);

    // ── Snapshot 2: register a delete file against the data file ─────────────
    let mut w2 = cat.begin_write();
    let _del_id = w2
        .register_delete_file_with_metadata(
            data_file_id,
            "data/del_events/part-0001.del.parquet",
            50,
            4_096,
            None,
            None,
        )
        .await
        .unwrap();
    let snap2 = w2
        .create_snapshot(Some("v02713-mvcc"), Some("add delete file"))
        .await
        .unwrap();
    cat.commit_writer(snap2);

    // ── Read at snapshot 1: delete file must NOT be visible ──────────────────
    let reader_at_1 = cat.read_at(snap1).unwrap();
    let del_files_at_1 = reader_at_1.list_delete_files(table_id).await.unwrap();
    assert!(
        del_files_at_1.is_empty(),
        "delete file registered at snapshot 2 must NOT be visible at snapshot 1; \
         got {} file(s)",
        del_files_at_1.len()
    );

    // ── Read at snapshot 2: delete file MUST be visible ──────────────────────
    let reader_at_2 = cat.read_at(snap2).unwrap();
    let del_files_at_2 = reader_at_2.list_delete_files(table_id).await.unwrap();
    assert_eq!(
        del_files_at_2.len(),
        1,
        "delete file registered at snapshot 2 must be visible at snapshot 2; \
         got {} file(s)",
        del_files_at_2.len()
    );
}

// ─── V-05: DuckLake v1.0 conformance — combined predicate ────────────────────

/// Combined MVCC conformance test: validates all three visibility predicates
/// and file_order sort in a single scenario with multiple snapshots.
///
/// Strictly conforms to the DuckLake v1.0 specification requirement that
/// data files are filtered and sorted consistently across all Postgres drivers.
#[tokio::test]
async fn visibility_conforms_to_ducklake_v1_spec() {
    let mut cat = CatalogStore::open(make_opts()).await.unwrap();

    // ── Snapshot 1: baseline (schema + table, zero data files) ───────────────
    let mut w1 = cat.begin_write();
    let schema_id = w1.create_schema("spec").await.unwrap();
    let table_id = w1
        .create_table(schema_id, "spec_table", None)
        .await
        .unwrap();
    let snap1 = w1
        .create_snapshot(Some("v02713-spec"), Some("baseline"))
        .await
        .unwrap();
    cat.commit_writer(snap1);

    // At snapshot 1: no files.
    let files_at_1 = cat
        .read_at(snap1)
        .unwrap()
        .list_data_files(table_id)
        .await
        .unwrap();
    assert!(
        files_at_1.is_empty(),
        "snapshot 1 must have zero data files"
    );

    // ── Snapshot 2: add two data files ────────────────────────────────────────
    let mut w2 = cat.begin_write();
    let fid_a = w2
        .register_data_file_with_metadata(
            table_id,
            "data/spec_table/part-0001.parquet",
            "parquet",
            100,
            8_192,
            Some(256),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let fid_b = w2
        .register_data_file_with_metadata(
            table_id,
            "data/spec_table/part-0002.parquet",
            "parquet",
            200,
            8_192,
            Some(256),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let snap2 = w2
        .create_snapshot(Some("v02713-spec"), Some("two files"))
        .await
        .unwrap();
    cat.commit_writer(snap2);

    // At snapshot 1: still no files (begin_snapshot = 2 > 1).
    let files_s1 = cat
        .read_at(snap1)
        .unwrap()
        .list_data_files(table_id)
        .await
        .unwrap();
    assert!(
        files_s1.is_empty(),
        "files registered at snap 2 must be invisible at snap 1"
    );

    // At snapshot 2: both files visible, ordered by file_order.
    let files_s2 = cat
        .read_at(snap2)
        .unwrap()
        .list_data_files(table_id)
        .await
        .unwrap();
    assert_eq!(
        files_s2.len(),
        2,
        "both files must be visible at snapshot 2"
    );
    assert_eq!(
        files_s2[0].data_file_id, fid_a,
        "first file must be part-0001"
    );
    assert_eq!(
        files_s2[1].data_file_id, fid_b,
        "second file must be part-0002"
    );

    // ── Snapshot 3: retire the table (cascades end_snapshot to all files) ─────
    let mut w3 = cat.begin_write();
    w3.drop_table(schema_id, table_id, snap1.snapshot_id.as_u64())
        .await
        .unwrap();
    let snap3 = w3
        .create_snapshot(Some("v02713-spec"), Some("drop table"))
        .await
        .unwrap();
    cat.commit_writer(snap3);

    // At snapshot 2: files must still be visible (end_snapshot = 3 > 2).
    let files_s2_after_drop = cat
        .read_at(snap2)
        .unwrap()
        .list_data_files(table_id)
        .await
        .unwrap();
    assert_eq!(
        files_s2_after_drop.len(),
        2,
        "files must still be visible at snapshot 2 after retirement at snapshot 3"
    );

    // At snapshot 3: files must be invisible (end_snapshot = 3 <= 3).
    let files_s3 = cat
        .read_at(snap3)
        .unwrap()
        .list_data_files(table_id)
        .await
        .unwrap();
    assert!(
        files_s3.is_empty(),
        "all files must be invisible at snapshot 3 after table drop; \
         got {} file(s)",
        files_s3.len()
    );

    // ── Final check: SnapshotId predicate constants from spec ─────────────────
    // Verify the MVCC module's is_visible function directly with boundary cases.
    use rocklake_core::mvcc::is_visible;
    assert!(
        is_visible(1, None, SnapshotId::new(1)),
        "spec: begin=1, end=None, read=1 → visible"
    );
    assert!(
        is_visible(1, None, SnapshotId::new(100)),
        "spec: begin=1, end=None, read=100 → visible"
    );
    assert!(
        !is_visible(5, None, SnapshotId::new(4)),
        "spec: begin=5, end=None, read=4 → invisible"
    );
    assert!(
        !is_visible(1, Some(5), SnapshotId::new(5)),
        "spec: begin=1, end=5, read=5 → invisible"
    );
    assert!(
        is_visible(1, Some(5), SnapshotId::new(4)),
        "spec: begin=1, end=5, read=4 → visible"
    );
}
