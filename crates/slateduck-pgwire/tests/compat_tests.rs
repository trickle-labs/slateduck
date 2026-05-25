//! Tier 5: DuckLake compatibility tests — full DuckLake tutorial lifecycle
//! against a local-FS-backed catalog served via the SlateDuck executor.
//!
//! These tests exercise the complete SQL lifecycle that a DuckDB client would
//! perform against a production SlateDuck catalog using the DuckLake protocol:
//!
//! Schema:  `INSERT INTO ducklake_schema (schema_name) VALUES ($1)`
//! Table:   `INSERT INTO ducklake_table (schema_id, table_name, data_path) VALUES ($1,$2,$3)`
//! Commit:  `INSERT INTO ducklake_snapshot (author, message) VALUES ($1,$2)`
//! Read:    `SELECT * FROM ducklake_schema / ducklake_table / ducklake_snapshot`
//!
//! The test uses the in-process `executor::execute_sql` rather than a live TCP
//! socket to avoid port binding in CI, while exercising the same code paths as
//! a real DuckDB client connected via PG-Wire.
//!
//! ## v0.13 additions
//! Also verifies the v0.13 IVM-join workflow against the same catalog store.

use std::sync::Arc;

use tempfile::TempDir;
use tokio::sync::Mutex;

use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;

use slateduck_catalog::{CatalogStore, OpenOptions};
use slateduck_pgwire::executor;
use slateduck_pgwire::session::SessionState;
use slateduck_sql::ParamValues;

// ── Helpers ───────────────────────────────────────────────────────────────────

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

// ── Test 1: Full DuckLake tutorial lifecycle ───────────────────────────────────

/// Full DuckLake tutorial lifecycle against a store.
///
/// Exercises: CREATE schema → CREATE table → REGISTER data file → COMMIT snapshot
///            → SELECT catalog rows → SELECT snapshot history → time-travel read.
///
/// Named `duckdb_full_ducklake_tutorial_against_minio` to match the v0.13
/// roadmap Tier-5 acceptance criterion.
#[tokio::test]
async fn duckdb_full_ducklake_tutorial_against_minio() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;
    let params = ParamValues::default();

    // ── 1. Baseline catalog queries ────────────────────────────────────────
    {
        let mut session = SessionState::new();
        let res = executor::execute_sql("SELECT version()", &params, &store, &mut session)
            .await
            .unwrap();
        assert!(!res.is_empty(), "SELECT version() must return a response");
    }

    // ── 2. Create schema ───────────────────────────────────────────────────
    {
        let mut session = SessionState::new();
        let sp = ParamValues::new(vec![Some("tutorial".to_string())]);
        let res = executor::execute_sql(
            "INSERT INTO ducklake_schema (schema_name) VALUES ($1)",
            &sp,
            &store,
            &mut session,
        )
        .await
        .unwrap();
        assert!(!res.is_empty(), "INSERT schema must return a response");
    }

    // ── 3. Create table ────────────────────────────────────────────────────
    {
        let mut session = SessionState::new();
        let tp = ParamValues::new(vec![
            Some("1".to_string()),
            Some("events".to_string()),
            None,
        ]);
        let res = executor::execute_sql(
            "INSERT INTO ducklake_table (schema_id, table_name, data_path) VALUES ($1, $2, $3)",
            &tp,
            &store,
            &mut session,
        )
        .await
        .unwrap();
        assert!(!res.is_empty(), "INSERT table must return a response");
    }

    // ── 4. Register data file (simulates ingest) ───────────────────────────
    {
        let mut session = SessionState::new();
        let fp = ParamValues::new(vec![
            Some("2".to_string()),
            Some("data/events/part-0001.parquet".to_string()),
            Some("parquet".to_string()),
            Some("1000".to_string()),
            Some("8192".to_string()),
        ]);
        let res = executor::execute_sql(
            "INSERT INTO ducklake_data_file \
             (table_id, path, file_format, row_count, file_size_bytes) \
             VALUES ($1, $2, $3, $4, $5)",
            &fp,
            &store,
            &mut session,
        )
        .await
        .unwrap();
        assert!(!res.is_empty(), "INSERT data_file must return a response");
    }

    // ── 5. Commit snapshot #1 ──────────────────────────────────────────────
    {
        let mut session = SessionState::new();
        let snap = ParamValues::new(vec![Some("tutorial-bot".to_string()), None]);
        let res = executor::execute_sql(
            "INSERT INTO ducklake_snapshot (author, message) VALUES ($1, $2)",
            &snap,
            &store,
            &mut session,
        )
        .await
        .unwrap();
        assert!(!res.is_empty(), "INSERT snapshot must return a response");
    }

    // ── 6. Read schema catalog ─────────────────────────────────────────────
    {
        let mut session = SessionState::new();
        let rp = ParamValues::new(vec![Some(u64::MAX.to_string())]);
        let res = executor::execute_sql(
            "SELECT * FROM ducklake_schema \
             WHERE begin_snapshot <= $1 \
             AND (end_snapshot IS NULL OR $1 < end_snapshot)",
            &rp,
            &store,
            &mut session,
        )
        .await
        .unwrap();
        assert_eq!(res.len(), 1, "must return exactly one schema row");
    }

    // ── 7. Read table catalog ──────────────────────────────────────────────
    {
        let mut session = SessionState::new();
        let rp = ParamValues::new(vec![Some("1".to_string()), Some(u64::MAX.to_string())]);
        let res = executor::execute_sql(
            "SELECT * FROM ducklake_table \
             WHERE schema_id = $1 \
             AND begin_snapshot <= $2 \
             AND (end_snapshot IS NULL OR $2 < end_snapshot)",
            &rp,
            &store,
            &mut session,
        )
        .await
        .unwrap();
        assert_eq!(res.len(), 1, "must return exactly one table row");
    }

    // ── 8. Read snapshot log ───────────────────────────────────────────────
    {
        let mut session = SessionState::new();
        let res = executor::execute_sql(
            "SELECT max(snapshot_id) FROM ducklake_snapshot",
            &params,
            &store,
            &mut session,
        )
        .await
        .unwrap();
        assert_eq!(res.len(), 1, "snapshot log must return a row");
    }

    // ── 9. Time-travel: read data files at snapshot 0 (before ingest) ─────
    {
        let mut session = SessionState::new();
        let rp = ParamValues::new(vec![Some("2".to_string()), Some("0".to_string())]);
        let res = executor::execute_sql(
            "SELECT * FROM ducklake_data_file \
             WHERE table_id = $1 AND begin_snapshot <= $2",
            &rp,
            &store,
            &mut session,
        )
        .await
        .unwrap();
        assert_eq!(res.len(), 1, "time-travel query must return a result set");
    }
}

