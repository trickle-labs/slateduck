//! Background heartbeat task for shard lease renewal.
//!
//! The heartbeat loop extends the worker's lease on each owned shard every
//! `lease_ttl / 3` milliseconds.  If a heartbeat fails (e.g. generation
//! mismatch — another worker stole the shard), the shard is removed from the
//! worker's ownership set.
//!
//! ## Usage
//! ```ignore
//! let (tx, rx) = tokio::sync::mpsc::channel(16);
//! let handle = HeartbeatTask::spawn(config, store, tx, rx);
//! // … worker tick loop …
//! handle.abort();
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use slateduck_catalog::CatalogStore;

use crate::config::WorkerConfig;

/// Shared state between the worker and the heartbeat task.
#[derive(Debug, Default)]
pub struct LeaseRegistry {
    /// `(matview_id, shard_id) → generation` for leases currently held.
    pub generations: HashMap<(u64, u32), u64>,
}

/// A running heartbeat task handle.
pub struct HeartbeatHandle {
    task: tokio::task::JoinHandle<()>,
    /// Set to true to request a graceful stop.
    pub stop: Arc<std::sync::atomic::AtomicBool>,
}

impl HeartbeatHandle {
    /// Signal the heartbeat to stop and wait for it.
    pub async fn shutdown(self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = self.task.await;
    }
}

/// Spawn a background heartbeat task.
///
/// The task wakes every `lease_ttl / 3` ms, iterates over `registry.generations`
/// and calls `extend_matview_lease` for each entry.  If an extension fails
/// (e.g. `GenerationMismatch`), that shard is removed from the registry.
pub fn spawn(
    config: WorkerConfig,
    mut store: CatalogStore,
    registry: Arc<Mutex<LeaseRegistry>>,
) -> HeartbeatHandle {
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);
    let interval_ms = config.lease_duration_ms / 3;

    let task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(interval_ms));
        loop {
            interval.tick().await;
            if stop_clone.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }

            let now_ms = now_unix_ms();
            let new_expires = now_ms + config.lease_duration_ms;

            // Snapshot the current leases under the lock.
            let owned: Vec<((u64, u32), u64)> = {
                let reg = registry.lock().unwrap();
                reg.generations.iter().map(|(&k, &v)| (k, v)).collect()
            };

            for ((matview_id, shard_id), expected_generation) in owned {
                let mut writer = store.begin_write();
                match writer
                    .extend_matview_lease(
                        matview_id,
                        shard_id,
                        &config.worker_id,
                        expected_generation,
                        new_expires,
                    )
                    .await
                {
                    Ok(new_gen) => {
                        let mut reg = registry.lock().unwrap();
                        if let Some(g) = reg.generations.get_mut(&(matview_id, shard_id)) {
                            *g = new_gen;
                        }
                        tracing::debug!(matview_id, shard_id, new_gen, "heartbeat extended lease");
                    }
                    Err(e) => {
                        tracing::warn!(
                            matview_id,
                            shard_id,
                            %e,
                            "heartbeat failed to extend lease; removing from registry"
                        );
                        let mut reg = registry.lock().unwrap();
                        reg.generations.remove(&(matview_id, shard_id));
                    }
                }
            }
        }
    });

    HeartbeatHandle { task, stop }
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
