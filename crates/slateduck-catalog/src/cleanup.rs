//! Parquet data-file cleanup operations.
//!
//! - Orphaned-file sweep: scan object-store paths not referenced by any catalog row.
//! - Scheduled deletion: files marked for cleanup after no retained snapshot references them.
//! - verify_data_files: HEAD every referenced file and flag missing ones.

use object_store::path::Path as ObjectPath;
use object_store::ObjectStore;
use slatedb::Db;
use slateduck_core::keys;
use slateduck_core::rows::*;
use slateduck_core::tags::*;
use slateduck_core::values;
use std::collections::HashSet;
use std::sync::Arc;

use crate::error::{CatalogError, CatalogResult};

/// Result of orphaned-file sweep.
#[derive(Debug, Clone)]
pub struct OrphanedFileSweepResult {
    /// Files found in object store that are not referenced by any catalog row.
    pub orphaned_files: Vec<String>,
    /// Files that were deleted (only if apply=true).
    pub deleted_files: Vec<String>,
    /// Total files scanned.
    pub total_files_scanned: u64,
}

/// Result of data-file verification.
#[derive(Debug, Clone)]
pub struct VerifyDataFilesResult {
    /// Files that exist and are accessible.
    pub files_ok: u64,
    /// Files that are missing from object store.
    pub files_missing: Vec<String>,
    /// Files that returned errors (permissions, etc.).
    pub files_error: Vec<(String, String)>,
    /// Total files checked.
    pub total_checked: u64,
}

/// Collect all data file paths referenced in the catalog.
pub async fn collect_referenced_paths(db: &Db) -> CatalogResult<HashSet<String>> {
    let mut paths = HashSet::new();

    // Scan data files
    let prefix = keys::prefix_for_tag(TAG_DATA_FILE);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: DataFileRow = values::decode_value(&kv.value)?;
        paths.insert(row.path);
    }

    // Scan delete files
    let prefix = keys::prefix_for_tag(TAG_DELETE_FILE);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: DeleteFileRow = values::decode_value(&kv.value)?;
        paths.insert(row.path);
    }

    Ok(paths)
}

/// Scan object store for orphaned files not referenced in the catalog.
pub async fn orphaned_file_sweep(
    db: &Db,
    object_store: &Arc<dyn ObjectStore>,
    data_prefix: &ObjectPath,
    grace_period_secs: u64,
    apply: bool,
) -> CatalogResult<OrphanedFileSweepResult> {
    let referenced = collect_referenced_paths(db).await?;

    let mut orphaned_files = Vec::new();
    let mut deleted_files = Vec::new();
    let mut total_files_scanned = 0u64;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // List all objects under the data prefix
    let list_result = object_store
        .list(Some(data_prefix))
        .try_collect::<Vec<_>>()
        .await;

    match list_result {
        Ok(objects) => {
            for obj in &objects {
                total_files_scanned += 1;
                let path_str = obj.location.to_string();

                if !referenced.contains(&path_str) {
                    // Check grace period
                    let file_age_secs = now.saturating_sub(
                        obj.last_modified
                            .signed_duration_since(chrono::DateTime::UNIX_EPOCH)
                            .num_seconds() as u64,
                    );

                    if file_age_secs >= grace_period_secs {
                        orphaned_files.push(path_str.clone());
                        if apply {
                            match object_store.delete(&obj.location).await {
                                Ok(_) => deleted_files.push(path_str),
                                Err(e) => {
                                    tracing::warn!(
                                        "Failed to delete orphaned file {}: {e}",
                                        obj.location
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!("Failed to list data prefix: {e}");
        }
    }

    Ok(OrphanedFileSweepResult {
        orphaned_files,
        deleted_files,
        total_files_scanned,
    })
}

/// Verify that all referenced data files exist in the object store.
pub async fn verify_data_files(
    db: &Db,
    object_store: &Arc<dyn ObjectStore>,
) -> CatalogResult<VerifyDataFilesResult> {
    let referenced = collect_referenced_paths(db).await?;

    let mut files_ok = 0u64;
    let mut files_missing = Vec::new();
    let mut files_error = Vec::new();
    let total_checked = referenced.len() as u64;

    for path_str in &referenced {
        let path = ObjectPath::from(path_str.as_str());
        match object_store.head(&path).await {
            Ok(_) => files_ok += 1,
            Err(object_store::Error::NotFound { .. }) => {
                files_missing.push(path_str.clone());
            }
            Err(e) => {
                files_error.push((path_str.clone(), e.to_string()));
            }
        }
    }

    Ok(VerifyDataFilesResult {
        files_ok,
        files_missing,
        files_error,
        total_checked,
    })
}

/// Process scheduled file deletions.
pub async fn process_scheduled_deletions(
    db: &Db,
    object_store: &Arc<dyn ObjectStore>,
    retain_from: u64,
) -> CatalogResult<u64> {
    let prefix = keys::prefix_for_tag(TAG_FILES_SCHEDULED_FOR_DELETION);
    let mut deleted_count = 0u64;
    let mut to_remove_from_catalog = Vec::new();

    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: FilesScheduledForDeletionRow = values::decode_value(&kv.value)?;

        // Only delete if no retained snapshot references this file
        if retain_from > 0 && row.schedule_start < retain_from {
            let path = ObjectPath::from(row.path.as_str());
            match object_store.delete(&path).await {
                Ok(_) => {
                    deleted_count += 1;
                    to_remove_from_catalog.push(kv.key.to_vec());
                }
                Err(object_store::Error::NotFound { .. }) => {
                    // Already gone, remove from catalog
                    to_remove_from_catalog.push(kv.key.to_vec());
                }
                Err(e) => {
                    tracing::warn!("Failed to delete scheduled file {}: {e}", row.path);
                }
            }
        }
    }

    // Remove processed entries from catalog
    for key in to_remove_from_catalog {
        let _ = db.delete(&key).await;
    }

    Ok(deleted_count)
}

use futures::TryStreamExt;