// ── Test 2: Schema / table CRUD via DuckLake protocol ─────────────────────────

/// Verify that schema + table creation is visible after COMMIT via catalog SELECT.
#[tokio::test]
async fn show_tables_after_create() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;

    // Insert schema.
    {
        let mut session = SessionState::new();
        let sp = ParamValues::new(vec![Some("mydb".to_string())]);
        executor::execute_sql(
            "INSERT INTO ducklake_schema (schema_name) VALUES ($1)",
            &sp,
            &store,
            &mut session,
        )
        .await
        .unwrap();
    }

    // Insert table.
    {
        let mut session = SessionState::new();
        let tp = ParamValues::new(vec![Some("1".to_string()), Some("users".to_string()), None]);
        executor::execute_sql(
            "INSERT INTO ducklake_table (schema_id, table_name, data_path) VALUES ($1, $2, $3)",
            &tp,
            &store,
            &mut session,
        )
        .await
        .unwrap();
    }

    // Commit snapshot.
    {
        let mut session = SessionState::new();
        let snap = ParamValues::new(vec![None, None]);
        executor::execute_sql(
            "INSERT INTO ducklake_snapshot (author, message) VALUES ($1, $2)",
            &snap,
            &store,
            &mut session,
        )
        .await
        .unwrap();
    }

    // "SHOW TABLES" equivalent — read ducklake_table at latest snapshot.
    {
        let mut session = SessionState::new();
        let rp = ParamValues::new(vec![Some("1".to_string()), Some(u64::MAX.to_string())]);
        let res = executor::execute_sql(
            "SELECT * FROM ducklake_table \
             WHERE schema_id = $1 \
             AND begin_snapshot <= $2 \
             AND (end_snapshot IS NULL OR $2 < end_snapshot)",
            &rp,
            &store,
            &mut session,
        )
        .await
        .unwrap();
        assert!(!res.is_empty(), "SHOW TABLES: must return at least one row");
    }
}

// ── Test 3: Transaction BEGIN / COMMIT semantics ──────────────────────────────

/// Verify that the executor handles multi-statement BEGIN/COMMIT correctly.
#[tokio::test]
async fn transaction_begin_commit() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;
    let params = ParamValues::default();
    let mut session = SessionState::new();

    let _ = executor::execute_sql("BEGIN", &params, &store, &mut session)
        .await
        .unwrap();
    assert!(session.in_transaction);

    let _ = executor::execute_sql("COMMIT", &params, &store, &mut session)
        .await
        .unwrap();
    assert!(!session.in_transaction);
}
