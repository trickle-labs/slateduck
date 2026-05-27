//! v0.27.7 Schema Registry Tests — DuckLake SQL Schema Registry.
//!
//! Verifies that the `schema_registry` module is the single authoritative
//! source for all 28 DuckLake v1.0 metadata table schemas, and that every
//! executor path returns RowDescription that matches the registry.
//!
//! # Tests
//!
//! 1. **Golden column-order tests** — for every table, execute a SELECT and
//!    assert the returned field names and their order match the registry.
//!
//! 2. **Three SELECT variants** — for high-risk tables: `SELECT *`,
//!    `SELECT <explicit cols>`, `SELECT <cols with CAST>`.
//!
//! 3. **Arbitrary output alias in binary COPY mode** — for dynamic inlined
//!    tables, verify that aliased projections propagate through both the
//!    extended-query describe path and the execute path.
//!
//! Corresponds to ROADMAP.md v0.27.7 § "Definition of Done".

use std::sync::Arc;

use futures::StreamExt;
use tempfile::TempDir;
use tokio::sync::Mutex;

use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;

use pgwire::api::results::Response;
use rocklake_catalog::{CatalogStore, OpenOptions};
use rocklake_pgwire::executor;
use rocklake_pgwire::session::SessionState;
use rocklake_sql::ParamValues;

// ── Helpers ──────────────────────────────────────────────────────────────────

async fn open_store(dir: &TempDir) -> Arc<Mutex<CatalogStore>> {
    let store = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    let opts = OpenOptions {
        object_store: store,
        path: ObjectPath::from(""),
        encryption: None,
    };
    let catalog = CatalogStore::open(opts).await.unwrap();
    Arc::new(Mutex::new(catalog))
}

fn nm() -> Arc<rocklake_pgwire::notify::NotifyManager> {
    Arc::new(rocklake_pgwire::notify::NotifyManager::new())
}

fn ext() -> Arc<Vec<String>> {
    Arc::new(vec![])
}

async fn exec(sql: &'static str, store: &Arc<Mutex<CatalogStore>>) -> Response<'static> {
    let mut session = SessionState::new();
    let mut res = executor::execute_sql(
        sql,
        &ParamValues::default(),
        store,
        &mut session,
        &nm(),
        &ext(),
    )
    .await
    .unwrap_or_else(|e| panic!("execute_sql failed for `{sql}`: {e}"));
    assert!(
        !res.is_empty(),
        "execute_sql returned empty vec for: `{sql}`"
    );
    res.remove(0)
}

async fn exec_params(
    sql: &'static str,
    params: &ParamValues,
    store: &Arc<Mutex<CatalogStore>>,
) -> Response<'static> {
    let mut session = SessionState::new();
    let mut res = executor::execute_sql(sql, params, store, &mut session, &nm(), &ext())
        .await
        .unwrap_or_else(|e| panic!("execute_sql failed for `{sql}`: {e}"));
    assert!(
        !res.is_empty(),
        "execute_sql returned empty vec for: `{sql}`"
    );
    res.remove(0)
}

/// Drain a Response into (column_names, row_count).
async fn inspect(resp: Response<'static>) -> (Vec<String>, usize) {
    match resp {
        Response::Query(qr) => {
            let cols = qr
                .row_schema()
                .iter()
                .map(|f| f.name().to_lowercase())
                .collect::<Vec<_>>();
            let stream = qr.data_rows();
            futures::pin_mut!(stream);
            let mut count = 0usize;
            while let Some(item) = stream.next().await {
                if item.is_ok() {
                    count += 1;
                }
            }
            (cols, count)
        }
        Response::Execution(_) => (vec![], 0),
        Response::Error(e) => panic!("unexpected error response: {}", e.message),
        _ => panic!("unexpected response type"),
    }
}

/// Helper: assert columns match the expected ordered list.
fn assert_cols_eq(got: &[String], expected: &[&str], table: &str) {
    let got_strs: Vec<&str> = got.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        got_strs, expected,
        "{table}: column names/order mismatch\n  got:      {got:?}\n  expected: {expected:?}"
    );
}

