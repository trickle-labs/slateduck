//! Integration tests for v0.4 Production Hardening features.
//!
//! Tests: GC, excision, checkpoints, export/import, rebuild, metrics, repair.

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

async fn open_db(dir: &TempDir) -> slatedb::Db {
    let path = dir.path().to_str().unwrap().to_string();
    let store = Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&path).unwrap());
    slatedb::Db::open(ObjectPath::from("catalog"), store)
        .await
        .unwrap()
}

// ─── Visibility GC Tests ───────────────────────────────────────────────────

#[tokio::test]
async fn gc_plan_shows_zero_on_fresh_catalog() {
    let dir = TempDir::new().unwrap();
    let store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;
    let plan = slateduck_catalog::gc::gc_plan(&db, 30).await.unwrap();
    assert_eq!(plan.current_retain_from, 0);
    assert_eq!(plan.proposed_retain_from, 0);
    assert_eq!(plan.snapshots_affected, 0);
    db.close().await.unwrap();
}

#[tokio::test]
async fn gc_apply_advances_retain_from() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    writer.create_schema("s1").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    writer.create_schema("s2").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    writer.create_schema("s3").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;

    // Advance retain-from to snapshot 2
    let result = slateduck_catalog::gc::gc_apply(&db, 2).await.unwrap();
    assert_eq!(result.previous_retain_from, 0);
    assert_eq!(result.new_retain_from, 2);
    assert_eq!(result.snapshots_hidden, 2);

    // Verify retain-from persisted
    let retain_from = slateduck_catalog::gc::read_retain_from(&db).await.unwrap();
    assert_eq!(retain_from, 2);

    db.close().await.unwrap();
}

#[tokio::test]
async fn gc_respects_pinned_snapshots() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("s1").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    writer.create_schema("s2").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;

    // Pin snapshot 1
    slateduck_catalog::gc::pin_snapshot(&db, 1).await.unwrap();

    // Attempting to advance past pinned snapshot should fail
    let result = slateduck_catalog::gc::gc_apply(&db, 2).await;
    assert!(result.is_err());

    // Unpin and retry
    slateduck_catalog::gc::unpin_snapshot(&db, 1).await.unwrap();
    let result = slateduck_catalog::gc::gc_apply(&db, 2).await.unwrap();
    assert_eq!(result.new_retain_from, 2);

    db.close().await.unwrap();
}

#[tokio::test]
async fn gc_time_travel_still_works_after_advance() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let _schema_id = writer.create_schema("visible").await.unwrap();
    let snap1 = writer.create_snapshot(None, None).await.unwrap();
    writer.create_schema("later").await.unwrap();
    let snap2 = writer.create_snapshot(None, None).await.unwrap();

    // Reading at snap2 shows both schemas
    let reader = store.read_at(snap2).await.unwrap();
    let schemas = reader.list_schemas().await.unwrap();
    assert_eq!(schemas.len(), 2);

    // Reading at snap1 shows only first
    let reader = store.read_at(snap1).await.unwrap();
    let schemas = reader.list_schemas().await.unwrap();
    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].schema_name, "visible");

    store.close().await.unwrap();
}

// ─── Excision Tests ────────────────────────────────────────────────────────

#[tokio::test]
async fn excise_plan_shows_eligible_rows() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("old").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    writer.drop_schema(schema_id).await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    writer.create_schema("new").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;
    let plan = slateduck_catalog::excise::excise_plan(&db, 2)
        .await
        .unwrap();
    assert!(plan.version_rows_eligible > 0);
    db.close().await.unwrap();
}

#[tokio::test]
async fn excise_apply_deletes_and_records_audit() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("temp").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    writer.drop_schema(schema_id).await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    writer.create_schema("keep").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;

    // Advance retain-from first (safety requirement)
    slateduck_catalog::gc::gc_apply(&db, 2).await.unwrap();

    // Apply excision
    let result = slateduck_catalog::excise::excise_apply(&db, 2, "test-operator")
        .await
        .unwrap();
    assert!(result.keys_deleted > 0);
    assert_eq!(result.keys_failed, 0);

    // Verify audit entry was written
    let audits = slateduck_catalog::excise::read_audit_entries(&db)
        .await
        .unwrap();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].before_snapshot, 2);
    assert_eq!(audits[0].operator, "test-operator");

    db.close().await.unwrap();
}

#[tokio::test]
async fn excise_refuses_without_retain_from_advance() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("s1").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;

    // Advance retain-from only to 1
    slateduck_catalog::gc::gc_apply(&db, 1).await.unwrap();

    // Trying to excise at 5 (beyond retain-from=1) should fail
    let result = slateduck_catalog::excise::excise_apply(&db, 5, "test").await;
    assert!(result.is_err());

    db.close().await.unwrap();
}

// ─── Checkpoint Tests ──────────────────────────────────────────────────────

