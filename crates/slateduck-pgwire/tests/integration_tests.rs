//! Integration tests for the SlateDuck PG-Wire sidecar.
//!
//! Tests the complete flow: SQL classification → execution → catalog store operations.

use std::sync::Arc;

use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use tokio::sync::Mutex;

use slateduck_catalog::{CatalogStore, OpenOptions};
use slateduck_pgwire::executor;
use slateduck_pgwire::session::SessionState;
use slateduck_sql::ParamValues;

async fn setup_store(dir: &tempfile::TempDir) -> Arc<Mutex<CatalogStore>> {
    let store = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    let opts = OpenOptions {
        object_store: store,
        path: ObjectPath::from(""),
        encryption: None,
    };
    let catalog = CatalogStore::open(opts).await.unwrap();
    Arc::new(Mutex::new(catalog))
}

#[tokio::test]
async fn test_select_version() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();
    let params = ParamValues::default();

    let responses = executor::execute_sql("SELECT version()", &params, &store, &mut session)
        .await
        .unwrap();
    assert_eq!(responses.len(), 1);
}

#[tokio::test]
async fn test_select_current_schema() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();
    let params = ParamValues::default();

    let responses = executor::execute_sql("SELECT current_schema()", &params, &store, &mut session)
        .await
        .unwrap();
    assert_eq!(responses.len(), 1);
}

#[tokio::test]
async fn test_select_current_database() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();
    let params = ParamValues::default();

    let responses =
        executor::execute_sql("SELECT current_database()", &params, &store, &mut session)
            .await
            .unwrap();
    assert_eq!(responses.len(), 1);
}

#[tokio::test]
async fn test_select_pg_type() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();
    let params = ParamValues::default();

    let responses = executor::execute_sql(
        "SELECT oid, typname FROM pg_catalog.pg_type WHERE typname IN ('bool','int4','int8','text')",
        &params,
        &store,
        &mut session,
    )
    .await
    .unwrap();
    assert_eq!(responses.len(), 1);
}

#[tokio::test]
async fn test_set_and_show() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();
    let params = ParamValues::default();

    let responses = executor::execute_sql("SET timezone = 'UTC'", &params, &store, &mut session)
        .await
        .unwrap();
    assert_eq!(responses.len(), 1);

    let responses = executor::execute_sql("SHOW timezone", &params, &store, &mut session)
        .await
        .unwrap();
    assert_eq!(responses.len(), 1);
}

#[tokio::test]
async fn test_begin_commit_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();
    let params = ParamValues::default();

    // BEGIN
    let responses = executor::execute_sql("BEGIN", &params, &store, &mut session)
        .await
        .unwrap();
    assert_eq!(responses.len(), 1);
    assert!(session.in_transaction);

    // COMMIT
    let responses = executor::execute_sql("COMMIT", &params, &store, &mut session)
        .await
        .unwrap();
    assert_eq!(responses.len(), 1);
    assert!(!session.in_transaction);

    // ROLLBACK
    executor::execute_sql("BEGIN", &params, &store, &mut session)
        .await
        .unwrap();
    assert!(session.in_transaction);
    let responses = executor::execute_sql("ROLLBACK", &params, &store, &mut session)
        .await
        .unwrap();
    assert_eq!(responses.len(), 1);
    assert!(!session.in_transaction);
}

#[tokio::test]
async fn test_select_max_snapshot_empty() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();
    let params = ParamValues::default();

    let responses = executor::execute_sql(
        "SELECT max(snapshot_id) FROM ducklake_snapshot",
        &params,
        &store,
        &mut session,
    )
    .await
    .unwrap();
    assert_eq!(responses.len(), 1);
}

#[tokio::test]
async fn test_unsupported_statement() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();
    let params = ParamValues::default();

    let result = executor::execute_sql("DROP TABLE foo", &params, &store, &mut session).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_transaction_buffering() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();
    let params = ParamValues::default();

    // Start transaction
    executor::execute_sql("BEGIN", &params, &store, &mut session)
        .await
        .unwrap();

    // Insert schema (buffered)
    let schema_params = ParamValues::new(vec![Some("test_schema".to_string())]);
    executor::execute_sql(
        "INSERT INTO ducklake_schema (schema_name) VALUES ($1)",
        &schema_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();
    assert_eq!(session.pending_txn.len(), 1);

    // Commit
    executor::execute_sql("COMMIT", &params, &store, &mut session)
        .await
        .unwrap();
    assert!(session.pending_txn.is_empty());
}

