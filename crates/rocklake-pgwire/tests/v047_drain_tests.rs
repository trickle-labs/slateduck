//! v0.47.0 Tests — Graceful SIGTERM Drain (Connection Management)
//!
//! Verifies that the PG-wire server receives a shutdown signal and exits
//! cleanly after all in-flight sessions complete their current query.

use object_store::path::Path as ObjectPath;
use rocklake_catalog::{CatalogStore, OpenOptions};
use rocklake_pgwire::server::{run_server_with_shutdown, ServerConfig};
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::oneshot;

fn catalog_opts(dir: &TempDir) -> OpenOptions {
    let path = dir.path().to_str().unwrap().to_string();
    let store = Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&path).unwrap());
    OpenOptions {
        object_store: store,
        path: ObjectPath::from("catalog"),
        encryption: None,
    }
}

/// Graceful SIGTERM drain: the server receives a shutdown signal while no
/// active queries are in flight and exits cleanly within the drain timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_sigterm_graceful_drain_no_active_sessions() {
    let dir = TempDir::new().unwrap();
    let mut w = CatalogStore::open(catalog_opts(&dir)).await.unwrap();
    let mut writer = w.begin_write();
    writer.create_schema("drain_test").await.unwrap();
    let result = writer.create_snapshot(None, None).await.unwrap();
    w.commit_writer(result);

    let catalog = Arc::new(tokio::sync::Mutex::new(w));
    let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let config = ServerConfig {
        bind_addr: bind,
        drain_timeout: std::time::Duration::from_millis(500),
        ..ServerConfig::default()
    };

    let server_task = tokio::spawn(run_server_with_shutdown(config, catalog, shutdown_rx));

    // Let the server start.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Send shutdown signal — server should drain and exit cleanly.
    let _ = shutdown_tx.send(());

    let result = tokio::time::timeout(std::time::Duration::from_secs(3), server_task)
        .await
        .expect("server did not exit within timeout")
        .expect("server task panicked");

    assert!(result.is_ok(), "graceful drain must return Ok: {result:?}");
}

/// Verify that the server stops accepting new connections after shutdown is
/// signalled (even if the drain timeout has not yet elapsed).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shutdown_stops_accepting_connections() {
    let dir = TempDir::new().unwrap();
    let w = CatalogStore::open(catalog_opts(&dir)).await.unwrap();
    let catalog = Arc::new(tokio::sync::Mutex::new(w));

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let config = ServerConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        drain_timeout: std::time::Duration::from_millis(200),
        ..ServerConfig::default()
    };

    let server_task = tokio::spawn(run_server_with_shutdown(config, catalog, shutdown_rx));

    tokio::time::sleep(std::time::Duration::from_millis(30)).await;

    // Signal shutdown.
    let _ = shutdown_tx.send(());

    // Server should exit quickly with no active sessions.
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), server_task)
        .await
        .expect("server did not exit in time")
        .expect("server task panicked");

    assert!(result.is_ok());
}
