//! v0.41.0 — Migration Tooling & DuckLake Forward Compatibility tests.
//!
//! Test inventory (6 tests):
//! 1. migrate_from_in_memory_source
//! 2. migrate_dry_run_writes_nothing
//! 3. migrate_secondary_index_present
//! 4. migrate_v11_source_rejected_by_default
//! 5. migrate_v11_source_accepted_with_flag
//! 6. export_import_delete_file_end_snapshot

use std::io::BufReader;
use std::sync::Arc;

use object_store::memory::InMemory;
use object_store::path::Path as ObjectPath;

use rocklake_catalog::export::{export_catalog, import_catalog, ExportedRow};
use rocklake_catalog::migrate_from_ducklake::{
    migrate_from_source, InMemoryDuckLakeSource, ACCEPT_VERSION_V1_1_DEV_1,
    DUCKLAKE_V1_0_CATALOG_VERSION, DUCKLAKE_V1_1_DEV_1_CATALOG_VERSION,
};
use rocklake_catalog::CatalogError;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn fresh_db() -> slatedb::Db {
    let store = Arc::new(InMemory::new()) as Arc<dyn object_store::ObjectStore>;
    slatedb::Db::builder(ObjectPath::from("catalog"), store)
        .build()
        .await
        .unwrap()
}

fn snapshot_row(snapshot_id: u64, schema_version: u64) -> ExportedRow {
    ExportedRow {
        table: "ducklake_snapshot".to_string(),
        data: serde_json::json!({
            "snapshot_id": snapshot_id,
            "schema_version": schema_version,
            "snapshot_time": "2024-01-01T00:00:00Z",
            "author": null,
            "message": null,
        }),
    }
}

fn data_file_row(table_id: u64, data_file_id: u64, begin_snapshot: u64) -> ExportedRow {
    ExportedRow {
        table: "ducklake_data_file".to_string(),
        data: serde_json::json!({
            "data_file_id": data_file_id,
            "table_id": table_id,
            "path": format!("s3://bucket/test/{data_file_id}.parquet"),
            "file_format": "parquet",
            "record_count": 100_u64,
            "file_size_bytes": 1024_u64,
            "begin_snapshot": begin_snapshot,
            "end_snapshot": null,
            "footer_size": null,
        }),
    }
}

// ---------------------------------------------------------------------------
// Test 1: migrate_from_in_memory_source
// ---------------------------------------------------------------------------

#[tokio::test]
async fn migrate_from_in_memory_source() {
    let db = fresh_db().await;

    let mut source = InMemoryDuckLakeSource::new(DUCKLAKE_V1_0_CATALOG_VERSION);
    source.add_rows("ducklake_snapshot", vec![snapshot_row(1, 7)]);
    source.add_rows("ducklake_data_file", vec![data_file_row(10, 100, 1)]);

    let report = migrate_from_source(&mut source, &db, &[], false)
        .await
        .unwrap();

    assert_eq!(report.source_catalog_version, DUCKLAKE_V1_0_CATALOG_VERSION);
    assert!(!report.dry_run);
    assert_eq!(report.data_file_count, 1, "expected one data file");
    assert!(
        report.total_migrated() >= 1,
        "expected at least one migrated row"
    );
    assert_eq!(report.total_skipped(), 0, "expected no skipped rows");
}

// ---------------------------------------------------------------------------
// Test 2: migrate_dry_run_writes_nothing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn migrate_dry_run_writes_nothing() {
    let db = fresh_db().await;

    let mut source = InMemoryDuckLakeSource::new(DUCKLAKE_V1_0_CATALOG_VERSION);
    source.add_rows("ducklake_snapshot", vec![snapshot_row(1, 7)]);
    source.add_rows("ducklake_data_file", vec![data_file_row(10, 100, 1)]);

    let report = migrate_from_source(&mut source, &db, &[], true)
        .await
        .unwrap();

    assert!(report.dry_run, "report must mark dry_run = true");
    assert_eq!(
        report.data_file_count, 1,
        "dry-run must still count data files"
    );

    let mut iter = db.scan::<&[u8], _>(std::ops::RangeFull).await.unwrap();
    let first = iter.next().await.unwrap();
    assert!(
        first.is_none(),
        "dry-run must not write any keys to SlateDB"
    );
}

// ---------------------------------------------------------------------------
// Test 3: migrate_secondary_index_present
// ---------------------------------------------------------------------------