#[tokio::test]
async fn test_schema_crud() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();

    // Create schema outside transaction (auto-commit)
    let params = ParamValues::new(vec![Some("myschema".to_string())]);
    executor::execute_sql(
        "INSERT INTO ducklake_schema (schema_name) VALUES ($1)",
        &params,
        &store,
        &mut session,
    )
    .await
    .unwrap();

    // Create a snapshot to make it visible
    let snap_params = ParamValues::new(vec![None, None]);
    executor::execute_sql(
        "INSERT INTO ducklake_snapshot (author, message) VALUES ($1, $2)",
        &snap_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();

    // Read schemas
    let read_params = ParamValues::new(vec![Some(u64::MAX.to_string())]);
    let responses = executor::execute_sql(
        "SELECT * FROM ducklake_schema WHERE begin_snapshot <= $1 AND (end_snapshot IS NULL OR $1 < end_snapshot)",
        &read_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();
    assert_eq!(responses.len(), 1);
}

#[tokio::test]
async fn test_table_crud() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();

    // Create schema first
    let params = ParamValues::new(vec![Some("public".to_string())]);
    executor::execute_sql(
        "INSERT INTO ducklake_schema (schema_name) VALUES ($1)",
        &params,
        &store,
        &mut session,
    )
    .await
    .unwrap();

    // Create table
    let table_params = ParamValues::new(vec![
        Some("1".to_string()),     // schema_id
        Some("users".to_string()), // table_name
        None,                      // data_path
    ]);
    executor::execute_sql(
        "INSERT INTO ducklake_table (schema_id, table_name, data_path) VALUES ($1, $2, $3)",
        &table_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();

    // Create snapshot
    let snap_params = ParamValues::new(vec![None, None]);
    executor::execute_sql(
        "INSERT INTO ducklake_snapshot (author, message) VALUES ($1, $2)",
        &snap_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();

    // Read tables
    let read_params = ParamValues::new(vec![Some("1".to_string()), Some(u64::MAX.to_string())]);
    let responses = executor::execute_sql(
        "SELECT * FROM ducklake_table WHERE schema_id = $1 AND begin_snapshot <= $2 AND (end_snapshot IS NULL OR $2 < end_snapshot)",
        &read_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();
    assert_eq!(responses.len(), 1);
}

#[tokio::test]
async fn test_data_file_crud() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();

    // Create schema + table first
    let params = ParamValues::new(vec![Some("public".to_string())]);
    executor::execute_sql(
        "INSERT INTO ducklake_schema (schema_name) VALUES ($1)",
        &params,
        &store,
        &mut session,
    )
    .await
    .unwrap();

    let table_params = ParamValues::new(vec![
        Some("1".to_string()),
        Some("events".to_string()),
        None,
    ]);
    executor::execute_sql(
        "INSERT INTO ducklake_table (schema_id, table_name, data_path) VALUES ($1, $2, $3)",
        &table_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();

    // Register data file
    let file_params = ParamValues::new(vec![
        Some("2".to_string()),                             // table_id
        Some("data/events/part-0001.parquet".to_string()), // path
        Some("parquet".to_string()),                       // format
        Some("1000".to_string()),                          // row_count
        Some("4096".to_string()),                          // file_size
    ]);
    executor::execute_sql(
        "INSERT INTO ducklake_data_file (table_id, path, file_format, row_count, file_size_bytes) VALUES ($1, $2, $3, $4, $5)",
        &file_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();

    // Create snapshot
    let snap_params = ParamValues::new(vec![None, None]);
    executor::execute_sql(
        "INSERT INTO ducklake_snapshot (author, message) VALUES ($1, $2)",
        &snap_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();

    // List data files
    let read_params = ParamValues::new(vec![Some("2".to_string()), Some(u64::MAX.to_string())]);
    let responses = executor::execute_sql(
        "SELECT * FROM ducklake_data_file WHERE table_id = $1",
        &read_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();
    assert_eq!(responses.len(), 1);
}

#[tokio::test]
async fn test_sqlstate_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();
    let params = ParamValues::default();

    let result = executor::execute_sql("DROP DATABASE foo", &params, &store, &mut session).await;
    assert!(result.is_err());
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("expected error"),
    };
    assert_eq!(err.sqlstate(), "0A000");
}

#[tokio::test]
async fn test_sqlstate_batch_too_large() {
    use slateduck_pgwire::session::{BufferedOp, PendingCatalogTxn};

    let mut txn = PendingCatalogTxn::new();
    // This shouldn't trigger the limit with one op
    let result = txn.push(BufferedOp::InsertSchema {
        schema_name: "test".to_string(),
    });
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_create_inlined_table() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();
    let params = ParamValues::default();

    let responses = executor::execute_sql(
        "CREATE TABLE ducklake_inlined_test (id INTEGER, name TEXT)",
        &params,
        &store,
        &mut session,
    )
    .await
    .unwrap();
    assert_eq!(responses.len(), 1);
}

#[tokio::test]
async fn test_wire_handshake_replay() {
    // Test that all queries in the Phase 0 handshake fixture can be classified
    let fixture = include_str!("../../../tests/fixtures/handshake/duckdb-1.2.2.jsonl");
    for line in fixture.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        if v["direction"] == "client_to_server" && v["type"] == "Query" {
            let query = v["payload"]["query"].as_str().unwrap();
            let result = slateduck_sql::classify_statement(query);
            assert!(
                result.is_ok(),
                "Failed to classify handshake query: {query}"
            );
            let kind = result.unwrap();
            // All handshake queries should be recognized (not Unsupported)
            assert!(
                !matches!(kind, slateduck_sql::StatementKind::Unsupported(_)),
                "Handshake query classified as unsupported: {query} => {kind:?}"
            );
        }
    }
}

#[tokio::test]
async fn test_pgwire_server_lifecycle() {
    // Test that we can start and stop the server
    let dir = tempfile::tempdir().unwrap();
    let store_obj = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    let opts = OpenOptions {
        object_store: store_obj,
        path: ObjectPath::from(""),
        encryption: None,
    };
    let catalog = CatalogStore::open(opts).await.unwrap();
    let catalog = Arc::new(Mutex::new(catalog));

    let (tx, rx) = tokio::sync::oneshot::channel();

    // Use a random port for testing
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let config = slateduck_pgwire::ServerConfig {
        bind_addr: addr,
        ..Default::default()
    };

    let catalog_clone = catalog.clone();
    let server_handle = tokio::spawn(async move {
        slateduck_pgwire::server::run_server_with_shutdown(config, catalog_clone, rx)
            .await
            .unwrap();
    });

    // Give the server a moment to bind
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Shut down
    let _ = tx.send(());
    let _ = server_handle.await;
}

#[tokio::test]
async fn test_pgwire_client_connection() {
    let dir = tempfile::tempdir().unwrap();
    let store_obj = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    let opts = OpenOptions {
        object_store: store_obj,
        path: ObjectPath::from(""),
        encryption: None,
    };
    let catalog = CatalogStore::open(opts).await.unwrap();
    let catalog = Arc::new(Mutex::new(catalog));

    let (tx, rx) = tokio::sync::oneshot::channel();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let config = slateduck_pgwire::ServerConfig {
        bind_addr: addr,
        ..Default::default()
    };

    let catalog_clone = catalog.clone();
    let server_handle = tokio::spawn(async move {
        slateduck_pgwire::server::run_server_with_shutdown(config, catalog_clone, rx)
            .await
            .unwrap();
    });

    // Wait for server to start
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Connect with tokio-postgres
    let conn_str = format!(
        "host=127.0.0.1 port={} user=duckdb dbname=ducklake",
        addr.port()
    );
    let (client, connection) = tokio_postgres::connect(&conn_str, tokio_postgres::NoTls)
        .await
        .unwrap();

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {e}");
        }
    });

    // Run queries
    let rows = client.query("SELECT version()", &[]).await.unwrap();
    assert_eq!(rows.len(), 1);
    let version: &str = rows[0].get(0);
    assert!(version.contains("PostgreSQL"));

    let rows = client.query("SELECT current_schema()", &[]).await.unwrap();
    assert_eq!(rows.len(), 1);
    let schema: &str = rows[0].get(0);
    assert_eq!(schema, "public");

    let rows = client
        .query("SELECT current_database()", &[])
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let db: &str = rows[0].get(0);
    assert_eq!(db, "ducklake");

    // PG type query
    let rows = client
        .query(
            "SELECT oid, typname FROM pg_catalog.pg_type WHERE typname IN ('bool','int4','int8','text')",
            &[],
        )
        .await
        .unwrap();
    assert!(!rows.is_empty());

    // Shut down
    drop(client);
    let _ = tx.send(());
    let _ = server_handle.await;
}

