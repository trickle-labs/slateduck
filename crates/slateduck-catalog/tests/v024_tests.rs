//! v0.24 conformance tests: DuckLake v1.0 schema harness and interop-critical field fixes.
//!
//! Covers all 6 phases from the v0.24 roadmap:
//!   Phase 0 -- Conformance manifest parsing
//!   Phase 1 -- Snapshot and SnapshotChanges schema
//!   Phase 2 -- Spec-complete data file model
//!   Phase 3 -- Spec-complete delete file model
//!   Phase 4 -- Row ID tracking and table stats
//!   Phase 5 -- DROP TABLE cascade retirement

use object_store::path::Path as ObjectPath;
use slateduck_catalog::{CatalogStore, OpenOptions};
use slateduck_core::rows::*;
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

// --- Phase 0: Conformance Manifest -------------------------------------------

#[test]
fn conformance_manifest_exists_and_has_28_tables() {
    let manifest_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/ducklake-v1.0-schema.toml"
    );
    let content = std::fs::read_to_string(manifest_path)
        .expect("ducklake-v1.0-schema.toml must exist in tests/fixtures/");
    let table_count = content.matches("[[table]]").count();
    assert_eq!(
        table_count, 28,
        "Manifest must define exactly 28 tables, found {}",
        table_count
    );
}

#[test]
fn conformance_manifest_contains_required_table_names() {
    let manifest_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/ducklake-v1.0-schema.toml"
    );
    let content = std::fs::read_to_string(manifest_path).unwrap();
    let required = [
        "ducklake_snapshot",
        "ducklake_snapshot_changes",
        "ducklake_schema",
        "ducklake_table",
        "ducklake_column",
        "ducklake_data_file",
        "ducklake_delete_file",
        "ducklake_table_stats",
        "ducklake_metadata",
        "ducklake_tag",
        "ducklake_column_tag",
        "ducklake_file_column_stats",
        "ducklake_file_partition_value",
        "ducklake_partition_info",
        "ducklake_sort_info",
        "ducklake_column_mapping",
        "ducklake_column_name_mapping",
        "ducklake_view",
        "ducklake_macro",
        "ducklake_macro_impl",
        "ducklake_schema_version",
        "ducklake_inlined_data_table",
        "ducklake_inlined_data_rows",
        "ducklake_files_scheduled_for_deletion",
        "ducklake_file_variant_stats",
        "ducklake_geo_stats",
        "ducklake_secret",
        "ducklake_encryption_key",
    ];
    for name in &required {
        assert!(
            content.contains(name),
            "Manifest missing required table: {}",
            name
        );
    }
}

#[test]
fn conformance_manifest_data_file_has_v024_fields() {
    let manifest_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/ducklake-v1.0-schema.toml"
    );
    let content = std::fs::read_to_string(manifest_path).unwrap();
    for field in &[
        "record_count",
        "file_order",
        "path_is_relative",
        "row_id_start",
        "begin_snapshot",
        "end_snapshot",
        "mapping_id",
        "partial_max",
    ] {
        assert!(
            content.contains(field),
            "Manifest missing v0.24 data_file field: {}",
            field
        );
    }
}

// --- Phase 1: Snapshot and SnapshotChanges Schema ----------------------------

#[test]
fn snapshot_row_has_v024_fields() {
    let row = SnapshotRow {
        snapshot_id: 1,
        schema_version: 1,
        snapshot_time: "2025-01-01T00:00:00Z".to_string(),
        author: None,
        message: None,
        next_catalog_id: Some(42),
        next_file_id: Some(7),
    };
    assert_eq!(row.next_catalog_id, Some(42));
    assert_eq!(row.next_file_id, Some(7));
}

#[test]
fn snapshot_changes_row_has_v024_fields() {
    let row = SnapshotChangesRow {
        snapshot_id: 1,
        change_type: "inserted_into_table".to_string(),
        change_info: Some("42".to_string()),
        schema_id: None,
        table_id: Some(42),
        author: Some("alice".to_string()),
        commit_message: Some("load data".to_string()),
        commit_extra_info: None,
        changes_made: Some("inserted_into_table:42".to_string()),
    };
    assert_eq!(row.author.as_deref(), Some("alice"));
    assert_eq!(row.commit_message.as_deref(), Some("load data"));
    assert_eq!(row.changes_made.as_deref(), Some("inserted_into_table:42"));
}