#[tokio::test]
async fn migrate_secondary_index_present() {
    use rocklake_core::keys;

    let db = fresh_db().await;
    let table_id: u64 = 42;
    let file_id: u64 = 999;
    let begin_snapshot: u64 = 1;

    let mut source = InMemoryDuckLakeSource::new(DUCKLAKE_V1_0_CATALOG_VERSION);
    source.add_rows("ducklake_snapshot", vec![snapshot_row(1, 7)]);
    source.add_rows(
        "ducklake_data_file",
        vec![data_file_row(table_id, file_id, begin_snapshot)],
    );

    migrate_from_source(&mut source, &db, &[], false)
        .await
        .unwrap();

    let primary_val = db
        .get(&keys::key_data_file(table_id, file_id))
        .await
        .unwrap();
    assert!(
        primary_val.is_some(),
        "primary data file key must be present"
    );

    let idx_val = db
        .get(&keys::key_data_file_by_snapshot(
            table_id,
            begin_snapshot,
            file_id,
        ))
        .await
        .unwrap();
    assert!(idx_val.is_some(), "secondary index key must be present");

    assert_eq!(
        primary_val, idx_val,
        "primary and secondary must hold identical encoded values"
    );
}

// ---------------------------------------------------------------------------
// Test 4: migrate_v11_source_rejected_by_default
// ---------------------------------------------------------------------------

#[tokio::test]
async fn migrate_v11_source_rejected_by_default() {
    let db = fresh_db().await;

    let mut source = InMemoryDuckLakeSource::new(DUCKLAKE_V1_1_DEV_1_CATALOG_VERSION);
    source.add_rows("ducklake_snapshot", vec![snapshot_row(1, 8)]);

    let result = migrate_from_source(&mut source, &db, &[], false).await;

    match result {
        Err(CatalogError::UnsupportedDuckLakeVersion { version, .. }) => {
            assert_eq!(version, DUCKLAKE_V1_1_DEV_1_CATALOG_VERSION);
        }
        other => panic!("expected UnsupportedDuckLakeVersion, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Test 5: migrate_v11_source_accepted_with_flag
// ---------------------------------------------------------------------------

#[tokio::test]
async fn migrate_v11_source_accepted_with_flag() {
    let db = fresh_db().await;

    let mut source = InMemoryDuckLakeSource::new(DUCKLAKE_V1_1_DEV_1_CATALOG_VERSION);
    source.add_rows("ducklake_snapshot", vec![snapshot_row(1, 8)]);

    let report = migrate_from_source(&mut source, &db, &[ACCEPT_VERSION_V1_1_DEV_1], false)
        .await
        .unwrap();

    assert_eq!(
        report.source_catalog_version,
        DUCKLAKE_V1_1_DEV_1_CATALOG_VERSION
    );
}

// ---------------------------------------------------------------------------
// Test 6: export_import_delete_file_end_snapshot
// ---------------------------------------------------------------------------

/// Regression guard: export must include the `end_snapshot` field for delete files.
#[tokio::test]
async fn export_import_delete_file_end_snapshot() {
    let snap_ndjson = serde_json::json!({
        "table": "ducklake_snapshot",
        "data": {
            "snapshot_id": 1_u64,
            "schema_version": 7_u64,
            "snapshot_time": "2024-01-01T00:00:00Z",
            "author": null,
            "message": null
        }
    })
    .to_string();

    let df_ndjson = serde_json::json!({
        "table": "ducklake_data_file",
        "data": {
            "data_file_id": 1_u64,
            "table_id": 10_u64,
            "path": "s3://b/d.parquet",
            "file_format": "parquet",
            "record_count": 100_u64,
            "file_size_bytes": 1024_u64,
            "begin_snapshot": 1_u64,
            "end_snapshot": null,
            "footer_size": null
        }
    })
    .to_string();

    // Retire the delete file at snapshot 2 — end_snapshot must survive export.
    let del_ndjson = serde_json::json!({
        "table": "ducklake_delete_file",
        "data": {
            "delete_file_id": 1_u64,
            "data_file_id": 1_u64,
            "path": "s3://b/del.parquet",
            "delete_count": 5_u64,
            "file_size_bytes": 512_u64,
            "snapshot_id": 1_u64,
            "begin_snapshot": 1_u64,
            "end_snapshot": 2_u64
        }
    })
    .to_string();

    let ndjson = format!("{}\n{}\n{}\n", snap_ndjson, df_ndjson, del_ndjson);

    let db = fresh_db().await;
    let result = import_catalog(&db, BufReader::new(ndjson.as_bytes()))
        .await
        .unwrap();
    assert_eq!(result.rows_imported, 3, "all 3 rows must be imported");

    let mut buf = Vec::new();
    // Use snapshot_id=1 (the snapshot we imported) so the MVCC filter
    // includes rows visible at that snapshot, including the delete file.
    export_catalog(&db, Some(1), &mut buf).await.unwrap();
    let exported = String::from_utf8(buf).unwrap();

    assert!(
        exported.contains("\"end_snapshot\":2") || exported.contains("\"end_snapshot\": 2"),
        "exported NDJSON must contain end_snapshot=2; got:\n{}",
        exported
    );
}
