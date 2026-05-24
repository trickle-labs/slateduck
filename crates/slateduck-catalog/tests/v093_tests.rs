//! Integration tests for v0.9.3 Operational Safety features.
//!
//! Tests: retain-from enforcement, excision safety, checkpoint restore,
//!        typed import validation, rebuild_catalog, NaN pruning, pg_migrate escaping.

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

// ─── F-05: Retain-From Enforcement in Readers ─────────────────────────────

#[tokio::test]
async fn read_at_below_retain_from_returns_error() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("s1").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;
    // Advance retain-from to snapshot 5
    slateduck_catalog::gc::gc_apply(&db, 5).await.unwrap();
    db.close().await.unwrap();

    // read_at(3) should fail with SnapshotOutOfRetention
    let store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let result = store.read_at(SnapshotId::new(3));
    assert!(
        result.is_err(),
        "read_at below retain-from must return an error"
    );
    let err = result.err().unwrap().to_string();
    assert!(
        err.contains("22023"),
        "Error must mention SQLSTATE 22023, got: {err}"
    );
    store.close().await.unwrap();
}

#[tokio::test]
async fn read_at_at_retain_from_succeeds() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("s1").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;
    slateduck_catalog::gc::gc_apply(&db, 1).await.unwrap();
    db.close().await.unwrap();

    // read_at(1) == retain_from should succeed
    let store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let result = store.read_at(SnapshotId::new(1));
    assert!(
        result.is_ok(),
        "read_at at exactly retain-from must succeed"
    );
    store.close().await.unwrap();
}

#[tokio::test]
async fn read_at_before_any_gc_apply_succeeds() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("s1").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    // No gc_apply → retain_from == 0 → all reads allowed
    let store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let result = store.read_at(SnapshotId::new(1));
    assert!(
        result.is_ok(),
        "read_at with no retain-from set must succeed"
    );
    store.close().await.unwrap();
}

// ─── F-06: Excision Safety at retain_from == 0 ────────────────────────────

#[tokio::test]
async fn excise_apply_fails_when_retain_from_is_zero() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("s1").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;

    // retain_from is 0 (never set) — must refuse any excision
    let result = slateduck_catalog::excise::excise_apply(&db, 1, "operator").await;
    assert!(
        result.is_err(),
        "excise_apply must fail when retain_from == 0"
    );

    db.close().await.unwrap();
}

#[tokio::test]
async fn excise_plan_shows_unsafe_when_retain_from_is_zero() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("s1").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;

    // retain_from is 0 — plan should mark is_safe = false
    let plan = slateduck_catalog::excise::excise_plan(&db, 1)
        .await
        .unwrap();
    assert!(
        !plan.is_safe,
        "excise_plan must report is_safe=false when retain_from == 0"
    );

    db.close().await.unwrap();
}

// ─── F-07: Checkpoint Restore Prevents Snapshot ID Reuse ──────────────────

#[tokio::test]
async fn checkpoint_restore_hides_post_checkpoint_facts() {
    let dir = TempDir::new().unwrap();

    // Write initial facts and checkpoint
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("pre_checkpoint").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;
    let cp = slateduck_catalog::checkpoint::create_checkpoint(&db, Some("cp1"))
        .await
        .unwrap();
    db.close().await.unwrap();

    // Add more facts after checkpoint
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("post_checkpoint").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    // Restore to checkpoint
    let db = open_db(&dir).await;
    slateduck_catalog::checkpoint::restore_checkpoint(&db, cp.id)
        .await
        .unwrap();
    db.close().await.unwrap();

    // After restore: reading at checkpoint snapshot should see pre-checkpoint schema only
    let store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let reader = store.read_at(SnapshotId::new(cp.snapshot_id)).unwrap();
    let schemas = reader.list_schemas().await.unwrap();
    let names: Vec<_> = schemas.iter().map(|s| s.schema_name.as_str()).collect();
    assert!(
        names.contains(&"pre_checkpoint"),
        "pre-checkpoint schema must be visible"
    );
    assert!(
        !names.contains(&"post_checkpoint"),
        "post-checkpoint schema must be hidden after restore, got: {names:?}"
    );
    store.close().await.unwrap();
}

#[tokio::test]
async fn checkpoint_restore_new_writes_visible() {
    let dir = TempDir::new().unwrap();

    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("original").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;
    let cp = slateduck_catalog::checkpoint::create_checkpoint(&db, None)
        .await
        .unwrap();
    db.close().await.unwrap();

    // Add more facts, then restore
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("discarded").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;
    slateduck_catalog::checkpoint::restore_checkpoint(&db, cp.id)
        .await
        .unwrap();
    db.close().await.unwrap();

    // New writes after restore must succeed and be visible at the new snapshot
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("post_restore").await.unwrap();
    let new_snap = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&writer);
    let reader = store.read_at(new_snap).unwrap();
    let schemas = reader.list_schemas().await.unwrap();
    let names: Vec<_> = schemas.iter().map(|s| s.schema_name.as_str()).collect();
    assert!(
        names.contains(&"post_restore"),
        "post-restore schema must be visible, got: {names:?}"
    );
    assert!(
        !names.contains(&"discarded"),
        "discarded schema must remain hidden"
    );
    store.close().await.unwrap();
}

