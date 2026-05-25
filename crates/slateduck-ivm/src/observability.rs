//! Observability stubs: metrics and tracing events for IVM workers.
//!
//! In v0.11 all metrics are emitted as `tracing` events.  A real Prometheus
//! integration is planned for v0.12.

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