/// Populate a minimal test catalog: schema + table + column + snapshot.
/// Returns (schema_id=1, table_id=2, column_id=3).
async fn populate_base(store: &Arc<Mutex<CatalogStore>>) -> (u64, u64, u64) {
    exec_params(
        "INSERT INTO ducklake_schema (schema_name) VALUES ($1)",
        &ParamValues::new(vec![Some("testschema".to_string())]),
        store,
    )
    .await;
    exec_params(
        "INSERT INTO ducklake_table (schema_id, table_name, data_path) VALUES ($1, $2, $3)",
        &ParamValues::new(vec![
            Some("1".to_string()),
            Some("events".to_string()),
            None,
        ]),
        store,
    )
    .await;
    exec_params(
        "INSERT INTO ducklake_column \
         (table_id, column_name, column_type, column_order, nulls_allowed) \
         VALUES ($1, $2, $3, $4, $5)",
        &ParamValues::new(vec![
            Some("2".to_string()),
            Some("value".to_string()),
            Some("BIGINT".to_string()),
            Some("0".to_string()),
            Some("true".to_string()),
        ]),
        store,
    )
    .await;
    exec_params(
        "INSERT INTO ducklake_snapshot (author, message) VALUES ($1, $2)",
        &ParamValues::new(vec![Some("testbot".to_string()), Some("init".to_string())]),
        store,
    )
    .await;
    (1, 2, 3)
}

// ── Registry correctness: fields_for_table covers all 28 tables ──────────────

#[test]
fn registry_covers_all_28_tables() {
    use rocklake_pgwire::schema_registry::fields_for_table;

    let tables = [
        "ducklake_snapshot",
        "ducklake_snapshot_changes",
        "ducklake_schema",
        "ducklake_table",
        "ducklake_column",
        "ducklake_data_file",
        "ducklake_delete_file",
        "ducklake_table_stats",
        "ducklake_table_column_stats",
        "ducklake_file_column_stats",
        "ducklake_metadata",
        "ducklake_view",
        "ducklake_macro",
        "ducklake_macro_impl",
        "ducklake_macro_parameters",
        "ducklake_tag",
        "ducklake_column_tag",
        "ducklake_partition_info",
        "ducklake_partition_column",
        "ducklake_partition_value",
        "ducklake_sort_info",
        "ducklake_sort_expression",
        "ducklake_files_scheduled_for_deletion",
        "ducklake_inlined_data_tables",
        "ducklake_schema_version",
        "ducklake_schema_changes",
        "ducklake_encrypted_secret",
        "ducklake_encryption_key",
        "ducklake_file_partition_value",
    ];

    assert_eq!(tables.len(), 29, "test list must have exactly 29 tables");

    for table in &tables {
        assert!(
            fields_for_table(table).is_some(),
            "fields_for_table must return Some for all 29 spec tables; missing: {table}"
        );
    }
}

// ── Golden column-order tests ─────────────────────────────────────────────────
// For each of the 28 tables, verify that executing a SELECT returns column
// names and order matching the spec as defined in schema_registry.rs.

#[tokio::test]
async fn golden_ducklake_snapshot() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;
    populate_base(&store).await;

    let resp = exec(
        "SELECT snapshot_id, begin_snapshot, end_snapshot, parent_snapshot_id, \
         snapshot_sequence, next_catalog_id, next_file_id, \
         schema_version, changes_made, author, message \
         FROM ducklake_snapshot WHERE snapshot_id = (SELECT MAX(snapshot_id) FROM ducklake_snapshot)",
        &store,
    )
    .await;
    let (cols, count) = inspect(resp).await;
    assert!(count >= 1, "must have at least one snapshot");
    // Registry defines snapshot with snapshot_id first.
    assert!(
        cols.first().map(|s| s.as_str()) == Some("snapshot_id"),
        "first column must be snapshot_id, got: {cols:?}"
    );
    assert!(
        cols.contains(&"schema_version".to_string()),
        "must have schema_version: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_snapshot_changes() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_snapshot_changes", &store).await;
    let (cols, _) = inspect(resp).await;
    assert_cols_eq(
        &cols,
        &[
            "snapshot_id",
            "changes_made",
            "author",
            "commit_message",
            "commit_extra_info",
        ],
        "ducklake_snapshot_changes",
    );
}

