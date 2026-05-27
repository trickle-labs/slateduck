//! Integration tests for the catalog store.

use object_store::path::Path as ObjectPath;
use rocklake_catalog::writer::stats::FileColumnStatsInput;
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

#[tokio::test]
async fn catalog_open_and_initialize() {
    let dir = TempDir::new().unwrap();
    let store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    store.close().await.unwrap();
}

#[tokio::test]
async fn catalog_reopen_preserves_state() {
    let dir = TempDir::new().unwrap();
    let opts = test_opts(&dir);

    // First open
    let mut store = CatalogStore::open(opts.clone()).await.unwrap();
    let mut writer = store.begin_write();
    let schema_id = writer.create_schema("test_schema").await.unwrap();
    let _snap = writer
        .create_snapshot(Some("test"), Some("initial"))
        .await
        .unwrap();
    store.close().await.unwrap();

    // Reopen
    let store = CatalogStore::open(opts).await.unwrap();
    let reader = store.read_at(SnapshotId::new(1)).unwrap();
    let schemas = reader.list_schemas().await.unwrap();
    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].schema_name, "test_schema");
    assert_eq!(schemas[0].schema_id, schema_id);
    store.close().await.unwrap();
}

#[tokio::test]
async fn create_schema_and_table() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "users", Some("s3://bucket/data/users/"))
        .await
        .unwrap();
    let _snap = writer.create_snapshot(None, None).await.unwrap();

    let reader = store.read_at(SnapshotId::new(1)).unwrap();
    let schemas = reader.list_schemas().await.unwrap();
    assert_eq!(schemas.len(), 1);

    let tables = reader.list_tables(schema_id).await.unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].table_name, "users");
    assert_eq!(tables[0].table_id, table_id);

    store.close().await.unwrap();
}

#[tokio::test]
async fn add_and_describe_columns() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "users", None).await.unwrap();

    writer
        .add_column(table_id, "id", "INTEGER", 0, false, None)
        .await
        .unwrap();
    writer
        .add_column(table_id, "name", "VARCHAR", 1, true, None)
        .await
        .unwrap();
    writer
        .add_column(table_id, "email", "VARCHAR", 2, true, None)
        .await
        .unwrap();

    let _snap = writer.create_snapshot(None, None).await.unwrap();

    let reader = store.read_at(SnapshotId::new(1)).unwrap();
    let desc = reader.describe_table(table_id).await.unwrap().unwrap();
    let (table, columns) = desc;
    assert_eq!(table.table_name, "users");
    assert_eq!(columns.len(), 3);
    assert_eq!(columns[0].column_name, "id");
    assert_eq!(columns[1].column_name, "name");
    assert_eq!(columns[2].column_name, "email");

    store.close().await.unwrap();
}

#[tokio::test]
async fn drop_schema_makes_invisible() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("temp").await.unwrap();
    let snap1 = writer.create_snapshot(None, None).await.unwrap();

    writer.drop_schema(schema_id).await.unwrap();
    let snap2 = writer.create_snapshot(None, None).await.unwrap();

    // Visible at snapshot 1
    let reader1 = store.read_at(snap1).unwrap();
    let schemas1 = reader1.list_schemas().await.unwrap();
    assert_eq!(schemas1.len(), 1);

    // Not visible at snapshot 2
    let reader2 = store.read_at(snap2).unwrap();
    let schemas2 = reader2.list_schemas().await.unwrap();
    assert_eq!(schemas2.len(), 0);

    store.close().await.unwrap();
}

#[tokio::test]
async fn register_data_files() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "events", None)
        .await
        .unwrap();

    let file1 = writer
        .register_data_file(
            table_id,
            "data/events/part-0001.parquet",
            "parquet",
            1000,
            50000,
        )
        .await
        .unwrap();
    let file2 = writer
        .register_data_file(
            table_id,
            "data/events/part-0002.parquet",
            "parquet",
            2000,
            100000,
        )
        .await
        .unwrap();

    let snap = writer.create_snapshot(None, None).await.unwrap();

    let reader = store.read_at(snap).unwrap();
    let files = reader.list_data_files(table_id).await.unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].data_file_id, file1);
    assert_eq!(files[1].data_file_id, file2);

    store.close().await.unwrap();
}

#[tokio::test]
async fn inlined_insert_and_delete() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "small", None).await.unwrap();

    // Insert a row
    writer
        .register_inlined_insert(table_id, 1, 0, b"row data 0".to_vec())
        .await
        .unwrap();
    writer
        .register_inlined_insert(table_id, 1, 1, b"row data 1".to_vec())
        .await
        .unwrap();

    let snap1 = writer.create_snapshot(None, None).await.unwrap();

    // Mark row 0 as deleted
    writer
        .mark_inlined_insert_deleted(table_id, 1, 0)
        .await
        .unwrap();

    let snap2 = writer.create_snapshot(None, None).await.unwrap();

    // At snapshot 1, both rows visible
    let reader1 = store.read_at(snap1).unwrap();
    let rows1 = reader1.list_inlined_inserts(table_id).await.unwrap();
    assert_eq!(rows1.len(), 2);

    // At snapshot 2, only row 1 visible
    let reader2 = store.read_at(snap2).unwrap();
    let rows2 = reader2.list_inlined_inserts(table_id).await.unwrap();
    assert_eq!(rows2.len(), 1);
    assert_eq!(rows2[0].row_id, 1);

    store.close().await.unwrap();
}

