//! TCP server and configuration for the RockLake PG-Wire sidecar.
//!
//! Supports optional TLS (--tls-cert, --tls-key) and password authentication.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use rocklake_catalog::CatalogStore;

use crate::handler::RockLakeServerHandlers;
use crate::notify::NotifyManager;

/// Monotonically-tracked session counters used to populate Prometheus gauges.
///
/// `active_sessions`  — sessions that currently have a command in flight.
/// `idle_sessions`    — sessions that are connected but waiting for the next query.
///
/// Both are signed to allow correct delta-based arithmetic even if a session
/// races with a snapshot.  They are always ≥ 0 in steady state.
#[derive(Default)]
pub struct SessionCounters {
    /// Connections that are currently executing a query.
    pub active_sessions: AtomicI64,
    /// Connections that are open but idle (waiting for the next query).
    pub idle_sessions: AtomicI64,
}

impl SessionCounters {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

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
    /// Use SCRAM-SHA-256 instead of cleartext password authentication.
    ///
    /// When `true` the server initiates a SASL/SCRAM-SHA-256 exchange so
    /// that the plaintext credential is never transmitted over the wire.
    /// Requires `username` and `password` to be set.
    pub scram_sha256: bool,
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
    /// Duration after which an idle connection is closed (default: 60 s).
    pub idle_connection_timeout: std::time::Duration,
    /// Grace period for in-flight queries during SIGTERM drain (default: 30 s).
    pub drain_timeout: std::time::Duration,
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
            idle_connection_timeout: std::time::Duration::from_secs(60),
            drain_timeout: std::time::Duration::from_secs(30),
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

/// Run the RockLake PG-Wire server.
///
/// This function does not return until the process receives SIGTERM (Unix) or
/// a hard error on the listener.  On SIGTERM it stops accepting new connections
/// and waits up to `config.drain_timeout` for in-flight sessions to finish.
pub async fn run_server(
    config: ServerConfig,
    catalog: Arc<Mutex<CatalogStore>>,
) -> std::io::Result<()> {
    #[cfg(unix)]
    let shutdown_signal = async {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        sigterm.recv().await;
    };
    #[cfg(not(unix))]
    let shutdown_signal = tokio::signal::ctrl_c();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let _ = shutdown_signal.await;
        let _ = shutdown_tx.send(());
    });

    run_server_with_shutdown(config, catalog, shutdown_rx).await
}

/// Run the server with a shutdown signal (for testing and graceful drain).
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
        info!("RockLake serving on {} (TLS enabled)", config.bind_addr);
    } else {
        info!("RockLake serving on {}", config.bind_addr);
    }

    let session_semaphore = Arc::new(tokio::sync::Semaphore::new(config.max_sessions));
    let auth_config = Arc::new(config.auth);
    let tls_required = config.tls.required;
    let notify_manager = Arc::new(NotifyManager::new());
    let extension_schemas = Arc::new(config.extension_schemas);
    let drain_timeout = config.drain_timeout;

    // Session counters exposed as Prometheus gauges.
    let counters = SessionCounters::new();
    // Active-session tracking for graceful drain.
    let active_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    tokio::select! {
        result = async {
            loop {
                let (socket, addr) = listener.accept().await?;
                let catalog = catalog.clone();
                let semaphore = session_semaphore.clone();
                let tls = tls_acceptor.clone();
                let auth = auth_config.clone();
                let nm = notify_manager.clone();
                let es = extension_schemas.clone();
                let counters_ref = counters.clone();
                let active_ref = active_count.clone();

                tokio::spawn(async move {
                    let _permit = match semaphore.acquire().await {
                        Ok(p) => p,
                        Err(_) => return,
                    };

                    // Track idle → active transition.
                    counters_ref.idle_sessions.fetch_add(1, Ordering::Relaxed);
                    active_ref.fetch_add(1, Ordering::AcqRel);

                    info!("New connection from {addr}");
                    let handlers =
                        RockLakeServerHandlers::new_with_config(catalog, auth, tls_required, nm, es);

                    if let Err(e) = pgwire::tokio::process_socket(socket, tls, handlers).await {
                        error!("Connection error from {addr}: {e}");
                    }

                    counters_ref.idle_sessions.fetch_sub(1, Ordering::Relaxed);
                    active_ref.fetch_sub(1, Ordering::AcqRel);
                });
            }
            #[allow(unreachable_code)]
            Ok::<(), std::io::Error>(())
        } => { result }
        _ = shutdown => {
            info!("Shutdown signal received; draining in-flight sessions (timeout: {:?})", drain_timeout);
            // Stop the listener (drop it) and wait for active sessions to drain.
            drop(listener);
            let deadline = tokio::time::Instant::now() + drain_timeout;
            loop {
                if active_count.load(std::sync::atomic::Ordering::Acquire) == 0 {
                    info!("All sessions drained");
                    break;
                }
                if tokio::time::Instant::now() >= deadline {
                    warn!("Drain timeout exceeded; forcing shutdown with {} active session(s)",
                        active_count.load(std::sync::atomic::Ordering::Relaxed));
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Ok(())
        }
    }
}