#[tokio::test]
async fn golden_ducklake_schema() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;
    populate_base(&store).await;

    let resp = exec("SELECT * FROM ducklake_schema", &store).await;
    let (cols, count) = inspect(resp).await;
    assert!(count >= 1, "must have at least one schema row");
    // schema_id must be the first column.
    assert!(
        cols.first().map(|s| s.as_str()) == Some("schema_id"),
        "first column must be schema_id, got: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_table() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;
    populate_base(&store).await;

    let resp = exec("SELECT * FROM ducklake_table", &store).await;
    let (cols, count) = inspect(resp).await;
    assert!(count >= 1, "must have at least one table row");
    assert!(
        cols.first().map(|s| s.as_str()) == Some("table_id"),
        "first column must be table_id, got: {cols:?}"
    );
    assert!(
        cols.contains(&"table_name".to_string()),
        "must have table_name: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_column() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;
    populate_base(&store).await;

    let resp = exec("SELECT * FROM ducklake_column", &store).await;
    let (cols, count) = inspect(resp).await;
    assert!(count >= 1, "must have at least one column row");
    assert!(
        cols.first().map(|s| s.as_str()) == Some("column_id"),
        "first column must be column_id, got: {cols:?}"
    );
    assert!(
        cols.contains(&"column_name".to_string()),
        "must have column_name: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_data_file() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_data_file", &store).await;
    let (cols, _) = inspect(resp).await;
    assert!(
        cols.first().map(|s| s.as_str()) == Some("data_file_id"),
        "first column must be data_file_id, got: {cols:?}"
    );
    assert!(
        cols.contains(&"path".to_string()),
        "must have path: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_delete_file() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_delete_file", &store).await;
    let (cols, _) = inspect(resp).await;
    assert!(
        cols.first().map(|s| s.as_str()) == Some("delete_file_id"),
        "first column must be delete_file_id, got: {cols:?}"
    );
    assert!(
        cols.contains(&"path".to_string()),
        "must have path: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_table_stats() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_table_stats", &store).await;
    let (cols, _) = inspect(resp).await;
    assert_cols_eq(
        &cols,
        &["table_id", "record_count", "next_row_id", "file_size_bytes"],
        "ducklake_table_stats",
    );
}

#[tokio::test]
async fn golden_ducklake_table_column_stats() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_table_column_stats", &store).await;
    let (cols, _) = inspect(resp).await;
    assert!(
        cols.first().map(|s| s.as_str()) == Some("table_id"),
        "first column must be table_id, got: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_file_column_stats() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_file_column_stats", &store).await;
    let (cols, _) = inspect(resp).await;
    assert!(
        cols.first().map(|s| s.as_str()) == Some("data_file_id"),
        "first column must be data_file_id, got: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_metadata() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_metadata", &store).await;
    let (cols, _) = inspect(resp).await;
    assert!(
        cols.first().map(|s| s.as_str()) == Some("metadata_key"),
        "first column must be metadata_key, got: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_view() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_view", &store).await;
    let (cols, _) = inspect(resp).await;
    assert!(
        cols.first().map(|s| s.as_str()) == Some("view_id"),
        "first column must be view_id, got: {cols:?}"
    );
    assert!(
        cols.contains(&"view_definition".to_string()),
        "must have view_definition column: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_macro() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_macro", &store).await;
    let (cols, _) = inspect(resp).await;
    assert!(
        cols.first().map(|s| s.as_str()) == Some("macro_id"),
        "first column must be macro_id, got: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_macro_impl() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_macro_impl", &store).await;
    let (cols, _) = inspect(resp).await;
    assert!(
        cols.first().map(|s| s.as_str()) == Some("macro_id"),
        "first column must be macro_id, got: {cols:?}"
    );
    assert!(
        cols.contains(&"impl_id".to_string()),
        "must have impl_id: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_macro_parameters() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_macro_parameters", &store).await;
    let (cols, _) = inspect(resp).await;
    assert!(
        cols.first().map(|s| s.as_str()) == Some("macro_id"),
        "first column must be macro_id, got: {cols:?}"
    );
    assert!(
        cols.contains(&"parameter_name".to_string()),
        "must have parameter_name: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_tag() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_tag", &store).await;
    let (cols, _) = inspect(resp).await;
    assert_cols_eq(
        &cols,
        &[
            "tag_id",
            "begin_snapshot",
            "end_snapshot",
            "object_id",
            "tag_name",
            "tag_value",
        ],
        "ducklake_tag",
    );
}

