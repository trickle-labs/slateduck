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
        .unwrap_or_default()
        .as_millis() as u64;
    // Guard: if the wall-clock millis would collide with an existing key
    // (e.g. two checkpoints created in the same millisecond under automation),
    // advance the counter past the collision.  We always use
    // `COUNTER_NEXT_CHECKPOINT_ID` as the authoritative ID source so checkpoint
    // IDs are unique and strictly monotonic regardless of clock resolution.
    let counter_key = keys::key_counter(COUNTER_NEXT_CHECKPOINT_ID);
    let next_counter = match db.get(&counter_key).await? {
        Some(data) => values::decode_counter(&data)?,
        None => 0,
    };
    // Use the larger of the wall-clock millis and the persisted counter so the
    // ID space is roughly time-ordered for human readability while guaranteeing
    // no collision.
    let id = id.max(next_counter);
    // Persist counter = id + 1 so the next checkpoint always gets id + 1 at minimum.
    db.put(&counter_key, &values::encode_counter(id + 1))
        .await?;

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

// ─── Checkpoint Pin API ────────────────────────────────────────────────────

/// Information about a named checkpoint pin.
#[derive(Debug, Clone)]
pub struct CheckpointPin {
    /// User-assigned name for this pin.
    pub name: String,
    /// The `dl_snapshot_id` this pin is anchored to.
    pub snapshot_id: u64,
    /// RFC-3339 creation timestamp.
    pub created_at: String,
}

/// Pin a named checkpoint at a specific `dl_snapshot_id`.
///
/// The pin is stored under `TAG_SYSTEM | "checkpoint-pin:" | name`.
/// It survives process restart and prevents GC from advancing past the
/// pinned snapshot.
pub async fn pin_checkpoint(db: &Db, name: &str, snapshot_id: u64) -> CatalogResult<CheckpointPin> {
    let created_at = chrono::Utc::now().to_rfc3339();
    let key = checkpoint_pin_key(name);

    // Re-use CheckpointMetadata with id=0 (not used for pins) and label=name.
    let meta = CheckpointMetadata {
        id: 0,
        created_at: created_at.clone(),
        snapshot_id,
        label: Some(name.to_string()),
    };

    db.put(&key, &values::encode_value(&meta)).await?;

    Ok(CheckpointPin {
        name: name.to_string(),
        snapshot_id,
        created_at,
    })
}

/// Remove a named checkpoint pin.
///
/// Returns `CatalogError::NotFound` if no pin with the given name exists.
pub async fn unpin_checkpoint(db: &Db, name: &str) -> CatalogResult<()> {
    let key = checkpoint_pin_key(name);
    // Verify pin exists before deleting.
    db.get(&key)
        .await?
        .ok_or_else(|| CatalogError::NotFound(format!("checkpoint pin '{name}'")))?;
    db.delete(&key).await?;
    Ok(())
}

/// List all named checkpoint pins.
pub async fn list_checkpoint_pins(db: &Db) -> CatalogResult<Vec<CheckpointPin>> {
    let prefix = checkpoint_pin_prefix();
    let mut pins = Vec::new();
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let meta: CheckpointMetadata = values::decode_value(&kv.value)?;
        // Extract the name from the key suffix (skip prefix bytes).
        let prefix_len = prefix.len();
        let name = String::from_utf8_lossy(&kv.key[prefix_len..]).to_string();
        pins.push(CheckpointPin {
            name,
            snapshot_id: meta.snapshot_id,
            created_at: meta.created_at,
        });
    }
    pins.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(pins)
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

fn checkpoint_pin_prefix() -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 16);
    buf.push(TAG_SYSTEM);
    buf.extend_from_slice(b"checkpoint-pin:");
    buf
}

fn checkpoint_pin_key(name: &str) -> Vec<u8> {
    let mut buf = checkpoint_pin_prefix();
    buf.extend_from_slice(name.as_bytes());
    buf
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    use object_store::local::LocalFileSystem;
    use object_store::path::Path as ObjectPath;
    use std::sync::Arc;
    use tempfile::TempDir;

    async fn open_test_db(dir: &std::path::Path) -> slatedb::Db {
        let fs: Arc<dyn object_store::ObjectStore> =
            Arc::new(LocalFileSystem::new_with_prefix(dir).unwrap());
        slatedb::Db::open(ObjectPath::from("catalog"), fs)
            .await
            .unwrap()
    }

    /// Two checkpoints created in rapid succession must have distinct,
    /// monotonically increasing IDs even if the wall clock has not advanced.
    #[tokio::test]
    async fn two_rapid_checkpoints_have_distinct_ids() {
        let dir = TempDir::new().unwrap();
        let db = open_test_db(dir.path()).await;

        let c1 = create_checkpoint(&db, None).await.unwrap();
        let c2 = create_checkpoint(&db, None).await.unwrap();

        assert_ne!(
            c1.id, c2.id,
            "consecutive checkpoints must have distinct IDs"
        );
        assert!(c2.id > c1.id, "checkpoint IDs must be strictly increasing");

        db.close().await.unwrap();
    }

    /// Pin and list checkpoint pins.
    #[tokio::test]
    async fn pin_and_list_checkpoint_pins() {
        let dir = TempDir::new().unwrap();
        let db = open_test_db(dir.path()).await;

        pin_checkpoint(&db, "alpha", 10).await.unwrap();
        pin_checkpoint(&db, "beta", 20).await.unwrap();

        let pins = list_checkpoint_pins(&db).await.unwrap();
        assert_eq!(pins.len(), 2);
        assert_eq!(pins[0].name, "alpha");
        assert_eq!(pins[0].snapshot_id, 10);
        assert_eq!(pins[1].name, "beta");
        assert_eq!(pins[1].snapshot_id, 20);

        db.close().await.unwrap();
    }

    /// Unpin removes the named pin and returns NotFound on re-unpin.
    #[tokio::test]
    async fn unpin_removes_pin() {
        let dir = TempDir::new().unwrap();
        let db = open_test_db(dir.path()).await;

        pin_checkpoint(&db, "gamma", 5).await.unwrap();
        unpin_checkpoint(&db, "gamma").await.unwrap();

        let pins = list_checkpoint_pins(&db).await.unwrap();
        assert!(pins.is_empty(), "no pins should remain after unpin");

        // Second unpin should return NotFound.
        let err = unpin_checkpoint(&db, "gamma").await.unwrap_err();
        assert!(
            matches!(err, CatalogError::NotFound(_)),
            "expected NotFound, got {err:?}"
        );

        db.close().await.unwrap();
    }
}
