//! Orphan-file sweep: identify and optionally delete Parquet files present in
//! object storage but not referenced by any live catalog snapshot.
//!
//! This module provides the `SweepOrphansConfig` / `SweepResult` types used by
//! `rocklake sweep-orphans`. The underlying scan logic delegates to
//! `cleanup::orphaned_file_sweep`.
//!
//! Usage:
//!   rocklake sweep-orphans --catalog <path> [--data-root <prefix>]
//!                          [--grace-period-hours N] [--apply]

#![allow(missing_docs)]

use std::sync::Arc;

use object_store::path::Path as ObjectPath;
use object_store::ObjectStore;
use slatedb::Db;

use crate::cleanup::orphaned_file_sweep;
use crate::error::CatalogResult;

/// Configuration for `rocklake sweep-orphans`.
#[derive(Debug, Clone)]
pub struct SweepOrphansConfig {
    /// Minimum age of an orphan file (in hours) before it qualifies for
    /// deletion. Files younger than this are skipped even with `--apply`.
    /// Default: 24 hours.
    pub grace_period_hours: u64,
    /// If `true`, actually delete orphan files.
    /// If `false` (default), only report them.
    pub apply: bool,
    /// Object-store path prefix where Parquet data files are stored.
    pub data_root: String,
}

impl Default for SweepOrphansConfig {
    fn default() -> Self {
        Self {
            grace_period_hours: 24,
            apply: false,
            data_root: String::new(),
        }
    }
}

/// Result of a `sweep-orphans` run.
#[derive(Debug, Default)]
pub struct SweepResult {
    /// Parquet files found in object storage but not in any live snapshot
    /// and older than the grace period.
    pub orphan_files: Vec<String>,
    /// Files deleted (only non-zero when `apply=true`).
    pub deleted: usize,
    /// Total files scanned.
    pub total_scanned: u64,
}

impl SweepResult {
    /// Human-readable single-line summary.
    pub fn summary(&self) -> String {
        format!(
            "scanned={} orphans={} deleted={}",
            self.total_scanned,
            self.orphan_files.len(),
            self.deleted
        )
    }
}

/// Run the orphan-file sweep, delegating to `cleanup::orphaned_file_sweep`.
pub async fn sweep_orphans(
    db: &Db,
    store: Arc<dyn ObjectStore>,
    config: &SweepOrphansConfig,
) -> CatalogResult<SweepResult> {
    let data_prefix = ObjectPath::from(config.data_root.as_str());
    let grace_secs = config.grace_period_hours * 3600;

    let inner = orphaned_file_sweep(db, &store, &data_prefix, grace_secs, config.apply).await?;

    Ok(SweepResult {
        orphan_files: inner.orphaned_files,
        deleted: inner.deleted_files.len(),
        total_scanned: inner.total_files_scanned,
    })
}
