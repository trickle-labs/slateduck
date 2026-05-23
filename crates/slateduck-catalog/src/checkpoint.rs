//! Checkpoint management: create, list, and restore catalog checkpoints.
//!
//! Thin wrapper around SlateDB's checkpoint functionality.

use slatedb::Db;
use slateduck_core::keys;
use slateduck_core::tags::*;
use slateduck_core::values;

use crate::error::{CatalogError, CatalogResult};

/// Information about a checkpoint.
#[derive(Debug, Clone)]
pub struct CheckpointInfo {
    /// Checkpoint ID (timestamp-based).
    pub id: u64,
    /// When the checkpoint was created.
    pub created_at: String,
    /// Snapshot ID at checkpoint time.
    pub snapshot_id: u64,
    /// Human-readable label.
    pub label: Option<String>,
}

/// Checkpoint metadata stored under system keys.
#[derive(Clone, PartialEq, prost::Message)]
pub struct CheckpointMetadata {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(string, tag = "2")]
    pub created_at: String,
    #[prost(uint64, tag = "3")]
    pub snapshot_id: u64,
    #[prost(string, optional, tag = "4")]
    pub label: Option<String>,
}

/// Create a new checkpoint of the current catalog state.
pub async fn create_checkpoint(db: &Db, label: Option<&str>) -> CatalogResult<CheckpointInfo> {
    // Get current snapshot ID
    let snapshot_key = keys::key_counter(COUNTER_NEXT_SNAPSHOT_ID);
    let current_snapshot = match db.get(&snapshot_key).await? {
        Some(data) => {
            let next = values::decode_counter(&data)?;
            if next > 0 {
                next - 1
            } else {
                0
            }
        }
        None => 0,
    };

    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    let created_at = chrono::Utc::now().to_rfc3339();

    let meta = CheckpointMetadata {
        id,
        created_at: created_at.clone(),
        snapshot_id: current_snapshot,
        label: label.map(|s| s.to_string()),
    };

    // Store checkpoint metadata
    let key = checkpoint_key(id);
    db.put(&key, &values::encode_value(&meta)).await?;

    Ok(CheckpointInfo {
        id,
        created_at,
        snapshot_id: current_snapshot,
        label: label.map(|s| s.to_string()),
    })
}

/// List all available checkpoints.
pub async fn list_checkpoints(db: &Db) -> CatalogResult<Vec<CheckpointInfo>> {
    let prefix = checkpoint_prefix();
    let mut checkpoints = Vec::new();
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let meta: CheckpointMetadata = values::decode_value(&kv.value)?;
        checkpoints.push(CheckpointInfo {
            id: meta.id,
            created_at: meta.created_at,
            snapshot_id: meta.snapshot_id,
            label: meta.label,
        });
    }
    checkpoints.sort_by_key(|c| c.id);
    Ok(checkpoints)
}

/// Restore catalog to a checkpoint by resetting the retain-from and snapshot counter.
/// Note: This is a logical restore — it makes the catalog read at the checkpoint's snapshot.
pub async fn restore_checkpoint(db: &Db, checkpoint_id: u64) -> CatalogResult<CheckpointInfo> {
    // Find the checkpoint
    let key = checkpoint_key(checkpoint_id);
    let data = db
        .get(&key)
        .await?
        .ok_or_else(|| CatalogError::NotFound(format!("checkpoint {checkpoint_id}")))?;

    let meta: CheckpointMetadata = values::decode_value(&data)?;

    // Reset the snapshot counter to restore point + 1
    // This effectively makes new writes continue from the restored snapshot
    let counter_key = keys::key_counter(COUNTER_NEXT_SNAPSHOT_ID);
    db.put(&counter_key, &values::encode_counter(meta.snapshot_id + 1))
        .await?;

    Ok(CheckpointInfo {
        id: meta.id,
        created_at: meta.created_at,
        snapshot_id: meta.snapshot_id,
        label: meta.label,
    })
}

// ─── Helpers ───────────────────────────────────────────────────────────────

fn checkpoint_prefix() -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 11);
    buf.push(TAG_SYSTEM);
    buf.extend_from_slice(b"checkpoint:");
    buf
}

fn checkpoint_key(id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 11 + 8);
    buf.push(TAG_SYSTEM);
    buf.extend_from_slice(b"checkpoint:");
    buf.extend_from_slice(&id.to_be_bytes());
    buf
}