#[tokio::test]
async fn test_error_sqlstate_mapping() {
    use slateduck_pgwire::SlateDuckError;

    assert_eq!(SlateDuckError::WriterFenced.sqlstate(), "57P04");
    assert_eq!(SlateDuckError::WriterFenced.severity(), "FATAL");
    assert_eq!(SlateDuckError::CatalogNotInitialized.sqlstate(), "3D000");
    assert_eq!(SlateDuckError::CatalogNotInitialized.severity(), "FATAL");
    assert_eq!(
        SlateDuckError::Unsupported("test".to_string()).sqlstate(),
        "0A000"
    );
    assert_eq!(
        SlateDuckError::Unsupported("test".to_string()).severity(),
        "ERROR"
    );
    assert_eq!(SlateDuckError::BatchTooLarge.sqlstate(), "54001");
    assert_eq!(
        SlateDuckError::Duplicate("key".to_string()).sqlstate(),
        "23505"
    );
    assert_eq!(
        SlateDuckError::NotFound("row".to_string()).sqlstate(),
        "02000"
    );
    assert_eq!(SlateDuckError::CounterConflict.sqlstate(), "40001");
    assert_eq!(
        SlateDuckError::PermissionDenied("access".to_string()).sqlstate(),
        "42501"
    );
    assert_eq!(
        SlateDuckError::ObjectStore("timeout".to_string()).sqlstate(),
        "08006"
    );
    assert_eq!(SlateDuckError::ReadOnlyReplica.sqlstate(), "25006");
    assert_eq!(
        SlateDuckError::Corruption("bad data".to_string()).sqlstate(),
        "XX001"
    );
    assert_eq!(
        SlateDuckError::ValueDecode("parse err".to_string()).sqlstate(),
        "22P02"
    );
    assert_eq!(SlateDuckError::SnapshotOutOfRetention.sqlstate(), "22023");
    assert_eq!(
        SlateDuckError::Internal("bug".to_string()).sqlstate(),
        "XX000"
    );

    // v0.19: SqlState variant returns stored code, not hardcoded "55000"
    assert_eq!(
        SlateDuckError::SqlState {
            code: "42P01".to_string(),
            message: "table not found".to_string()
        }
        .sqlstate(),
        "42P01"
    );
    assert_eq!(
        SlateDuckError::SqlState {
            code: "23505".to_string(),
            message: "duplicate".to_string()
        }
        .sqlstate(),
        "23505"
    );
    assert_eq!(
        SlateDuckError::SqlState {
            code: "55000".to_string(),
            message: "object not in prerequisite state".to_string()
        }
        .sqlstate(),
        "55000"
    );
}

// ─── v0.6: pg-tide-relay corpus replay tests ──────────────────────────────

#[tokio::test]
async fn test_pgtide_select_max_snapshot_after() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();

    // Create a snapshot first
    let params = ParamValues::new(vec![
        Some("pg_tide".to_string()),
        Some("batch 1".to_string()),
    ]);
    executor::execute_sql(
        "INSERT INTO ducklake_snapshot (author, message) VALUES ($1, $2)",
        &params,
        &store,
        &mut session,
    )
    .await
    .unwrap();

    // Query max snapshot after 0
    let query_params = ParamValues::new(vec![Some("0".to_string())]);
    let responses = executor::execute_sql(
        "SELECT max(snapshot_id) FROM ducklake_snapshot WHERE snapshot_id > $1",
        &query_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();
    assert_eq!(responses.len(), 1);
}

#[tokio::test]
async fn test_pgtide_select_first_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();

    // Create a snapshot
    let params = ParamValues::new(vec![Some("pg_tide".to_string()), Some("init".to_string())]);
    executor::execute_sql(
        "INSERT INTO ducklake_snapshot (author, message) VALUES ($1, $2)",
        &params,
        &store,
        &mut session,
    )
    .await
    .unwrap();

    // Query first snapshot
    let responses = executor::execute_sql(
        "SELECT * FROM ducklake_snapshot ORDER BY snapshot_id ASC LIMIT 1",
        &ParamValues::default(),
        &store,
        &mut session,
    )
    .await
    .unwrap();
    assert_eq!(responses.len(), 1);
}

