//! Graceful shutdown for the IVM worker.
//!
//! Listens for SIGTERM (Unix) or Ctrl-C (all platforms) and sets a shared
//! shutdown flag.  The main tick loop polls this flag and initiates an orderly
//! drain: finish the current batch, checkpoint all shards, release all
//! leases, then exit 0.
//!
//! ## Drain timeout
//! If draining takes longer than `max_drain_time_ms` the worker exits with
//! status 1 to let Kubernetes replace the pod.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

/// Shared shutdown signal.
#[derive(Clone)]
pub struct ShutdownSignal {
    inner: Arc<AtomicBool>,
}

impl ShutdownSignal {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns `true` once a shutdown has been requested.
    pub fn is_requested(&self) -> bool {
        self.inner.load(Ordering::Relaxed)
    }

    /// Trigger a shutdown (called by the signal handler or tests).
    pub fn request(&self) {
        self.inner.store(true, Ordering::Relaxed);
    }
}

impl Default for ShutdownSignal {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn a background task that listens for SIGTERM / Ctrl-C and sets the
/// shutdown signal.
///
/// Returns the [`ShutdownSignal`] that the caller should poll.
pub fn install() -> ShutdownSignal {
    let signal = ShutdownSignal::new();
    let signal_clone = signal.clone();

    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm =
                signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
            tokio::select! {
                _ = sigterm.recv() => {
                    tracing::info!("SIGTERM received — initiating graceful shutdown");
                    signal_clone.request();
                }
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("Ctrl-C received — initiating graceful shutdown");
                    signal_clone.request();
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("Ctrl-C received — initiating graceful shutdown");
            signal_clone.request();
        }
    });

    signal
}

/// Wait until the shutdown signal fires OR `timeout_ms` milliseconds have
/// elapsed.  Returns `true` if shutdown was requested, `false` if timed out.
pub async fn wait_or_timeout(signal: &ShutdownSignal, timeout_ms: u64) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if signal.is_requested() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