#[tokio::test]
async fn snapshot_persists_next_catalog_and_file_ids() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut w = store.begin_write();
    w.create_schema("myschema").await.unwrap();
    let snap = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);
    let row = store
        .read_at(snap)
        .unwrap()
        .get_snapshot()
        .await
        .unwrap()
        .expect("snapshot must exist");
    assert!(
        row.next_catalog_id.unwrap_or(0) > 0,
        "next_catalog_id must be populated"
    );
    assert!(
        row.next_file_id.unwrap_or(0) > 0,
        "next_file_id must be populated"
    );
}

// --- Phase 2: Data File Model ------------------------------------------------

#[test]
fn data_file_row_uses_begin_snapshot_not_snapshot_id() {
    let row = DataFileRow {
        data_file_id: 1,
        table_id: 10,
        path: "s3://bucket/file.parquet".to_string(),
        file_format: "parquet".to_string(),
        record_count: 1000,
        file_size_bytes: 5_000_000,
        footer_size: None,
        encryption_key: None,
        begin_snapshot: Some(3),
        end_snapshot: None,
        file_order: Some(1),
        path_is_relative: Some(false),
        row_id_start: Some(0),
        partition_id: None,
        mapping_id: None,
        partial_max: None,
    };
    assert_eq!(row.begin_snapshot, Some(3));
    assert_eq!(row.record_count, 1000);
    assert_eq!(row.file_order, Some(1));
    assert_eq!(row.path_is_relative, Some(false));
    assert_eq!(row.row_id_start, Some(0));
}

#[tokio::test]
async fn data_file_mvcc_visibility_and_file_order() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let table_id = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        let _cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(_cr);
        tid
    };
    {
        let mut w = store.begin_write();
        w.register_data_file(table_id, "s3://b/a.parquet", "parquet", 100, 1000)
            .await
            .unwrap();
        let _cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(_cr);
    }
    {
        let mut w = store.begin_write();
        w.register_data_file(table_id, "s3://b/b.parquet", "parquet", 200, 2000)
            .await
            .unwrap();
        let _cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(_cr);
    }
    let snap = store.read_latest().snapshot_id();
    let files = store
        .read_at(snap)
        .unwrap()
        .list_data_files(table_id)
        .await
        .unwrap();
    assert_eq!(files.len(), 2, "both files visible at latest snapshot");
    if files.len() == 2 {
        let ord_0 = files[0].file_order.unwrap_or(0);
        let ord_1 = files[1].file_order.unwrap_or(0);
        assert!(ord_0 <= ord_1, "files must be ordered by file_order");
    }
}

// --- Phase 3: Delete File Model ----------------------------------------------

#[test]
fn delete_file_row_has_v024_fields() {
    let row = DeleteFileRow {
        delete_file_id: 1,
        data_file_id: 10,
        path: "s3://bucket/delete.parquet".to_string(),
        delete_count: 500,
        file_size_bytes: 100_000,
        snapshot_id: 0,
        table_id: Some(42),
        begin_snapshot: Some(3),
        end_snapshot: None,
        path_is_relative: Some(false),
        format: Some("parquet".to_string()),
        footer_size: None,
        partial_max: None,
    };
    assert_eq!(row.delete_count, 500);
    assert_eq!(row.table_id, Some(42));
    assert_eq!(row.begin_snapshot, Some(3));
    assert_eq!(row.format.as_deref(), Some("parquet"));
    assert_eq!(row.path_is_relative, Some(false));
}

#[tokio::test]
async fn delete_file_registration_and_list() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let table_id = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        let fid = w
            .register_data_file(tid, "s3://b/data.parquet", "parquet", 1000, 10000)
            .await
            .unwrap();
        w.register_delete_file(fid, "s3://b/delete.parquet", 50, 500)
            .await
            .unwrap();
        let _cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(_cr);
        tid
    };
    let snap = store.read_latest().snapshot_id();
    // list_delete_files API must not panic
    let _files = store
        .read_at(snap)
        .unwrap()
        .list_delete_files(table_id)
        .await
        .unwrap();
}

