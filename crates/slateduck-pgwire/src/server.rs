//! TCP server and configuration for the SlateDuck PG-Wire sidecar.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::{error, info};

use slateduck_catalog::CatalogStore;

use crate::handler::SlateDuckServerHandlers;

/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Bind address (default: 0.0.0.0:5432).
    pub bind_addr: SocketAddr,
    /// Maximum concurrent sessions (default: 50).
    pub max_sessions: usize,
    /// Maximum active scans (default: 25).
    pub max_active_scans: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:5432".parse().unwrap(),
            max_sessions: 50,
            max_active_scans: 25,
        }
    }
}

/// Run the SlateDuck PG-Wire server.
pub async fn run_server(
    config: ServerConfig,
    catalog: Arc<Mutex<CatalogStore>>,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(config.bind_addr).await?;
    info!("SlateDuck serving on {}", config.bind_addr);

    let session_semaphore = Arc::new(tokio::sync::Semaphore::new(config.max_sessions));

    loop {
        let (socket, addr) = listener.accept().await?;
        let catalog = catalog.clone();
        let semaphore = session_semaphore.clone();

        tokio::spawn(async move {
            let _permit = match semaphore.acquire().await {
                Ok(p) => p,
                Err(_) => return,
            };

            info!("New connection from {addr}");
            let handlers = SlateDuckServerHandlers::new(catalog);

            if let Err(e) = pgwire::tokio::process_socket(socket, None, handlers).await {
                error!("Connection error from {addr}: {e}");
            }
        });
    }
}

/// Run the server with a shutdown signal (for testing).
pub async fn run_server_with_shutdown(
    config: ServerConfig,
    catalog: Arc<Mutex<CatalogStore>>,
    shutdown: tokio::sync::oneshot::Receiver<()>,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(config.bind_addr).await?;
    info!("SlateDuck serving on {}", config.bind_addr);

    let session_semaphore = Arc::new(tokio::sync::Semaphore::new(config.max_sessions));

    tokio::select! {
        _ = async {
            loop {
                let (socket, addr) = match listener.accept().await {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Accept error: {e}");
                        continue;
                    }
                };
                let catalog = catalog.clone();
                let semaphore = session_semaphore.clone();

                tokio::spawn(async move {
                    let _permit = match semaphore.acquire().await {
                        Ok(p) => p,
                        Err(_) => return,
                    };

                    info!("New connection from {addr}");
                    let handlers = SlateDuckServerHandlers::new(catalog);

                    if let Err(e) = pgwire::tokio::process_socket(socket, None, handlers).await {
                        error!("Connection error from {addr}: {e}");
                    }
                });
            }
        } => {}
        _ = shutdown => {
            info!("Shutdown signal received");
        }
    }
    Ok(())
}
