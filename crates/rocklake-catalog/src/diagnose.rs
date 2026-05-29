//! Catalog diagnostics: structured health report for `rocklake diagnose`.
//!
//! Produces a comprehensive health snapshot covering:
//! - Catalog format version and writer epoch
//! - Current snapshot ID and retain-from floor
//! - Secondary index (TAG_DATA_FILE_BY_SNAPSHOT) consistency check
//! - Orphan Parquet file detection (when object store is provided)
//! - Snapshot gap detection between retain-from and current

#![allow(missing_docs)]

use std::sync::Arc;

use object_store::path::Path as ObjectPath;
use object_store::ObjectStore;
use slatedb::Db;

use rocklake_core::keys;
use rocklake_core::rows::*;
use rocklake_core::tags::*;
use rocklake_core::values;

use crate::cleanup::collect_referenced_paths;
use crate::error::{CatalogError, CatalogResult};
use crate::inspect::inspect_snapshot;

/// Severity of a diagnostic finding.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FindingSeverity {
    /// Critical issue — catalog may not be usable.
    P0,
    /// Important issue — degraded state, action required.
    P1,
    /// Advisory — worth reviewing but not immediately actionable.
    P2,
}

/// A single diagnostic finding.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiagnosticFinding {
    pub severity: FindingSeverity,
    pub category: String,
    pub message: String,
}

/// Full health report produced by `rocklake diagnose`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiagnoseReport {
    /// Catalog format version stored in SlateDB.
    pub format_version: u32,
    /// Current writer epoch (monotonic counter).
    pub writer_epoch: u64,
    /// Latest committed snapshot ID.
    pub latest_snapshot_id: u64,
    /// Retain-from snapshot floor (GC lower bound).
    pub retain_from: u64,
    /// Number of schemas.
    pub schema_count: u64,
    /// Number of tables.
    pub table_count: u64,
    /// Number of data files in primary index.
    pub data_file_count: u64,
    /// Number of data-file primary rows checked for secondary-index consistency.
    pub secondary_index_entries_checked: u64,
    /// Number of primary data-file rows with no matching secondary-index entry.
    pub secondary_index_gaps: u64,
    /// Orphan Parquet files found in object storage (not in any live snapshot).
    pub orphan_files: Vec<String>,
    /// Snapshot IDs missing between retain-from and latest_snapshot_id.
    pub snapshot_gaps: Vec<u64>,
    /// All findings (P0/P1/P2).
    pub findings: Vec<DiagnosticFinding>,
    /// Overall health: "ok", "degraded", or "critical".
    pub overall_status: String,
}

impl DiagnoseReport {
    /// Returns true if no P0 findings are present.
    pub fn is_ok(&self) -> bool {
        !self
            .findings
            .iter()
            .any(|f| f.severity == FindingSeverity::P0)
    }
}

/// Run the full diagnostic suite against the catalog.
///
/// `object_store` and `data_root` are used for orphan-file detection; pass
/// `None` to skip that check.
pub async fn diagnose_catalog(
    db: &Db,
    object_store: Option<(Arc<dyn ObjectStore>, String)>,
) -> CatalogResult<DiagnoseReport> {
    let mut findings: Vec<DiagnosticFinding> = Vec::new();

    // 1. Basic snapshot inspection.
    let info = inspect_snapshot(db).await?;

    // 2. Secondary index consistency check.
    let (secondary_entries, secondary_gaps) = check_secondary_index(db, &mut findings).await?;

    // 3. Snapshot gap detection.
    let snapshot_gaps =
        detect_snapshot_gaps(db, info.retain_from, info.latest_snapshot_id, &mut findings).await?;

    // 4. Orphan file detection.
    let orphan_files = if let Some((store, data_root)) = object_store {
        detect_orphan_files(db, store, &data_root, &mut findings).await?
    } else {
        Vec::new()
    };

    // 5. Compute overall status.
    let overall_status = if findings.iter().any(|f| f.severity == FindingSeverity::P0) {
        "critical".to_string()
    } else if findings.iter().any(|f| f.severity == FindingSeverity::P1) {
        "degraded".to_string()
    } else {
        "ok".to_string()
    };

    Ok(DiagnoseReport {
        format_version: info.format_version,
        writer_epoch: info.writer_epoch,
        latest_snapshot_id: info.latest_snapshot_id,
        retain_from: info.retain_from,
        schema_count: info.schema_count,
        table_count: info.table_count,
        data_file_count: info.data_file_count,
        secondary_index_entries_checked: secondary_entries,
        secondary_index_gaps: secondary_gaps,
        orphan_files,
        snapshot_gaps,
        findings,
        overall_status,
    })
}

/// Check that every primary data-file key has a corresponding secondary-index entry.
async fn check_secondary_index(
    db: &Db,
    findings: &mut Vec<DiagnosticFinding>,
) -> CatalogResult<(u64, u64)> {
    let mut entries_checked: u64 = 0;
    let mut gaps: u64 = 0;

    let prefix = keys::prefix_for_tag(TAG_DATA_FILE);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: DataFileRow = match values::decode_value(&kv.value) {
            Ok(r) => r,
            Err(_) => continue,
        };
        entries_checked += 1;

        // Build secondary key using table_id + begin_snapshot + data_file_id.
        let begin_snap = row.begin_snapshot.unwrap_or(0);
        let secondary_key =
            keys::key_data_file_by_snapshot(row.table_id, begin_snap, row.data_file_id);
        let secondary_key = bytes::Bytes::from(secondary_key);

        match db.get(&secondary_key).await {
            Ok(None) => {
                gaps += 1;
                if gaps <= 10 {
                    findings.push(DiagnosticFinding {
                        severity: FindingSeverity::P1,
                        category: "secondary-index".to_string(),
                        message: format!(
                            "Data file {} (table {}, snap {}) has no secondary-index entry",
                            row.data_file_id, row.table_id, begin_snap
                        ),
                    });
                }
            }
            Ok(Some(_)) => {}
            Err(e) => {
                findings.push(DiagnosticFinding {
                    severity: FindingSeverity::P1,
                    category: "secondary-index".to_string(),
                    message: format!("Error checking secondary key: {e}"),
                });
            }
        }
    }

    if gaps > 10 {
        findings.push(DiagnosticFinding {
            severity: FindingSeverity::P1,
            category: "secondary-index".to_string(),
            message: format!("{gaps} total secondary-index gaps (first 10 detailed above)"),
        });
    }

    Ok((entries_checked, gaps))
}