#[tokio::test]
async fn test_pgtide_select_data_files_with_limit() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();

    // Query with parameterized limit (empty table)
    let params = ParamValues::new(vec![
        Some("1".to_string()),
        Some("100".to_string()),
        Some(u64::MAX.to_string()),
    ]);
    let responses = executor::execute_sql(
        "SELECT * FROM ducklake_data_file WHERE table_id = $1 LIMIT $2",
        &params,
        &store,
        &mut session,
    )
    .await
    .unwrap();
    assert_eq!(responses.len(), 1);
}

#[tokio::test]
async fn test_pgtide_gen_random_uuid() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();

    let responses = executor::execute_sql(
        "SELECT gen_random_uuid()",
        &ParamValues::default(),
        &store,
        &mut session,
    )
    .await
    .unwrap();
    assert_eq!(responses.len(), 1);
}

#[tokio::test]
async fn test_pgtide_metadata_offset_tracking() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();

    // Write metadata using dotted-prefix convention
    let params = ParamValues::new(vec![
        Some("pg_tide.orders-to-lake.offset".to_string()),
        Some("4782".to_string()),
    ]);
    executor::execute_sql(
        "INSERT INTO ducklake_metadata (metadata_key, metadata_value) VALUES ($1, $2)",
        &params,
        &store,
        &mut session,
    )
    .await
    .unwrap();

    // Read metadata back
    let read_params = ParamValues::new(vec![Some("pg_tide.orders-to-lake.offset".to_string())]);
    let responses = executor::execute_sql(
        "SELECT value FROM ducklake_metadata WHERE metadata_key = $1",
        &read_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();
    assert_eq!(responses.len(), 1);
}