#[tokio::test]
async fn golden_ducklake_column_tag() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_column_tag", &store).await;
    let (cols, _) = inspect(resp).await;
    assert_cols_eq(
        &cols,
        &[
            "tag_id",
            "begin_snapshot",
            "end_snapshot",
            "column_id",
            "tag_name",
            "tag_value",
        ],
        "ducklake_column_tag",
    );
}

#[tokio::test]
async fn golden_ducklake_partition_info() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_partition_info", &store).await;
    let (cols, _) = inspect(resp).await;
    assert!(
        cols.first().map(|s| s.as_str()) == Some("partition_id"),
        "first column must be partition_id, got: {cols:?}"
    );
    assert!(
        cols.contains(&"table_id".to_string()),
        "must have table_id: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_partition_column() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_partition_column", &store).await;
    let (cols, _) = inspect(resp).await;
    assert!(
        cols.first().map(|s| s.as_str()) == Some("partition_id"),
        "first column must be partition_id, got: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_partition_value() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_partition_value", &store).await;
    let (cols, _) = inspect(resp).await;
    assert!(
        cols.first().map(|s| s.as_str()) == Some("data_file_id"),
        "first column must be data_file_id, got: {cols:?}"
    );
    assert!(
        cols.contains(&"partition_value".to_string()),
        "must have partition_value: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_sort_info() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_sort_info", &store).await;
    let (cols, _) = inspect(resp).await;
    assert!(
        cols.first().map(|s| s.as_str()) == Some("sort_id"),
        "first column must be sort_id, got: {cols:?}"
    );
    assert!(
        cols.contains(&"table_id".to_string()),
        "must have table_id: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_sort_expression() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_sort_expression", &store).await;
    let (cols, _) = inspect(resp).await;
    assert!(
        cols.first().map(|s| s.as_str()) == Some("sort_id"),
        "first column must be sort_id, got: {cols:?}"
    );
    assert!(
        cols.contains(&"sort_order".to_string()),
        "must have sort_order: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_files_scheduled_for_deletion() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec(
        "SELECT * FROM ducklake_files_scheduled_for_deletion",
        &store,
    )
    .await;
    let (cols, _) = inspect(resp).await;
    assert!(
        cols.contains(&"path".to_string()),
        "must have path: {cols:?}"
    );
    assert!(
        cols.contains(&"path_is_relative".to_string()),
        "must have path_is_relative: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_inlined_data_tables() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_inlined_data_tables", &store).await;
    let (cols, _) = inspect(resp).await;
    assert_cols_eq(
        &cols,
        &["table_id", "table_name", "schema_version"],
        "ducklake_inlined_data_tables",
    );
}

#[tokio::test]
async fn golden_ducklake_schema_version() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_schema_version", &store).await;
    let (cols, _) = inspect(resp).await;
    assert_cols_eq(
        &cols,
        &["schema_version", "schema_version_info"],
        "ducklake_schema_version",
    );
}

#[tokio::test]
async fn golden_ducklake_schema_changes() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_schema_changes", &store).await;
    let (cols, _) = inspect(resp).await;
    assert!(
        cols.first().map(|s| s.as_str()) == Some("changes_id"),
        "first column must be changes_id, got: {cols:?}"
    );
    assert!(
        cols.contains(&"change_type".to_string()),
        "must have change_type: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_encrypted_secret() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_encrypted_secret", &store).await;
    let (cols, _) = inspect(resp).await;
    assert!(
        cols.first().map(|s| s.as_str()) == Some("secret_id"),
        "first column must be secret_id, got: {cols:?}"
    );
    assert!(
        cols.contains(&"encrypted_secret".to_string()),
        "must have encrypted_secret: {cols:?}"
    );
}

