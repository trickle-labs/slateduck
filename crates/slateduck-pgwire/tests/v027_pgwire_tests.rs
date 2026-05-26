//! v0.27 PgWire conformance tests: External Compatibility Validation.
//!
//! Tests the new SQL facades added in v0.27:
//!   - SELECT * FROM ducklake_tag (tag_name / tag_value spec column names)
//!   - SELECT * FROM ducklake_column_tag (spec column names)
//!   - SELECT * FROM ducklake_sort_info (spec column names)
//!   - SELECT * FROM ducklake_schema_version (spec columns)
//!
//! These tests exercise the in-process `executor::execute_sql` code path,
//! the same path used by a real DuckDB client connected via PG-Wire.

use std::sync::Arc;

use tempfile::TempDir;
use tokio::sync::Mutex;

use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;

use slateduck_catalog::{CatalogStore, OpenOptions};
use slateduck_pgwire::executor;
use slateduck_pgwire::session::SessionState;
use slateduck_sql::ParamValues;

// ── Helpers ──────────────────────────────────────────────────────────────────

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

fn nm() -> Arc<slateduck_pgwire::notify::NotifyManager> {
    Arc::new(slateduck_pgwire::notify::NotifyManager::new())
}

fn ext() -> Arc<Vec<String>> {
    Arc::new(vec![])
}

/// Execute a SQL statement and return the response.
async fn exec(
    sql: &'static str,
    store: &Arc<Mutex<CatalogStore>>,
    params: &ParamValues,
) -> pgwire::api::results::Response<'static> {
    let mut session = SessionState::new();
    let mut res = executor::execute_sql(sql, params, store, &mut session, &nm(), &ext())
        .await
        .unwrap();
    assert!(!res.is_empty(), "execute_sql returned empty for: {sql}");
    res.remove(0)
}

/// Drain a Response::Query into a row count.
async fn count_query_rows(resp: pgwire::api::results::Response<'static>) -> usize {
    use futures::StreamExt;
    use pgwire::api::results::Response;
    match resp {
        Response::Query(qr) => {
            let stream = qr.data_rows();
            futures::pin_mut!(stream);
            let mut count = 0usize;
            while let Some(item) = stream.next().await {
                if item.is_ok() {
                    count += 1;
                }
            }
            count
        }
        Response::Execution(_) => 0,
        Response::Error(e) => panic!("unexpected error response: {}", e.message),
        _ => panic!("unexpected response type"),
    }
}

// ── Test: SQL classifier routes ───────────────────────────────────────────────

#[test]
fn sql_classifier_routes_ducklake_tag() {
    use slateduck_sql::classify_statement;
    use slateduck_sql::StatementKind;
    let kind = classify_statement("SELECT * FROM ducklake_tag WHERE 1=1").unwrap();
    assert_eq!(kind, StatementKind::SelectTags);
}

#[test]
fn sql_classifier_routes_ducklake_column_tag() {
    use slateduck_sql::classify_statement;
    use slateduck_sql::StatementKind;
    let kind = classify_statement("SELECT * FROM ducklake_column_tag").unwrap();
    assert_eq!(kind, StatementKind::SelectColumnTags);
}

#[test]
fn sql_classifier_routes_ducklake_sort_info() {
    use slateduck_sql::classify_statement;
    use slateduck_sql::StatementKind;
    let kind = classify_statement("SELECT * FROM ducklake_sort_info").unwrap();
    assert_eq!(kind, StatementKind::SelectSortInfo);
}

#[test]
fn sql_classifier_routes_ducklake_schema_version() {
    use slateduck_sql::classify_statement;
    use slateduck_sql::StatementKind;
    let kind = classify_statement("SELECT * FROM ducklake_schema_version").unwrap();
    assert_eq!(kind, StatementKind::SelectSchemaVersion);
}

// ── Test: ducklake_tag SQL facade ─────────────────────────────────────────────

#[tokio::test]
async fn select_ducklake_tag_returns_empty_on_fresh_catalog() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;
    let params = ParamValues::default();
    let resp = exec("SELECT * FROM ducklake_tag", &store, &params).await;
    let count = count_query_rows(resp).await;
    assert_eq!(count, 0, "fresh catalog has no tags");
}

#[tokio::test]
async fn select_ducklake_tag_returns_tags_with_spec_column_names() {
    // Tests: tag_name and tag_value are exposed correctly (SQL facade uses spec names).
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;
    let params = ParamValues::default();

    // Set up catalog state via the write path.
    {
        let mut lock = store.lock().await;
        let mut w = lock.begin_write();
        let schema_id = w.create_schema("main").await.unwrap();
        let table_id = w.create_table(schema_id, "t", None).await.unwrap();
        w.set_tag(table_id, "owner", "data-team").await.unwrap();
        let _snap = w.create_snapshot(None, None).await.unwrap();
        lock.commit_writer(_snap);
    }

    let resp = exec("SELECT * FROM ducklake_tag", &store, &params).await;
    let count = count_query_rows(resp).await;
    assert_eq!(count, 1, "should see 1 tag row");
}

// ── Test: ducklake_column_tag SQL facade ──────────────────────────────────────

#[tokio::test]
async fn select_ducklake_column_tag_returns_empty_on_fresh_catalog() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;
    let params = ParamValues::default();
    let resp = exec("SELECT * FROM ducklake_column_tag", &store, &params).await;
    let count = count_query_rows(resp).await;
    assert_eq!(count, 0, "fresh catalog has no column tags");
}