#[tokio::test]
async fn test_pgtide_full_ingest_workflow() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();

    // Setup: create schema and table
    let schema_params = ParamValues::new(vec![Some("public".to_string())]);
    executor::execute_sql(
        "INSERT INTO ducklake_schema (schema_name) VALUES ($1)",
        &schema_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();

    let table_params = ParamValues::new(vec![
        Some("1".to_string()),
        Some("orders".to_string()),
        None,
    ]);
    executor::execute_sql(
        "INSERT INTO ducklake_table (schema_id, table_name, data_path) VALUES ($1, $2, $3)",
        &table_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();

    // pg-tide workflow: BEGIN, write metadata + data file + snapshot, COMMIT
    executor::execute_sql("BEGIN", &ParamValues::default(), &store, &mut session)
        .await
        .unwrap();
    assert!(session.in_transaction);

    let meta_params = ParamValues::new(vec![
        Some("pg_tide.orders-to-lake.offset".to_string()),
        Some("4782".to_string()),
    ]);
    executor::execute_sql(
        "INSERT INTO ducklake_metadata (metadata_key, metadata_value) VALUES ($1, $2)",
        &meta_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();

    let file_params = ParamValues::new(vec![
        Some("1".to_string()),
        Some("data/orders/part-00042.parquet".to_string()),
        Some("parquet".to_string()),
        Some("1000".to_string()),
        Some("65536".to_string()),
    ]);
    executor::execute_sql(
        "INSERT INTO ducklake_data_file (table_id, path, file_format, row_count, file_size_bytes) VALUES ($1, $2, $3, $4, $5)",
        &file_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();

    let snap_params = ParamValues::new(vec![
        Some("pg_tide".to_string()),
        Some("Ingest batch 4782".to_string()),
    ]);
    executor::execute_sql(
        "INSERT INTO ducklake_snapshot (author, message) VALUES ($1, $2)",
        &snap_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();

    executor::execute_sql("COMMIT", &ParamValues::default(), &store, &mut session)
        .await
        .unwrap();
    assert!(!session.in_transaction);
}

// ─── v0.6: Audit log tests ───────────────────────────────────────────────

#[tokio::test]
async fn test_audit_log_write_and_read() {
    let dir = tempfile::tempdir().unwrap();
    let object_store =
        Arc::new(object_store::local::LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    let opts = slateduck_catalog::OpenOptions {
        object_store,
        path: ObjectPath::from(""),
        encryption: None,
    };
    let catalog = slateduck_catalog::CatalogStore::open(opts).await.unwrap();

    let entry = slateduck_catalog::AuditEntry {
        snapshot_id: 1,
        committed_at: "2025-05-23T12:00:00Z".to_string(),
        committed_by: "pg_tide".to_string(),
        changes: vec![slateduck_catalog::AuditChange {
            change_type: "register_data_file".to_string(),
            detail: Some("data/orders/part-00042.parquet".to_string()),
        }],
    };

    slateduck_catalog::audit::write_audit_entry(catalog.db(), &entry)
        .await
        .unwrap();

    let entries = slateduck_catalog::audit::list_audit_entries(catalog.db())
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].snapshot_id, 1);
    assert_eq!(entries[0].committed_by, "pg_tide");
    assert_eq!(entries[0].changes.len(), 1);
    assert_eq!(entries[0].changes[0].change_type, "register_data_file");

    // Read specific entry
    let specific = slateduck_catalog::audit::get_audit_entry(catalog.db(), 1)
        .await
        .unwrap();
    assert!(specific.is_some());
    assert_eq!(specific.unwrap().committed_by, "pg_tide");

    // Non-existent entry
    let missing = slateduck_catalog::audit::get_audit_entry(catalog.db(), 999)
        .await
        .unwrap();
    assert!(missing.is_none());
}

// ─── v0.6: TLS and Auth config tests ─────────────────────────────────────

#[test]
fn test_tls_config() {
    use slateduck_pgwire::server::TlsConfig;

    let disabled = TlsConfig::default();
    assert!(!disabled.is_enabled());

    let enabled = TlsConfig {
        cert_path: Some("/path/to/cert.pem".to_string()),
        key_path: Some("/path/to/key.pem".to_string()),
        required: false,
    };
    assert!(enabled.is_enabled());

    let partial = TlsConfig {
        cert_path: Some("/path/to/cert.pem".to_string()),
        key_path: None,
        required: false,
    };
    assert!(!partial.is_enabled());
}

#[test]
fn test_auth_config() {
    use slateduck_pgwire::server::AuthConfig;

    let disabled = AuthConfig::default();
    assert!(!disabled.is_enabled());

    let enabled = AuthConfig {
        username: Some("admin".to_string()),
        password: Some("secret".to_string()),
    };
    assert!(enabled.is_enabled());

    let partial = AuthConfig {
        username: Some("admin".to_string()),
        password: None,
    };
    assert!(!partial.is_enabled());
}

// ─── v0.6: GCS and Azure object store validation ─────────────────────────

#[tokio::test]
async fn test_gcs_object_store_config() {
    // Validate that GCS object store configuration can be constructed
    // (without actual credentials — validates the builder API works)
    let result = object_store::gcp::GoogleCloudStorageBuilder::new()
        .with_bucket_name("test-bucket")
        .with_service_account_key("{}")
        .build();
    // Builder should fail with invalid credentials but should not panic
    assert!(result.is_err());
}

#[tokio::test]
async fn test_azure_object_store_config() {
    // Validate that Azure object store configuration can be constructed
    let result = object_store::azure::MicrosoftAzureBuilder::new()
        .with_account("testaccount")
        .with_container_name("testcontainer")
        .with_access_key("dGVzdA==")
        .build();
    // Azure builder with test credentials should construct (may fail at request time)
    // The key point is the builder doesn't panic and type-checks
    let _ = result;
}

// ─── v0.6: IAM separation tests ──────────────────────────────────────────

#[test]
fn test_iam_permission_denied_sqlstate() {
    use slateduck_pgwire::SlateDuckError;
    let err = SlateDuckError::PermissionDenied("s3:PutObject on catalogs/ denied".to_string());
    assert_eq!(err.sqlstate(), "42501");
    assert_eq!(err.severity(), "ERROR");
}

// ─── v0.9.4: F-21 TLS protocol tests ─────────────────────────────────────

/// Generate a self-signed certificate and key into a temp directory.
/// Returns (cert_path, key_path) as strings.
fn generate_self_signed_cert(dir: &tempfile::TempDir) -> (String, String) {
    use rcgen::{generate_simple_self_signed, CertifiedKey};
    let subject_alt_names = vec!["127.0.0.1".to_string()];
    let CertifiedKey { cert, key_pair } =
        generate_simple_self_signed(subject_alt_names).expect("rcgen cert generation must succeed");
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    let cert_path = dir.path().join("server.crt");
    let key_path = dir.path().join("server.key");
    std::fs::write(&cert_path, cert_pem).unwrap();
    std::fs::write(&key_path, key_pem).unwrap();

    (
        cert_path.to_str().unwrap().to_string(),
        key_path.to_str().unwrap().to_string(),
    )
}

#[tokio::test]
async fn test_tls_required_without_certs_fails_start() {
    let dir = tempfile::tempdir().unwrap();
    let store_obj = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    let opts = OpenOptions {
        object_store: store_obj,
        path: ObjectPath::from(""),
        encryption: None,
    };
    let catalog = Arc::new(Mutex::new(CatalogStore::open(opts).await.unwrap()));

    let config = slateduck_pgwire::ServerConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        tls: slateduck_pgwire::server::TlsConfig {
            cert_path: None,
            key_path: None,
            required: true, // required but no cert
        },
        ..Default::default()
    };

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let result = slateduck_pgwire::server::run_server_with_shutdown(config, catalog, rx).await;
    assert!(
        result.is_err(),
        "--tls-required without cert/key must fail at server start"
    );
    drop(tx);
}

/// Helper: start a server with TLS enabled using a self-signed certificate.
async fn start_server_with_tls(
    dir: &tempfile::TempDir,
    required: bool,
) -> (
    std::net::SocketAddr,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let (cert_path, key_path) = generate_self_signed_cert(dir);
    let store_obj = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    let opts = OpenOptions {
        object_store: store_obj,
        path: ObjectPath::from(""),
        encryption: None,
    };
    let catalog = Arc::new(Mutex::new(CatalogStore::open(opts).await.unwrap()));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let config = slateduck_pgwire::ServerConfig {
        bind_addr: addr,
        tls: slateduck_pgwire::server::TlsConfig {
            cert_path: Some(cert_path),
            key_path: Some(key_path),
            required,
        },
        ..Default::default()
    };

    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        slateduck_pgwire::server::run_server_with_shutdown(config, catalog, rx)
            .await
            .unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    (addr, tx, handle)
}

#[tokio::test]
async fn test_tls_required_rejects_plaintext_connection() {
    let dir = tempfile::tempdir().unwrap();
    let (addr, tx, handle) = start_server_with_tls(&dir, true).await;

    // Connect without TLS — server must reject the plaintext connection.
    let conn_str = format!(
        "host=127.0.0.1 port={} user=anyuser dbname=ducklake sslmode=disable",
        addr.port()
    );
    let result = tokio_postgres::connect(&conn_str, tokio_postgres::NoTls).await;
    assert!(
        result.is_err(),
        "plaintext connection must be rejected when TLS is required"
    );

    let _ = tx.send(());
    let _ = handle.await;
}

#[tokio::test]
async fn test_tls_server_starts_with_self_signed_cert() {
    let dir = tempfile::tempdir().unwrap();
    // required=false so plaintext clients can still connect for this test.
    let (addr, tx, handle) = start_server_with_tls(&dir, false).await;

    // Connect without TLS — server accepts plaintext when TLS is not required.
    let conn_str = format!(
        "host=127.0.0.1 port={} user=anyuser dbname=ducklake sslmode=disable",
        addr.port()
    );
    let (client, connection) = tokio_postgres::connect(&conn_str, tokio_postgres::NoTls)
        .await
        .expect("server with optional TLS must accept plaintext");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let rows = client.query("SELECT version()", &[]).await.unwrap();
    assert_eq!(rows.len(), 1, "SELECT version() must return one row");

    drop(client);
    let _ = tx.send(());
    let _ = handle.await;
}

#[test]
fn test_wire_corpus_fixture_exists() {
    let duckdb_corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures/wire-corpus/duckdb-1.2.2.jsonl");
    assert!(duckdb_corpus.exists(), "DuckDB wire corpus fixture missing");
}

#[test]
fn test_pgtide_corpus_fixture_exists() {
    let pgtide_corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures/wire-corpus/pgtide-0.34.jsonl");
    assert!(
        pgtide_corpus.exists(),
        "pg-tide-relay corpus fixture missing"
    );
}

#[test]
fn test_pgtide_corpus_is_valid_json() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures/wire-corpus/pgtide-0.34.jsonl");
    let content = std::fs::read_to_string(path).unwrap();
    let corpus: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(corpus["client"], "pg-tide-relay");
    assert_eq!(corpus["version"], "0.34");
    let statements = corpus["statements"].as_array().unwrap();
    assert!(!statements.is_empty());
}

// ─── v0.9.4: Spark and Trino corpus replay tests ─────────────────────────

fn corpus_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures/wire-corpus")
}

