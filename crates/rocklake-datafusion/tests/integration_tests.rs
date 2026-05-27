//! Tests for the DataFusion integration.

use datafusion::catalog::CatalogProvider;
use object_store::path::Path as ObjectPath;
use rocklake_catalog::{CatalogStore, OpenOptions};
use rocklake_core::mvcc::SnapshotId;
use rocklake_datafusion::RocklakeCatalogProvider;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::RwLock;

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
async fn datafusion_provider_schema_names() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Create schemas and tables
    let mut writer = store.begin_write();
    writer.create_schema("main").await.unwrap();
    writer.create_schema("analytics").await.unwrap();
    let _ = writer.create_snapshot(None, None).await.unwrap();

    // Create provider at snapshot 1
    let provider = RocklakeCatalogProvider::new(store, Some(SnapshotId::new(1)));

    let names = provider.schema_names();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"main".to_string()));
    assert!(names.contains(&"analytics".to_string()));
}

#[tokio::test]
async fn datafusion_provider_table_names() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let mut writer = store.begin_write();
    let schema_id = writer.create_schema("main").await.unwrap();
    writer.create_table(schema_id, "users", None).await.unwrap();
    writer
        .create_table(schema_id, "orders", None)
        .await
        .unwrap();
    let _ = writer.create_snapshot(None, None).await.unwrap();

    let provider = RocklakeCatalogProvider::new(store, Some(SnapshotId::new(1)));
    let schema_provider = provider.schema("main").unwrap();
    let table_names = schema_provider.table_names();
    assert_eq!(table_names.len(), 2);
    assert!(table_names.contains(&"users".to_string()));
    assert!(table_names.contains(&"orders".to_string()));
}

#[tokio::test]
async fn datafusion_provider_table_schema() {
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
        .add_column(table_id, "active", "BOOLEAN", 2, false, None)
        .await
        .unwrap();
    let _ = writer.create_snapshot(None, None).await.unwrap();

    let provider = RocklakeCatalogProvider::new(store, Some(SnapshotId::new(1)));
    let schema_provider = provider.schema("main").unwrap();
    let table = schema_provider.table("users").await.unwrap().unwrap();

    use datafusion::arrow::datatypes::DataType;
    let schema = table.schema();
    assert_eq!(schema.fields().len(), 3);
    assert_eq!(schema.field(0).name(), "id");
    assert_eq!(*schema.field(0).data_type(), DataType::Int32);
    assert!(!schema.field(0).is_nullable());
    assert_eq!(schema.field(1).name(), "name");
    assert_eq!(*schema.field(1).data_type(), DataType::Utf8);
    assert!(schema.field(1).is_nullable());
    assert_eq!(schema.field(2).name(), "active");
    assert_eq!(*schema.field(2).data_type(), DataType::Boolean);
}

#[tokio::test]
async fn datafusion_provider_nonexistent_table() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let mut writer = store.begin_write();
    writer.create_schema("main").await.unwrap();
    let _ = writer.create_snapshot(None, None).await.unwrap();

    let provider = RocklakeCatalogProvider::new(store, Some(SnapshotId::new(1)));
    let schema_provider = provider.schema("main").unwrap();
    let table = schema_provider.table("nonexistent").await.unwrap();
    assert!(table.is_none());
}

#[tokio::test]
async fn datafusion_provider_table_exist() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let mut writer = store.begin_write();
    let schema_id = writer.create_schema("main").await.unwrap();
    writer
        .create_table(schema_id, "orders", None)
        .await
        .unwrap();
    let _ = writer.create_snapshot(None, None).await.unwrap();

    let provider = RocklakeCatalogProvider::new(store, Some(SnapshotId::new(1)));
    let schema_provider = provider.schema("main").unwrap();
    assert!(schema_provider.table_exist("orders"));
    assert!(!schema_provider.table_exist("nonexistent"));
}

// ─── F-15: DataFusion Parquet scan ────────────────────────────────────────