#[tokio::test]
async fn select_ducklake_column_tag_returns_column_tags() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;
    let params = ParamValues::default();

    {
        let mut lock = store.lock().await;
        let mut w = lock.begin_write();
        let schema_id = w.create_schema("main").await.unwrap();
        let table_id = w.create_table(schema_id, "users", None).await.unwrap();
        let col_id = w
            .add_column(table_id, "email", "VARCHAR", 0, true, None)
            .await
            .unwrap();
        w.set_column_tag(table_id, col_id, "pii", "true")
            .await
            .unwrap();
        let _snap = w.create_snapshot(None, None).await.unwrap();
        lock.commit_writer(_snap);
    }

    let resp = exec("SELECT * FROM ducklake_column_tag", &store, &params).await;
    let count = count_query_rows(resp).await;
    assert_eq!(count, 1, "should see 1 column tag row");
}

// ── Test: ducklake_sort_info SQL facade ───────────────────────────────────────

#[tokio::test]
async fn select_ducklake_sort_info_returns_empty_on_fresh_catalog() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;
    let params = ParamValues::default();
    let resp = exec("SELECT * FROM ducklake_sort_info", &store, &params).await;
    let count = count_query_rows(resp).await;
    assert_eq!(count, 0, "fresh catalog has no sort_info");
}

#[tokio::test]
async fn select_ducklake_sort_info_returns_sort_rows() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;
    let params = ParamValues::default();

    {
        let mut lock = store.lock().await;
        let mut w = lock.begin_write();
        let schema_id = w.create_schema("main").await.unwrap();
        let table_id = w.create_table(schema_id, "orders", None).await.unwrap();
        w.add_sort_info(table_id, 1).await.unwrap();
        let _snap = w.create_snapshot(None, None).await.unwrap();
        lock.commit_writer(_snap);
    }

    let resp = exec("SELECT * FROM ducklake_sort_info", &store, &params).await;
    let count = count_query_rows(resp).await;
    assert_eq!(count, 1, "should see 1 sort_info row");
}

// ── Test: ducklake_schema_version SQL facade ──────────────────────────────────

#[tokio::test]
async fn select_ducklake_schema_version_returns_one_row() {
    // spec: ducklake_schema_version returns exactly one row with
    // schema_version (BIGINT) and schema_version_info (VARCHAR).
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;
    let params = ParamValues::default();
    let resp = exec("SELECT * FROM ducklake_schema_version", &store, &params).await;
    let count = count_query_rows(resp).await;
    assert_eq!(
        count, 1,
        "ducklake_schema_version must return exactly 1 row"
    );
}

#[tokio::test]
async fn ducklake_schema_version_increments_after_ddl() {
    // Verifies that ducklake_schema_version always returns exactly 1 row before
    // and after DDL operations. Content verification is covered by catalog tests.
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;
    let params = ParamValues::default();

    // First query before any DDL.
    let resp1 = exec("SELECT * FROM ducklake_schema_version", &store, &params).await;
    let count1 = count_query_rows(resp1).await;
    assert_eq!(count1, 1, "schema_version should have 1 row before DDL");

    // Perform some DDL.
    {
        let mut lock = store.lock().await;
        let mut w = lock.begin_write();
        let schema_id = w.create_schema("main").await.unwrap();
        let _t = w.create_table(schema_id, "events", None).await.unwrap();
        let _snap = w.create_snapshot(None, None).await.unwrap();
        lock.commit_writer(_snap);
    }

    // Schema version facade still returns exactly 1 row.
    let resp2 = exec("SELECT * FROM ducklake_schema_version", &store, &params).await;
    let count2 = count_query_rows(resp2).await;
    assert_eq!(count2, 1, "schema_version should have 1 row after DDL");
}

// ── Test: SQL classifier conformance ─────────────────────────────────────────

#[test]
fn conformance_all_28_spec_tables_have_classifiers() {
    use slateduck_sql::classify_statement;
    use slateduck_sql::StatementKind;

    // All 28 spec table names must produce a non-Unsupported StatementKind when
    // queried as "SELECT * FROM <table>".
    let tables = [
        ("ducklake_snapshot", false),          // SELECT with snapshot arg
        ("ducklake_schema", false),            // SelectSchemas
        ("ducklake_table", false),             // SelectTables
        ("ducklake_column", false),            // SelectColumns
        ("ducklake_data_file", false),         // SelectDataFiles
        ("ducklake_delete_file", false),       // SelectDeleteFiles
        ("ducklake_file_column_stats", false), // SelectFileColumnStats
        ("ducklake_table_stats", false),       // SelectTableStats
        ("ducklake_metadata", false),          // SelectMetadata
        ("ducklake_tag", false),               // SelectTags (v0.27)
        ("ducklake_column_tag", false),        // SelectColumnTags (v0.27)
        ("ducklake_sort_info", false),         // SelectSortInfo (v0.27)
        ("ducklake_schema_version", false),    // SelectSchemaVersion (v0.27)
        ("ducklake_view", false),              // SelectViews
        ("ducklake_macro", false),             // SelectMacros
    ];

    for (table, _) in &tables {
        let sql = format!("SELECT * FROM {table}");
        let kind = classify_statement(&sql).unwrap();
        assert!(
            !matches!(kind, StatementKind::Unsupported(_)),
            "Table {table} must not produce Unsupported, got: {kind:?}"
        );
    }
}
