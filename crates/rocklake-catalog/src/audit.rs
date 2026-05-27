//! Audit log: structured entries for every snapshot commit.
//!
//! Records who committed, when, and what changed in each snapshot.
//! Stored under `0xFF | "audit"` prefix for accumulation without overwriting.

use slatedb::Db;
use rocklake_core::keys;
use rocklake_core::values;

use crate::error::{CatalogError, CatalogResult};

/// An audit log entry for a snapshot commit.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditEntry {
    /// The snapshot ID committed.
    pub snapshot_id: u64,
    /// Timestamp of the commit (RFC 3339).
    pub committed_at: String,
    /// Who performed the commit (author or system).
    pub committed_by: String,
    /// Summary of what changed.
    pub changes: Vec<AuditChange>,
}

/// A single change recorded in the audit log.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditChange {
    /// Type of change: "create_schema", "create_table", "register_data_file", etc.
    pub change_type: String,
    /// Optional detail (e.g., table name, file path).
    pub detail: Option<String>,
}

/// Write an audit log entry for a snapshot commit.
pub async fn write_audit_entry(db: &Db, entry: &AuditEntry) -> CatalogResult<()> {
    let key = keys::key_audit(entry.snapshot_id);
    let value = serde_json::to_vec(entry)
        .map_err(|e| CatalogError::Internal(format!("audit entry serialize: {e}")))?;
    let encoded = values::encode_raw_value(&value);
    db.put(&key, &encoded).await?;
    Ok(())
}

/// Read all audit log entries.
pub async fn list_audit_entries(db: &Db) -> CatalogResult<Vec<AuditEntry>> {
    let prefix = keys::audit_prefix();
    let mut entries = Vec::new();

    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let raw = values::decode_raw_value(&kv.value)?;
        if let Ok(entry) = serde_json::from_slice::<AuditEntry>(&raw) {
            entries.push(entry);
        }
    }

    Ok(entries)
}

/// Read an audit log entry for a specific snapshot.
pub async fn get_audit_entry(db: &Db, snapshot_id: u64) -> CatalogResult<Option<AuditEntry>> {
    let key = keys::key_audit(snapshot_id);
    match db.get(&key).await? {
        Some(value) => {
            let raw = values::decode_raw_value(&value)?;
            let entry = serde_json::from_slice::<AuditEntry>(&raw)
                .map_err(|e| CatalogError::Internal(format!("audit entry deserialize: {e}")))?;
            Ok(Some(entry))
        }
        None => Ok(None),
    }
}