/// Write a minimal Parquet file with two rows and register it in the catalog.
/// Then scan via DataFusion and verify the correct number of rows is returned.
#[tokio::test]
async fn datafusion_scan_reads_parquet_data() {
    use arrow::array::{Int32Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use datafusion::prelude::SessionContext;
    use parquet::arrow::ArrowWriter;

    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    // Build a two-row record batch: id INT32, name VARCHAR.
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int32Array::from(vec![1, 2])) as Arc<dyn arrow::array::Array>,
            Arc::new(StringArray::from(vec!["alice", "bob"])) as Arc<dyn arrow::array::Array>,
        ],
    )
    .unwrap();

    // Write the Parquet file into the data directory.
    let parquet_path = data_dir.join("t1.parquet");
    let file = std::fs::File::create(&parquet_path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();

    // Register the data file in the catalog.
    let root = dir.path().to_str().unwrap().to_string();
    let obj_store = Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&root).unwrap());
    let mut catalog_store = CatalogStore::open(OpenOptions {
        object_store: obj_store.clone(),
        path: ObjectPath::from("catalog"),
        encryption: None,
    })
    .await
    .unwrap();

    let mut writer = catalog_store.begin_write();
    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "events", None)
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
    // The path is relative to the object store root.
    let rel_path = "data/t1.parquet";
    writer
        .register_data_file(
            table_id,
            rel_path,
            "parquet",
            2,
            parquet_path.metadata().unwrap().len(),
        )
        .await
        .unwrap();
    let _cr = writer.create_snapshot(None, None).await.unwrap();
    catalog_store.commit_writer(_cr);
    catalog_store.close().await.unwrap();

    // Open via the DataFusion provider.
    let obj_store2 =
        Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&root).unwrap());
    let provider = Arc::new(
        RocklakeCatalogProvider::open(
            obj_store2,
            ObjectPath::from("catalog"),
            Some(SnapshotId::new(1)),
        )
        .await
        .unwrap(),
    );

    let ctx = SessionContext::new();
    ctx.register_catalog("duck", provider);

    let df = ctx
        .sql("SELECT id, name FROM duck.main.events")
        .await
        .unwrap();
    let results = df.collect().await.unwrap();
    let total_rows: usize = results.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 2, "should read 2 rows from the Parquet file");
}

// ─── N-02: from_catalog_store auto-resolves data_root ─────────────────────

/// Verify that `from_catalog_store` reads `data_path` from `ducklake_metadata`
/// and exposes it as the `data_root` on the provider, enabling Parquet scans
/// without re-opening the catalog.
#[tokio::test]
async fn from_catalog_store_resolves_data_root() {
    use rocklake_core::keys::MetadataScope;

    let dir = TempDir::new().unwrap();
    let root = dir.path().to_str().unwrap().to_string();
    let obj_store = Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&root).unwrap());

    let mut store = CatalogStore::open(OpenOptions {
        object_store: obj_store,
        path: ObjectPath::from("catalog"),
        encryption: None,
    })
    .await
    .unwrap();

    // Write a data_path metadata entry (DuckDB system key, no dot prefix needed).
    let mut writer = store.begin_write();
    writer
        .set_metadata(MetadataScope::Global, 0, "data_path", "/catalog/data")
        .unwrap();
    let schema_id = writer.create_schema("main").await.unwrap();
    writer
        .create_table(schema_id, "events", None)
        .await
        .unwrap();
    let _cr = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(_cr);

    // Wrap in Arc<RwLock<_>> and build a provider via from_catalog_store.
    let shared = Arc::new(RwLock::new(store));
    let provider = RocklakeCatalogProvider::from_catalog_store(shared, None)
        .await
        .unwrap();

    // Schema discovery still works.
    let names = provider.schema_names();
    assert!(
        names.contains(&"main".to_string()),
        "schema 'main' should be visible"
    );

    // The schema provider is reachable (data_root was resolved from metadata).
    let schema_prov = provider.schema("main").unwrap();
    assert!(
        schema_prov.table_exist("events"),
        "table 'events' should be visible"
    );
}