// ─── F-09: Typed Import Validation ────────────────────────────────────────

#[tokio::test]
async fn import_catalog_rejects_malformed_ndjson() {
    let dir = TempDir::new().unwrap();
    let db = open_db(&dir).await;

    // Missing required 'snapshot_id' field on a ducklake_snapshot row
    let bad_ndjson = r#"{"table":"ducklake_snapshot","data":{"schema_version":1,"snapshot_time":"2024-01-01T00:00:00Z"}}"#;

    let reader = std::io::BufReader::new(bad_ndjson.as_bytes());
    let result = slateduck_catalog::export::import_catalog(&db, reader).await;
    assert!(
        result.is_err(),
        "import must reject rows missing required fields"
    );
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("snapshot_id") || err.to_string().contains("import error"),
        "Error must mention missing field or 'import error', got: {err}"
    );

    db.close().await.unwrap();
}

#[tokio::test]
async fn import_catalog_rejects_invalid_base64() {
    let dir = TempDir::new().unwrap();
    let db = open_db(&dir).await;

    // Invalid base64 in payload field
    let bad_ndjson = r#"{"table":"ducklake_inlined_insert","data":{"table_id":1,"schema_version":1,"row_id":1,"payload":"!!!not_valid_base64!!!","begin_snapshot":1}}"#;

    let reader = std::io::BufReader::new(bad_ndjson.as_bytes());
    let result = slateduck_catalog::export::import_catalog(&db, reader).await;
    assert!(
        result.is_err(),
        "import must reject invalid base64 in payload"
    );
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("base64") || err.to_string().contains("import error"),
        "Error must mention base64 issue, got: {err}"
    );

    db.close().await.unwrap();
}

// ─── F-10: Rebuild Catalog Coherence ──────────────────────────────────────

#[tokio::test]
async fn rebuild_catalog_produces_queryable_catalog() {
    let dir = TempDir::new().unwrap();
    let db = open_db(&dir).await;

    let paths = vec![
        "s3://bucket/data/part-0001.parquet".to_string(),
        "s3://bucket/data/part-0002.parquet".to_string(),
    ];
    let count = slateduck_catalog::export::rebuild_catalog(&db, &paths)
        .await
        .unwrap();
    assert_eq!(count, 2);
    db.close().await.unwrap();

    // Open as store and verify we can query schemas, tables, and data files
    let store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let reader = store.read_at(SnapshotId::new(1)).unwrap();

    let schemas = reader.list_schemas().await.unwrap();
    assert_eq!(schemas.len(), 1, "rebuild must create exactly one schema");
    assert_eq!(schemas[0].schema_name, "main");

    let tables = reader.list_tables(schemas[0].schema_id).await.unwrap();
    assert_eq!(tables.len(), 1, "rebuild must create exactly one table");
    assert_eq!(tables[0].table_name, "default");

    let files = reader.list_data_files(tables[0].table_id).await.unwrap();
    assert_eq!(files.len(), 2, "rebuild must register all data files");

    store.close().await.unwrap();
}

// ─── F-07m: NaN Comparison in Pruning ─────────────────────────────────────

#[test]
fn nan_comparison_is_fail_closed() {
    use slateduck_core::types::{DuckLakeType, PruneResult, TypeCompareError};

    let float_type = DuckLakeType::Float { width_bits: 64 };

    // NaN min or max comparison → error from type_aware_compare
    let result = slateduck_core::types::type_aware_compare("5.0", "NaN", &float_type);
    assert_eq!(
        result,
        Err(TypeCompareError::NanComparison),
        "NaN comparison must return NanComparison error"
    );

    // prune_file with NaN stats → must keep the file (fail closed)
    let prune =
        slateduck_core::types::prune_file("5.0", Some("NaN"), Some("10.0"), false, &float_type);
    assert_eq!(
        prune,
        Ok(PruneResult::Keep),
        "prune_file with NaN min must keep the file"
    );
}

// ─── F-08m: pg_migrate SQL Escaping ───────────────────────────────────────

#[tokio::test]
async fn pg_migrate_escapes_single_quotes() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    // Schema name with a single quote
    let schema_id = writer.create_schema("o'reilly").await.unwrap();
    let table_id = writer.create_table(schema_id, "tab's", None).await.unwrap();
    writer
        .add_column(table_id, "col'a", "VARCHAR", 0, true, None)
        .await
        .unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;
    let mut export_buf = Vec::new();
    slateduck_catalog::export::export_catalog(&db, None, &mut export_buf)
        .await
        .unwrap();
    db.close().await.unwrap();

    let reader = std::io::BufReader::new(&export_buf[..]);
    let mut output = Vec::new();
    let count = slateduck_catalog::export::pg_migrate(reader, &mut output).unwrap();
    assert!(count > 0);

    let sql = String::from_utf8(output).unwrap();
    // Verify that single quotes in identifiers are escaped (doubled)
    assert!(
        sql.contains("o''reilly"),
        "single quote in schema name must be escaped, got:\n{sql}"
    );
    assert!(
        !sql.contains("o'reilly"),
        "raw single quote must not appear unescaped"
    );
}
