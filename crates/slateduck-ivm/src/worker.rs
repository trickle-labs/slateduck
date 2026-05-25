//! IVM worker: discovers matviews, acquires leases, drives the compute loop.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use slateduck_catalog::{CatalogStore, ClaimOutcome};
use slateduck_core::rows::OutputMode;

use crate::circuit::ZDelta;
use crate::config::WorkerConfig;
use crate::observability;
use crate::plan::IvmPlan;
use crate::shard_key::{auto_detect_shard_key, compute_key_ranges, shard_index_for};
use crate::trace::IvmTrace;

/// IVM-specific errors.
#[derive(Debug, thiserror::Error)]
pub enum IvmError {
    #[error("catalog error: {0}")]
    Catalog(String),
    #[error("plan parse error: {0}")]
    PlanParse(String),
    #[error("output error: {0}")]
    Output(String),
    #[error("worker error: {0}")]
    Worker(String),
}

/// IVM worker — drives incremental computation for all matviews in the catalog.
pub struct IvmWorker {
    pub config: WorkerConfig,
    pub store: CatalogStore,
    /// Cached plans keyed by matview_id.
    plans: HashMap<u64, IvmPlan>,
    /// Live trace state keyed by (matview_id, shard_id).
    traces: HashMap<(u64, u32), IvmTrace>,
    /// Running generation values for held leases.
    generations: HashMap<(u64, u32), u64>,
}

impl IvmWorker {
    /// Create a new IVM worker.
    pub fn new(config: WorkerConfig, store: CatalogStore) -> Self {
        Self {
            config,
            store,
            plans: HashMap::new(),
            traces: HashMap::new(),
            generations: HashMap::new(),
        }
    }

    /// Count the number of shards currently held by this worker.
    pub fn held_shard_count(&self) -> u32 {
        self.generations.len() as u32
    }

    /// Run one complete tick across all claimable shards.
    pub async fn tick(&mut self) -> Result<(), IvmError> {
        let now_unix_ms = now_ms();
        let latest_snapshot = {
            let reader = self.store.read_latest();
            reader
                .get_snapshot()
                .await
                .map_err(|e| IvmError::Catalog(e.to_string()))?
                .map(|s| s.snapshot_id)
                .unwrap_or(0)
        };

        // Discover matviews and refresh plan cache.
        let matviews = self
            .store
            .read_latest()
            .list_matviews()
            .await
            .map_err(|e| IvmError::Catalog(e.to_string()))?;

        for mv in &matviews {
            if let std::collections::hash_map::Entry::Vacant(e) = self.plans.entry(mv.matview_id) {
                match IvmPlan::parse(&mv.view_sql) {
                    Ok(plan) => {
                        e.insert(plan);
                    }
                    Err(err) => {
                        tracing::warn!(matview_id = mv.matview_id, %err, "failed to parse view SQL, skipping");
                        continue;
                    }
                }
            }

            let output_mode = OutputMode::from_u32(mv.output_mode);

            for shard_id in 0..mv.shard_count {
                // Enforce shard_limit: skip if at capacity and not already holding.
                if self.config.shard_limit > 0
                    && self.held_shard_count() >= self.config.shard_limit
                    && !self.generations.contains_key(&(mv.matview_id, shard_id))
                {
                    tracing::debug!(
                        shard_limit = self.config.shard_limit,
                        "shard_limit reached, skipping shard ({},{})",
                        mv.matview_id,
                        shard_id
                    );
                    continue;
                }

                self.process_shard(
                    mv.matview_id,
                    shard_id,
                    mv.output_table_id,
                    latest_snapshot,
                    now_unix_ms,
                    output_mode,
                    &mv.shard_key_column.clone(),
                    mv.shard_count,
                )
                .await?;
            }
        }

        Ok(())
    }

    /// Release all currently-held leases.  Called during graceful drain.
    pub async fn release_all_leases(&mut self) -> Result<(), IvmError> {
        let held: Vec<(u64, u32)> = self.generations.keys().copied().collect();
        for (matview_id, shard_id) in held {
            let mut writer = self.store.begin_write();
            let _ = writer
                .release_matview_lease(matview_id, shard_id, &self.config.worker_id)
                .await;
        }
        self.generations.clear();
        Ok(())
    }