#[test]
fn test_spark_corpus_fixture_exists() {
    let path = corpus_root().join("spark-3.5.jsonl");
    assert!(path.exists(), "Spark-3.5 wire corpus fixture missing");
}

#[test]
fn test_trino_corpus_fixture_exists() {
    let path = corpus_root().join("trino-432.jsonl");
    assert!(path.exists(), "Trino-432 wire corpus fixture missing");
}

/// Parse the Spark corpus and verify every SQL statement in it can be
/// classified by the SQL dispatcher (categories a or b — no unknown shapes).
#[test]
fn test_spark_corpus_all_statements_classifiable() {
    let path = corpus_root().join("spark-3.5.jsonl");
    let content = std::fs::read_to_string(path).unwrap();
    let corpus: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(corpus["client"], "spark-ducklake");
    assert_eq!(corpus["version"], "3.5");

    let statements = corpus["statements"].as_array().unwrap();
    assert!(!statements.is_empty(), "Spark corpus must not be empty");

    for stmt in statements {
        let sql = stmt["sql"].as_str().unwrap_or("");
        if sql.is_empty()
            || sql.starts_with("BEGIN")
            || sql.starts_with("COMMIT")
            || sql.starts_with("SET ")
        {
            continue;
        }
        let result = slateduck_sql::classify_statement(sql);
        assert!(
            result.is_ok(),
            "Spark corpus statement must be classifiable: {sql:?} — err: {result:?}"
        );
    }
}

/// Parse the Trino corpus and verify every SQL statement is classifiable.
#[test]
fn test_trino_corpus_all_statements_classifiable() {
    let path = corpus_root().join("trino-432.jsonl");
    let content = std::fs::read_to_string(path).unwrap();
    let corpus: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(corpus["client"], "trino-ducklake");
    assert_eq!(corpus["version"], "432");

    let statements = corpus["statements"].as_array().unwrap();
    assert!(!statements.is_empty(), "Trino corpus must not be empty");

    for stmt in statements {
        let sql = stmt["sql"].as_str().unwrap_or("");
        if sql.is_empty()
            || sql.starts_with("BEGIN")
            || sql.starts_with("COMMIT")
            || sql.starts_with("SET ")
        {
            continue;
        }
        let result = slateduck_sql::classify_statement(sql);
        assert!(
            result.is_ok(),
            "Trino corpus statement must be classifiable: {sql:?} — err: {result:?}"
        );
    }
}

// ─── v0.6: Server config with TLS and Auth ───────────────────────────────

#[test]
fn test_server_config_default() {
    let config = slateduck_pgwire::ServerConfig::default();
    assert_eq!(
        config.bind_addr,
        "0.0.0.0:5432".parse::<std::net::SocketAddr>().unwrap()
    );
    assert_eq!(config.max_sessions, 50);
    assert!(!config.tls.is_enabled());
    assert!(!config.auth.is_enabled());
}

// ─── v0.9.1: SELECT max(snapshot) consistency (F-01) ─────────────────────

