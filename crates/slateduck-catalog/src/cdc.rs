//! CDC (Change Data Capture) output: snapshot diffs to S3, Kafka, NATS, or webhook.
//!
//! # Overview
//!
//! When a DuckLake snapshot is committed, the diff between the previous and
//! current snapshot is a natural change stream.  This module provides:
//!
//! - **S3 CDC files**: per-snapshot JSON-lines diff files under
//!   `{warehouse}/cdc/{table_id}/snapshot-{id}.jsonl`
//! - **Webhook CDC**: HTTP POST on each snapshot commit (structure only;
//!   actual HTTP is left to the calling application)
//! - **`slateduck-cdc` sidecar support**: a polling loop that tails the
//!   catalog and exports each new snapshot diff

use serde::{Deserialize, Serialize};
use slateduck_core::mvcc::SnapshotId;

use crate::error::CatalogResult;
use crate::reader::SnapshotDiff;

// ─── CDC Event Types ────────────────────────────────────────────────────────

/// The kind of change captured by a CDC event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CdcChangeKind {
    /// A new row was added at this snapshot.
    Add,
    /// A row was retired at this snapshot.
    Retire,
}

/// A single CDC event describing one changed catalog fact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CdcEvent {
    /// The snapshot at which this change occurred.
    pub snapshot_id: u64,
    /// The catalog table affected (e.g. `"ducklake_table"`, `"ducklake_data_file"`).
    pub table: String,
    /// Whether the fact was added or retired.
    pub kind: CdcChangeKind,
    /// The changed row serialised as JSON.
    pub row: serde_json::Value,
}

/// A complete CDC export for one snapshot transition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CdcSnapshot {
    pub from_snapshot: u64,
    pub to_snapshot: u64,
    pub events: Vec<CdcEvent>,
}

impl CdcSnapshot {
    /// Convert a `SnapshotDiff` to a `CdcSnapshot`.
    pub fn from_diff(diff: &SnapshotDiff) -> Self {
        let mut events: Vec<CdcEvent> = Vec::new();
        let to = diff.to_snapshot.as_u64();

        for row in &diff.added_schemas {
            events.push(CdcEvent {
                snapshot_id: to,
                table: "ducklake_schema".to_string(),
                kind: CdcChangeKind::Add,
                row: serde_json::json!({
                    "schema_id": row.schema_id,
                    "schema_name": row.schema_name,
                    "begin_snapshot": row.begin_snapshot,
                }),
            });
        }
        for row in &diff.retired_schemas {
            events.push(CdcEvent {
                snapshot_id: to,
                table: "ducklake_schema".to_string(),
                kind: CdcChangeKind::Retire,
                row: serde_json::json!({
                    "schema_id": row.schema_id,
                    "schema_name": row.schema_name,
                    "end_snapshot": row.end_snapshot,
                }),
            });
        }
        for row in &diff.added_tables {
            events.push(CdcEvent {
                snapshot_id: to,
                table: "ducklake_table".to_string(),
                kind: CdcChangeKind::Add,
                row: serde_json::json!({
                    "table_id": row.table_id,
                    "schema_id": row.schema_id,
                    "table_name": row.table_name,
                    "begin_snapshot": row.begin_snapshot,
                }),
            });
        }
        for row in &diff.retired_tables {
            events.push(CdcEvent {
                snapshot_id: to,
                table: "ducklake_table".to_string(),
                kind: CdcChangeKind::Retire,
                row: serde_json::json!({
                    "table_id": row.table_id,
                    "schema_id": row.schema_id,
                    "table_name": row.table_name,
                    "end_snapshot": row.end_snapshot,
                }),
            });
        }
        for row in &diff.added_columns {
            events.push(CdcEvent {
                snapshot_id: to,
                table: "ducklake_column".to_string(),
                kind: CdcChangeKind::Add,
                row: serde_json::json!({
                    "column_id": row.column_id,
                    "table_id": row.table_id,
                    "column_name": row.column_name,
                    "column_type": row.data_type,
                    "begin_snapshot": row.begin_snapshot,
                }),
            });
        }
        for row in &diff.retired_columns {
            events.push(CdcEvent {
                snapshot_id: to,
                table: "ducklake_column".to_string(),
                kind: CdcChangeKind::Retire,
                row: serde_json::json!({
                    "column_id": row.column_id,
                    "table_id": row.table_id,
                    "column_name": row.column_name,
                    "column_type": row.data_type,
                    "end_snapshot": row.end_snapshot,
                }),
            });
        }
        for row in &diff.added_data_files {
            events.push(CdcEvent {
                snapshot_id: to,
                table: "ducklake_data_file".to_string(),
                kind: CdcChangeKind::Add,
                row: serde_json::json!({
                    "data_file_id": row.data_file_id,
                    "table_id": row.table_id,
                    "path": row.path,
                    "file_format": row.file_format,
                    "record_count": row.record_count,
                    "file_size_bytes": row.file_size_bytes,
                    "begin_snapshot": row.begin_snapshot,
                }),
            });
        }
        for row in &diff.retired_data_files {
            events.push(CdcEvent {
                snapshot_id: to,
                table: "ducklake_data_file".to_string(),
                kind: CdcChangeKind::Retire,
                row: serde_json::json!({
                    "data_file_id": row.data_file_id,
                    "table_id": row.table_id,
                    "path": row.path,
                    "end_snapshot": row.end_snapshot,
                }),
            });
        }

        CdcSnapshot {
            from_snapshot: diff.from_snapshot.as_u64(),
            to_snapshot: to,
            events,
        }
    }

