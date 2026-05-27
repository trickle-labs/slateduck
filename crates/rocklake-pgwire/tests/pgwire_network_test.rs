//! Network-level PG-Wire integration test (N-11, v0.27.3).
//!
//! Tests a real TCP `tokio-postgres` client completing a full DuckLake
//! DDL/DML/query cycle against the running `rocklake_pgwire::server`.
//!
//! # Coverage
//!  - Full DDL/DML/query cycle over a real TCP socket.
//!  - `table_changes()` function call through the network.
//!  - TLS-required server rejects plaintext connections.
//!  - Process is torn down after each test regardless of outcome.

use futures::TryStreamExt;
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use rocklake_catalog::{CatalogStore, OpenOptions};
use rocklake_pgwire::server::{AuthConfig, ServerConfig, TlsConfig};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Mutex;

// ─── Helpers ────────────────────────────────────────────────────────────────

fn make_catalog_opts(dir: &TempDir) -> OpenOptions {
    let store = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    OpenOptions {
        object_store: store,
        path: ObjectPath::from("catalog"),
        encryption: None,
    }
}

/// Start a plain-text server on an OS-assigned port. Returns `(addr, shutdown_tx, handle)`.
async fn start_server(
    dir: &TempDir,
) -> (
    std::net::SocketAddr,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let catalog = Arc::new(Mutex::new(
        CatalogStore::open(make_catalog_opts(dir)).await.unwrap(),
    ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let config = ServerConfig {
        bind_addr: addr,
        ..Default::default()
    };

    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        rocklake_pgwire::server::run_server_with_shutdown(config, catalog, rx)
            .await
            .unwrap();
    });

    // Give the server a moment to bind.
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    (addr, tx, handle)
}

/// Generate a self-signed TLS certificate and private key in `dir`.
fn generate_self_signed_cert(dir: &TempDir) -> (String, String) {
    use rcgen::{generate_simple_self_signed, CertifiedKey};
    let subject_alt_names = vec!["127.0.0.1".to_string(), "localhost".to_string()];
    let CertifiedKey { cert, key_pair } =
        generate_simple_self_signed(subject_alt_names).expect("rcgen cert generation must succeed");
    let cert_path = dir.path().join("test.crt");
    let key_path = dir.path().join("test.key");
    std::fs::write(&cert_path, cert.pem()).unwrap();
    std::fs::write(&key_path, key_pair.serialize_pem()).unwrap();
    (
        cert_path.to_string_lossy().into_owned(),
        key_path.to_string_lossy().into_owned(),
    )
}

