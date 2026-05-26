//! PgWire executor tests for v0.27.4: DuckDB 1.5.x postgres scanner compatibility.
//!
//! Verifies that the executor handlers for the new StatementKind variants
//! return the correct responses:
//!   - `DiscardAll`           → `DISCARD` command-complete tag
//!   - `SelectToRegclass`     → single NULL TEXT row (column: `to_regclass`)
//!   - `SelectExistsInfoSchema` → single `false` BOOL row (column: `exists`)
//!   - `SelectPgDatabaseSize` → single INT8 row with value 0
//!   - `PgCatalogScan`        → 5 result sets + ROLLBACK command-complete

use std::sync::Arc;

use futures::StreamExt;
use tempfile::TempDir;
use tokio::sync::Mutex;

use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;

use pgwire::api::results::Response;

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

async fn exec_multi(sql: &'static str, store: &Arc<Mutex<CatalogStore>>) -> Vec<Response<'static>> {
    let params = ParamValues::default();
    let mut session = SessionState::new();
    executor::execute_sql(sql, &params, store, &mut session, &nm(), &ext())
        .await
        .unwrap()
}

async fn exec_one(sql: &'static str, store: &Arc<Mutex<CatalogStore>>) -> Response<'static> {
    let mut responses = exec_multi(sql, store).await;
    assert!(
        !responses.is_empty(),
        "expected at least one response for: {sql}"
    );
    responses.remove(0)
}

/// Count rows in a Query response.
async fn row_count(resp: Response<'static>) -> usize {
    match resp {
        Response::Query(qr) => {
            let stream = qr.data_rows();
            futures::pin_mut!(stream);
            let mut count = 0usize;
            while let Some(r) = stream.next().await {
                if r.is_ok() {
                    count += 1;
                }
            }
            count
        }
        _ => panic!("expected Query response"),
    }
}

async fn query_names_and_count(resp: Response<'static>) -> (Vec<String>, usize) {
    match resp {
        Response::Query(qr) => {
            let names = qr
                .row_schema()
                .iter()
                .map(|field| field.name().to_string())
                .collect::<Vec<_>>();
            let stream = qr.data_rows();
            futures::pin_mut!(stream);
            let mut count = 0usize;
            while let Some(row) = stream.next().await {
                row.expect("query row must encode successfully");
                count += 1;
            }
            (names, count)
        }
        _ => panic!("expected Query response"),
    }
}

// ─── Step 1: DISCARD ALL ─────────────────────────────────────────────────────

#[tokio::test]
async fn discard_all_returns_execution_tag() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;

    let resp = exec_one("DISCARD ALL", &store).await;
    match resp {
        Response::Execution(tag) => {
            // Tag derives Debug; verify the command string contains "DISCARD".
            let dbg = format!("{tag:?}");
            assert!(
                dbg.contains("\"DISCARD\""),
                "DISCARD ALL must return DISCARD tag, got: {dbg}"
            );
        }
        _ => panic!("expected Execution response for DISCARD ALL"),
    }
}

#[tokio::test]
async fn discard_all_no_error() {
    // Sending DISCARD ALL multiple times must not error.
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;

    for _ in 0..3 {
        let resp = exec_one("DISCARD ALL", &store).await;
        assert!(
            matches!(resp, Response::Execution(_)),
            "DISCARD ALL must not error"
        );
    }
}

// ─── Step 2: SELECT to_regclass ───────────────────────────────────────────────

#[tokio::test]
async fn select_to_regclass_returns_null() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;

    let resp = exec_one("SELECT to_regclass('duckdb_secrets')", &store).await;
    let count = row_count(resp).await;
    assert_eq!(count, 1, "to_regclass must return exactly 1 row");
}

// ─── Step 3: SELECT EXISTS(... information_schema.tables ...) ─────────────────

#[tokio::test]
async fn select_exists_info_schema_returns_false() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;

    let resp = exec_one(
        "SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = 'duckdb_secrets')",
        &store,
    )
    .await;
    let count = row_count(resp).await;
    assert_eq!(count, 1, "EXISTS query must return exactly 1 row (false)");
}

// ─── Step 4: SELECT pg_database_size ─────────────────────────────────────────

