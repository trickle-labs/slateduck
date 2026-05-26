//! Excision: physically deletes catalog facts and Parquet files older than a floor.
//!
//! Invoked only via `slateduck excise plan` / `slateduck excise apply --before <snapshot>`.
//! Always requires explicit operator invocation; never runs in the background.
//! Records an audit entry under `0xFF | "excised"` so the audit trail accumulates.
//!
//! On per-key deletion failure: log and skip; do not retry aggressively.

#![allow(missing_docs)]

use slatedb::Db;
use slateduck_core::keys;
use slateduck_core::rows::*;
use slateduck_core::tags::*;
use slateduck_core::values;

use crate::error::{CatalogError, CatalogResult};
use crate::gc;

/// Result of an excision plan operation.
#[derive(Debug, Clone)]
pub struct ExcisePlan {
    /// Snapshot floor: everything below this is eligible for excision.
    pub before_snapshot: u64,
    /// Number of version rows eligible for physical deletion.
    pub version_rows_eligible: u64,
    /// Number of inlined insert rows eligible.
    pub inlined_inserts_eligible: u64,
    /// Number of inlined delete markers eligible.
    pub inlined_deletes_eligible: u64,
    /// Data file paths eligible for deletion.
    pub data_files_eligible: Vec<String>,
    /// Whether the operation is safe (retain-from >= before_snapshot).
    pub is_safe: bool,
}

/// Result of an excision apply operation.
#[derive(Debug, Clone)]
pub struct ExciseResult {
    /// Number of catalog keys physically deleted.
    pub keys_deleted: u64,
    /// Number of keys that failed to delete (logged and skipped).
    pub keys_failed: u64,
    /// Audit entry ID.
    pub audit_entry_id: u64,
}

/// Audit entry for excision events.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ExciseAuditEntry {
    #[prost(uint64, tag = "1")]
    pub timestamp_millis: u64,
    #[prost(uint64, tag = "2")]
    pub before_snapshot: u64,
    #[prost(uint64, tag = "3")]
    pub keys_deleted: u64,
    #[prost(uint64, tag = "4")]
    pub keys_failed: u64,
    #[prost(string, tag = "5")]
    pub operator: String,
}

/// Plan an excision operation without executing it.
pub async fn excise_plan(db: &Db, before_snapshot: u64) -> CatalogResult<ExcisePlan> {
    let retain_from = gc::read_retain_from(db).await?;

    // Safety check: retain-from must be set and >= before_snapshot
    let is_safe = retain_from > 0 && retain_from >= before_snapshot;

    let mut version_rows_eligible = 0u64;

    // Scan versioned tables for rows with end_snapshot <= before_snapshot
    version_rows_eligible += count_excisable_schemas(db, before_snapshot).await?;
    version_rows_eligible += count_excisable_tables(db, before_snapshot).await?;
    version_rows_eligible += count_excisable_columns(db, before_snapshot).await?;

    // Scan inlined inserts
    let inlined_inserts_eligible = count_excisable_inlined_inserts(db, before_snapshot).await?;

    // Scan inlined deletes
    let inlined_deletes_eligible = count_excisable_inlined_deletes(db, before_snapshot).await?;

    // Check data files not referenced by any retained snapshot
    let data_files_eligible = find_excisable_data_files(db, before_snapshot).await?;

    Ok(ExcisePlan {
        before_snapshot,
        version_rows_eligible,
        inlined_inserts_eligible,
        inlined_deletes_eligible,
        data_files_eligible,
        is_safe,
    })
}

/// Apply an excision: physically delete eligible keys and record audit entry.
#[tracing::instrument(skip(db), fields(before_snapshot, operator))]
pub async fn excise_apply(
    db: &Db,
    before_snapshot: u64,
    operator: &str,
) -> CatalogResult<ExciseResult> {
    let retain_from = gc::read_retain_from(db).await?;

    // Safety: refuse if retain-from is unset or hasn't been advanced past the excision point
    if retain_from == 0 || retain_from < before_snapshot {
        return Err(CatalogError::ExcisionUnsafe {
            retain_from,
            before_snapshot,
        });
    }

    let mut keys_deleted = 0u64;
    let mut keys_failed = 0u64;

    // Delete excisable schema versions
    let (d, f) = delete_excisable_schemas(db, before_snapshot).await?;
    keys_deleted += d;
    keys_failed += f;

    // Delete excisable table versions
    let (d, f) = delete_excisable_tables(db, before_snapshot).await?;
    keys_deleted += d;
    keys_failed += f;

    // Delete excisable column versions
    let (d, f) = delete_excisable_columns(db, before_snapshot).await?;
    keys_deleted += d;
    keys_failed += f;

    // Delete excisable inlined inserts
    let (d, f) = delete_excisable_inlined_inserts(db, before_snapshot).await?;
    keys_deleted += d;
    keys_failed += f;

    // Delete excisable inlined deletes
    let (d, f) = delete_excisable_inlined_deletes(db, before_snapshot).await?;
    keys_deleted += d;
    keys_failed += f;

    // Delete old snapshots themselves
    let (d, f) = delete_old_snapshots(db, before_snapshot).await?;
    keys_deleted += d;
    keys_failed += f;

    // Record audit entry
    let audit_entry_id =
        record_audit_entry(db, before_snapshot, keys_deleted, keys_failed, operator).await?;

    Ok(ExciseResult {
        keys_deleted,
        keys_failed,
        audit_entry_id,
    })
}