    /// Process a single shard: acquire lease → read input → update state → write output.
    #[allow(clippy::too_many_arguments)]
    async fn process_shard(
        &mut self,
        matview_id: u64,
        shard_id: u32,
        output_table_id: u64,
        up_to_snapshot: u64,
        now_unix_ms: u64,
        output_mode: OutputMode,
        shard_key_column: &str,
        shard_count: u32,
    ) -> Result<(), IvmError> {
        let plan = match self.plans.get(&matview_id) {
            Some(p) => p.clone(),
            None => return Ok(()),
        };

        // Acquire lease.
        let mut writer = self.store.begin_write();

        let outcome = writer
            .claim_matview_shard(
                matview_id,
                shard_id,
                &self.config.worker_id,
                self.config.lease_duration_ms,
                now_unix_ms,
            )
            .await
            .map_err(|e| IvmError::Catalog(e.to_string()))?;

        let generation = match outcome {
            ClaimOutcome::Acquired {
                generation,
                expires_unix_ms,
            } => {
                observability::emit_lease_acquired(
                    matview_id,
                    shard_id,
                    generation,
                    expires_unix_ms,
                );
                generation
            }
            ClaimOutcome::AlreadyOwned { generation } => generation,
            ClaimOutcome::Contended { current_owner } => {
                observability::emit_lease_contended(matview_id, shard_id, &current_owner);
                return Ok(());
            }
        };

        self.generations.insert((matview_id, shard_id), generation);

        // Commit the lease acquisition.
        self.store.commit_writer(&writer);

        // Get or create trace state.
        let trace_key = (matview_id, shard_id);
        self.traces
            .entry(trace_key)
            .or_insert_with(|| IvmTrace::new(plan.clone()));

        // Read new inputs.
        let reader = self
            .store
            .read_at(slateduck_core::mvcc::SnapshotId::new(up_to_snapshot))
            .map_err(|e| IvmError::Catalog(e.to_string()))?;

        // Find the base table: use the first dep's base_table_id.
        let deps = reader
            .list_matview_deps(matview_id)
            .await
            .map_err(|e| IvmError::Catalog(e.to_string()))?;
        let base_table_id = match deps.first() {
            Some(d) => d.base_table_id,
            None => return Ok(()),
        };

        let input_rows = reader
            .list_inlined_inserts(base_table_id)
            .await
            .map_err(|e| IvmError::Catalog(e.to_string()))?;

        let trace = self.traces.get_mut(&trace_key).unwrap();
        let last = trace.last_input_snapshot;

        // Only process rows added since the last checkpoint,
        // filtered to those belonging to this shard's key range.
        let key_ranges = compute_key_ranges(shard_count);
        let effective_shard_key = if shard_key_column.is_empty() {
            // Fall back to auto-detect from GROUP BY columns.
            auto_detect_shard_key(&plan.group_by_cols)
                .map(|s| s.to_string())
                .unwrap_or_default()
        } else {
            shard_key_column.to_string()
        };

        let new_rows: Vec<ZDelta> = input_rows
            .into_iter()
            .filter(|r| r.begin_snapshot > last && r.begin_snapshot <= up_to_snapshot)
            .filter_map(|r| {
                serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(&r.payload)
                    .ok()
                    .map(|fields| ZDelta {
                        fields: fields.into_iter().collect(),
                        weight: 1,
                    })
            })
            .filter(|delta| {
                // Shard-key routing: only process rows that belong to this shard.
                // If shard_count == 1 or no shard key, all rows belong to shard 0.
                if shard_count <= 1 || effective_shard_key.is_empty() {
                    return true;
                }
                if let Some(val) = delta.fields.get(&effective_shard_key) {
                    let key_str = val.to_string();
                    shard_index_for(&key_str, &key_ranges) == shard_id as usize
                } else {
                    // Row does not have the shard key column; broadcast to shard 0.
                    shard_id == 0
                }
            })
            .collect();

        let rows_processed = new_rows.len();
        trace.circuit.push_batch(&new_rows);

        // Write output.
        let output_rows = trace.circuit.read_output();
        let mut out_writer = self.store.begin_write();

        // In PerShard mode, each shard publishes independently.
        // In Consistent mode (default), the caller (tick loop) merges
        // all shards before committing — here we just stage the rows.
        let _ = output_mode; // consumed by the tick coordinator in consistent mode
        crate::output::write_output_rows(&mut out_writer, output_table_id, shard_id, &output_rows)
            .await?;

        let new_seq = trace.seq + 1;
        out_writer
            .update_matview_checkpoint(
                matview_id,
                shard_id,
                new_seq,
                up_to_snapshot,
                up_to_snapshot,
                up_to_snapshot,
                &self.config.worker_id,
            )
            .await
            .map_err(|e| IvmError::Catalog(e.to_string()))?;

        let output_snapshot = out_writer
            .create_snapshot(Some(&self.config.worker_id), Some("ivm checkpoint"))
            .await
            .map_err(|e| IvmError::Catalog(e.to_string()))?
            .as_u64();

        self.store.commit_writer(&out_writer);

        trace.advance_checkpoint(up_to_snapshot, output_snapshot);
        observability::emit_checkpoint(matview_id, shard_id, new_seq, output_snapshot);

        let lag = self
            .store
            .read_latest()
            .matview_lag_ms(matview_id, shard_id, now_ms())
            .await
            .map_err(|e| IvmError::Catalog(e.to_string()))?;
        observability::emit_tick(matview_id, shard_id, rows_processed, lag);

        Ok(())
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