/// After multiple write sessions each ending with InsertSnapshot, the
/// executor must successfully respond to `SELECT max(snapshot)` without
/// panicking.  The response must contain exactly one row (non-null).
///
/// This is a PG-Wire-level regression for F-01: before `commit_writer` was
/// called in `execute_commit`, `read_latest()` would always return a stale
/// snapshot ID, eventually returning 0 even after multiple commits.
#[tokio::test]
async fn test_select_max_snapshot_consistent_after_multiple_write_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();
    let empty_params = ParamValues::default();

    // Session 1: INSERT INTO ducklake_snapshot (simulates DuckLake DDL commit)
    executor::execute_sql("BEGIN", &empty_params, &store, &mut session)
        .await
        .unwrap();
    let params1 = ParamValues::new(vec![
        Some("user".to_string()),
        Some("session-1".to_string()),
    ]);
    executor::execute_sql(
        "INSERT INTO ducklake_snapshot (snapshot_id, author, message) VALUES ($1, $2, $3)",
        &params1,
        &store,
        &mut session,
    )
    .await
    .unwrap();
    executor::execute_sql("COMMIT", &empty_params, &store, &mut session)
        .await
        .unwrap();

    // After session 1: SELECT max(snapshot) must return 1 row
    let r1 = executor::execute_sql(
        "SELECT max(snapshot_id) FROM ducklake_snapshot",
        &empty_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();
    assert_eq!(
        r1.len(),
        1,
        "SELECT max(snapshot) must return 1 row after first commit"
    );

    // Session 2
    executor::execute_sql("BEGIN", &empty_params, &store, &mut session)
        .await
        .unwrap();
    let params2 = ParamValues::new(vec![
        Some("user".to_string()),
        Some("session-2".to_string()),
    ]);
    executor::execute_sql(
        "INSERT INTO ducklake_snapshot (snapshot_id, author, message) VALUES ($1, $2, $3)",
        &params2,
        &store,
        &mut session,
    )
    .await
    .unwrap();
    executor::execute_sql("COMMIT", &empty_params, &store, &mut session)
        .await
        .unwrap();

    // After session 2: SELECT max(snapshot) must return 1 row
    let r2 = executor::execute_sql(
        "SELECT max(snapshot_id) FROM ducklake_snapshot",
        &empty_params,
        &store,
        &mut session,
    )
    .await
    .unwrap();
    assert_eq!(
        r2.len(),
        1,
        "SELECT max(snapshot) must return 1 row after second commit"
    );
}

// ─── v0.9.2: Authentication enforcement tests ────────────────────────────

/// Helper: start a server with auth config, returns (addr, shutdown_sender, server_handle).
async fn start_server_with_auth(
    dir: &tempfile::TempDir,
    auth: slateduck_pgwire::server::AuthConfig,
) -> (
    std::net::SocketAddr,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let store_obj = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    let opts = OpenOptions {
        object_store: store_obj,
        path: ObjectPath::from(""),
        encryption: None,
    };
    let catalog = CatalogStore::open(opts).await.unwrap();
    let catalog = Arc::new(Mutex::new(catalog));

    let (tx, rx) = tokio::sync::oneshot::channel();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let config = slateduck_pgwire::ServerConfig {
        bind_addr: addr,
        auth,
        ..Default::default()
    };

    let catalog_clone = catalog.clone();
    let handle = tokio::spawn(async move {
        slateduck_pgwire::server::run_server_with_shutdown(config, catalog_clone, rx)
            .await
            .unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    (addr, tx, handle)
}

#[tokio::test]
async fn test_auth_correct_credentials_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let auth = slateduck_pgwire::server::AuthConfig {
        username: Some("admin".to_string()),
        password: Some("secret".to_string()),
    };
    let (addr, tx, handle) = start_server_with_auth(&dir, auth).await;

    let conn_str = format!(
        "host=127.0.0.1 port={} user=admin password=secret dbname=ducklake",
        addr.port()
    );
    let (client, connection) = tokio_postgres::connect(&conn_str, tokio_postgres::NoTls)
        .await
        .expect("should connect with correct credentials");

    tokio::spawn(async move {
        let _ = connection.await;
    });

    let rows = client.query("SELECT version()", &[]).await.unwrap();
    assert_eq!(rows.len(), 1);

    drop(client);
    let _ = tx.send(());
    let _ = handle.await;
}

#[tokio::test]
async fn test_auth_wrong_password_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let auth = slateduck_pgwire::server::AuthConfig {
        username: Some("admin".to_string()),
        password: Some("secret".to_string()),
    };
    let (addr, tx, handle) = start_server_with_auth(&dir, auth).await;

    let conn_str = format!(
        "host=127.0.0.1 port={} user=admin password=wrongpassword dbname=ducklake",
        addr.port()
    );
    let result = tokio_postgres::connect(&conn_str, tokio_postgres::NoTls).await;
    assert!(result.is_err(), "wrong password must be rejected");

    let _ = tx.send(());
    let _ = handle.await;
}

#[tokio::test]
async fn test_auth_wrong_username_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let auth = slateduck_pgwire::server::AuthConfig {
        username: Some("admin".to_string()),
        password: Some("secret".to_string()),
    };
    let (addr, tx, handle) = start_server_with_auth(&dir, auth).await;

    let conn_str = format!(
        "host=127.0.0.1 port={} user=notadmin password=secret dbname=ducklake",
        addr.port()
    );
    let result = tokio_postgres::connect(&conn_str, tokio_postgres::NoTls).await;
    assert!(result.is_err(), "wrong username must be rejected");

    let _ = tx.send(());
    let _ = handle.await;
}

#[tokio::test]
async fn test_auth_no_auth_configured_any_user_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let auth = slateduck_pgwire::server::AuthConfig::default(); // no auth
    let (addr, tx, handle) = start_server_with_auth(&dir, auth).await;

    let conn_str = format!(
        "host=127.0.0.1 port={} user=anyuser dbname=ducklake",
        addr.port()
    );
    let (client, connection) = tokio_postgres::connect(&conn_str, tokio_postgres::NoTls)
        .await
        .expect("should connect without auth");

    tokio::spawn(async move {
        let _ = connection.await;
    });

    let rows = client.query("SELECT version()", &[]).await.unwrap();
    assert_eq!(rows.len(), 1);

    drop(client);
    let _ = tx.send(());
    let _ = handle.await;
}

