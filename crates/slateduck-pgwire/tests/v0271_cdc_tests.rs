//! v0.27.1: PG-Wire CDC integration test.
//!
//! Exercises `table_changes()` through the full PG-Wire executor stack:
//!   - Write a real Parquet file to a `TempDir`-backed `LocalFileSystem` store.
//!   - Register the file as a `DataFileRow` in the catalog.
//!   - Execute `SELECT * FROM table_changes(...)` through `executor::execute_sql`.
//!   - Assert the call succeeds and returns a query response.
//!
//! Also covers fault injection: a registered path that does not exist in the
//! object store must produce a SQLSTATE 58030 error, not a panic.

use std::sync::Arc;

use arrow::array::{Int32Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use parquet::arrow::ArrowWriter;
use tempfile::TempDir;
use tokio::sync::Mutex;

use slateduck_catalog::{CatalogStore, OpenOptions};
use slateduck_pgwire::executor;
use slateduck_pgwire::session::SessionState;
use slateduck_sql::ParamValues;

fn nm() -> Arc<slateduck_pgwire::notify::NotifyManager> {
    Arc::new(slateduck_pgwire::notify::NotifyManager::new())
}

fn ext() -> Arc<Vec<String>> {
    Arc::new(vec![])
}

async fn setup_store(dir: &TempDir) -> Arc<Mutex<CatalogStore>> {
    let store = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    let opts = OpenOptions {
        object_store: store,
        path: ObjectPath::from(""),
        encryption: None,
    };
    let catalog = CatalogStore::open(opts).await.unwrap();
    Arc::new(Mutex::new(catalog))
}

fn write_parquet(dir: &TempDir, filename: &str) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int32Array::from(vec![10, 20])) as Arc<dyn arrow::array::Array>,
            Arc::new(StringArray::from(vec!["alice", "bob"])) as Arc<dyn arrow::array::Array>,
        ],
    )
    .unwrap();
    let path = dir.path().join(filename);
    let file = std::fs::File::create(path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
}

/// Full stack test: write Parquet file -> register in catalog -> call
/// `table_changes()` via the PG-Wire executor -> assert a query response
/// is returned (not an error).
#[tokio::test]
async fn pgwire_table_changes_returns_query_response() {
    let dir = TempDir::new().unwrap();
    write_parquet(&dir, "events.parquet");

    let store = setup_store(&dir).await;

    let table_id;
    {
        let mut lock = store.lock().await;
        let mut writer = lock.begin_write();
        let schema_id = writer.create_schema("events").await.unwrap();
        table_id = writer.create_table(schema_id, "logs", None).await.unwrap();
        writer
            .add_column(table_id, "id", "INTEGER", 0, false, None)
            .await
            .unwrap();
        writer
            .add_column(table_id, "name", "VARCHAR", 1, true, None)
            .await
            .unwrap();
        writer.create_snapshot(None, None).await.unwrap();
        lock.commit_writer(&writer);
    }

    {
        let mut lock = store.lock().await;
        let mut writer = lock.begin_write();
        writer
            .register_data_file(table_id, "events.parquet", "parquet", 2, 0)
            .await
            .unwrap();
        writer.create_snapshot(None, None).await.unwrap();
        lock.commit_writer(&writer);
    }

    let sql = "SELECT * FROM table_changes('events.logs', 1, 2)";
    let params = ParamValues::default();
    let mut session = SessionState::new();
    let mut responses = executor::execute_sql(sql, &params, &store, &mut session, &nm(), &ext())
        .await
        .expect("table_changes must not return an error");

    assert_eq!(responses.len(), 1, "expected exactly one response");
    match responses.remove(0) {
        pgwire::api::results::Response::Query(_) => {}
        _ => panic!("expected a Query response from table_changes"),
    }
}

/// When a registered data file does not exist in the object store,
/// `table_changes()` must return SQLSTATE 58030, not panic.
#[tokio::test]
async fn pgwire_table_changes_missing_file_returns_storage_error() {
    let dir = TempDir::new().unwrap();
    // No Parquet file written -- the registered path is intentionally missing.

    let store = setup_store(&dir).await;

    {
        let mut lock = store.lock().await;
        let mut writer = lock.begin_write();
        let schema_id = writer.create_schema("analytics").await.unwrap();
        let table_id = writer
            .create_table(schema_id, "events", None)
            .await
            .unwrap();
        writer
            .register_data_file(table_id, "missing.parquet", "parquet", 5, 0)
            .await
            .unwrap();
        writer.create_snapshot(None, None).await.unwrap();
        lock.commit_writer(&writer);
    }

    let sql = "SELECT * FROM table_changes('analytics.events', 0, 1)";
    let params = ParamValues::default();
    let mut session = SessionState::new();
    let result = executor::execute_sql(sql, &params, &store, &mut session, &nm(), &ext()).await;

    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("missing data file must return an error, not succeed"),
    };

    assert_eq!(
        err.sqlstate(),
        "58030",
        "expected SQLSTATE 58030 (storage error), got SQLSTATE {}",
        err.sqlstate()
    );
}