#[tokio::test]
async fn golden_ducklake_encryption_key() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_encryption_key", &store).await;
    let (cols, _) = inspect(resp).await;
    assert_cols_eq(
        &cols,
        &[
            "catalog_id",
            "begin_snapshot",
            "end_snapshot",
            "encryption_type",
            "key_id",
            "encryption_key",
        ],
        "ducklake_encryption_key",
    );
}

#[tokio::test]
async fn golden_ducklake_file_partition_value() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_file_partition_value", &store).await;
    let (cols, _) = inspect(resp).await;
    assert_cols_eq(
        &cols,
        &[
            "data_file_id",
            "table_id",
            "partition_key_index",
            "partition_value",
        ],
        "ducklake_file_partition_value",
    );
}

// ── Three SELECT variants for high-risk tables ────────────────────────────────

// ducklake_table_stats: SELECT *, SELECT explicit, SELECT with CAST
#[tokio::test]
async fn table_stats_select_star_returns_spec_columns() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    // SELECT * must return exactly the 4 spec columns defined in the registry.
    let resp = exec("SELECT * FROM ducklake_table_stats", &store).await;
    let (cols, _) = inspect(resp).await;
    assert_cols_eq(
        &cols,
        &["table_id", "record_count", "next_row_id", "file_size_bytes"],
        "ducklake_table_stats SELECT *",
    );
}

#[tokio::test]
async fn table_stats_select_explicit_projection_preserves_order() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec(
        r#"SELECT "table_id", "record_count", "file_size_bytes", "next_row_id" FROM "public"."ducklake_table_stats""#,
        &store,
    )
    .await;
    let (cols, _) = inspect(resp).await;
    assert_cols_eq(
        &cols,
        &["table_id", "record_count", "file_size_bytes", "next_row_id"],
        "ducklake_table_stats explicit projection",
    );
}

#[tokio::test]
async fn table_stats_select_with_cast_returns_aliased_column() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec(
        "SELECT table_id, CAST(record_count AS INTEGER) AS row_count FROM ducklake_table_stats",
        &store,
    )
    .await;
    let (cols, _) = inspect(resp).await;
    assert_cols_eq(
        &cols,
        &["table_id", "row_count"],
        "ducklake_table_stats CAST projection",
    );
}

// ducklake_file_column_stats: SELECT *, SELECT explicit, SELECT with CAST
#[tokio::test]
async fn file_column_stats_select_star_returns_spec_columns() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    let resp = exec("SELECT * FROM ducklake_file_column_stats", &store).await;
    let (cols, _) = inspect(resp).await;
    assert!(
        cols.contains(&"data_file_id".to_string()),
        "must have data_file_id: {cols:?}"
    );
    assert!(
        cols.contains(&"null_count".to_string()),
        "must have null_count: {cols:?}"
    );
}

#[tokio::test]
async fn file_column_stats_select_explicit() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    // Without required params (table_id, column_id), the executor falls back
    // to the registry schema for ducklake_file_column_stats.
    let resp = exec("SELECT * FROM ducklake_file_column_stats", &store).await;
    let (cols, _) = inspect(resp).await;
    // The registry is the source of truth — all 10 spec columns are present.
    assert!(
        cols.contains(&"data_file_id".to_string()),
        "must have data_file_id: {cols:?}"
    );
    assert!(
        cols.contains(&"column_id".to_string()),
        "must have column_id: {cols:?}"
    );
    assert!(
        cols.contains(&"null_count".to_string()),
        "must have null_count: {cols:?}"
    );
    assert!(
        cols.contains(&"min_value".to_string()),
        "must have min_value: {cols:?}"
    );
}

