//! IvmWorkerHarness: in-process IVM worker driver for integration tests.
//!
//! `IvmWorkerHarness` wraps an `IvmWorker` and provides convenience methods
//! for tests:
//! - `tick_n`: run N worker ticks.
//! - `wait_for_lag_below`: poll until `MATVIEW_LAG` falls below a threshold.
//! - `output_row_count`: count rows in the output table via the catalog reader.
//! - `kill_and_replace`: simulate a worker process restart mid-computation.
//!
//! All harness operations are async and designed to work with
//! `DeterministicClock`.

use object_store::path::Path as ObjectPath;
use slateduck_catalog::{CatalogError, CatalogStore, OpenOptions};
use slateduck_ivm::{IvmWorker, WorkerConfig};
use std::sync::Arc;

/// Shared object-store handle used to re-open stores in multi-worker tests.
pub type SharedObjectStore = Arc<dyn object_store::ObjectStore>;

/// In-process IVM worker harness for integration tests.
pub struct IvmWorkerHarness {
    pub worker: IvmWorker,
    /// Keep the open options so we can spawn additional workers against the same store.
    opts: OpenOptions,
}

impl IvmWorkerHarness {
    /// Create a new harness with the given store and config.
    pub fn new(worker: IvmWorker, opts: OpenOptions) -> Self {
        Self { worker, opts }
    }

    /// Borrow the underlying catalog store (held by the worker).
    pub fn store(&self) -> &CatalogStore {
        &self.worker.store
    }

    /// Borrow the underlying catalog store mutably.
    pub fn store_mut(&mut self) -> &mut CatalogStore {
        &mut self.worker.store
    }

    /// Open a fresh in-memory catalog and create a harness.
    pub async fn with_temp_store(worker_id: &str) -> Result<Self, CatalogError> {
        let object_store: SharedObjectStore = Arc::new(object_store::memory::InMemory::new());
        let opts = OpenOptions {
            object_store,
            path: ObjectPath::from("test-catalog"),
            encryption: None,
        };
        Self::with_opts(opts, worker_id).await
    }

    /// Open a harness from explicit `OpenOptions`.
    pub async fn with_opts(opts: OpenOptions, worker_id: &str) -> Result<Self, CatalogError> {
        let catalog = CatalogStore::open(opts.clone()).await?;
        let config = WorkerConfig {
            worker_id: worker_id.to_string(),
            ..Default::default()
        };
        let worker = IvmWorker::new(config, catalog);
        Ok(Self { worker, opts })
    }

    /// Run `n` ticks of the worker.
    pub async fn tick_n(&mut self, n: usize) -> Result<(), slateduck_ivm::worker::IvmError> {
        for _ in 0..n {
            self.worker.tick().await?;
        }
        Ok(())
    }

    /// Count the number of inlined insert rows in `output_table_id`.
    pub async fn output_row_count(&self, output_table_id: u64) -> usize {
        self.worker
            .store
            .read_latest()
            .list_inlined_inserts(output_table_id)
            .await
            .map(|rows| rows.len())
            .unwrap_or(0)
    }

    /// Poll until the max lag across all shards of `matview_id` falls below
    /// `target_ms`, or until `max_ticks` ticks have been driven.
    ///
    /// Returns `true` if the target was reached before `max_ticks`.
    pub async fn wait_for_lag_below(
        &mut self,
        matview_id: u64,
        target_ms: u64,
        max_ticks: usize,
    ) -> bool {
        for _ in 0..max_ticks {
            let _ = self.worker.tick().await;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            if let Ok(Some(lag)) = self
                .worker
                .store
                .read_latest()
                .matview_max_lag_ms(matview_id, now)
                .await
            {
                if lag < target_ms {
                    return true;
                }
            }
        }
        false
    }

    /// Simulate a worker kill-and-restart: release all leases held by the
    /// current worker, then open a fresh worker against the same object store.
    pub async fn kill_and_replace(
        &mut self,
        new_worker_id: &str,
    ) -> Result<IvmWorkerHarness, CatalogError> {
        // Release leases so the new worker can claim immediately.
        let _ = self.worker.release_all_leases().await;
        let new_config = WorkerConfig {
            worker_id: new_worker_id.to_string(),
            ..self.worker.config.clone()
        };
        let new_catalog = CatalogStore::open(self.opts.clone()).await?;
        let new_worker = IvmWorker::new(new_config, new_catalog);
        Ok(IvmWorkerHarness {
            worker: new_worker,
            opts: self.opts.clone(),
        })
    }
}

/// A pair of harnesses sharing the same object store, for multi-worker tests.
pub struct TwoWorkerHarness {
    pub worker_a: IvmWorkerHarness,
    pub worker_b: IvmWorkerHarness,
}

impl TwoWorkerHarness {
    /// Create two workers sharing the same in-memory catalog object store.
    pub async fn new() -> Result<Self, CatalogError> {
        // A single Arc<InMemory> is shared by both stores.
        let object_store: SharedObjectStore = Arc::new(object_store::memory::InMemory::new());
        let path = ObjectPath::from("test-catalog");
        let opts = OpenOptions {
            object_store: Arc::clone(&object_store),
            path: path.clone(),
            encryption: None,
        };
        // Open the catalog once to initialise it.
        let _init = CatalogStore::open(opts).await?;
        // Reopen twice for worker_a and worker_b.
        let opts_a = OpenOptions {
            object_store: Arc::clone(&object_store),
            path: path.clone(),
            encryption: None,
        };
        let opts_b = OpenOptions {
            object_store: Arc::clone(&object_store),
            path,
            encryption: None,
        };
        let worker_a = IvmWorkerHarness::with_opts(opts_a, "worker-a").await?;
        let worker_b = IvmWorkerHarness::with_opts(opts_b, "worker-b").await?;
        Ok(TwoWorkerHarness { worker_a, worker_b })
    }
}