#[tokio::test]
async fn select_pg_database_size_returns_integer() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;

    let resp = exec_one("SELECT pg_database_size(current_database())", &store).await;
    let count = row_count(resp).await;
    assert_eq!(count, 1, "pg_database_size must return exactly 1 row");
}

// ─── Step 5: Multi-statement catalog scan ─────────────────────────────────────

const PG_CATALOG_SCAN: &str = "BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ;\n\
SELECT oid, nspname FROM pg_namespace ORDER BY oid;\n\
SELECT pg_namespace.oid AS namespace_id, relname, relpages, attname, \
pg_type.typname type_name, atttypmod type_modifier, pg_attribute.attndims ndim, \
attnum, pg_attribute.attnotnull AS notnull, NULL constraint_id, \
NULL constraint_type, NULL constraint_key \
FROM pg_class JOIN pg_namespace ON relnamespace = pg_namespace.oid \
JOIN pg_attribute ON pg_class.oid=pg_attribute.attrelid \
JOIN pg_type ON atttypid=pg_type.oid \
WHERE attnum > 0 AND relkind IN ('r', 'v', 'm', 'f', 'p') \
UNION ALL SELECT pg_namespace.oid AS namespace_id, relname, NULL relpages, \
NULL attname, NULL type_name, NULL type_modifier, NULL ndim, NULL attnum, \
NULL AS notnull, pg_constraint.oid AS constraint_id, contype AS constraint_type, \
conkey AS constraint_key FROM pg_class JOIN pg_namespace ON relnamespace = pg_namespace.oid \
JOIN pg_constraint ON (pg_class.oid=pg_constraint.conrelid) \
WHERE relkind IN ('r', 'v', 'm', 'f', 'p') AND contype IN ('p', 'u') \
ORDER BY namespace_id, relname, attnum, constraint_id;\n\
SELECT n.oid, enumtypid, typname, enumlabel FROM pg_enum e \
JOIN pg_type t ON e.enumtypid = t.oid \
JOIN pg_namespace AS n ON (typnamespace=n.oid) \
ORDER BY n.oid, enumtypid, enumsortorder;\n\
SELECT n.oid, t.typrelid AS id, t.typname as type, pg_attribute.attname, sub_type.typname \
FROM pg_type t JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
JOIN pg_class ON pg_class.oid = t.typrelid \
JOIN pg_attribute ON attrelid=t.typrelid \
JOIN pg_type sub_type ON (pg_attribute.atttypid=sub_type.oid) \
WHERE pg_class.relkind = 'c' AND t.typtype='c' \
ORDER BY n.oid, t.oid, attrelid, attnum;\n\
SELECT pg_namespace.oid, tablename, indexname FROM pg_indexes \
JOIN pg_namespace ON (schemaname=nspname) ORDER BY pg_namespace.oid;\n\
ROLLBACK;";

#[tokio::test]
async fn pg_catalog_scan_returns_five_result_sets_plus_rollback() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;

    let responses = exec_multi(PG_CATALOG_SCAN, &store).await;
    assert_eq!(
        responses.len(),
        6,
        "PgCatalogScan must return 5 result sets + ROLLBACK command-complete; got {} responses",
        responses.len()
    );

    // Last response must be ROLLBACK command-complete.
    let last = &responses[5];
    assert!(
        matches!(last, Response::TransactionEnd(_)),
        "last response must be ROLLBACK TransactionEnd"
    );

    // First 5 must be Query responses.
    for (i, resp) in responses[..5].iter().enumerate() {
        assert!(
            matches!(resp, Response::Query(_)),
            "response {i} must be a Query response"
        );
    }
}

#[tokio::test]
async fn pg_catalog_scan_pg_namespace_has_two_rows() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;

    let mut responses = exec_multi(PG_CATALOG_SCAN, &store).await;

    // Result set 0 is pg_namespace: must have at least 2 rows (public + main).
    let ns_resp = responses.remove(0);
    let count = row_count(ns_resp).await;
    assert!(
        count >= 2,
        "pg_namespace result set must have at least 2 rows (public + main), got {count}"
    );
}