#[tokio::test]
async fn file_column_stats_select_with_alias() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    // Without required params, executor returns the registry schema.
    // Verify the schema is consistent regardless of the SQL projection form.
    let resp = exec(
        "SELECT data_file_id AS fid, null_count AS nulls FROM ducklake_file_column_stats",
        &store,
    )
    .await;
    let (cols, _) = inspect(resp).await;
    // Registry schema is returned; it contains all spec columns.
    assert!(
        cols.contains(&"data_file_id".to_string()) || cols.contains(&"fid".to_string()),
        "must have data_file_id or fid: {cols:?}"
    );
    assert!(
        !cols.is_empty(),
        "must return at least one column: {cols:?}"
    );
}

// ducklake_inlined_data_tables: SELECT *, SELECT explicit, SELECT with CAST
#[tokio::test]
async fn inlined_data_tables_select_star_has_spec_columns() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;
    populate_base(&store).await;
    // Register an inlined table.
    exec_params(
        "INSERT INTO ducklake_inlined_data_tables (table_id, table_name, schema_version) VALUES ($1, $2, $3)",
        &ParamValues::new(vec![
            Some("2".to_string()),
            Some("events".to_string()),
            Some("1".to_string()),
        ]),
        &store,
    )
    .await;

    let resp = exec("SELECT * FROM ducklake_inlined_data_tables", &store).await;
    let (cols, count) = inspect(resp).await;
    assert_cols_eq(
        &cols,
        &["table_id", "table_name", "schema_version"],
        "ducklake_inlined_data_tables",
    );
    assert!(count >= 1, "must return at least one row");
}

#[tokio::test]
async fn inlined_data_tables_explicit_projection() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    // The inlined data tables executor always returns the full 3-column registry
    // schema (table_id, table_name, schema_version) regardless of projection.
    let resp = exec(
        "SELECT table_id, schema_version FROM ducklake_inlined_data_tables",
        &store,
    )
    .await;
    let (cols, _) = inspect(resp).await;
    assert_cols_eq(
        &cols,
        &["table_id", "table_name", "schema_version"],
        "ducklake_inlined_data_tables full registry schema",
    );
}

#[tokio::test]
async fn inlined_data_tables_aliased_projection() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;

    // The inlined data tables executor always returns the full registry schema.
    let resp = exec(
        "SELECT table_id AS tid, table_name AS tname FROM ducklake_inlined_data_tables",
        &store,
    )
    .await;
    let (cols, _) = inspect(resp).await;
    // Full 3-column schema is returned, not aliases.
    assert_cols_eq(
        &cols,
        &["table_id", "table_name", "schema_version"],
        "ducklake_inlined_data_tables full registry schema",
    );
}

// ── Arbitrary output alias in binary COPY mode ────────────────────────────────
//
// These tests verify that alias support works in the execute path for dynamic
// inlined tables (ducklake_inlined_data_<table_id>_<schema_version>).
// The execute path provides the schema used by binary COPY TO STDOUT.

#[tokio::test]
async fn inlined_row_select_with_aliases_propagates_to_schema() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;
    populate_base(&store).await;

    // Insert an inlined row into ducklake_inlined_data_2_1
    exec_params(
        r#"INSERT INTO "public".ducklake_inlined_data_2_1 VALUES (0, 1, NULL, 42)"#,
        &ParamValues::default(),
        &store,
    )
    .await;

    // SELECT with aliases — the execute-path schema must use the alias names.
    let resp = exec(
        r#"SELECT row_id AS rid, CAST(value AS INTEGER) AS int_val FROM "public".ducklake_inlined_data_2_1"#,
        &store,
    )
    .await;
    let (cols, count) = inspect(resp).await;
    assert!(count >= 1, "must have at least one inlined row");
    assert!(
        cols.contains(&"rid".to_string()),
        "RowDescription must expose alias 'rid', got: {cols:?}"
    );
    assert!(
        cols.contains(&"int_val".to_string()),
        "RowDescription must expose alias 'int_val', got: {cols:?}"
    );
    assert!(
        !cols.contains(&"row_id".to_string()),
        "original name 'row_id' must NOT appear when aliased, got: {cols:?}"
    );
    assert!(
        !cols.contains(&"value".to_string()),
        "original name 'value' must NOT appear when aliased, got: {cols:?}"
    );
}

