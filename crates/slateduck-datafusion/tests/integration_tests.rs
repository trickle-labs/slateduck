//! Tests for the DataFusion integration.

use datafusion::catalog::CatalogProvider;
use object_store::path::Path as ObjectPath;
use slateduck_catalog::{CatalogStore, OpenOptions};
use slateduck_core::mvcc::SnapshotId;
use slateduck_datafusion::SlateDuckCatalogProvider;
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

#[tokio::test]
async fn datafusion_provider_schema_names() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Create schemas and tables
    let mut writer = store.begin_write();
    writer.create_schema("main").await.unwrap();
    writer.create_schema("analytics").await.unwrap();
    writer.create_snapshot(None, None).await.unwrap();

    // Create provider at snapshot 1
    let provider = SlateDuckCatalogProvider::new(store, Some(SnapshotId::new(1)));

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
    writer.create_snapshot(None, None).await.unwrap();

    let provider = SlateDuckCatalogProvider::new(store, Some(SnapshotId::new(1)));
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
    writer.create_snapshot(None, None).await.unwrap();

    let provider = SlateDuckCatalogProvider::new(store, Some(SnapshotId::new(1)));
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
    writer.create_snapshot(None, None).await.unwrap();

    let provider = SlateDuckCatalogProvider::new(store, Some(SnapshotId::new(1)));
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
    writer.create_snapshot(None, None).await.unwrap();

    let provider = SlateDuckCatalogProvider::new(store, Some(SnapshotId::new(1)));
    let schema_provider = provider.schema("main").unwrap();
    assert!(schema_provider.table_exist("orders"));
    assert!(!schema_provider.table_exist("nonexistent"));
}