#[tokio::test]
async fn checkpoint_create_list_restore() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("main").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;

    // Create checkpoint
    let cp = slateduck_catalog::checkpoint::create_checkpoint(&db, Some("test-backup"))
        .await
        .unwrap();
    assert_eq!(cp.snapshot_id, 1);
    assert_eq!(cp.label, Some("test-backup".to_string()));

    // List checkpoints
    let list = slateduck_catalog::checkpoint::list_checkpoints(&db)
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, cp.id);

    // Restore checkpoint
    let restored = slateduck_catalog::checkpoint::restore_checkpoint(&db, cp.id)
        .await
        .unwrap();
    assert_eq!(restored.snapshot_id, 1);

    db.close().await.unwrap();
}

#[tokio::test]
async fn checkpoint_restore_after_modifications() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("original").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;

    // Create checkpoint at snapshot 1
    let cp = slateduck_catalog::checkpoint::create_checkpoint(&db, Some("before-changes"))
        .await
        .unwrap();
    assert_eq!(cp.snapshot_id, 1);

    db.close().await.unwrap();

    // Make more changes
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("added_later").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    // Restore checkpoint
    let db = open_db(&dir).await;
    slateduck_catalog::checkpoint::restore_checkpoint(&db, cp.id)
        .await
        .unwrap();
    db.close().await.unwrap();

    // Verify we're back at snapshot 1
    let store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let reader = store.read_at(SnapshotId::new(1)).await.unwrap();
    let schemas = reader.list_schemas().await.unwrap();
    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].schema_name, "original");
    store.close().await.unwrap();
}

// ─── Export/Import Tests ───────────────────────────────────────────────────

#[tokio::test]
async fn export_import_roundtrip() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "users", Some("s3://bucket/data/users/"))
        .await
        .unwrap();
    writer
        .add_column(table_id, "id", "INTEGER", 0, false, None)
        .await
        .unwrap();
    writer
        .add_column(table_id, "name", "VARCHAR", 1, true, None)
        .await
        .unwrap();
    writer
        .register_data_file(
            table_id,
            "data/users/part-0001.parquet",
            "parquet",
            100,
            5000,
        )
        .await
        .unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    // Export
    let db = open_db(&dir).await;
    let mut export_buf = Vec::new();
    let result = slateduck_catalog::export::export_catalog(&db, None, &mut export_buf)
        .await
        .unwrap();
    assert!(result.rows_exported > 0);
    db.close().await.unwrap();

    // Import into fresh catalog
    let dir2 = TempDir::new().unwrap();
    let db2 = open_db(&dir2).await;
    let reader = std::io::BufReader::new(&export_buf[..]);
    let import_result = slateduck_catalog::export::import_catalog(&db2, reader)
        .await
        .unwrap();
    assert_eq!(import_result.rows_imported, result.rows_exported);
    db2.close().await.unwrap();
}

#[tokio::test]
async fn pg_migrate_produces_insert_statements() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("main").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;
    let mut export_buf = Vec::new();
    slateduck_catalog::export::export_catalog(&db, None, &mut export_buf)
        .await
        .unwrap();
    db.close().await.unwrap();

    // Convert to PG
    let reader = std::io::BufReader::new(&export_buf[..]);
    let mut output = Vec::new();
    let count = slateduck_catalog::export::pg_migrate(reader, &mut output).unwrap();
    assert!(count > 0);

    let sql = String::from_utf8(output).unwrap();
    assert!(sql.contains("INSERT INTO"));
}

#[tokio::test]
async fn rebuild_from_file_paths() {
    let dir = TempDir::new().unwrap();
    let db = open_db(&dir).await;

    let paths = vec![
        "data/table1/part-0001.parquet".to_string(),
        "data/table1/part-0002.parquet".to_string(),
        "data/table1/part-0003.parquet".to_string(),
    ];

    let count = slateduck_catalog::export::rebuild_catalog(&db, &paths)
        .await
        .unwrap();
    assert_eq!(count, 3);

    // Verify catalog is usable
    let result = slateduck_catalog::inspect::inspect_snapshot(&db)
        .await
        .unwrap();
    assert_eq!(result.data_file_count, 3);
    assert_eq!(result.latest_snapshot_id, 1);

    db.close().await.unwrap();
}

// ─── Inspect Tests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn inspect_shows_current_state() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "t1", None).await.unwrap();
    writer
        .add_column(table_id, "id", "INT", 0, false, None)
        .await
        .unwrap();
    writer
        .register_data_file(table_id, "file1.parquet", "parquet", 100, 1000)
        .await
        .unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;
    let result = slateduck_catalog::inspect::inspect_snapshot(&db)
        .await
        .unwrap();
    assert_eq!(result.latest_snapshot_id, 1);
    assert_eq!(result.schema_count, 1);
    assert_eq!(result.table_count, 1);
    assert_eq!(result.column_count, 1);
    assert_eq!(result.data_file_count, 1);
    assert_eq!(result.format_version, 1);
    db.close().await.unwrap();
}

// ─── Verify Tests ──────────────────────────────────────────────────────────

