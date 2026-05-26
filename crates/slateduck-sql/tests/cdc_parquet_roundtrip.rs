//! v0.27.1: End-to-end CDC round-trip tests.
//!
//! Each test writes a real Parquet file to a `TempDir`-backed `LocalFileSystem`
//! object store, then calls `extract_rows_from_parquet()` and asserts that the
//! returned `columns_json` matches the original row data.
//!
//! Coverage:
//!   - Single-file insert window with exact column value verification.
//!   - Multi-file window: insert file at snapshot N, delete file at snapshot N+2.
//!   - Fault injection: `ObjectStore` returns `NotFound` for a registered path.

use std::sync::Arc;

use arrow::array::{Int32Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use object_store::local::LocalFileSystem;
use parquet::arrow::ArrowWriter;
use tempfile::TempDir;

use slateduck_sql::table_changes::{
    compute_table_changes, extract_rows_from_parquet, DEFAULT_CDC_BATCH_SIZE,
};

// ── helpers ──────────────────────────────────────────────────────────────────

/// Write a two-row Parquet file with columns `id` (Int32) and `name` (Utf8).
/// Returns the relative path within the object store root.
fn write_test_parquet(dir: &TempDir, filename: &str) -> String {
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

    let path = dir.path().join(filename);
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();

    filename.to_string()
}

/// Write a Parquet file with a single row: `id=99, name="deleted"`.
fn write_delete_parquet(dir: &TempDir, filename: &str) -> String {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int32Array::from(vec![99])) as Arc<dyn arrow::array::Array>,
            Arc::new(StringArray::from(vec!["deleted"])) as Arc<dyn arrow::array::Array>,
        ],
    )
    .unwrap();

    let path = dir.path().join(filename);
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();

    filename.to_string()
}

// ── test 1: single-file insert — verify column values ────────────────────────

/// Write a Parquet file with two rows, scan it via `extract_rows_from_parquet`,
/// and assert that the returned `columns_json` fields match the written data.
/// This is the primary N-01 verification: no more synthetic `"{}"` payloads.
#[tokio::test]
async fn cdc_parquet_roundtrip_single_file() {
    let dir = TempDir::new().unwrap();
    let rel_path = write_test_parquet(&dir, "insert1.parquet");

    let store: Arc<dyn object_store::ObjectStore> =
        Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());

    let rows = extract_rows_from_parquet(
        &store,
        &rel_path,
        0,       // base_rowid
        Some(2), // expected_record_count matches
        DEFAULT_CDC_BATCH_SIZE,
    )
    .await
    .unwrap();

    assert_eq!(rows.len(), 2, "should scan 2 rows");

    // Row 0: id=1, name="alice"
    let row0: serde_json::Value = serde_json::from_str(&rows[0].columns_json).unwrap();
    assert_eq!(row0["id"], serde_json::json!(1));
    assert_eq!(row0["name"], serde_json::json!("alice"));
    assert_eq!(rows[0].rowid, 0);

    // Row 1: id=2, name="bob"
    let row1: serde_json::Value = serde_json::from_str(&rows[1].columns_json).unwrap();
    assert_eq!(row1["id"], serde_json::json!(2));
    assert_eq!(row1["name"], serde_json::json!("bob"));
    assert_eq!(rows[1].rowid, 1);
}

// ── test 2: multi-file window ─────────────────────────────────────────────────