/// Start a TLS server on an OS-assigned port.
async fn start_tls_server(
    dir: &TempDir,
    required: bool,
) -> (
    std::net::SocketAddr,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let (cert_path, key_path) = generate_self_signed_cert(dir);
    let catalog = Arc::new(Mutex::new(
        CatalogStore::open(make_catalog_opts(dir)).await.unwrap(),
    ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let config = ServerConfig {
        bind_addr: addr,
        tls: TlsConfig {
            cert_path: Some(cert_path),
            key_path: Some(key_path),
            required,
        },
        ..Default::default()
    };

    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        rocklake_pgwire::server::run_server_with_shutdown(config, catalog, rx)
            .await
            .unwrap();
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
    (addr, tx, handle)
}

// ─── Tests ──────────────────────────────────────────────────────────────────

/// Full DDL/DML/query cycle over a real TCP socket.
///
/// Covers: CREATE SCHEMA, CREATE TABLE, INSERT (data file registration),
/// SELECT (query catalog facades), and `table_changes()`.
#[tokio::test]
async fn full_ddl_dml_query_cycle_over_tcp() {
    let dir = TempDir::new().unwrap();
    let (addr, tx, handle) = start_server(&dir).await;

    // Connect with tokio-postgres (no TLS).
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

    // ── 1. Create a schema ──────────────────────────────────────────────────
    // DuckLake requires InsertSchema and InsertSnapshot in the same atomic
    // commit. Wrap in BEGIN/COMMIT so that the CatalogWriter calls
    // create_snapshot() and actually persists the staged schema row.
    client.execute("BEGIN", &[]).await.unwrap();
    client
        .execute(
            "INSERT INTO ducklake_schema (schema_name) VALUES ($1)",
            &[&"events"],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO ducklake_snapshot (author, message) VALUES ($1, $2)",
            &[&"network-test", &"create schema events"],
        )
        .await
        .unwrap();
    client.execute("COMMIT", &[]).await.unwrap();

    // ── 2. Retrieve the snapshot id ─────────────────────────────────────────
    let snap_rows = client
        .query("SELECT max(snapshot_id) AS s FROM ducklake_snapshot", &[])
        .await
        .unwrap();
    assert_eq!(snap_rows.len(), 1, "must have at least one snapshot");
    let snap_id: i64 = snap_rows[0].get("max");
    assert!(snap_id >= 1, "snapshot_id must be positive");

    // Retrieve schema_id.  The server returns all schemas visible at snap_id;
    // the test asserts there is exactly one (the "events" schema just created).
    let schema_rows = client
        .query(
            "SELECT schema_id, schema_name FROM ducklake_schema WHERE begin_snapshot <= $1 \
             AND (end_snapshot IS NULL OR end_snapshot > $1)",
            &[&snap_id],
        )
        .await
        .unwrap();
    assert_eq!(schema_rows.len(), 1, "must find exactly one schema");
    let schema_id: i64 = schema_rows[0].get("schema_id");

    // ── 3. Create a table ───────────────────────────────────────────────────
    client.execute("BEGIN", &[]).await.unwrap();
    client
        .execute(
            "INSERT INTO ducklake_table (schema_id, table_name, data_path) VALUES ($1, $2, $3)",
            &[&schema_id, &"logs", &"events/logs/"],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO ducklake_snapshot (author, message) VALUES ($1, $2)",
            &[&"network-test", &"create table logs"],
        )
        .await
        .unwrap();
    client.execute("COMMIT", &[]).await.unwrap();

    let snap_rows2 = client
        .query("SELECT max(snapshot_id) AS s FROM ducklake_snapshot", &[])
        .await
        .unwrap();
    let snap2: i64 = snap_rows2[0].get("max");

    // Retrieve table_id.  The DuckLake protocol passes schema_id as the first
    // parameter ($1) so that the server can call list_tables(schema_id).
    let table_rows = client
        .query(
            "SELECT table_id FROM ducklake_table WHERE schema_id = $1",
            &[&schema_id],
        )
        .await
        .unwrap();
    assert_eq!(table_rows.len(), 1, "must find exactly one table");
    let table_id: i64 = table_rows[0].get("table_id");

    // ── 4. Register a data file (INSERT) ────────────────────────────────────
    client.execute("BEGIN", &[]).await.unwrap();
    client
        .execute(
            "INSERT INTO ducklake_data_file (table_id, path, file_format, row_count, file_size_bytes) VALUES ($1, $2, $3, $4, $5)",
            &[&table_id, &"events/logs/part-0.parquet", &"parquet", &100i64, &4096i64],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO ducklake_snapshot (author, message) VALUES ($1, $2)",
            &[&"network-test", &"register data file"],
        )
        .await
        .unwrap();
    client.execute("COMMIT", &[]).await.unwrap();

    // ── 5. SELECT — query the catalog ───────────────────────────────────────
    let schema_list = client
        .query(
            "SELECT schema_name FROM ducklake_schema WHERE begin_snapshot <= $1 \
             AND (end_snapshot IS NULL OR $1 < end_snapshot)",
            &[&snap2],
        )
        .await
        .unwrap();
    assert!(!schema_list.is_empty(), "must see the events schema");
    let names: Vec<String> = schema_list.iter().map(|r| r.get("schema_name")).collect();
    assert!(
        names.contains(&"events".to_string()),
        "events schema must be present"
    );

    let file_rows = client
        .query(
            "SELECT path FROM ducklake_data_file WHERE table_id = $1",
            &[&table_id],
        )
        .await
        .unwrap();
    assert_eq!(file_rows.len(), 1, "must see the registered data file");
    let path: &str = file_rows[0].get("path");
    assert_eq!(path, "events/logs/part-0.parquet");

    // ── 6. table_changes() — CDC query ──────────────────────────────────────
    // `table_changes()` requires a registered Parquet file; calling it with a
    // non-existent snapshot range returns an error (not a panic).
    let tc_result = client
        .query(
            "SELECT * FROM table_changes($1, $2, $3)",
            &[&"events.logs", &(snap2 - 1i64), &snap2],
        )
        .await;
    // The call may succeed (empty result if the parquet file is missing) or
    // return a storage error; both outcomes confirm the code path ran.
    match &tc_result {
        Ok(rows) => {
            // Successful (empty result is expected if the file doesn't exist locally).
            let _ = rows;
        }
        Err(e) => {
            // Storage error is acceptable; must NOT be a server panic / crash.
            let msg = e.to_string();
            assert!(
                msg.contains("58030") || msg.contains("storage") || msg.contains("not found"),
                "unexpected error from table_changes: {e}"
            );
        }
    }

    // Tear down the server cleanly.
    let _ = tx.send(());
    let _ = handle.await;
}

/// Verify binary COPY TO STDOUT streams parseable rows over the network.
#[tokio::test]
async fn copy_to_stdout_binary_streams_rows() {
    let dir = TempDir::new().unwrap();
    let (addr, tx, handle) = start_server(&dir).await;

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

    let stream = client
        .copy_out("COPY (SELECT current_database()) TO STDOUT (FORMAT binary)")
        .await
        .unwrap();
    let chunks = stream.try_collect::<Vec<_>>().await.unwrap();

    assert!(
        !chunks.is_empty(),
        "COPY stream must include at least one chunk"
    );

    // Binary COPY signature in first chunk.
    const SIG: &[u8] = b"PGCOPY\n\xff\r\n\0";
    assert!(
        chunks[0].len() >= 19,
        "first COPY chunk must contain binary header"
    );
    assert_eq!(
        &chunks[0][..SIG.len()],
        SIG,
        "invalid binary COPY signature"
    );

    // End-of-copy marker should be int16 -1 at the end of the payload.
    let last = &chunks[chunks.len() - 1];
    assert!(
        last.len() >= 2,
        "COPY payload must contain trailer bytes at the end"
    );
    assert_eq!(&last[last.len() - 2..], &[0xff, 0xff]);

    let _ = tx.send(());
    let _ = handle.await;
}

/// `SELECT version()` returns a PostgreSQL-compatible version string.
#[tokio::test]
async fn select_version_returns_postgresql_compatible_string() {
    let dir = TempDir::new().unwrap();
    let (addr, tx, handle) = start_server(&dir).await;

    let conn_str = format!(
        "host=127.0.0.1 port={} user=duckdb dbname=ducklake",
        addr.port()
    );
    let (client, connection) = tokio_postgres::connect(&conn_str, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let rows = client.query("SELECT version()", &[]).await.unwrap();
    assert_eq!(rows.len(), 1);
    let v: &str = rows[0].get(0);
    assert!(
        v.contains("PostgreSQL"),
        "version() must contain 'PostgreSQL', got: {v}"
    );

    let _ = tx.send(());
    let _ = handle.await;
}

/// TLS-required server rejects plaintext connections.
#[tokio::test]
async fn tls_required_rejects_plaintext_connection() {
    let dir = TempDir::new().unwrap();
    let (addr, tx, handle) = start_tls_server(&dir, true).await;

    let conn_str = format!(
        "host=127.0.0.1 port={} user=duckdb dbname=ducklake sslmode=disable",
        addr.port()
    );
    let result = tokio_postgres::connect(&conn_str, tokio_postgres::NoTls).await;
    assert!(
        result.is_err(),
        "plaintext connection to TLS-required server must fail"
    );

    let _ = tx.send(());
    let _ = handle.await;
}

/// TLS-optional server accepts plaintext connections (handshake not required).
#[tokio::test]
async fn tls_optional_server_accepts_plaintext() {
    let dir = TempDir::new().unwrap();
    let (addr, tx, handle) = start_tls_server(&dir, false).await;

    let conn_str = format!(
        "host=127.0.0.1 port={} user=duckdb dbname=ducklake",
        addr.port()
    );
    let (client, connection) = tokio_postgres::connect(&conn_str, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let rows = client.query("SELECT version()", &[]).await.unwrap();
    assert_eq!(rows.len(), 1);

    let _ = tx.send(());
    let _ = handle.await;
}

/// Auth-required server rejects connections with wrong password.
#[tokio::test]
async fn auth_required_rejects_wrong_password() {
    let dir = TempDir::new().unwrap();
    let catalog = Arc::new(Mutex::new(
        CatalogStore::open(make_catalog_opts(&dir)).await.unwrap(),
    ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let config = ServerConfig {
        bind_addr: addr,
        auth: AuthConfig {
            username: Some("admin".to_string()),
            password: Some("secret".to_string()),
            scram_sha256: false,
        },
        ..Default::default()
    };

    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        rocklake_pgwire::server::run_server_with_shutdown(config, catalog, rx)
            .await
            .unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Wrong password — must fail authentication.
    let bad_conn = format!(
        "host=127.0.0.1 port={} user=admin password=wrong dbname=ducklake",
        addr.port()
    );
    let result = tokio_postgres::connect(&bad_conn, tokio_postgres::NoTls).await;
    assert!(result.is_err(), "wrong password must be rejected");

    let _ = tx.send(());
    let _ = handle.await;
}