// --- Phase 4: Row ID Tracking and Table Stats --------------------------------

#[test]
fn table_stats_row_has_v024_fields() {
    let row = TableStatsRow {
        table_id: 1,
        record_count: 200,
        internal_file_count: 2,
        file_size_bytes: 3_000_000,
        next_row_id: Some(200),
    };
    assert_eq!(row.record_count, 200);
    assert_eq!(row.file_size_bytes, 3_000_000);
    assert_eq!(row.next_row_id, Some(200));
}

#[tokio::test]
async fn data_file_registration_tracks_next_row_id() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let table_id = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        let _cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(_cr);
        tid
    };
    {
        let mut w = store.begin_write();
        w.register_data_file(table_id, "s3://b/f1.parquet", "parquet", 100, 1000)
            .await
            .unwrap();
        // update_table_stats accumulates incremental deltas (matching DuckLake batch protocol).
        w.update_table_stats(table_id, 100, 1, 1000).await.unwrap();
        let _cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(_cr);
    }
    {
        let mut w = store.begin_write();
        w.register_data_file(table_id, "s3://b/f2.parquet", "parquet", 50, 500)
            .await
            .unwrap();
        // update_table_stats takes incremental deltas per DuckLake batch protocol.
        // Second batch: 50 new records, 1 new file, 500 new bytes.
        w.update_table_stats(table_id, 50, 1, 500).await.unwrap();
        let _cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(_cr);
    }
    let snap = store.read_latest().snapshot_id();
    let stats = store
        .read_at(snap)
        .unwrap()
        .get_table_stats(table_id)
        .await
        .unwrap();
    let stats = stats.expect("table stats must exist");
    assert_eq!(
        stats.record_count, 150,
        "record_count must be 150 (100 + 50 accumulated deltas)"
    );
    assert!(
        stats.next_row_id.unwrap_or(0) > 0,
        "next_row_id must be populated"
    );
}

// --- Phase 5: DROP TABLE Cascade Retirement ----------------------------------

#[tokio::test]
async fn drop_table_makes_table_invisible() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let (schema_id, table_id, snap_before) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        w.add_column(tid, "id", "BIGINT", 0, false, None)
            .await
            .unwrap();
        w.add_column(tid, "name", "VARCHAR", 1, true, None)
            .await
            .unwrap();
        let snap = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(snap);
        (sid, tid, snap)
    };
    let snap_after = {
        let mut w = store.begin_write();
        w.drop_table(schema_id, table_id, snap_before.as_u64())
            .await
            .unwrap();
        let snap = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(snap);
        snap
    };
    let before = store
        .read_at(snap_before)
        .unwrap()
        .describe_table(table_id)
        .await
        .unwrap();
    assert!(before.is_some(), "table must be visible before drop");
    let after = store
        .read_at(snap_after)
        .unwrap()
        .describe_table(table_id)
        .await
        .unwrap();
    assert!(after.is_none(), "table must be invisible at drop snapshot");
}

#[tokio::test]
async fn drop_table_retires_data_files() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let (schema_id, table_id, snap_before) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("s").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        w.register_data_file(tid, "s3://b/file.parquet", "parquet", 100, 1000)
            .await
            .unwrap();
        let snap = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(snap);
        (sid, tid, snap)
    };
    let snap_after = {
        let mut w = store.begin_write();
        w.drop_table(schema_id, table_id, snap_before.as_u64())
            .await
            .unwrap();
        let snap = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(snap);
        snap
    };
    let files_before = store
        .read_at(snap_before)
        .unwrap()
        .list_data_files(table_id)
        .await
        .unwrap();
    assert_eq!(files_before.len(), 1, "data file visible before drop");
    let files_after = store
        .read_at(snap_after)
        .unwrap()
        .list_data_files(table_id)
        .await
        .unwrap();
    assert_eq!(
        files_after.len(),
        0,
        "data file must be retired after DROP TABLE cascade"
    );
}
