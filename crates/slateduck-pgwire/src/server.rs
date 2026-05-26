//! TCP server and configuration for the SlateDuck PG-Wire sidecar.
//!
//! Supports optional TLS (--tls-cert, --tls-key) and password authentication.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use slateduck_catalog::CatalogStore;

use crate::handler::SlateDuckServerHandlers;
use crate::notify::NotifyManager;

/// TLS configuration.
#[derive(Debug, Clone, Default)]
pub struct TlsConfig {
    /// Path to the TLS certificate file (PEM format).
    pub cert_path: Option<String>,
    /// Path to the TLS private key file (PEM format).
    pub key_path: Option<String>,
    /// Reject plaintext connections when TLS is not configured.
    /// Requires `cert_path` and `key_path` to be set.
    pub required: bool,
}

impl TlsConfig {
    pub fn is_enabled(&self) -> bool {
        self.cert_path.is_some() && self.key_path.is_some()
    }
}

/// Authentication configuration.
#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    /// Username for password authentication (None = no auth).
    pub username: Option<String>,
    /// Password for password authentication.
    pub password: Option<String>,
}

impl AuthConfig {
    pub fn is_enabled(&self) -> bool {
        self.username.is_some() && self.password.is_some()
    }
}

/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Bind address (default: 0.0.0.0:5432).
    pub bind_addr: SocketAddr,
    /// Maximum concurrent sessions (default: 50).
    pub max_sessions: usize,
    /// Maximum active scans (default: 25).
    pub max_active_scans: usize,
    /// TLS configuration.
    pub tls: TlsConfig,
    /// Authentication configuration.
    pub auth: AuthConfig,
    /// Allowed extension schema names (default: `["pgtrickle"]`).
    pub extension_schemas: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::from(([0, 0, 0, 0], 5432)),
            max_sessions: 50,
            max_active_scans: 25,
            tls: TlsConfig::default(),
            auth: AuthConfig::default(),
            extension_schemas: vec!["public".to_string(), "pgtrickle".to_string()],
        }
    }
}

/// Build a TLS acceptor from cert and key paths.
fn build_tls_acceptor(tls_config: &TlsConfig) -> std::io::Result<Arc<tokio_rustls::TlsAcceptor>> {
    use std::io::BufReader;
    use tokio_rustls::rustls::{self, pki_types::PrivateKeyDer};

    // Ensure a crypto provider is installed (no-op if already set).
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cert_path = tls_config.cert_path.as_ref().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "TLS cert path not configured",
        )
    })?;
    let key_path = tls_config.key_path.as_ref().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "TLS key path not configured",
        )
    })?;

    let cert_file = std::fs::File::open(cert_path)
        .map_err(|e| std::io::Error::other(format!("cannot open TLS cert: {e}")))?;
    let mut cert_reader = BufReader::new(cert_file);
    let certs: Vec<_> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<_, _>>()
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid cert: {e}"),
            )
        })?;

    let key_file = std::fs::File::open(key_path)
        .map_err(|e| std::io::Error::other(format!("cannot open TLS key: {e}")))?;
    let mut key_reader = BufReader::new(key_file);
    let key: PrivateKeyDer = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, format!("invalid key: {e}"))
        })?
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "no private key found")
        })?;

    let server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("TLS config error: {e}"),
            )
        })?;

    Ok(Arc::new(tokio_rustls::TlsAcceptor::from(Arc::new(
        server_config,
    ))))
}

/// Run the SlateDuck PG-Wire server.
pub async fn run_server(
    config: ServerConfig,
    catalog: Arc<Mutex<CatalogStore>>,
) -> std::io::Result<()> {
    let tls_acceptor = if config.tls.is_enabled() {
        Some(build_tls_acceptor(&config.tls)?)
    } else if config.tls.required {
        return Err(std::io::Error::other(
            "--tls-required is set but no TLS certificate/key were provided",
        ));
    } else {
        None
    };

    // Warn when auth is enabled but TLS is not: credentials sent in plaintext.
    if config.auth.is_enabled() && tls_acceptor.is_none() {
        warn!(
            "Password authentication is enabled without TLS. Credentials will be sent \
             in plaintext. Use --tls-cert / --tls-key to enable TLS, or pass \
             --insecure-no-tls-warning-suppress if this is intentional."
        );
    }

    let listener = TcpListener::bind(config.bind_addr).await?;
    if tls_acceptor.is_some() {
        info!("SlateDuck serving on {} (TLS enabled)", config.bind_addr);
    } else {
        info!("SlateDuck serving on {}", config.bind_addr);
    }

    let session_semaphore = Arc::new(tokio::sync::Semaphore::new(config.max_sessions));
    let auth_config = Arc::new(config.auth);
    let tls_required = config.tls.required;
    let notify_manager = Arc::new(NotifyManager::new());
    let extension_schemas = Arc::new(config.extension_schemas);

    loop {
        let (socket, addr) = listener.accept().await?;
        let catalog = catalog.clone();
        let semaphore = session_semaphore.clone();
        let tls = tls_acceptor.clone();
        let auth = auth_config.clone();
        let nm = notify_manager.clone();
        let es = extension_schemas.clone();

        tokio::spawn(async move {
            let _permit = match semaphore.acquire().await {
                Ok(p) => p,
                Err(_) => return,
            };

            info!("New connection from {addr}");
            let handlers =
                SlateDuckServerHandlers::new_with_config(catalog, auth, tls_required, nm, es);

            if let Err(e) = pgwire::tokio::process_socket(socket, tls, handlers).await {
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
    let tls_acceptor = if config.tls.is_enabled() {
        Some(build_tls_acceptor(&config.tls)?)
    } else if config.tls.required {
        return Err(std::io::Error::other(
            "--tls-required is set but no TLS certificate/key were provided",
        ));
    } else {
        None
    };

    // Warn when auth is enabled but TLS is not.
    if config.auth.is_enabled() && tls_acceptor.is_none() {
        warn!(
            "Password authentication is enabled without TLS. Credentials will be sent \
             in plaintext. Use --tls-cert / --tls-key to enable TLS, or pass \
             --insecure-no-tls-warning-suppress if this is intentional."
        );
    }

    let listener = TcpListener::bind(config.bind_addr).await?;
    info!("SlateDuck serving on {}", config.bind_addr);

    let session_semaphore = Arc::new(tokio::sync::Semaphore::new(config.max_sessions));
    let auth_config = Arc::new(config.auth);
    let tls_required = config.tls.required;
    let notify_manager = Arc::new(NotifyManager::new());
    let extension_schemas = Arc::new(config.extension_schemas);

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
                let tls = tls_acceptor.clone();
                let auth = auth_config.clone();
                let nm = notify_manager.clone();
                let es = extension_schemas.clone();

                tokio::spawn(async move {
                    let _permit = match semaphore.acquire().await {
                        Ok(p) => p,
                        Err(_) => return,
                    };

                    info!("New connection from {addr}");
                    let handlers =
                        SlateDuckServerHandlers::new_with_config(catalog, auth, tls_required, nm, es);

                    if let Err(e) = pgwire::tokio::process_socket(socket, tls, handlers).await {
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