    /// Serialize this CDC snapshot to JSON-lines format.
    pub fn to_jsonl(&self) -> String {
        let mut out = String::new();
        // Header line: snapshot metadata.
        out.push_str(
            &serde_json::to_string(&serde_json::json!({
                "_type": "cdc_snapshot_header",
                "from_snapshot": self.from_snapshot,
                "to_snapshot": self.to_snapshot,
                "event_count": self.events.len(),
            }))
            .unwrap_or_default(),
        );
        out.push('\n');
        // One line per event.
        for event in &self.events {
            out.push_str(&serde_json::to_string(event).unwrap_or_default());
            out.push('\n');
        }
        out
    }
}

// ─── S3 CDC File Writer ─────────────────────────────────────────────────────

/// Path for the CDC file for a given to_snapshot.
///
/// Format: `{warehouse_prefix}/cdc/snapshot-{to_snapshot:020}.jsonl`
pub fn cdc_s3_path(warehouse_prefix: &str, to_snapshot: u64) -> String {
    format!(
        "{}/cdc/snapshot-{:020}.jsonl",
        warehouse_prefix.trim_end_matches('/'),
        to_snapshot
    )
}

/// Write a `CdcSnapshot` as JSON-lines to an in-memory writer (S3 object body).
///
/// In production this would write to `object_store`, but the function signature
/// accepts any `std::io::Write` so tests can use a `Vec<u8>`.
pub fn write_cdc_jsonl<W: std::io::Write>(
    cdc: &CdcSnapshot,
    writer: &mut W,
) -> CatalogResult<usize> {
    let jsonl = cdc.to_jsonl();
    let bytes = jsonl.len();
    writer
        .write_all(jsonl.as_bytes())
        .map_err(|e| crate::error::CatalogError::Internal(e.to_string()))?;
    Ok(bytes)
}

// ─── CDC Tailer ─────────────────────────────────────────────────────────────

/// State for the `slateduck-cdc` sidecar polling loop.
///
/// Call `poll_once` repeatedly (e.g. every 100 ms) to export each new
/// snapshot diff.  The tailer tracks the last-exported snapshot so it
/// never re-exports the same diff.
pub struct CdcTailer {
    /// Last snapshot whose diff was exported.
    last_exported: SnapshotId,
    /// Path prefix in the object store for CDC files.
    pub warehouse_prefix: String,
}

impl CdcTailer {
    /// Create a new tailer starting from `start_snapshot` (exclusive).
    pub fn new(start_snapshot: SnapshotId, warehouse_prefix: &str) -> Self {
        Self {
            last_exported: start_snapshot,
            warehouse_prefix: warehouse_prefix.to_string(),
        }
    }

    /// Export the diff from `last_exported` to `current_snapshot` if there is
    /// a new snapshot.  Returns the CDC snapshot if one was produced, or `None`
    /// if there is nothing new.
    pub async fn poll_once(
        &mut self,
        store: &crate::store::CatalogStore,
    ) -> CatalogResult<Option<CdcSnapshot>> {
        let reader = store.read_latest();
        let current = reader.snapshot_id();

        if current.as_u64() <= self.last_exported.as_u64() {
            return Ok(None);
        }

        let from = self.last_exported;
        let to = current;

        let diff = reader.snapshot_diff(from, to).await?;
        if diff.is_empty() {
            self.last_exported = to;
            return Ok(None);
        }

        let cdc = CdcSnapshot::from_diff(&diff);
        self.last_exported = to;
        Ok(Some(cdc))
    }
}

// ─── Webhook Payload ────────────────────────────────────────────────────────

/// Webhook payload sent on each snapshot commit.
///
/// The receiving server can use `diff_url` to fetch the full diff file from S3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookPayload {
    pub snapshot_id: u64,
    pub from_snapshot: u64,
    pub affected_tables: Vec<u64>,
    /// Pre-signed URL (or path) to the diff JSONL file.
    pub diff_url: String,
    pub event_count: usize,
}

impl WebhookPayload {
    /// Build a webhook payload from a CDC snapshot and its S3 path.
    pub fn from_cdc(cdc: &CdcSnapshot, diff_url: &str) -> Self {
        let mut affected_tables: std::collections::HashSet<u64> = std::collections::HashSet::new();
        for event in &cdc.events {
            if let Some(table_id) = event.row.get("table_id").and_then(|v| v.as_u64()) {
                affected_tables.insert(table_id);
            }
        }
        Self {
            snapshot_id: cdc.to_snapshot,
            from_snapshot: cdc.from_snapshot,
            affected_tables: affected_tables.into_iter().collect(),
            diff_url: diff_url.to_string(),
            event_count: cdc.events.len(),
        }
    }
}