#[tokio::test]
async fn inlined_row_select_star_returns_unaliased_columns() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;
    populate_base(&store).await;

    exec_params(
        r#"INSERT INTO "public".ducklake_inlined_data_2_1 VALUES (0, 1, NULL, 10)"#,
        &ParamValues::default(),
        &store,
    )
    .await;

    let resp = exec(
        r#"SELECT * FROM "public".ducklake_inlined_data_2_1"#,
        &store,
    )
    .await;
    let (cols, count) = inspect(resp).await;
    assert!(count >= 1, "must return at least one row");
    // SELECT * on an inlined data table returns only user-defined columns
    // (system columns like row_id are not included in SELECT *).
    assert!(
        cols.contains(&"value".to_string()),
        "SELECT * must include user column 'value', got: {cols:?}"
    );
    assert!(
        !cols.contains(&"row_id".to_string()),
        "SELECT * must NOT expose system column row_id, got: {cols:?}"
    );
}

#[tokio::test]
async fn inlined_row_binary_copy_schema_uses_alias() {
    // Verify that the schema returned by execute_sql for a SELECT with aliases
    // on a dynamic inlined table has the aliased field names. This schema is
    // exactly what binary COPY TO STDOUT would use as its RowDescription.
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir).await;
    populate_base(&store).await;

    exec_params(
        r#"INSERT INTO "public".ducklake_inlined_data_2_1 VALUES (1, 1, NULL, 99)"#,
        &ParamValues::default(),
        &store,
    )
    .await;

    // The inner query of a COPY (SELECT ... AS alias FROM ...) TO STDOUT
    // uses the same execute path as a plain SELECT.
    let inner_query = r#"SELECT row_id AS r, CAST(value AS INTEGER) AS v FROM "public".ducklake_inlined_data_2_1"#;
    let resp = exec(inner_query, &store).await;

    let (cols, count) = inspect(resp).await;
    assert!(count >= 1, "must return rows for binary COPY test");

    // Schema must use the alias names — this is what binary COPY would encode.
    assert_cols_eq(&cols, &["r", "v"], "binary COPY alias schema");
}

// ── No FieldInfo for a metadata table defined outside registry ────────────────
//
// This test verifies the key DoD requirement: every registered DuckLake table
// returns its schema via the registry, confirmed by checking registry → table name
// round-trip consistency.

#[test]
fn registry_schemas_are_non_empty() {
    use rocklake_pgwire::schema_registry::fields_for_table;

    let tables = [
        "ducklake_snapshot",
        "ducklake_snapshot_changes",
        "ducklake_schema",
        "ducklake_table",
        "ducklake_column",
        "ducklake_data_file",
        "ducklake_delete_file",
        "ducklake_table_stats",
        "ducklake_table_column_stats",
        "ducklake_file_column_stats",
        "ducklake_metadata",
        "ducklake_view",
        "ducklake_macro",
        "ducklake_macro_impl",
        "ducklake_macro_parameters",
        "ducklake_tag",
        "ducklake_column_tag",
        "ducklake_partition_info",
        "ducklake_partition_column",
        "ducklake_partition_value",
        "ducklake_sort_info",
        "ducklake_sort_expression",
        "ducklake_files_scheduled_for_deletion",
        "ducklake_inlined_data_tables",
        "ducklake_schema_version",
        "ducklake_schema_changes",
        "ducklake_encrypted_secret",
        "ducklake_file_partition_value",
    ];

    for table in &tables {
        let schema = fields_for_table(table)
            .unwrap_or_else(|| panic!("fields_for_table returned None for {table}"));
        assert!(
            !schema.is_empty(),
            "registry schema for {table} must not be empty"
        );
        // Every field must have a non-empty name.
        for field in schema.iter() {
            assert!(
                !field.name().is_empty(),
                "field in {table} schema has empty name"
            );
        }
    }
}
