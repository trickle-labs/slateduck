//! Checkpoint management: create, list, and restore catalog checkpoints.
//!
//! Thin wrapper around SlateDB's checkpoint functionality.

#![allow(missing_docs)]

use rocklake_core::keys;
use rocklake_core::rows::*;
use rocklake_core::tags::*;
use rocklake_core::values;
use slatedb::Db;

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

/// Restore catalog to a checkpoint by hiding post-checkpoint facts and advancing
/// the snapshot counter so new writes cannot reuse historical snapshot IDs.
pub async fn restore_checkpoint(db: &Db, checkpoint_id: u64) -> CatalogResult<CheckpointInfo> {
    // Find the checkpoint
    let key = checkpoint_key(checkpoint_id);
    let data = db
        .get(&key)
        .await?
        .ok_or_else(|| CatalogError::NotFound(format!("checkpoint {checkpoint_id}")))?;

    let meta: CheckpointMetadata = values::decode_value(&data)?;

    // Read the current next_snapshot_id. This is the "hide snapshot": facts created
    // after the checkpoint will have their end_snapshot set to this value, hiding
    // them from reads at or after hide_snapshot.
    let counter_key = keys::key_counter(COUNTER_NEXT_SNAPSHOT_ID);
    let hide_snapshot = match db.get(&counter_key).await? {
        Some(data) => values::decode_counter(&data)?,
        None => meta.snapshot_id + 1,
    };

    if hide_snapshot > meta.snapshot_id + 1 {
        // Post-checkpoint facts exist: mark them hidden and advance the counter
        // past hide_snapshot so it cannot be reused as a live snapshot ID.
        hide_post_checkpoint_facts(db, meta.snapshot_id, hide_snapshot).await?;
        db.put(&counter_key, &values::encode_counter(hide_snapshot + 1))
            .await?;
    }
    // When hide_snapshot == meta.snapshot_id + 1 no facts were written after
    // the checkpoint, so hide_snapshot is already the correct next-snapshot-id.
    // Skip the +1 advance: the counter is already at the right value.

    Ok(CheckpointInfo {
        id: meta.id,
        created_at: meta.created_at,
        snapshot_id: meta.snapshot_id,
        label: meta.label,
    })
}

/// Scan all versioned rows and set `end_snapshot = hide_snapshot` for any row
/// whose `begin_snapshot > checkpoint_snapshot_id`.  This prevents those facts
/// from appearing in reads at snapshot IDs >= hide_snapshot, while keeping them
/// readable via their original historical snapshot IDs.
async fn hide_post_checkpoint_facts(
    db: &Db,
    checkpoint_snapshot_id: u64,
    hide_snapshot: u64,
) -> CatalogResult<()> {
    // Schema rows
    {
        let prefix = keys::prefix_for_tag(TAG_SCHEMA);
        let mut iter = db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let mut row: SchemaRow = values::decode_value(&kv.value)?;
            if row.begin_snapshot > checkpoint_snapshot_id
                && row.end_snapshot.is_none_or(|e| e > checkpoint_snapshot_id)
            {
                row.end_snapshot = Some(hide_snapshot);
                db.put(&kv.key, &values::encode_value(&row)).await?;
            }
        }
    }

    // Table rows
    {
        let prefix = keys::prefix_for_tag(TAG_TABLE);
        let mut iter = db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let mut row: TableRow = values::decode_value(&kv.value)?;
            if row.begin_snapshot > checkpoint_snapshot_id
                && row.end_snapshot.is_none_or(|e| e > checkpoint_snapshot_id)
            {
                row.end_snapshot = Some(hide_snapshot);
                db.put(&kv.key, &values::encode_value(&row)).await?;
            }
        }
    }

    // Column rows
    {
        let prefix = keys::prefix_for_tag(TAG_COLUMN);
        let mut iter = db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let mut row: ColumnRow = values::decode_value(&kv.value)?;
            if row.begin_snapshot > checkpoint_snapshot_id
                && row.end_snapshot.is_none_or(|e| e > checkpoint_snapshot_id)
            {
                row.end_snapshot = Some(hide_snapshot);
                db.put(&kv.key, &values::encode_value(&row)).await?;
            }
        }
    }

    // View rows
    {
        let prefix = keys::prefix_for_tag(TAG_VIEW);
        let mut iter = db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let mut row: ViewRow = values::decode_value(&kv.value)?;
            if row.begin_snapshot > checkpoint_snapshot_id
                && row.end_snapshot.is_none_or(|e| e > checkpoint_snapshot_id)
            {
                row.end_snapshot = Some(hide_snapshot);
                db.put(&kv.key, &values::encode_value(&row)).await?;
            }
        }
    }

    // Macro rows
    {
        let prefix = keys::prefix_for_tag(TAG_MACRO);
        let mut iter = db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let mut row: MacroRow = values::decode_value(&kv.value)?;
            if row.begin_snapshot > checkpoint_snapshot_id
                && row.end_snapshot.is_none_or(|e| e > checkpoint_snapshot_id)
            {
                row.end_snapshot = Some(hide_snapshot);
                db.put(&kv.key, &values::encode_value(&row)).await?;
            }
        }
    }

    // Inlined insert rows
    {
        let prefix = vec![TAG_INLINED_ROWS, INLINED_SUBTYPE_INSERT];
        let mut iter = db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let mut row: InlinedInsertRow = values::decode_value(&kv.value)?;
            if row.begin_snapshot > checkpoint_snapshot_id
                && row.end_snapshot.is_none_or(|e| e > checkpoint_snapshot_id)
            {
                row.end_snapshot = Some(hide_snapshot);
                db.put(&kv.key, &values::encode_value(&row)).await?;
            }
        }
    }

    Ok(())
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