/// Read all excision audit entries.
pub async fn read_audit_entries(db: &Db) -> CatalogResult<Vec<ExciseAuditEntry>> {
    let prefix = excise_audit_prefix();
    let mut entries = Vec::new();
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let entry: ExciseAuditEntry = values::decode_value(&kv.value)?;
        entries.push(entry);
    }
    Ok(entries)
}

// ─── Internal helpers ──────────────────────────────────────────────────────

fn excise_audit_prefix() -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + SYSTEM_EXCISED_PREFIX.len());
    buf.push(TAG_SYSTEM);
    buf.extend_from_slice(SYSTEM_EXCISED_PREFIX);
    buf
}

fn excise_audit_key(timestamp_millis: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + SYSTEM_EXCISED_PREFIX.len() + 1 + 8);
    buf.push(TAG_SYSTEM);
    buf.extend_from_slice(SYSTEM_EXCISED_PREFIX);
    buf.push(b':');
    buf.extend_from_slice(&timestamp_millis.to_be_bytes());
    buf
}

async fn record_audit_entry(
    db: &Db,
    before_snapshot: u64,
    keys_deleted: u64,
    keys_failed: u64,
    operator: &str,
) -> CatalogResult<u64> {
    let timestamp_millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    let entry = ExciseAuditEntry {
        timestamp_millis,
        before_snapshot,
        keys_deleted,
        keys_failed,
        operator: operator.to_string(),
    };

    let key = excise_audit_key(timestamp_millis);
    db.put(&key, &values::encode_value(&entry)).await?;
    Ok(timestamp_millis)
}

async fn count_excisable_schemas(db: &Db, before_snapshot: u64) -> CatalogResult<u64> {
    let prefix = keys::prefix_for_tag(TAG_SCHEMA);
    let mut count = 0u64;
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: SchemaRow = values::decode_value(&kv.value)?;
        if let Some(end) = row.end_snapshot {
            if end <= before_snapshot {
                count += 1;
            }
        }
    }
    Ok(count)
}

async fn count_excisable_tables(db: &Db, before_snapshot: u64) -> CatalogResult<u64> {
    let prefix = keys::prefix_for_tag(TAG_TABLE);
    let mut count = 0u64;
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: TableRow = values::decode_value(&kv.value)?;
        if let Some(end) = row.end_snapshot {
            if end <= before_snapshot {
                count += 1;
            }
        }
    }
    Ok(count)
}

async fn count_excisable_columns(db: &Db, before_snapshot: u64) -> CatalogResult<u64> {
    let prefix = keys::prefix_for_tag(TAG_COLUMN);
    let mut count = 0u64;
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: ColumnRow = values::decode_value(&kv.value)?;
        if let Some(end) = row.end_snapshot {
            if end <= before_snapshot {
                count += 1;
            }
        }
    }
    Ok(count)
}

async fn count_excisable_inlined_inserts(db: &Db, before_snapshot: u64) -> CatalogResult<u64> {
    let buf = vec![TAG_INLINED_ROWS, INLINED_SUBTYPE_INSERT];
    let mut count = 0u64;
    let mut iter = db.scan_prefix(&buf).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: InlinedInsertRow = values::decode_value(&kv.value)?;
        if let Some(end) = row.end_snapshot {
            if end <= before_snapshot {
                count += 1;
            }
        }
    }
    Ok(count)
}

async fn count_excisable_inlined_deletes(db: &Db, before_snapshot: u64) -> CatalogResult<u64> {
    let buf = vec![TAG_INLINED_ROWS, INLINED_SUBTYPE_DELETE];
    let mut count = 0u64;
    let mut iter = db.scan_prefix(&buf).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: InlinedDeleteRow = values::decode_value(&kv.value)?;
        if row.begin_snapshot < before_snapshot {
            count += 1;
        }
    }
    Ok(count)
}

async fn find_excisable_data_files(db: &Db, before_snapshot: u64) -> CatalogResult<Vec<String>> {
    let prefix = keys::prefix_for_tag(TAG_DATA_FILE);
    let mut paths = Vec::new();
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: DataFileRow = values::decode_value(&kv.value)?;
        if row.begin_snapshot.unwrap_or(0) < before_snapshot {
            paths.push(row.path);
        }
    }
    Ok(paths)
}

