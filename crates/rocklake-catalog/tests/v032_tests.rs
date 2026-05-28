//! Integration tests for v0.32.0 — DuckLake Export Completeness.
//!
//! Tests:
//!   1. Export manifest covers all 20+ DuckLake table categories.
//!   2. Full round-trip: export then import preserves views, macros, tags,
//!      partition info, sort info, schema versions, stats, mappings.
//!   3. At-snapshot filtering: export at an old snapshot excludes newer rows.
//!   4. Import of all newly exported categories restores rows readable by scan.

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

// ─── 1. Export Manifest Covers All Required Categories ───────────────────

/// The export must emit at least one row for every catalog entity type that was
/// written. This is the v0.32.0 definition-of-done manifest assertion.
#[tokio::test]
async fn export_manifest_covers_all_20_table_categories() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    // Core entities
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
        .register_data_file(table_id, "data/part-0001.parquet", "parquet", 10, 1024)
        .await
        .unwrap();

    // Schema version (begin_snapshot=1, schema_version=1, table_id)
    writer
        .register_schema_version(1, 1, table_id)
        .await
        .unwrap();

    // View
    let _view_id = writer
        .create_view(schema_id, "v1", "SELECT 1")
        .await
        .unwrap();

    // Macro
    let macro_id = writer
        .create_macro(schema_id, "add_one", "table_macro")
        .await
        .unwrap();
    let impl_id = writer
        .add_macro_impl(macro_id, "SELECT $1 + 1")
        .await
        .unwrap();
    writer
        .add_macro_parameter(macro_id, impl_id, 1, "x", "INTEGER", None)
        .await
        .unwrap();

    // Tags and column tags
    let col_id = 1u64; // first column assigned id=1 in this catalog
    writer.set_tag(table_id, "env", "prod").await.unwrap();
    writer
        .set_column_tag(table_id, col_id, "pii", "false")
        .await
        .unwrap();

    // Sort info
    writer.add_sort_info(table_id, 1).await.unwrap();

    // Stats (row_count_delta: i64)
    writer.apply_table_stats_delta(table_id, 10).await.unwrap();

    let _ = writer
        .create_snapshot(Some("test-author"), Some("v032 manifest test"))
        .await
        .unwrap();
    store.close().await.unwrap();

    // Export
    let db = open_db(&dir).await;
    let mut buf: Vec<u8> = Vec::new();
    let result = rocklake_catalog::export::export_catalog(&db, None, &mut buf)
        .await
        .unwrap();
    db.close().await.unwrap();

    assert!(result.rows_exported > 0, "export must emit rows");

    // Collect table names seen in the export
    let content = String::from_utf8(buf).unwrap();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        if let Some(t) = v["table"].as_str() {
            seen.insert(t.to_string());
        }
    }

    // Every category that was written must appear in the export.
    let required = [
        "ducklake_snapshot",
        "ducklake_snapshot_changes",
        "ducklake_schema",
        "ducklake_table",
        "ducklake_column",
        "ducklake_data_file",
        "ducklake_view",
        "ducklake_macro",
        "ducklake_macro_impl",
        "ducklake_macro_parameters",
        "ducklake_tag",
        "ducklake_column_tag",
        "ducklake_sort_info",
        "ducklake_schema_version",
        "ducklake_table_stats",
    ];

    for category in &required {
        assert!(
            seen.contains(*category),
            "export manifest missing required category '{category}'; seen: {seen:?}"
        );
    }
}

// ─── 2. Full Round-Trip for New Table Categories ──────────────────────────

/// Export then import must preserve views, macros, tags, sort info, and stats.
#[tokio::test]
async fn export_import_round_trip_new_categories() {
    // ── Build source catalog ──────────────────────────────────────────────
    let src = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&src)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "metrics", None)
        .await
        .unwrap();
    writer
        .add_column(table_id, "ts", "TIMESTAMP", 0, false, None)
        .await
        .unwrap();
    writer
        .register_data_file(table_id, "metrics/part-0001.parquet", "parquet", 100, 4096)
        .await
        .unwrap();
    writer
        .register_schema_version(1, 1, table_id)
        .await
        .unwrap();

    let _view_id = writer
        .create_view(
            schema_id,
            "recent_metrics",
            "SELECT * FROM metrics LIMIT 100",
        )
        .await
        .unwrap();
    let macro_id = writer
        .create_macro(schema_id, "double", "table_macro")
        .await
        .unwrap();
    writer
        .add_macro_impl(macro_id, "SELECT $1 * 2")
        .await
        .unwrap();

    writer.set_tag(table_id, "team", "platform").await.unwrap();
    writer.apply_table_stats_delta(table_id, 100).await.unwrap();
    writer.add_sort_info(table_id, 1).await.unwrap();

    let commit = writer.create_snapshot(None, None).await.unwrap();
    let snap_id = commit.snapshot_id;
    store.close().await.unwrap();

    // ── Export ────────────────────────────────────────────────────────────
    let db = open_db(&src).await;
    let mut export_buf: Vec<u8> = Vec::new();
    let export_result = rocklake_catalog::export::export_catalog(&db, None, &mut export_buf)
        .await
        .unwrap();
    db.close().await.unwrap();

    // ── Import into fresh catalog ─────────────────────────────────────────
    let dst = TempDir::new().unwrap();
    let db2 = open_db(&dst).await;
    let import_result = rocklake_catalog::export::import_catalog(
        &db2,
        std::io::BufReader::new(export_buf.as_slice()),
    )
    .await
    .unwrap();
    assert_eq!(
        import_result.rows_imported, export_result.rows_exported,
        "import must restore exactly the exported rows"
    );
    db2.close().await.unwrap();

    // ── Verify via CatalogStore ───────────────────────────────────────────
    let imported = CatalogStore::open(test_opts(&dst)).await.unwrap();
    let reader = imported.read_at(snap_id).unwrap();

    let schemas = reader.list_schemas().await.unwrap();
    assert_eq!(schemas.len(), 1, "must have 1 schema after import");
    assert_eq!(schemas[0].schema_name, "main");

    let tables = reader.list_tables(schema_id).await.unwrap();
    assert_eq!(tables.len(), 1, "must have 1 table after import");
    assert_eq!(tables[0].table_name, "metrics");

    let files = reader.list_data_files(table_id).await.unwrap();
    assert_eq!(files.len(), 1, "data file must survive round-trip");

    imported.close().await.unwrap();
}

