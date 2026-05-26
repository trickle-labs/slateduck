//! Real DuckDB binary integration tests (v0.27.4).
//!
//! These tests spawn the actual `duckdb` binary against a live SlateDuck server
//! to verify that real DuckDB 1.5.x wire-protocol behavior is handled correctly.
//!
//! # Running
//!
//! All tests in this file require the `duckdb` binary to be in `$PATH` and the
//! `ducklake` extension to be installed. Tests that depend on not-yet-implemented
//! features are marked `#[ignore]` and must be run explicitly:
//!
//! ```sh
//! cargo test -p slateduck-pgwire --test duckdb_binary_tests -- --include-ignored
//! ```
//!
//! # What these tests cover
//!
//! - `duckdb_connects_and_pings` — DuckDB can connect, auth, and get a
//!   response for `SELECT 1` without the ducklake extension.
//! - `duckdb_attach_full_lifecycle` —
//!   Full `ATTACH 'ducklake:postgres:...'` + `USE` + `CREATE TABLE` + `SELECT`.

use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use slateduck_catalog::{CatalogStore, OpenOptions};
use slateduck_pgwire::server::ServerConfig;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::Mutex;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_catalog_opts(dir: &TempDir) -> OpenOptions {
    let store = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    OpenOptions {
        object_store: store,
        path: ObjectPath::from("catalog"),
        encryption: None,
    }
}

/// Returns `false` if the `duckdb` binary is not on `$PATH`.
fn duckdb_available() -> bool {
    Command::new("duckdb").arg("--version").output().is_ok()
}

/// Start a plain-text server on an OS-assigned port.
/// Returns `(port, shutdown_tx, handle)`.
async fn start_server(
    dir: &TempDir,
) -> (
    u16,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let catalog = Arc::new(Mutex::new(
        CatalogStore::open(make_catalog_opts(dir)).await.unwrap(),
    ));

    // Bind on port 0 to get a free port, then drop and re-bind via ServerConfig.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let config = ServerConfig {
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..Default::default()
    };

    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        slateduck_pgwire::server::run_server_with_shutdown(config, catalog, rx)
            .await
            .unwrap();
    });

    // Give the server a moment to bind.
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
    (port, tx, handle)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Verify DuckDB can connect to SlateDuck and execute a simple query using
/// `tokio-postgres` (same PgWire path, without triggering COPY).
///
/// This validates the full TCP + PgWire handshake. If this breaks after a
/// slateduck server change, the basic protocol handling is broken.
#[tokio::test]
async fn duckdb_compatible_client_connects_and_pings() {
    let dir = TempDir::new().unwrap();
    let (port, shutdown_tx, handle) = start_server(&dir).await;

    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let (client, conn) = tokio_postgres::connect(
            &format!("host=127.0.0.1 port={port} dbname=slateduck user=duckdb"),
            tokio_postgres::NoTls,
        )
        .await?;
        tokio::spawn(conn);
        let rows = client.query("SELECT 1", &[]).await?;
        Ok::<_, tokio_postgres::Error>(rows)
    })
    .await;

    let _ = shutdown_tx.send(());
    let _ = handle.await;

    let rows = result
        .expect("connection timed out after 10s")
        .expect("tokio-postgres query must succeed");
    assert_eq!(rows.len(), 1, "SELECT 1 must return 1 row");
}

/// Verify DuckDB binary can connect to SlateDuck and that the server handles
/// the postgres-scanner handshake without hanging.
///
/// Uses a 20s timeout. If the process doesn't exit within 20s, the server is
/// blocking DuckDB (e.g., an unimplemented protocol command with no response).
/// A non-zero DuckDB exit code is acceptable here — the key assertion is
/// no-hang, not success.
#[tokio::test]
async fn duckdb_binary_connects_without_hang() {
    if !duckdb_available() {
        eprintln!("SKIP: duckdb binary not available");
        return;
    }

    let dir = TempDir::new().unwrap();
    let (port, shutdown_tx, handle) = start_server(&dir).await;

    let mut child = tokio::process::Command::new("duckdb")
        .arg("-c")
        .arg(format!(
            "LOAD postgres_scanner; \
             ATTACH 'host=127.0.0.1 port={port} dbname=slateduck' AS sd (TYPE POSTGRES); \
             SELECT 1 AS connected;"
        ))
        .kill_on_drop(true)
        .spawn()
        .expect("duckdb process must start");

    let result = tokio::time::timeout(Duration::from_secs(20), child.wait()).await;

    let _ = shutdown_tx.send(());
    let _ = handle.await;

    match result {
        Err(_) => panic!("duckdb process did not exit within 20s — server may be hanging on an unimplemented protocol command"),
        Ok(Ok(_status)) => {
            // DuckDB exited (with or without error is fine — no hang is the assertion).
        }
        Ok(Err(e)) => panic!("failed to wait on duckdb process: {e}"),
    }
}

/// Full DuckLake ATTACH lifecycle: INSTALL ducklake → LOAD ducklake → ATTACH →
/// USE → CREATE TABLE → INSERT → SELECT.
///
/// This test verifies full DuckDB 1.5.x compatibility including:
/// - Postgres scanner initialization sequence
/// - COPY FROM STDIN (binary format) for catalog tables
/// - COPY TO STDOUT (binary format) for catalog reads
/// - Schema/table creation through DuckLake protocol
#[tokio::test]
#[ignore = "requires external ducklake extension install/load and can be slow/flaky in CI"]
async fn duckdb_attach_full_lifecycle() {
    let dir = TempDir::new().unwrap();
    let data_dir = TempDir::new().unwrap();
    let data_path = data_dir.path().to_string_lossy().into_owned();
    let (port, shutdown_tx, handle) = start_server(&dir).await;

    let sql = format!(
        "INSTALL ducklake; \
         LOAD ducklake; \
         ATTACH 'ducklake:postgres:host=127.0.0.1 port={port} dbname=slateduck' AS my_lake \
             (DATA_PATH '{data_path}'); \
         USE my_lake; \
         CREATE SCHEMA IF NOT EXISTS analytics; \
         CREATE TABLE analytics.events (id INTEGER, ts TIMESTAMP, payload VARCHAR); \
         INSERT INTO analytics.events VALUES (1, NOW(), 'hello'); \
         SELECT id, payload FROM analytics.events;"
    );

    let output = Command::new("duckdb")
        .arg("-c")
        .arg(&sql)
        .output()
        .expect("duckdb process must start");

    let _ = shutdown_tx.send(());
    let _ = handle.await;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "duckdb ATTACH lifecycle failed: {}\nstdout: {stdout}\nstderr: {stderr}",
        output.status
    );
    assert!(
        stdout.contains("hello"),
        "SELECT must return inserted row; got:\n{stdout}\nstderr:\n{stderr}"
    );
}
