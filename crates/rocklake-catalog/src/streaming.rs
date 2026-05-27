//! Streaming ingest: RocklakeSink and exactly-once delivery semantics.
//!
//! # Overview
//!
//! `RocklakeSink` is the DuckLake-native streaming ingest endpoint.  It
//! accepts record batches from any ordered source (Kafka, NATS, webhook, etc.)
//! and commits them — together with a consumer offset — in **one atomic
//! SlateDB transaction**.
//!
//! # Exactly-Once Delivery
//!
//! The two-phase pattern is:
//! 1. Write Parquet files to S3 (outside the catalog transaction).
//! 2. In one catalog `create_snapshot()`:
//!    - Register the new data files via `CatalogWriter::add_data_file_*`.
//!    - Update the consumer offset key via `CatalogWriter::set_metadata`.
//!
//! If the process dies between steps 1 and 2, the orphaned Parquet files are
//! cleaned up by the orphan-file sweep after the grace period.  The consumer
//! re-reads from its last committed offset (not yet advanced) and
//! re-registers the same data files.  Because data-file registration is
//! idempotent for a given Parquet file path, the retry is safe.
//!
//! # Application Metadata Namespace
//!
//! Consumer offsets and other application state are stored in
//! `ducklake_metadata` under the `{app}.{instance}.{key}` namespace:
//! ```text
//! pg_tide.orders-to-lake.offset  →  "4782"
//! ```
//! Multiple applications coexist using distinct prefixes.

#![allow(missing_docs)]

use rocklake_core::keys::MetadataScope;
use rocklake_core::mvcc::SnapshotId;

use crate::error::CatalogResult;
use crate::store::CatalogStore;
use crate::writer::validate_app_metadata_key;

/// A single inlined record for streaming ingest (key → JSON value).
#[derive(Debug, Clone)]
pub struct IngestRecord {
    pub key: String,
    pub value: serde_json::Value,
}

/// Result of a `RocklakeSink::ingest_batch` call.
#[derive(Debug, Clone)]
pub struct IngestResult {
    /// The snapshot committed for this batch.
    pub snapshot_id: SnapshotId,
    /// Number of records committed in this batch.
    pub records_committed: usize,
    /// The new consumer offset after this batch (if offset tracking is used).
    pub new_offset: Option<String>,
}

/// Streaming ingest sink for a single DuckLake table.
///
/// Registers data files (represented as paths to pre-written Parquet files)
/// and advances a consumer offset atomically in one snapshot commit.
///
/// # Construction
///
/// ```
/// use rocklake_catalog::RocklakeSink;
/// let sink = RocklakeSink::new("pg_tide.orders-to-lake.offset").unwrap();
/// assert_eq!(sink.offset_key, "pg_tide.orders-to-lake.offset");
/// ```
pub struct RocklakeSink {
    /// Consumer group application metadata key for offset tracking.
    /// Must follow the `{app}.{instance}.{key}` naming convention.
    pub offset_key: String,
}

impl RocklakeSink {
    /// Create a new `RocklakeSink`.
    ///
    /// # Arguments
    /// * `offset_key` — application metadata key for consumer offset, e.g.
    ///   `"pg_tide.orders-to-lake.offset"`.  Must follow the
    ///   `{app}.{instance}.{key}` convention.
    pub fn new(offset_key: &str) -> CatalogResult<Self> {
        validate_app_metadata_key(offset_key)?;
        Ok(Self {
            offset_key: offset_key.to_string(),
        })
    }