async fn delete_excisable_schemas(db: &Db, before_snapshot: u64) -> CatalogResult<(u64, u64)> {
    let prefix = keys::prefix_for_tag(TAG_SCHEMA);
    let mut deleted = 0u64;
    let mut failed = 0u64;
    let mut to_delete = Vec::new();

    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: SchemaRow = values::decode_value(&kv.value)?;
        if let Some(end) = row.end_snapshot {
            if end <= before_snapshot {
                to_delete.push(kv.key.to_vec());
            }
        }
    }

    for key in to_delete {
        match db.delete(&key).await {
            Ok(_) => deleted += 1,
            Err(e) => {
                tracing::warn!("Failed to delete schema key: {e}");
                failed += 1;
            }
        }
    }
    Ok((deleted, failed))
}

async fn delete_excisable_tables(db: &Db, before_snapshot: u64) -> CatalogResult<(u64, u64)> {
    let prefix = keys::prefix_for_tag(TAG_TABLE);
    let mut deleted = 0u64;
    let mut failed = 0u64;
    let mut to_delete = Vec::new();

    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: TableRow = values::decode_value(&kv.value)?;
        if let Some(end) = row.end_snapshot {
            if end <= before_snapshot {
                to_delete.push(kv.key.to_vec());
            }
        }
    }

    for key in to_delete {
        match db.delete(&key).await {
            Ok(_) => deleted += 1,
            Err(e) => {
                tracing::warn!("Failed to delete table key: {e}");
                failed += 1;
            }
        }
    }
    Ok((deleted, failed))
}

async fn delete_excisable_columns(db: &Db, before_snapshot: u64) -> CatalogResult<(u64, u64)> {
    let prefix = keys::prefix_for_tag(TAG_COLUMN);
    let mut deleted = 0u64;
    let mut failed = 0u64;
    let mut to_delete = Vec::new();

    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: ColumnRow = values::decode_value(&kv.value)?;
        if let Some(end) = row.end_snapshot {
            if end <= before_snapshot {
                to_delete.push(kv.key.to_vec());
            }
        }
    }

    for key in to_delete {
        match db.delete(&key).await {
            Ok(_) => deleted += 1,
            Err(e) => {
                tracing::warn!("Failed to delete column key: {e}");
                failed += 1;
            }
        }
    }
    Ok((deleted, failed))
}

async fn delete_excisable_inlined_inserts(
    db: &Db,
    before_snapshot: u64,
) -> CatalogResult<(u64, u64)> {
    let prefix = vec![TAG_INLINED_ROWS, INLINED_SUBTYPE_INSERT];
    let mut deleted = 0u64;
    let mut failed = 0u64;
    let mut to_delete = Vec::new();

    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: InlinedInsertRow = values::decode_value(&kv.value)?;
        if let Some(end) = row.end_snapshot {
            if end <= before_snapshot {
                to_delete.push(kv.key.to_vec());
            }
        }
    }

    for key in to_delete {
        match db.delete(&key).await {
            Ok(_) => deleted += 1,
            Err(e) => {
                tracing::warn!("Failed to delete inlined insert key: {e}");
                failed += 1;
            }
        }
    }
    Ok((deleted, failed))
}

async fn delete_excisable_inlined_deletes(
    db: &Db,
    before_snapshot: u64,
) -> CatalogResult<(u64, u64)> {
    let prefix = vec![TAG_INLINED_ROWS, INLINED_SUBTYPE_DELETE];
    let mut deleted = 0u64;
    let mut failed = 0u64;
    let mut to_delete = Vec::new();

    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: InlinedDeleteRow = values::decode_value(&kv.value)?;
        if row.begin_snapshot < before_snapshot {
            to_delete.push(kv.key.to_vec());
        }
    }

    for key in to_delete {
        match db.delete(&key).await {
            Ok(_) => deleted += 1,
            Err(e) => {
                tracing::warn!("Failed to delete inlined delete key: {e}");
                failed += 1;
            }
        }
    }
    Ok((deleted, failed))
}

async fn delete_old_snapshots(db: &Db, before_snapshot: u64) -> CatalogResult<(u64, u64)> {
    let prefix = keys::prefix_for_tag(TAG_SNAPSHOT);
    let mut deleted = 0u64;
    let mut failed = 0u64;
    let mut to_delete = Vec::new();

    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: SnapshotRow = values::decode_value(&kv.value)?;
        if row.snapshot_id < before_snapshot {
            to_delete.push(kv.key.to_vec());
        }
    }

    for key in to_delete {
        match db.delete(&key).await {
            Ok(_) => deleted += 1,
            Err(e) => {
                tracing::warn!("Failed to delete snapshot key: {e}");
                failed += 1;
            }
        }
    }
    Ok((deleted, failed))
}
