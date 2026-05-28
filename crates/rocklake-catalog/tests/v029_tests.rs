//! Integration tests for v0.29.0 — Recovery Correctness.
//!
//! Tests:
//!   1. Import + list_data_files() round-trip (secondary index after import).
//!   2. Retired data files are excluded from exports at the correct snapshot.
//!   3. Export manifest covers all expected table categories.
//!   4. Checkpoint restore noop: counter is not over-advanced when there are
//!      no post-checkpoint facts.

use object_store::path::Path as ObjectPath;
use rocklake_catalog::{CatalogStore, OpenOptions};
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

// ─── 1. Import + list_data_files() Round-trip ─────────────────────────────

/// After `import_catalog()` the secondary `TAG_DATA_FILE_BY_SNAPSHOT` index
/// must be populated so that `CatalogReader::list_data_files()` returns the
/// correct file list.
#[tokio::test]
async fn import_list_data_files_round_trip() {
    // ── Build source catalog ──────────────────────────────────────────────
    let src = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&src)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "events", Some("s3://bucket/events/"))
        .await
        .unwrap();
    writer
        .add_column(table_id, "id", "INTEGER", 0, false, None)
        .await
        .unwrap();
    writer
        .register_data_file(table_id, "events/part-0001.parquet", "parquet", 500, 8192)
        .await
        .unwrap();
    writer
        .register_data_file(table_id, "events/part-0002.parquet", "parquet", 750, 12288)
        .await
        .unwrap();
    let commit = writer.create_snapshot(None, None).await.unwrap();
    let snap_id = commit.snapshot_id;
    store.close().await.unwrap();

    // ── Export ────────────────────────────────────────────────────────────
    let db = open_db(&src).await;
    let mut export_buf: Vec<u8> = Vec::new();
    let export_result = rocklake_catalog::export::export_catalog(&db, None, &mut export_buf)
        .await
        .unwrap();
    assert!(
        export_result.rows_exported >= 5,
        "expected at least snapshot+schema+table+column+2 data files"
    );
    db.close().await.unwrap();

    // ── Import into a fresh catalog ───────────────────────────────────────
    let dst = TempDir::new().unwrap();
    let db2 = open_db(&dst).await;
    let reader_ndjson = std::io::BufReader::new(export_buf.as_slice());
    let import_result = rocklake_catalog::export::import_catalog(&db2, reader_ndjson)
        .await
        .unwrap();
    assert_eq!(import_result.rows_imported, export_result.rows_exported);
    db2.close().await.unwrap();

    // ── Open CatalogStore on the imported catalog and verify readers ───────
    let imported_store = CatalogStore::open(test_opts(&dst)).await.unwrap();
    let reader = imported_store.read_at(snap_id).unwrap();

    // list_schemas must return the "main" schema
    let schemas = reader.list_schemas().await.unwrap();
    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].schema_name, "main");

    // list_tables must return the "events" table
    let tables = reader.list_tables(schema_id).await.unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].table_name, "events");

    // list_data_files is the key assertion: the secondary index must be present
    let files = reader.list_data_files(table_id).await.unwrap();
    assert_eq!(
        files.len(),
        2,
        "expected 2 data files after import; secondary index missing?"
    );
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"events/part-0001.parquet"));
    assert!(paths.contains(&"events/part-0002.parquet"));

    imported_store.close().await.unwrap();
}

// ─── 2. Retired Data File Excluded from Export ────────────────────────────

/// A data file that is retired (end_snapshot set) before the export snapshot
/// must not appear in the export NDJSON, and therefore must not appear after
/// import.
#[tokio::test]
async fn export_excludes_retired_data_file() {
    // ── Build source catalog: snapshot 1 has a data file ─────────────────
    let src = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&src)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "logs", None).await.unwrap();
    writer
        .add_column(table_id, "ts", "TIMESTAMP", 0, false, None)
        .await
        .unwrap();
    writer
        .register_data_file(table_id, "logs/part-0001.parquet", "parquet", 100, 4096)
        .await
        .unwrap();
    let _ = writer.create_snapshot(None, None).await.unwrap(); // snapshot 1
    store.close().await.unwrap();

    // ── Snapshot 2: drop the table, which retires the data file ──────────
    let mut store = CatalogStore::open(test_opts(&src)).await.unwrap();
    let mut writer = store.begin_write();
    writer.drop_table(schema_id, table_id, 1).await.unwrap();
    let _ = writer.create_snapshot(None, None).await.unwrap(); // snapshot 2
    store.close().await.unwrap();

    // ── Export at snapshot 2 (latest) ────────────────────────────────────
    let db = open_db(&src).await;
    let mut export_buf: Vec<u8> = Vec::new();
    rocklake_catalog::export::export_catalog(&db, None, &mut export_buf)
        .await
        .unwrap();
    db.close().await.unwrap();

    // Verify the NDJSON does not contain any ducklake_data_file rows.
    let content = String::from_utf8(export_buf.clone()).unwrap();
    let has_data_file_row = content
        .lines()
        .any(|line| line.contains("\"ducklake_data_file\""));
    assert!(
        !has_data_file_row,
        "export should not contain retired data files, but found a ducklake_data_file row"
    );

    // ── Import and double-check via inspect_snapshot ──────────────────────
    let dst = TempDir::new().unwrap();
    let db2 = open_db(&dst).await;
    let reader_ndjson = std::io::BufReader::new(export_buf.as_slice());
    rocklake_catalog::export::import_catalog(&db2, reader_ndjson)
        .await
        .unwrap();
    let state = rocklake_catalog::inspect::inspect_snapshot(&db2)
        .await
        .unwrap();
    assert_eq!(
        state.data_file_count, 0,
        "imported catalog must contain no data files after retired-file export"
    );
    db2.close().await.unwrap();
}