    /// Commit a batch of ingest records as inlined data plus update the
    /// consumer offset, all in one atomic snapshot.
    ///
    /// # Exactly-once guarantee
    ///
    /// Before staging any work, this method reads the current offset from the
    /// catalog. If `expected_current_offset` is `Some(v)` and the stored
    /// offset equals `v`, the batch is committed and the offset is advanced to
    /// `next_offset`.  If the stored offset is already equal to `next_offset`
    /// (i.e., the batch was committed on a previous attempt), the call is a
    /// no-op and returns `Ok` with `records_committed = 0`.
    ///
    /// # Arguments
    /// * `store` — the catalog store.
    /// * `records` — the records to commit as inlined inserts.
    /// * `table_id` — the target table.
    /// * `expected_current_offset` — the offset the consumer expects to be
    ///   current (for exactly-once fencing).  `None` skips the check.
    /// * `next_offset` — the new offset to write atomically with the batch.
    /// * `author` — optional author string for the snapshot.
    pub async fn commit_batch(
        &self,
        store: &mut CatalogStore,
        records: &[IngestRecord],
        table_id: u64,
        expected_current_offset: Option<&str>,
        next_offset: &str,
        author: Option<&str>,
    ) -> CatalogResult<IngestResult> {
        // ── Exactly-once idempotency check ───────────────────────────────────
        {
            let reader = store.read_latest();
            let current = reader
                .get_metadata(MetadataScope::Global, 0, &self.offset_key)
                .await?;
            let current_val = current.as_ref().map(|r| r.value.as_str());

            // If the stored offset already equals next_offset, this batch was
            // already committed on a previous attempt — return idempotently.
            if current_val == Some(next_offset) {
                let snap = reader.snapshot_id();
                return Ok(IngestResult {
                    snapshot_id: snap,
                    records_committed: 0,
                    new_offset: Some(next_offset.to_string()),
                });
            }

            // If an expected current offset is given, verify it matches.
            if let Some(expected) = expected_current_offset {
                if current_val != Some(expected) {
                    return Err(crate::error::CatalogError::InvalidInput(format!(
                        "exactly-once fencing: expected offset {:?}, found {:?}",
                        expected, current_val
                    )));
                }
            }
        }

        // ── Stage inlined records + offset update, commit atomically ─────────
        let mut writer = store.begin_write();
        let schema_version = writer.schema_version();

        // Use the next_offset as a base for deterministic row_id generation.
        // This ensures re-submitted identical batches produce the same row_ids,
        // supporting idempotent inlined-insert registration.
        let offset_base: u64 = next_offset
            .parse::<u64>()
            .unwrap_or(0)
            .saturating_mul(records.len() as u64);

        // Write each record as an inlined insert for the table.
        for (idx, record) in records.iter().enumerate() {
            let row_id = offset_base.wrapping_add(idx as u64);
            let payload = serde_json::to_vec(&record.value).unwrap_or_default();
            writer
                .register_inlined_insert(table_id, schema_version, row_id, payload)
                .await?;
        }

        // Update the consumer offset atomically with the data records.
        writer.set_metadata(MetadataScope::Global, 0, &self.offset_key, next_offset)?;

        let commit = writer
            .create_snapshot(
                author,
                Some(&format!(
                    "streaming ingest batch: {} records",
                    records.len()
                )),
            )
            .await?;
        let snapshot_id = commit.snapshot_id;

        store.commit_writer(commit);

        Ok(IngestResult {
            snapshot_id,
            records_committed: records.len(),
            new_offset: Some(next_offset.to_string()),
        })
    }
}

/// Simulate ingest throughput: commit `total_records` records in batches of
/// `batch_size` and measure per-batch catalog commit latency.
///
/// Returns `(throughput_rps, p95_commit_ms)` — used by the performance test.
pub async fn measure_ingest_throughput(
    store: &mut CatalogStore,
    sink: &RocklakeSink,
    table_id: u64,
    total_records: usize,
    batch_size: usize,
) -> CatalogResult<(f64, f64)> {
    use std::time::Instant;

    let mut latencies_ms: Vec<f64> = Vec::new();
    let mut records_committed = 0usize;
    let wall_start = Instant::now();

    let mut batch_num = 0u64;
    while records_committed < total_records {
        let this_batch = batch_size.min(total_records - records_committed);
        let batch_records: Vec<IngestRecord> = (0..this_batch)
            .map(|i| IngestRecord {
                key: "id".to_string(),
                value: serde_json::json!(records_committed + i),
            })
            .collect();

        let next_offset = (batch_num + 1).to_string();
        let expected = if batch_num == 0 {
            None
        } else {
            Some(batch_num.to_string())
        };

        let t0 = Instant::now();
        sink.commit_batch(
            store,
            &batch_records,
            table_id,
            expected.as_deref(),
            &next_offset,
            None,
        )
        .await?;
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
        latencies_ms.push(elapsed_ms);

        records_committed += this_batch;
        batch_num += 1;
    }

    let total_secs = wall_start.elapsed().as_secs_f64();
    let throughput = total_records as f64 / total_secs;

    // Compute p95 latency.
    latencies_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p95_idx = ((latencies_ms.len() as f64 * 0.95) as usize).saturating_sub(1);
    let p95_ms = latencies_ms.get(p95_idx).copied().unwrap_or(0.0);

    Ok((throughput, p95_ms))
}
