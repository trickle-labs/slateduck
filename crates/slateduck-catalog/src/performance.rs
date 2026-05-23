//! Performance optimizations for catalog hot paths.
//!
//! - Hot-key cold-start: single GET pulls current snapshot + table file counts
//! - Secondary indexes: skip-index for snapshot-scoped file lookups
//! - Metadata packing: composite value per table for single-read planning
//! - SlateDB tuning: configurable block size, bloom filters, compaction

use slatedb::Db;
use slateduck_core::keys;
use slateduck_core::rows::*;
use slateduck_core::values;

use crate::error::{CatalogError, CatalogResult};

// ─── Hot Key ───────────────────────────────────────────────────────────────

/// Hot-key state for cold-start optimization.
/// A single GET retrieves everything needed to resume a cold DuckDB process.
#[derive(Debug, Clone)]
pub struct HotKeyState {
    pub current_snapshot_id: u64,
    pub table_file_counts: Vec<(u64, u64)>,
}

/// Read the hot key from the catalog.
/// Returns `None` if no hot key has been written yet.
pub async fn read_hot_key(db: &Db) -> CatalogResult<Option<HotKeyState>> {
    let key = keys::key_hot();
    match db.get(&key).await? {
        None => Ok(None),
        Some(data) => {
            let row: HotKeyValue = values::decode_value(&data)?;
            Ok(Some(HotKeyState {
                current_snapshot_id: row.current_snapshot_id,
                table_file_counts: row
                    .table_file_counts
                    .into_iter()
                    .map(|e| (e.table_id, e.file_count))
                    .collect(),
            }))
        }
    }
}

/// Write the hot key to the catalog.
/// Called after every snapshot creation to keep cold-start data fresh.
pub async fn write_hot_key(db: &Db, state: &HotKeyState) -> CatalogResult<()> {
    let row = HotKeyValue {
        current_snapshot_id: state.current_snapshot_id,
        table_file_counts: state
            .table_file_counts
            .iter()
            .map(|(table_id, file_count)| TableFileCount {
                table_id: *table_id,
                file_count: *file_count,
            })
            .collect(),
    };
    let key = keys::key_hot();
    db.put(&key, values::encode_value(&row)).await?;
    Ok(())
}

// ─── Secondary Index ───────────────────────────────────────────────────────

/// Write a secondary index entry for a data file registered at a given snapshot.
/// This enables O(1) lookups of files added at a specific snapshot for a table,
/// avoiding full MVCC scans.
pub async fn write_secondary_index(
    db: &Db,
    snapshot_id: u64,
    table_id: u64,
    data_file_id: u64,
    path: &str,
) -> CatalogResult<()> {
    let entry = SecondaryIndexEntry {
        data_file_id,
        path: path.to_string(),
    };
    let key = keys::key_secondary_index(snapshot_id, table_id, data_file_id);
    db.put(&key, values::encode_value(&entry)).await?;
    Ok(())
}

/// Read all data file IDs added at a specific snapshot for a table via the secondary index.
/// Returns an empty vec if no index entries exist (falls back to full scan).
pub async fn read_secondary_index(
    db: &Db,
    snapshot_id: u64,
    table_id: u64,
) -> CatalogResult<Vec<SecondaryIndexEntry>> {
    let prefix = keys::prefix_secondary_index(snapshot_id, table_id);
    let mut entries = Vec::new();
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let entry: SecondaryIndexEntry = values::decode_value(&kv.value)?;
        entries.push(entry);
    }
    Ok(entries)
}

// ─── Metadata Packing ──────────────────────────────────────────────────────

/// Write packed table metadata. Stores columns, partitions, sort info, and stats
/// as a single composite value under a system key.
pub async fn write_packed_metadata(db: &Db, metadata: &PackedTableMetadata) -> CatalogResult<()> {
    let key = keys::key_packed_table_metadata(metadata.table_id);
    db.put(&key, values::encode_value(metadata)).await?;
    Ok(())
}

/// Read packed table metadata. Returns `None` if not yet packed.
pub async fn read_packed_metadata(
    db: &Db,
    table_id: u64,
) -> CatalogResult<Option<PackedTableMetadata>> {
    let key = keys::key_packed_table_metadata(table_id);
    match db.get(&key).await? {
        None => Ok(None),
        Some(data) => Ok(Some(values::decode_value::<PackedTableMetadata>(&data)?)),
    }
}

// ─── SlateDB Tuning Configuration ─────────────────────────────────────────

/// Tuning parameters for SlateDB performance.
#[derive(Debug, Clone)]
pub struct SlateDbTuning {
    /// Block size in bytes for SST files (default: 4096).
    pub block_size: usize,
    /// Whether to enable bloom filters for point lookups.
    pub bloom_filter_enabled: bool,
    /// Bloom filter false positive rate (default: 0.01).
    pub bloom_filter_fp_rate: f64,
    /// Number of L0 SSTs before triggering compaction (default: 4).
    pub l0_sst_count_threshold: usize,
    /// Maximum write batch size in bytes (default: 64 MiB).
    pub max_write_batch_bytes: usize,
    /// Compaction aggressiveness: how eagerly to merge dead LSM entries.
    /// Range 1-10, where 10 is most aggressive (default: 5).
    pub compaction_aggressiveness: u8,
}

impl Default for SlateDbTuning {
    fn default() -> Self {
        Self {
            block_size: 4096,
            bloom_filter_enabled: true,
            bloom_filter_fp_rate: 0.01,
            l0_sst_count_threshold: 4,
            max_write_batch_bytes: 64 * 1024 * 1024,
            compaction_aggressiveness: 5,
        }
    }
}

impl SlateDbTuning {
    /// Create tuning optimized for high-ingest (update-heavy) workloads.
    /// More aggressive compaction to merge dead LSM entries earlier.
    pub fn high_ingest() -> Self {
        Self {
            block_size: 4096,
            bloom_filter_enabled: true,
            bloom_filter_fp_rate: 0.01,
            l0_sst_count_threshold: 2,
            max_write_batch_bytes: 64 * 1024 * 1024,
            compaction_aggressiveness: 8,
        }
    }

    /// Create tuning optimized for read-heavy workloads.
    /// Larger blocks and lower compaction frequency.
    pub fn read_heavy() -> Self {
        Self {
            block_size: 8192,
            bloom_filter_enabled: true,
            bloom_filter_fp_rate: 0.005,
            l0_sst_count_threshold: 8,
            max_write_batch_bytes: 64 * 1024 * 1024,
            compaction_aggressiveness: 3,
        }
    }
}

// ─── Benchmark Results ─────────────────────────────────────────────────────

/// A single benchmark measurement.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BenchmarkMeasurement {
    pub operation: String,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
}

/// A complete benchmark report.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BenchmarkReport {
    pub version: String,
    pub timestamp: String,
    pub storage: String,
    pub measurements: Vec<BenchmarkMeasurement>,
    pub comparison_vs_baseline: Vec<ComparisonEntry>,
}

/// Comparison of current performance vs. baseline.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ComparisonEntry {
    pub operation: String,
    pub baseline_p50_us: u64,
    pub current_p50_us: u64,
    pub ratio: f64,
}
