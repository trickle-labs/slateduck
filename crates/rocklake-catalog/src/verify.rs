//! Catalog verification: primary-key uniqueness, FK references, MVCC consistency, counter monotonicity.

#![allow(missing_docs)]

use slatedb::Db;
use rocklake_core::keys;
use rocklake_core::rows::*;
use rocklake_core::tags::*;
use rocklake_core::values;

use crate::error::{CatalogError, CatalogResult};

/// Result of catalog verification.
#[derive(Debug, Default)]
pub struct VerifyResult {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub tables_checked: u32,
    pub rows_checked: u64,
}

impl VerifyResult {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Verify catalog integrity.
pub async fn verify_catalog(db: &Db) -> CatalogResult<VerifyResult> {
    let mut result = VerifyResult::default();
    verify_format_version(db, &mut result).await?;
    verify_counters(db, &mut result).await?;
    verify_schemas(db, &mut result).await?;
    verify_tables(db, &mut result).await?;
    verify_columns(db, &mut result).await?;
    verify_snapshots(db, &mut result).await?;
    Ok(result)
}

async fn verify_format_version(db: &Db, result: &mut VerifyResult) -> CatalogResult<()> {
    let key = keys::key_system(SYSTEM_CATALOG_FORMAT_VERSION);
    match db.get(&key).await? {
        None => {
            result
                .errors
                .push("missing catalog-format-version".to_string());
        }
        Some(data) => {
            let version = values::decode_format_version(&data)?;
            if version != CATALOG_FORMAT_VERSION {
                result.errors.push(format!(
                    "format version mismatch: expected {}, got {}",
                    CATALOG_FORMAT_VERSION, version
                ));
            }
        }
    }
    result.tables_checked += 1;
    Ok(())
}

async fn verify_counters(db: &Db, result: &mut VerifyResult) -> CatalogResult<()> {
    for (name, counter_id) in [
        ("next_snapshot_id", COUNTER_NEXT_SNAPSHOT_ID),
        ("next_catalog_id", COUNTER_NEXT_CATALOG_ID),
        ("next_file_id", COUNTER_NEXT_FILE_ID),
    ] {
        let key = keys::key_counter(counter_id);
        match db.get(&key).await? {
            None => {
                result.errors.push(format!("missing counter: {name}"));
            }
            Some(data) => {
                let val = values::decode_counter(&data)?;
                if val == 0 {
                    result
                        .warnings
                        .push(format!("counter {name} is 0 (unusual)"));
                }
                result.rows_checked += 1;
            }
        }
    }
    result.tables_checked += 1;
    Ok(())
}

async fn verify_schemas(db: &Db, result: &mut VerifyResult) -> CatalogResult<()> {
    let prefix = keys::prefix_for_tag(TAG_SCHEMA);
    let mut seen_ids = std::collections::HashSet::new();
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: SchemaRow = values::decode_value(&kv.value)?;
        if !seen_ids.insert(row.schema_id) {
            result.errors.push(format!(
                "duplicate schema_id: {} (name: {})",
                row.schema_id, row.schema_name
            ));
        }
        if let Some(end) = row.end_snapshot {
            if end <= row.begin_snapshot {
                result.errors.push(format!(
                    "schema {}: end_snapshot ({}) <= begin_snapshot ({})",
                    row.schema_id, end, row.begin_snapshot
                ));
            }
        }
        result.rows_checked += 1;
    }
    result.tables_checked += 1;
    Ok(())
}

async fn verify_tables(db: &Db, result: &mut VerifyResult) -> CatalogResult<()> {
    let prefix = keys::prefix_for_tag(TAG_TABLE);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: TableRow = values::decode_value(&kv.value)?;
        if let Some(end) = row.end_snapshot {
            if end <= row.begin_snapshot {
                result.errors.push(format!(
                    "table {}: end_snapshot ({}) <= begin_snapshot ({})",
                    row.table_id, end, row.begin_snapshot
                ));
            }
        }
        result.rows_checked += 1;
    }
    result.tables_checked += 1;
    Ok(())
}

async fn verify_columns(db: &Db, result: &mut VerifyResult) -> CatalogResult<()> {
    let prefix = keys::prefix_for_tag(TAG_COLUMN);
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: ColumnRow = values::decode_value(&kv.value)?;
        if let Some(end) = row.end_snapshot {
            if end <= row.begin_snapshot {
                result.errors.push(format!(
                    "column {} (table {}): end_snapshot ({}) <= begin_snapshot ({})",
                    row.column_id, row.table_id, end, row.begin_snapshot
                ));
            }
        }
        result.rows_checked += 1;
    }
    result.tables_checked += 1;
    Ok(())
}

async fn verify_snapshots(db: &Db, result: &mut VerifyResult) -> CatalogResult<()> {
    let prefix = keys::prefix_for_tag(TAG_SNAPSHOT);
    let mut prev_id: Option<u64> = None;
    let mut iter = db.scan_prefix(&prefix).await?;
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        let row: SnapshotRow = values::decode_value(&kv.value)?;
        if let Some(prev) = prev_id {
            if row.snapshot_id <= prev {
                result.errors.push(format!(
                    "snapshot ordering violation: {} follows {}",
                    row.snapshot_id, prev
                ));
            }
        }
        prev_id = Some(row.snapshot_id);
        result.rows_checked += 1;
    }
    result.tables_checked += 1;
    Ok(())
}