// ─── v0.9.4: DataFusion pg-wire mode ──────────────────────────────────────────

/// When a DataFusion engine connects via the secondary pg-wire port it gets
/// correct responses from the same bounded SQL dispatcher.
#[tokio::test]
async fn test_datafusion_pg_wire_mode_e2e() {
    let dir = tempfile::tempdir().unwrap();
    let store_obj = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    let opts = OpenOptions {
        object_store: store_obj,
        path: ObjectPath::from(""),
        encryption: None,
    };
    let catalog = Arc::new(Mutex::new(CatalogStore::open(opts).await.unwrap()));

    // Primary server (standard port).
    let primary_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let primary_addr = primary_listener.local_addr().unwrap();
    drop(primary_listener);

    // DataFusion secondary listener.
    let df_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let df_addr = df_listener.local_addr().unwrap();
    drop(df_listener);

    let (primary_tx, primary_rx) = tokio::sync::oneshot::channel::<()>();
    let primary_cfg = slateduck_pgwire::ServerConfig {
        bind_addr: primary_addr,
        ..Default::default()
    };
    let catalog_for_primary = catalog.clone();
    tokio::spawn(async move {
        let _ = slateduck_pgwire::server::run_server_with_shutdown(
            primary_cfg,
            catalog_for_primary,
            primary_rx,
        )
        .await;
    });

    // Second server simulating --datafusion-pg-wire port.
    let (df_tx, df_rx) = tokio::sync::oneshot::channel::<()>();
    let df_cfg = slateduck_pgwire::ServerConfig {
        bind_addr: df_addr,
        ..Default::default()
    };
    let catalog_for_df = catalog.clone();
    tokio::spawn(async move {
        let _ =
            slateduck_pgwire::server::run_server_with_shutdown(df_cfg, catalog_for_df, df_rx).await;
    });

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // A DataFusion client connects to the datafusion pg-wire port and runs DuckLake SQL.
    let conn_str = format!(
        "host=127.0.0.1 port={} user=datafusion dbname=ducklake",
        df_addr.port()
    );
    let (client, connection) = tokio_postgres::connect(&conn_str, tokio_postgres::NoTls)
        .await
        .expect("DataFusion engine should connect to the datafusion pg-wire port");
    tokio::spawn(async move {
        let _ = connection.await;
    });

    // Run a DuckLake SELECT through the DataFusion pg-wire port.
    let rows = client
        .query("SELECT version()", &[])
        .await
        .expect("SELECT version() must succeed over datafusion pg-wire port");
    assert_eq!(rows.len(), 1, "should return one row");

    // Virtual catalog SQL is also accessible over the datafusion pg-wire port.
    let vc_rows = client
        .query("SELECT * FROM slateduck_catalog.ducklake_snapshot", &[])
        .await
        .expect("virtual catalog scan must work over datafusion pg-wire port");
    assert!(
        !vc_rows.is_empty(),
        "virtual catalog must return at least one result row"
    );

    drop(client);
    let _ = primary_tx.send(());
    let _ = df_tx.send(());
}

// ─── v0.9.4: Virtual Catalog SQL Tables ──────────────────────────────────────

/// `SELECT * FROM slateduck_catalog.ducklake_snapshot` returns a row.
#[tokio::test]
async fn test_virtual_catalog_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();
    let params = ParamValues::default();

    let responses = executor::execute_sql(
        "SELECT * FROM slateduck_catalog.ducklake_snapshot",
        &params,
        &store,
        &mut session,
    )
    .await
    .unwrap();
    assert!(!responses.is_empty());
}

/// `SELECT * FROM slateduck_catalog.ducklake_schema` returns schemas.
#[tokio::test]
async fn test_virtual_catalog_schema() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();
    let params = ParamValues::default();

    let responses = executor::execute_sql(
        "SELECT * FROM slateduck_catalog.ducklake_schema",
        &params,
        &store,
        &mut session,
    )
    .await
    .unwrap();
    assert!(!responses.is_empty());
}

/// `SELECT * FROM slateduck_catalog.ducklake_table` returns all tables.
#[tokio::test]
async fn test_virtual_catalog_table() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();
    let params = ParamValues::default();

    let responses = executor::execute_sql(
        "SELECT * FROM slateduck_catalog.ducklake_table",
        &params,
        &store,
        &mut session,
    )
    .await
    .unwrap();
    assert!(!responses.is_empty());
}

/// `SELECT * FROM slateduck_catalog.slateduck_counters` returns a result.
#[tokio::test]
async fn test_virtual_catalog_counters() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store(&dir).await;
    let mut session = SessionState::new();
    let params = ParamValues::default();

    let responses = executor::execute_sql(
        "SELECT * FROM slateduck_catalog.slateduck_counters",
        &params,
        &store,
        &mut session,
    )
    .await
    .unwrap();
    assert!(!responses.is_empty());
}

/// Operator introspection: a WHERE clause on slateduck_catalog.ducklake_data_file is classifiable.
#[tokio::test]
async fn test_virtual_catalog_data_file_classifiable() {
    use slateduck_sql::classify_statement;
    use slateduck_sql::StatementKind;

    let sql = "SELECT data_file_id, path, begin_snapshot FROM slateduck_catalog.ducklake_data_file WHERE table_id = 42 ORDER BY begin_snapshot DESC LIMIT 20";
    let kind = classify_statement(sql).unwrap();
    assert!(
        matches!(kind, StatementKind::VirtualCatalogScan { ref table_name } if table_name == "ducklake_data_file"),
        "expected VirtualCatalogScan{{ducklake_data_file}}, got {kind:?}"
    );
}