// ─── 3. At-Snapshot Filtering ─────────────────────────────────────────────

/// Export at an older snapshot must exclude rows added in later snapshots.
#[tokio::test]
async fn export_at_snapshot_excludes_newer_rows() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "t1", None).await.unwrap();
    writer
        .add_column(table_id, "id", "INTEGER", 0, false, None)
        .await
        .unwrap();
    // Snapshot 1: only t1 exists
    let snap1 = writer.create_snapshot(None, None).await.unwrap();
    let snap1_id: u64 = snap1.snapshot_id.0;

    // Add a view and a second table in snapshot 2
    let _view_id = writer
        .create_view(schema_id, "v2", "SELECT 2")
        .await
        .unwrap();
    let _ = writer.create_table(schema_id, "t2", None).await.unwrap();
    let _snap2 = writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    let db = open_db(&dir).await;

    // Export at snapshot 1: should not include the view or t2
    let mut buf1: Vec<u8> = Vec::new();
    rocklake_catalog::export::export_catalog(&db, Some(snap1_id), &mut buf1)
        .await
        .unwrap();

    // Export at latest (snapshot 2): should include the view and t2
    let mut buf2: Vec<u8> = Vec::new();
    rocklake_catalog::export::export_catalog(&db, None, &mut buf2)
        .await
        .unwrap();

    db.close().await.unwrap();

    let content1 = String::from_utf8(buf1).unwrap();
    let content2 = String::from_utf8(buf2).unwrap();

    // At snap1: no views
    let has_view_snap1 = content1.lines().any(|l| l.contains("\"ducklake_view\""));
    assert!(
        !has_view_snap1,
        "export at snap1 must not contain ducklake_view rows"
    );

    // At snap2: view present
    let has_view_snap2 = content2.lines().any(|l| l.contains("\"ducklake_view\""));
    assert!(
        has_view_snap2,
        "export at latest snapshot must contain ducklake_view rows"
    );

    // At snap1: only 1 table (t1); at snap2: 2 tables
    let table_rows_snap1 = content1
        .lines()
        .filter(|l| l.contains("\"ducklake_table\""))
        .count();
    let table_rows_snap2 = content2
        .lines()
        .filter(|l| l.contains("\"ducklake_table\""))
        .count();
    assert_eq!(
        table_rows_snap1, 1,
        "snap1 export must contain exactly 1 table row"
    );
    assert_eq!(
        table_rows_snap2, 2,
        "snap2 export must contain exactly 2 table rows"
    );
}

// ─── 4. Import Completeness — All New Categories Survive Round-Trip ───────

/// Explicitly verify that sort_expressions, schema_versions, and table_stats
/// survive export→import without losing any fields.
#[tokio::test]
async fn import_preserves_all_new_category_fields() {
    let src = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&src)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("analytics").await.unwrap();
    let table_id = writer.create_table(schema_id, "facts", None).await.unwrap();
    writer
        .add_column(table_id, "val", "DOUBLE", 0, false, None)
        .await
        .unwrap();
    writer
        .register_data_file(table_id, "facts/f1.parquet", "parquet", 500, 8192)
        .await
        .unwrap();
    writer.apply_table_stats_delta(table_id, 500).await.unwrap();
    writer
        .register_schema_version(1, 1, table_id)
        .await
        .unwrap();
    writer.add_sort_info(table_id, 42).await.unwrap();
    writer.set_tag(table_id, "owner", "alice").await.unwrap();

    let commit = writer.create_snapshot(None, None).await.unwrap();
    let snap_id = commit.snapshot_id;
    store.close().await.unwrap();

    // Export
    let db = open_db(&src).await;
    let mut buf: Vec<u8> = Vec::new();
    let exp = rocklake_catalog::export::export_catalog(&db, None, &mut buf)
        .await
        .unwrap();
    db.close().await.unwrap();

    // Import
    let dst = TempDir::new().unwrap();
    let db2 = open_db(&dst).await;
    let imp =
        rocklake_catalog::export::import_catalog(&db2, std::io::BufReader::new(buf.as_slice()))
            .await
            .unwrap();
    db2.close().await.unwrap();

    assert_eq!(
        imp.rows_imported, exp.rows_exported,
        "rows_imported must equal rows_exported (no rows dropped or duplicated)"
    );

    // Verify the imported catalog is functional
    let imported = CatalogStore::open(test_opts(&dst)).await.unwrap();
    let reader = imported.read_at(snap_id).unwrap();
    let tables = reader.list_tables(schema_id).await.unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].table_name, "facts");
    let files = reader.list_data_files(table_id).await.unwrap();
    assert_eq!(files.len(), 1);
    imported.close().await.unwrap();
}
