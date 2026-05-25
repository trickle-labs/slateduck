//! Visibility GC: advances the `retain-from` key without deleting bytes.
//!
//! `slateduck gc plan` — shows what snapshots would be excluded from queries.
//! `slateduck gc apply` — transactionally advances `retain-from`.
//!
//! Never deletes bytes. Only changes the query-visibility floor.

use slatedb::Db;
use slateduck_core::keys;
use slateduck_core::rows::SnapshotRow;
use slateduck_core::tags::*;
use slateduck_core::values;

use crate::error::{CatalogError, CatalogResult};

/// Result of a GC plan operation.
#[derive(Debug, Clone)]
pub struct GcPlan {
    /// Current retain-from value.
    pub current_retain_from: u64,
    /// Proposed new retain-from value.
    pub proposed_retain_from: u64,
    /// Number of snapshots that would become invisible.
    pub snapshots_affected: u64,
    /// Pinned snapshots that block advancement.
    pub pinned_snapshots: Vec<u64>,
}

/// Result of a GC apply operation.
#[derive(Debug, Clone)]
pub struct GcApplyResult {
    /// Previous retain-from value.
    pub previous_retain_from: u64,
    /// New retain-from value after apply.
    pub new_retain_from: u64,
    /// Number of snapshots now below the visibility floor.
    pub snapshots_hidden: u64,
}

/// Read the current `retain-from` value from the catalog.
pub async fn read_retain_from(db: &Db) -> CatalogResult<u64> {
    let key = keys::key_system(SYSTEM_RETAIN_FROM);
    match db.get(&key).await? {
        None => Ok(0), // infinite retention by default
        Some(data) => Ok(values::decode_counter(&data)?),
    }
}

/// Plan a GC operation: determine what would change if retain-from is advanced.
pub async fn gc_plan(db: &Db, retention_days: u64) -> CatalogResult<GcPlan> {
    let current_retain_from = read_retain_from(db).await?;

    // Find the latest snapshot
    let latest_snapshot_id = read_latest_snapshot_id(db).await?;

    // Calculate the proposed retain-from based on retention_days
    let proposed_retain_from = if retention_days == 0 {
        // No advancement (infinite retention)
        current_retain_from
    } else {
        // Find the snapshot that is `retention_days` old
        let cutoff_time = chrono::Utc::now() - chrono::Duration::days(retention_days as i64);
        find_snapshot_before_time(db, &cutoff_time.to_rfc3339(), latest_snapshot_id).await?
    };

    // Count affected snapshots
    let snapshots_affected = proposed_retain_from.saturating_sub(current_retain_from);

    // Check for pinned snapshots
    let pinned_snapshots = read_pinned_snapshots(db).await?;

    // Adjust proposed_retain_from to respect pins
    let effective_proposed = if let Some(&min_pin) = pinned_snapshots.iter().min() {
        if proposed_retain_from >= min_pin {
            min_pin.saturating_sub(1)
        } else {
            proposed_retain_from
        }
    } else {
        proposed_retain_from
    };

    Ok(GcPlan {
        current_retain_from,
        proposed_retain_from: effective_proposed,
        snapshots_affected,
        pinned_snapshots,
    })
}

/// Apply a GC plan: advance the retain-from key transactionally.
#[tracing::instrument(skip(db), fields(new_retain_from))]
pub async fn gc_apply(db: &Db, new_retain_from: u64) -> CatalogResult<GcApplyResult> {
    let current_retain_from = read_retain_from(db).await?;

    if new_retain_from <= current_retain_from {
        return Ok(GcApplyResult {
            previous_retain_from: current_retain_from,
            new_retain_from: current_retain_from,
            snapshots_hidden: 0,
        });
    }

    // Check pinned snapshots block advancement
    let pinned = read_pinned_snapshots(db).await?;
    if let Some(&min_pin) = pinned.iter().min() {
        if new_retain_from >= min_pin {
            return Err(CatalogError::PinnedSnapshotBlocks {
                pinned_snapshot: min_pin,
                requested_retain_from: new_retain_from,
            });
        }
    }

    // v0.18: Check snapshot leases block advancement
    if let Some(min_leased) = crate::lease::minimum_leased_snapshot(db).await? {
        if new_retain_from > min_leased {
            return Err(CatalogError::PinnedSnapshotBlocks {
                pinned_snapshot: min_leased,
                requested_retain_from: new_retain_from,
            });
        }
    }

    // Transactionally advance retain-from
    let key = keys::key_system(SYSTEM_RETAIN_FROM);
    db.put(&key, &values::encode_counter(new_retain_from))
        .await?;

    let snapshots_hidden = new_retain_from - current_retain_from;

    Ok(GcApplyResult {
        previous_retain_from: current_retain_from,
        new_retain_from,
        snapshots_hidden,
    })
}

/// Pin a snapshot to prevent GC from advancing past it.
pub async fn pin_snapshot(db: &Db, snapshot_id: u64) -> CatalogResult<()> {
    let key = key_pinned_snapshot(snapshot_id);
    db.put(&key, &values::encode_counter(snapshot_id)).await?;
    Ok(())
}

/// Unpin a snapshot.
pub async fn unpin_snapshot(db: &Db, snapshot_id: u64) -> CatalogResult<()> {
    let key = key_pinned_snapshot(snapshot_id);
    db.delete(&key).await?;
    Ok(())
}

/// Read all pinned snapshots.
pub async fn read_pinned_snapshots(db: &Db) -> CatalogResult<Vec<u64>> {
    let prefix = pinned_snapshot_prefix();
    let mut pinned = Vec::new();
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let id = values::decode_counter(&kv.value)?;
        pinned.push(id);
    }
    Ok(pinned)
}

// ─── Helpers ───────────────────────────────────────────────────────────────

fn key_pinned_snapshot(snapshot_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 7 + 8);
    buf.push(TAG_SYSTEM);
    buf.extend_from_slice(b"pinned:");
    buf.extend_from_slice(&snapshot_id.to_be_bytes());
    buf
}

fn pinned_snapshot_prefix() -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 7);
    buf.push(TAG_SYSTEM);
    buf.extend_from_slice(b"pinned:");
    buf
}

async fn read_latest_snapshot_id(db: &Db) -> CatalogResult<u64> {
    let key = keys::key_counter(COUNTER_NEXT_SNAPSHOT_ID);
    match db.get(&key).await? {
        None => Ok(0),
        Some(data) => {
            let next = values::decode_counter(&data)?;
            Ok(if next > 0 { next - 1 } else { 0 })
        }
    }
}

async fn find_snapshot_before_time(
    db: &Db,
    cutoff_time: &str,
    max_snapshot: u64,
) -> CatalogResult<u64> {
    // Scan snapshots to find the last one before the cutoff time
    let prefix = keys::prefix_for_tag(TAG_SNAPSHOT);
    let mut last_before_cutoff = 0u64;
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: SnapshotRow = values::decode_value(&kv.value)?;
        if row.snapshot_id <= max_snapshot
            && row.snapshot_time.as_str() < cutoff_time
            && row.snapshot_id > last_before_cutoff
        {
            last_before_cutoff = row.snapshot_id;
        }
    }
    Ok(last_before_cutoff)
}

/// Check if a snapshot is within the retention window.
pub fn is_within_retention(snapshot_id: u64, retain_from: u64) -> bool {
    retain_from == 0 || snapshot_id >= retain_from
}