// ─── 3. Export Manifest Covers All Expected Table Categories ─────────────

/// The export must emit at least one row for every catalog entity type that was
/// written.  This is a prerequisite for the full-coverage work in v0.32.0.
#[tokio::test]
async fn export_manifest_covers_expected_tables() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "t1", None).await.unwrap();
    writer
        .add_column(table_id, "id", "INTEGER", 0, false, None)
        .await
        .unwrap();
    writer
        .register_data_file(table_id, "data/part-0001.parquet", "parquet", 10, 1024)
        .await
        .unwrap();
    let _ = writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;
    let mut export_buf: Vec<u8> = Vec::new();
    rocklake_catalog::export::export_catalog(&db, None, &mut export_buf)
        .await
        .unwrap();
    db.close().await.unwrap();

    // Collect every "table" value that appears in the NDJSON stream.
    let content = String::from_utf8(export_buf).unwrap();
    let mut seen_tables: std::collections::HashSet<String> = std::collections::HashSet::new();
    for line in content.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        if let Some(tbl) = v.get("table").and_then(|t| t.as_str()) {
            seen_tables.insert(tbl.to_string());
        }
    }

    // Every category present in this catalog must appear in the manifest.
    let required = [
        "ducklake_snapshot",
        "ducklake_schema",
        "ducklake_table",
        "ducklake_column",
        "ducklake_data_file",
    ];
    for category in &required {
        assert!(
            seen_tables.contains(*category),
            "export manifest is missing category '{category}'"
        );
    }
}

// ─── 4. Checkpoint Restore Noop: Counter Not Over-Advanced ───────────────

/// When `restore_checkpoint()` is called immediately after
/// `create_checkpoint()` (no new writes), `hide_snapshot == meta.snapshot_id
/// + 1`, so the counter must stay at `meta.snapshot_id + 1`.  With the old
/// code the counter was incorrectly advanced to `meta.snapshot_id + 2`.
#[tokio::test]
async fn checkpoint_restore_noop_no_extra_counter_advance() {
    use rocklake_core::{keys, tags::COUNTER_NEXT_SNAPSHOT_ID, values};

    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("main").await.unwrap();
    let _ = writer.create_snapshot(None, None).await.unwrap(); // snapshot 1
    store.close().await.unwrap();

    let db = open_db(&dir).await;

    // Counter should be 2 right now (next snapshot would be 2).
    let counter_key = keys::key_counter(COUNTER_NEXT_SNAPSHOT_ID);
    let counter_before: u64 = db
        .get(&counter_key)
        .await
        .unwrap()
        .map(|d| values::decode_counter(&d).unwrap())
        .unwrap_or(0);
    assert_eq!(
        counter_before, 2,
        "expected next_snapshot_id=2 after snapshot 1"
    );

    // Create checkpoint at snapshot 1.
    let cp = rocklake_catalog::checkpoint::create_checkpoint(&db, Some("noop-test"))
        .await
        .unwrap();
    assert_eq!(cp.snapshot_id, 1);

    // Restore immediately (no writes between create and restore).
    rocklake_catalog::checkpoint::restore_checkpoint(&db, cp.id)
        .await
        .unwrap();

    // Counter must remain at 2, not be advanced to 3.
    let counter_after: u64 = db
        .get(&counter_key)
        .await
        .unwrap()
        .map(|d| values::decode_counter(&d).unwrap())
        .unwrap_or(0);
    assert_eq!(
        counter_after, 2,
        "counter must stay at meta.snapshot_id+1=2, not be over-advanced to 3"
    );

    db.close().await.unwrap();
}