#[tokio::test]
async fn schema_version_increments_on_ddl() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    // Schema change → increments
    let _schema_id = writer.create_schema("s1").await.unwrap();
    let snap1 = writer.create_snapshot(None, None).await.unwrap();

    let reader1 = store.read_at(snap1).unwrap();
    let s1 = reader1.get_snapshot().await.unwrap().unwrap();
    assert_eq!(s1.schema_version, 1);

    // Data-only (register file) → does NOT increment
    let schema_id = writer.create_schema("s2").await.unwrap();
    let table_id = writer.create_table(schema_id, "t1", None).await.unwrap();
    let snap2 = writer.create_snapshot(None, None).await.unwrap();

    // Another schema change
    let reader2 = store.read_at(snap2).unwrap();
    let s2 = reader2.get_snapshot().await.unwrap().unwrap();
    assert_eq!(s2.schema_version, 2); // create_schema + create_table both trigger

    // Now do a data-only op
    let _file_id = writer
        .register_data_file(table_id, "file.parquet", "parquet", 100, 5000)
        .await
        .unwrap();
    let snap3 = writer.create_snapshot(None, None).await.unwrap();

    let reader3 = store.read_at(snap3).unwrap();
    let s3 = reader3.get_snapshot().await.unwrap().unwrap();
    // Data-only op doesn't call mark_schema_changed, so version stays
    assert_eq!(s3.schema_version, 2);

    store.close().await.unwrap();
}

#[tokio::test]
async fn verify_catalog_passes_on_valid() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    writer.create_schema("main").await.unwrap();
    let _snap = writer.create_snapshot(None, None).await.unwrap();

    let result = rocklake_catalog::verify::verify_catalog(store.db())
        .await
        .unwrap();
    assert!(result.is_ok(), "verification errors: {:?}", result.errors);
    assert!(result.rows_checked > 0);

    store.close().await.unwrap();
}

#[tokio::test]
async fn concurrent_initialization_convergence() {
    // Two "processes" open the same catalog path simultaneously.
    // Only one coherent initial state should result.
    let dir = TempDir::new().unwrap();
    let opts = test_opts(&dir);

    let opts1 = opts.clone();
    let opts2 = opts.clone();

    let (r1, r2) = tokio::join!(CatalogStore::open(opts1), CatalogStore::open(opts2),);

    // At least one should succeed; we verify the catalog is coherent
    // by opening a third time and verifying.
    // (Due to single-writer constraint, second open may fail or succeed
    //  depending on timing, but either way the catalog must be valid.)
    drop(r1);
    drop(r2);

    // Final verification: open fresh and verify
    let store = CatalogStore::open(opts).await.unwrap();
    let result = rocklake_catalog::verify::verify_catalog(store.db())
        .await
        .unwrap();
    assert!(result.is_ok(), "verification errors: {:?}", result.errors);
    store.close().await.unwrap();
}

#[tokio::test]
async fn file_column_stats_and_pruning() {
    use rocklake_core::types::DuckLakeType;

    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "metrics", None)
        .await
        .unwrap();
    let col_id = writer
        .add_column(table_id, "value", "INTEGER", 0, false, None)
        .await
        .unwrap();

    let file1 = writer
        .register_data_file(table_id, "f1.parquet", "parquet", 100, 5000)
        .await
        .unwrap();
    let file2 = writer
        .register_data_file(table_id, "f2.parquet", "parquet", 100, 5000)
        .await
        .unwrap();

    // File 1: min=10, max=50
    writer
        .upsert_file_column_stats(FileColumnStatsInput {
            table_id,
            column_id: col_id,
            data_file_id: file1,
            contains_null: false,
            min_value: Some("10"),
            max_value: Some("50"),
            contains_nan: false,
            column_size_bytes: None,
            value_count: None,
            null_count: None,
            extra_stats: None,
        })
        .await
        .unwrap();
    // File 2: min=100, max=200
    writer
        .upsert_file_column_stats(FileColumnStatsInput {
            table_id,
            column_id: col_id,
            data_file_id: file2,
            contains_null: false,
            min_value: Some("100"),
            max_value: Some("200"),
            contains_nan: false,
            column_size_bytes: None,
            value_count: None,
            null_count: None,
            extra_stats: None,
        })
        .await
        .unwrap();

    let snap = writer.create_snapshot(None, None).await.unwrap();
    let reader = store.read_at(snap).unwrap();

    let col_type = DuckLakeType::Integer {
        signed: true,
        width_bits: 32,
    };

    // Query for value=30 → only file1 kept
    let kept = reader
        .prune_files(table_id, col_id, "30", &col_type)
        .await
        .unwrap();
    assert_eq!(kept, vec![file1]);

    // Query for value=150 → only file2 kept
    let kept = reader
        .prune_files(table_id, col_id, "150", &col_type)
        .await
        .unwrap();
    assert_eq!(kept, vec![file2]);

    // Query for value=5 → nothing kept (below all mins)
    let kept = reader
        .prune_files(table_id, col_id, "5", &col_type)
        .await
        .unwrap();
    assert!(kept.is_empty());

    store.close().await.unwrap();
}