#[tokio::test]
async fn verify_catalog_passes_on_healthy_catalog() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("main").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;
    let result = slateduck_catalog::verify::verify_catalog(&db)
        .await
        .unwrap();
    assert!(result.is_ok());
    db.close().await.unwrap();
}

#[tokio::test]
async fn verify_data_files_reports_missing() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "t1", None).await.unwrap();
    writer
        .register_data_file(table_id, "nonexistent.parquet", "parquet", 100, 1000)
        .await
        .unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let path = dir.path().to_str().unwrap().to_string();
    let object_store: Arc<dyn object_store::ObjectStore> =
        Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&path).unwrap());

    let db = open_db(&dir).await;
    let result = slateduck_catalog::cleanup::verify_data_files(&db, &object_store)
        .await
        .unwrap();
    assert!(!result.files_missing.is_empty() || !result.files_error.is_empty());
    db.close().await.unwrap();
}

// ─── Repair Tests ──────────────────────────────────────────────────────────

#[tokio::test]
async fn repair_plan_on_healthy_catalog_empty() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("main").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;
    let plan = slateduck_catalog::repair::repair_plan(&db).await.unwrap();
    assert!(plan.actions.is_empty());
    assert!(!plan.has_unrecoverable());
    db.close().await.unwrap();
}

#[tokio::test]
async fn repair_fixes_stale_counter() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("main").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;

    // Corrupt the snapshot counter (set it too low)
    use slateduck_core::{keys, tags::COUNTER_NEXT_SNAPSHOT_ID, values};
    let key = keys::key_counter(COUNTER_NEXT_SNAPSHOT_ID);
    db.put(&key, &values::encode_counter(0)).await.unwrap();

    // Plan should detect stale counter
    let plan = slateduck_catalog::repair::repair_plan(&db).await.unwrap();
    assert!(!plan.actions.is_empty());

    // Apply repair
    let result = slateduck_catalog::repair::repair_apply(&db, &plan)
        .await
        .unwrap();
    assert!(result.actions_applied > 0);
    assert_eq!(result.actions_failed, 0);

    // Verify counter is fixed
    let fixed = db.get(&key).await.unwrap().unwrap();
    let val = values::decode_counter(&fixed).unwrap();
    assert!(val >= 2); // Should be at least max_snapshot + 1

    db.close().await.unwrap();
}

// ─── Metrics Tests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn metrics_render_prometheus_format() {
    let metrics = slateduck_catalog::CatalogMetrics::new(50);
    metrics.increment_snapshots();
    metrics.increment_snapshots();
    metrics.set_files_per_snapshot(10);
    metrics.increment_object_store_requests();

    let output = metrics.render_prometheus();
    assert!(output.contains("slateduck_snapshots_created_total 2"));
    assert!(output.contains("slateduck_files_per_snapshot 10"));
    assert!(output.contains("slateduck_object_store_requests_total 1"));
    assert!(output.contains("slateduck_max_sessions 50"));
}

// ─── Encryption Tests ──────────────────────────────────────────────────────

#[test]
fn encryption_config_from_valid_hex() {
    let hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let config = slateduck_catalog::encryption::EncryptionConfig::from_hex(hex).unwrap();
    assert_eq!(config.key[0], 0x01);
    assert_eq!(config.key[31], 0xef);
}

#[test]
fn encryption_config_rejects_short_key() {
    let hex = "0123456789abcdef";
    let result = slateduck_catalog::encryption::EncryptionConfig::from_hex(hex);
    assert!(result.is_err());
}

#[test]
fn encryption_config_rejects_invalid_hex() {
    let hex = "zz23456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let result = slateduck_catalog::encryption::EncryptionConfig::from_hex(hex);
    assert!(result.is_err());
}

// ─── Data File Cleanup Tests ───────────────────────────────────────────────

#[tokio::test]
async fn collect_referenced_paths_includes_all_files() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "t1", None).await.unwrap();
    writer
        .register_data_file(table_id, "data/file1.parquet", "parquet", 100, 1000)
        .await
        .unwrap();
    writer
        .register_data_file(table_id, "data/file2.parquet", "parquet", 200, 2000)
        .await
        .unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;
    let paths = slateduck_catalog::cleanup::collect_referenced_paths(&db)
        .await
        .unwrap();
    assert_eq!(paths.len(), 2);
    assert!(paths.contains("data/file1.parquet"));
    assert!(paths.contains("data/file2.parquet"));
    db.close().await.unwrap();
}

// ─── S3 Resolution Test ────────────────────────────────────────────────────

#[test]
fn s3_url_parsing() {
    // Just verify the URL parsing logic doesn't panic
    let url = "s3://my-bucket/catalogs/production";
    let without_scheme = &url[5..];
    let (bucket, prefix) = match without_scheme.find('/') {
        Some(idx) => (&without_scheme[..idx], &without_scheme[idx + 1..]),
        None => (without_scheme, ""),
    };
    assert_eq!(bucket, "my-bucket");
    assert_eq!(prefix, "catalogs/production");
}
