//! PgWireHarness: spins up a PG-Wire server for client compatibility tests.
//!
//! Launches the Rocklake PG-Wire server on a random available port with an
//! in-memory catalog, providing a connection string that test code can use
//! with any PostgreSQL client library.
//!
//! ## Usage
//! ```ignore
//! let harness = PgWireHarness::start().await.unwrap();
//! let conn_str = harness.connection_string();
//! // Connect with tokio-postgres, sqlx, or psql...
//! harness.stop().await;
//! ```

use std::net::SocketAddr;
use std::sync::Arc;

use object_store::memory::InMemory;
use object_store::path::Path as ObjectPath;
use tokio::sync::Mutex;

use rocklake_catalog::{CatalogStore, OpenOptions};

/// PG-Wire server harness for Tier 5+ integration tests.
pub struct PgWireHarness {
    addr: SocketAddr,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    server_handle: Option<tokio::task::JoinHandle<()>>,
    catalog: Arc<Mutex<CatalogStore>>,
}

impl PgWireHarness {
    /// Start a PG-Wire server on a random port with an in-memory catalog.
    pub async fn start() -> Result<Self, PgWireHarnessError> {
        let object_store: Arc<dyn object_store::ObjectStore> = Arc::new(InMemory::new());
        let opts = OpenOptions {
            object_store,
            path: ObjectPath::from("pgwire-test-catalog"),
            encryption: None,
        };
        let catalog = CatalogStore::open(opts)
            .await
            .map_err(|e| PgWireHarnessError::Setup(e.to_string()))?;
        let catalog = Arc::new(Mutex::new(catalog));

        Self::start_with_catalog(catalog).await
    }

    /// Start a PG-Wire server using an existing shared catalog.
    pub async fn start_with_catalog(
        catalog: Arc<Mutex<CatalogStore>>,
    ) -> Result<Self, PgWireHarnessError> {
        // Find an available port.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| PgWireHarnessError::Setup(format!("bind failed: {e}")))?;
        let addr = listener
            .local_addr()
            .map_err(|e| PgWireHarnessError::Setup(format!("local_addr failed: {e}")))?;
        drop(listener);

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        let config = rocklake_pgwire::ServerConfig {
            bind_addr: addr,
            ..Default::default()
        };

        let server_catalog = catalog.clone();
        let server_handle = tokio::spawn(async move {
            if let Err(e) =
                rocklake_pgwire::run_server_with_shutdown(config, server_catalog, shutdown_rx)
                    .await
            {
                tracing::error!("PgWire test server error: {e}");
            }
        });

        // Give the server a moment to bind.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        Ok(Self {
            addr,
            shutdown_tx: Some(shutdown_tx),
            server_handle: Some(server_handle),
            catalog,
        })
    }

    /// The address the server is listening on.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// PostgreSQL-style connection string for this test server.
    pub fn connection_string(&self) -> String {
        format!(
            "host={} port={} dbname=rocklake sslmode=disable",
            self.addr.ip(),
            self.addr.port()
        )
    }

    /// Connection string as a URL (for libraries that prefer URL format).
    pub fn connection_url(&self) -> String {
        format!(
            "postgresql://{}:{}/rocklake",
            self.addr.ip(),
            self.addr.port()
        )
    }

    /// Get a reference to the catalog behind the server.
    pub fn catalog(&self) -> &Arc<Mutex<CatalogStore>> {
        &self.catalog
    }

    /// Gracefully stop the PG-Wire server.
    pub async fn stop(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.server_handle.take() {
            let _ = handle.await;
        }
    }
}

impl Drop for PgWireHarness {
    fn drop(&mut self) {
        // Best-effort shutdown if stop() wasn't called explicitly.
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Errors from the PgWire harness.
#[derive(Debug, thiserror::Error)]
pub enum PgWireHarnessError {
    #[error("setup error: {0}")]
    Setup(String),
}