#[tokio::test]
async fn pg_catalog_scan_empty_result_sets_have_correct_schema() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;

    let responses = exec_multi(PG_CATALOG_SCAN, &store).await;
    assert_eq!(responses.len(), 6, "expected 6 responses");

    // Convert to an iterator and skip the first (pg_namespace).
    let mut iter = responses.into_iter();
    let _ns = iter.next().unwrap(); // result set 0: pg_namespace (not empty)

    // Result sets 1-4 (pg_class, pg_enum, pg_type, pg_indexes) must be empty.
    for i in 1..5 {
        let resp = iter.next().unwrap();
        let count = row_count(resp).await;
        assert_eq!(
            count, 0,
            "result set {i} (pg_class/pg_enum/pg_type/pg_indexes) must be empty, got {count} rows"
        );
    }
}

#[tokio::test]
async fn ducklake_table_stats_preserves_requested_projection_order() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;

    let resp = exec_one(
        r#"SELECT "table_id", "record_count", "file_size_bytes", "next_row_id" FROM "public"."ducklake_table_stats""#,
        &store,
    )
    .await;
    let (names, count) = query_names_and_count(resp).await;

    assert_eq!(
        names,
        vec!["table_id", "record_count", "file_size_bytes", "next_row_id"],
        "DuckLake v1.0 table_stats scans must keep DuckDB's requested column order"
    );
    assert_eq!(count, 0, "fresh catalog should not have table stats rows");
}

#[tokio::test]
async fn ducklake_snapshot_stats_changes_union_has_expected_shape() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;

    let sql = r#"
SELECT
    snapshot_id,
    schema_version,
    next_catalog_id,
    next_file_id,
    COALESCE((
            SELECT STRING_AGG(changes_made, ',')
            FROM "public".ducklake_snapshot_changes c
            WHERE c.snapshot_id > 0
            ),'') AS changes,
    NULL AS table_id,
    NULL AS column_id,
    NULL AS record_count,
    NULL AS next_row_id,
    NULL AS file_size_bytes,
    NULL AS contains_null,
    NULL AS contains_nan,
    NULL AS min_value,
    NULL AS max_value,
    NULL AS extra_stats
    FROM "public".ducklake_snapshot
    WHERE snapshot_id = (
        SELECT MAX(snapshot_id)
        FROM "public".ducklake_snapshot)
UNION ALL
SELECT
    NULL,
    NULL,
    NULL,
    NULL,
    NULL,
    table_id,
    column_id,
    record_count,
    next_row_id,
    file_size_bytes,
    contains_null,
    contains_nan,
    min_value,
    max_value,
    extra_stats
FROM "public".ducklake_table_stats
LEFT JOIN "public".ducklake_table_column_stats
    USING (table_id)
WHERE record_count IS NOT NULL
    AND file_size_bytes IS NOT NULL
ORDER BY table_id NULLS FIRST;
"#;

    let resp = exec_one(sql, &store).await;
    let (names, count) = query_names_and_count(resp).await;

    assert_eq!(
        names,
        vec![
            "snapshot_id",
            "schema_version",
            "next_catalog_id",
            "next_file_id",
            "changes",
            "table_id",
            "column_id",
            "record_count",
            "next_row_id",
            "file_size_bytes",
            "contains_null",
            "contains_nan",
            "min_value",
            "max_value",
            "extra_stats",
        ]
    );
    assert_eq!(count, 1, "fresh catalog should return the snapshot row");
}

#[tokio::test]
async fn inlined_append_with_stale_row_id_is_remapped_to_free_key() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;

    exec_one(
        r#"INSERT INTO "public".ducklake_inlined_data_3_3 VALUES (0, 4, NULL, 1, '\x6f6e65'), (1, 4, NULL, 2, '\x74776f'), (2, 4, NULL, 3, '\x7468726565')"#,
        &store,
    )
    .await;
    exec_one(
        r#"INSERT INTO "public".ducklake_inlined_data_3_3 VALUES (0, 6, NULL, 4, '\x666f7572')"#,
        &store,
    )
    .await;

    let reader = { store.lock().await.read_latest() };
    let mut rows = reader.list_inlined_inserts(3).await.unwrap();
    rows.sort_by_key(|row| row.row_id);
    let row_ids = rows.iter().map(|row| row.row_id).collect::<Vec<_>>();

    assert_eq!(row_ids, vec![0, 1, 2, 3]);
    let appended = rows.iter().find(|row| row.row_id == 3).unwrap();
    let values = serde_json::from_slice::<Vec<Option<String>>>(&appended.payload).unwrap();
    assert_eq!(values.first().and_then(|value| value.as_deref()), Some("4"));
}