/// Detect missing snapshot IDs between retain-from and latest_snapshot_id.
async fn detect_snapshot_gaps(
    db: &Db,
    retain_from: u64,
    latest: u64,
    findings: &mut Vec<DiagnosticFinding>,
) -> CatalogResult<Vec<u64>> {
    let mut gaps = Vec::new();

    if latest == 0 {
        return Ok(gaps);
    }

    let start = retain_from.max(1);
    let scan_limit = 10_000u64;

    for (scanned, snapshot_id) in (start..=latest).enumerate() {
        if scanned as u64 >= scan_limit {
            findings.push(DiagnosticFinding {
                severity: FindingSeverity::P2,
                category: "snapshot-gaps".to_string(),
                message: "Snapshot gap scan truncated at 10,000 snapshots".to_string(),
            });
            break;
        }

        let key = bytes::Bytes::from(keys::key_snapshot(snapshot_id));
        match db.get(&key).await {
            Ok(None) => {
                gaps.push(snapshot_id);
            }
            Ok(Some(_)) => {}
            Err(e) => {
                findings.push(DiagnosticFinding {
                    severity: FindingSeverity::P1,
                    category: "snapshot-gaps".to_string(),
                    message: format!("Error reading snapshot {snapshot_id}: {e}"),
                });
            }
        }
    }

    if !gaps.is_empty() {
        findings.push(DiagnosticFinding {
            severity: FindingSeverity::P1,
            category: "snapshot-gaps".to_string(),
            message: format!(
                "{} snapshot IDs missing between retain-from={retain_from} and latest={latest}",
                gaps.len()
            ),
        });
    }

    Ok(gaps)
}

/// Detect Parquet files in object storage not referenced by any live catalog snapshot.
async fn detect_orphan_files(
    db: &Db,
    store: Arc<dyn ObjectStore>,
    data_root: &str,
    findings: &mut Vec<DiagnosticFinding>,
) -> CatalogResult<Vec<String>> {
    use futures::TryStreamExt;

    let referenced = collect_referenced_paths(db).await?;
    let root_path = ObjectPath::from(data_root);
    let mut orphans = Vec::new();

    let list_result = store.list(Some(&root_path)).try_collect::<Vec<_>>().await;

    match list_result {
        Ok(objects) => {
            for meta in &objects {
                let path_str = meta.location.to_string();
                if path_str.ends_with(".parquet") && !referenced.contains(&path_str) {
                    orphans.push(path_str);
                }
            }
        }
        Err(e) => {
            findings.push(DiagnosticFinding {
                severity: FindingSeverity::P2,
                category: "orphan-files".to_string(),
                message: format!("Error listing object store at {data_root}: {e}"),
            });
        }
    }

    if !orphans.is_empty() {
        findings.push(DiagnosticFinding {
            severity: FindingSeverity::P2,
            category: "orphan-files".to_string(),
            message: format!(
                "{} orphan Parquet files found in {data_root}",
                orphans.len()
            ),
        });
    }

    Ok(orphans)
}

/// Format a `DiagnoseReport` as human-readable text.
pub fn format_report_text(report: &DiagnoseReport) -> String {
    let mut out = String::new();
    out.push_str("=== RockLake Catalog Diagnostics ===\n\n");
    out.push_str(&format!(
        "Overall status:      {}\n",
        report.overall_status.to_uppercase()
    ));
    out.push_str(&format!("Format version:      {}\n", report.format_version));
    out.push_str(&format!("Writer epoch:        {}\n", report.writer_epoch));
    out.push_str(&format!(
        "Latest snapshot:     {}\n",
        report.latest_snapshot_id
    ));
    out.push_str(&format!("Retain-from:         {}\n", report.retain_from));
    out.push_str(&format!("Schemas:             {}\n", report.schema_count));
    out.push_str(&format!("Tables:              {}\n", report.table_count));
    out.push_str(&format!(
        "Data files:          {}\n",
        report.data_file_count
    ));
    out.push_str(&format!(
        "2nd-index checked:   {} ({} gaps)\n",
        report.secondary_index_entries_checked, report.secondary_index_gaps
    ));
    out.push_str(&format!(
        "Snapshot gaps:       {}\n",
        report.snapshot_gaps.len()
    ));
    out.push_str(&format!(
        "Orphan files:        {}\n",
        report.orphan_files.len()
    ));

    if !report.findings.is_empty() {
        out.push_str("\n--- Findings ---\n");
        for f in &report.findings {
            let sev = match f.severity {
                FindingSeverity::P0 => "P0",
                FindingSeverity::P1 => "P1",
                FindingSeverity::P2 => "P2",
            };
            out.push_str(&format!("[{sev}] [{}] {}\n", f.category, f.message));
        }
    } else {
        out.push_str("\nNo findings.\n");
    }

    out
}