/// Insert file at snapshot N, delete file at snapshot N+2.
/// Verify the CDC window `(N-1, N+2]` returns correct inserts and deletes.
#[tokio::test]
async fn cdc_parquet_roundtrip_multi_file_window() {
    let dir = TempDir::new().unwrap();
    let insert_path = write_test_parquet(&dir, "snap1_insert.parquet");
    let delete_path = write_delete_parquet(&dir, "snap2_delete.parquet");

    let store: Arc<dyn object_store::ObjectStore> =
        Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());

    // Scan "added" file (snapshot N — rows inserted).
    let added_rows =
        extract_rows_from_parquet(&store, &insert_path, 0, Some(2), DEFAULT_CDC_BATCH_SIZE)
            .await
            .unwrap();

    // Scan "retired" file (snapshot N+2 — row deleted).
    let removed_rows = extract_rows_from_parquet(
        &store,
        &delete_path,
        100, // base_rowid offset to avoid collision
        Some(1),
        DEFAULT_CDC_BATCH_SIZE,
    )
    .await
    .unwrap();

    // Compute CDC for window snapshot 0 → 3 with no GC floor.
    let result =
        compute_table_changes("public.events", 0, 3, 0, &added_rows, &removed_rows).unwrap();

    // Expect 2 inserts (rowid 0 and 1) and 1 delete (rowid 100).
    let inserts: Vec<_> = result
        .records
        .iter()
        .filter(|r| r.change_type == slateduck_sql::ChangeType::Insert)
        .collect();
    let deletes: Vec<_> = result
        .records
        .iter()
        .filter(|r| r.change_type == slateduck_sql::ChangeType::Delete)
        .collect();

    assert_eq!(inserts.len(), 2);
    assert_eq!(deletes.len(), 1);

    // Verify insert column values.
    let i0: serde_json::Value = serde_json::from_str(&inserts[0].columns_json).unwrap();
    assert!(i0.get("id").is_some(), "insert row must have 'id' column");
    assert!(
        i0.get("name").is_some(),
        "insert row must have 'name' column"
    );

    // Verify delete column values.
    let d0: serde_json::Value = serde_json::from_str(&deletes[0].columns_json).unwrap();
    assert_eq!(d0["id"], serde_json::json!(99));
    assert_eq!(d0["name"], serde_json::json!("deleted"));
}

// ── test 3: fault injection — NotFound ───────────────────────────────────────

/// ObjectStore returns a `NotFound` error for an unregistered data file path.
/// Verify that `extract_rows_from_parquet` returns `TableChangesError::Storage`
/// rather than panicking.
#[tokio::test]
async fn cdc_parquet_notfound_returns_storage_error() {
    let dir = TempDir::new().unwrap();
    let store: Arc<dyn object_store::ObjectStore> =
        Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());

    let result = extract_rows_from_parquet(
        &store,
        "does_not_exist.parquet",
        0,
        Some(1),
        DEFAULT_CDC_BATCH_SIZE,
    )
    .await;

    assert!(
        result.is_err(),
        "missing file should return an error, not panic"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(err, slateduck_sql::TableChangesError::Storage(_)),
        "error should be TableChangesError::Storage, got: {err}"
    );
    assert_eq!(err.sqlstate(), "58030");
}

// ── test 4: record_count mismatch counter ────────────────────────────────────

/// When the actual scanned row count differs from `expected_record_count`,
/// the global mismatch counter should increment (N-04).
#[tokio::test]
async fn cdc_record_count_mismatch_increments_counter() {
    let dir = TempDir::new().unwrap();
    let rel_path = write_test_parquet(&dir, "mismatch.parquet");
    let store: Arc<dyn object_store::ObjectStore> =
        Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());

    let before = slateduck_sql::cdc_record_count_mismatch_total();

    // The file has 2 rows but we claim it has 99 — this should fire a warning
    // and increment the mismatch counter.
    let rows = extract_rows_from_parquet(
        &store,
        &rel_path,
        0,
        Some(99), // deliberate mismatch
        DEFAULT_CDC_BATCH_SIZE,
    )
    .await
    .unwrap();

    // Scanning succeeds; the scanned (correct) count is used.
    assert_eq!(rows.len(), 2);

    let after = slateduck_sql::cdc_record_count_mismatch_total();
    assert!(
        after > before,
        "mismatch counter should have incremented (before={before}, after={after})"
    );
}