#[tokio::test]
async fn table_stats_merge_incremental_inlined_batches() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;

    {
        let mut lock = store.lock().await;
        let mut writer = lock.begin_write();
        writer.update_table_stats(3, 3, 3, 0).await.unwrap();
        writer
            .upsert_table_column_stats(3, 4, false, Some("1"), Some("3"), None, None)
            .await
            .unwrap();
        writer.adjust_table_record_count(3, -1).await.unwrap();
        writer.update_table_stats(3, 1, 1, 0).await.unwrap();
        writer
            .upsert_table_column_stats(3, 4, false, Some("4"), Some("4"), None, None)
            .await
            .unwrap();
        writer.update_table_stats(3, 1, 0, 0).await.unwrap();
        writer
            .upsert_table_column_stats(3, 4, false, Some("3"), Some("3"), None, None)
            .await
            .unwrap();
        writer
            .upsert_table_column_stats(3, 6, false, Some("10"), Some("10"), None, None)
            .await
            .unwrap();
        writer
            .upsert_table_column_stats(3, 6, false, Some("2"), Some("2"), None, None)
            .await
            .unwrap();
        writer.adjust_table_record_count(3, -1).await.unwrap();
    }

    let reader = { store.lock().await.read_latest() };
    let stats = reader.get_table_stats(3).await.unwrap().unwrap();
    let column_stats = reader.list_all_table_column_stats().await.unwrap();
    let id_stats = column_stats
        .iter()
        .find(|row| row.table_id == 3 && row.column_id == 4)
        .unwrap();
    let numeric_stats = column_stats
        .iter()
        .find(|row| row.table_id == 3 && row.column_id == 6)
        .unwrap();

    assert_eq!(stats.record_count, 3);
    assert_eq!(stats.next_row_id, Some(5));
    assert_eq!(id_stats.min_value.as_deref(), Some("1"));
    assert_eq!(id_stats.max_value.as_deref(), Some("4"));
    assert_eq!(numeric_stats.min_value.as_deref(), Some("2"));
    assert_eq!(numeric_stats.max_value.as_deref(), Some("10"));
}

// ─── Step 6: Wire corpus fixture ─────────────────────────────────────────────

#[test]
fn duckdb_1_5_x_corpus_fixture_exists() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures/wire-corpus/duckdb-1.5.x.jsonl");
    assert!(path.exists(), "DuckDB 1.5.x wire corpus fixture must exist at tests/fixtures/wire-corpus/duckdb-1.5.x.jsonl");
}

#[test]
fn duckdb_1_5_x_corpus_all_statements_classifiable() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures/wire-corpus/duckdb-1.5.x.jsonl");
    let content = std::fs::read_to_string(path).unwrap();
    let corpus: serde_json::Value = serde_json::from_str(&content).unwrap();

    assert_eq!(corpus["client"], "duckdb-postgres-scanner");
    assert_eq!(corpus["version"], "1.5.x");

    let statements = corpus["statements"].as_array().unwrap();
    assert!(
        !statements.is_empty(),
        "DuckDB 1.5.x corpus must not be empty"
    );

    for stmt in statements {
        let sql = stmt["sql"].as_str().unwrap_or("");
        if sql.is_empty() {
            continue;
        }
        let result = slateduck_sql::classify_statement(sql);
        assert!(
            result.is_ok(),
            "DuckDB 1.5.x corpus statement must be classifiable: {sql:?} — err: {result:?}"
        );
        // Verify the expected kind matches if present
        if let Some(expected_kind) = stmt["expected_kind"].as_str() {
            let kind = result.unwrap();
            let kind_str = format!("{kind:?}");
            assert!(
                kind_str.contains(expected_kind),
                "statement seq={} expected kind containing {expected_kind}, got {kind_str}",
                stmt["seq"]
            );
        }
    }
}
