//! Observability stubs: metrics and tracing events for IVM workers.
//!
//! In v0.11 all metrics are emitted as `tracing` events.  A real Prometheus
//! integration is planned for v0.12.
//!
//! ## v0.17: Per-view rolling statistics
//! Tracks rows_in, rows_out, ms_spent, last_full_cost for adaptive cost mode.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Emit a tick event at the end of each IVM processing cycle.
pub fn emit_tick(matview_id: u64, shard_id: u32, rows_processed: usize, lag_ms: Option<u64>) {
    info!(
        matview_id,
        shard_id,
        rows_processed,
        lag_ms = lag_ms.unwrap_or(0),
        "ivm.tick"
    );
}

/// Emit a checkpoint event when a checkpoint is durably written.
pub fn emit_checkpoint(matview_id: u64, shard_id: u32, seq: u64, output_snapshot: u64) {
    info!(matview_id, shard_id, seq, output_snapshot, "ivm.checkpoint");
}

/// Emit a lease acquisition event.
pub fn emit_lease_acquired(matview_id: u64, shard_id: u32, generation: u64, expires_unix_ms: u64) {
    debug!(
        matview_id,
        shard_id, generation, expires_unix_ms, "ivm.lease_acquired"
    );
}

/// Emit a contention event when another worker holds the lease.
pub fn emit_lease_contended(matview_id: u64, shard_id: u32, current_owner: &str) {
    debug!(matview_id, shard_id, current_owner, "ivm.lease_contended");
}

/// Per-view rolling statistics for adaptive cost mode switching.
///
/// Tracked in the state store and surfaced via observability metrics.
/// Used by `CostMode::Adaptive` to decide DIFFERENTIAL→FULL switching.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ViewRollingStats {
    /// Total input rows processed.
    pub rows_in: u64,
    /// Total output rows emitted.
    pub rows_out: u64,
    /// Total milliseconds spent in IVM computation.
    pub ms_spent: u64,
    /// Last FULL refresh cost in milliseconds.
    pub last_full_cost_ms: u64,
    /// Exponentially-smoothed delta ratio (α=0.3).
    pub smoothed_delta_ratio: f64,
    /// Number of ticks recorded.
    pub tick_count: u64,
}

impl ViewRollingStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a tick's statistics.
    pub fn record_tick(&mut self, delta_rows: u64, total_rows: u64, ms_elapsed: u64) {
        self.rows_in += delta_rows;
        self.ms_spent += ms_elapsed;
        self.tick_count += 1;

        if total_rows > 0 {
            let current_ratio = delta_rows as f64 / total_rows as f64;
            const ALPHA: f64 = 0.3;
            self.smoothed_delta_ratio =
                ALPHA * current_ratio + (1.0 - ALPHA) * self.smoothed_delta_ratio;
        }
    }

    /// Record a FULL refresh cost.
    pub fn record_full_refresh(&mut self, cost_ms: u64) {
        self.last_full_cost_ms = cost_ms;
    }

    /// Record output rows emitted.
    pub fn record_output(&mut self, rows_out: u64) {
        self.rows_out += rows_out;
    }
}

/// Store for per-view rolling statistics (keyed by matview_id).
#[derive(Debug, Clone, Default)]
pub struct ViewStatsStore {
    pub stats: HashMap<u64, ViewRollingStats>,
}

impl ViewStatsStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or create stats for a view.
    pub fn get_or_create(&mut self, matview_id: u64) -> &mut ViewRollingStats {
        self.stats.entry(matview_id).or_default()
    }

    /// Get stats for a view (read-only).
    pub fn get(&self, matview_id: u64) -> Option<&ViewRollingStats> {
        self.stats.get(&matview_id)
    }
}

/// Emit adaptive cost mode switching event.
pub fn emit_adaptive_switch(
    matview_id: u64,
    from_mode: &str,
    to_mode: &str,
    delta_ratio: f64,
    multiplier: f64,
) {
    info!(
        matview_id,
        from_mode, to_mode, delta_ratio, multiplier, "ivm.adaptive_switch"
    );
}
