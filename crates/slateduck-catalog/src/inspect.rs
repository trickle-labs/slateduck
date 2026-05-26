//! Inspect: snapshot, schema version, counters, file counts.

#![allow(missing_docs)]

use slatedb::Db;
use slateduck_core::keys;
use slateduck_core::rows::*;
use slateduck_core::tags::*;
use slateduck_core::values;

use crate::error::{CatalogError, CatalogResult};
use crate::gc;

/// Summary information about the current catalog state.
#[derive(Debug, Clone)]
pub struct InspectResult {
    pub latest_snapshot_id: u64,
    pub schema_version: u64,
    pub snapshot_time: String,
    pub next_snapshot_id: u64,
    pub next_catalog_id: u64,
    pub next_file_id: u64,
    pub schema_count: u64,
    pub table_count: u64,
    pub column_count: u64,
    pub data_file_count: u64,
    pub delete_file_count: u64,
    pub retain_from: u64,
    pub writer_epoch: u64,
    pub format_version: u32,
}

/// Inspect the current state of the catalog.
pub async fn inspect_snapshot(db: &Db) -> CatalogResult<InspectResult> {
    // Load counters
    let next_snapshot_id = load_counter(db, COUNTER_NEXT_SNAPSHOT_ID).await?;
    let next_catalog_id = load_counter(db, COUNTER_NEXT_CATALOG_ID).await?;
    let next_file_id = load_counter(db, COUNTER_NEXT_FILE_ID).await?;

    // Load latest snapshot
    let latest_snapshot_id = next_snapshot_id.saturating_sub(1);

    let (schema_version, snapshot_time) = if latest_snapshot_id > 0 {
        let key = keys::key_snapshot(latest_snapshot_id);
        match db.get(&key).await? {
            Some(data) => {
                let row: SnapshotRow = values::decode_value(&data)?;
                (row.schema_version, row.snapshot_time)
            }
            None => (0, String::new()),
        }
    } else {
        (0, String::new())
    };

    // Count entities
    let schema_count = count_tag(db, TAG_SCHEMA).await?;
    let table_count = count_tag(db, TAG_TABLE).await?;
    let column_count = count_tag(db, TAG_COLUMN).await?;
    let data_file_count = count_tag(db, TAG_DATA_FILE).await?;
    let delete_file_count = count_tag(db, TAG_DELETE_FILE).await?;

    // Load system keys
    let retain_from = gc::read_retain_from(db).await?;
    let writer_epoch = load_system_counter(db, SYSTEM_WRITER_EPOCH).await?;
    let format_version = load_format_version(db).await?;

    Ok(InspectResult {
        latest_snapshot_id,
        schema_version,
        snapshot_time,
        next_snapshot_id,
        next_catalog_id,
        next_file_id,
        schema_count,
        table_count,
        column_count,
        data_file_count,
        delete_file_count,
        retain_from,
        writer_epoch,
        format_version,
    })
}

async fn load_counter(db: &Db, counter_id: u8) -> CatalogResult<u64> {
    let key = keys::key_counter(counter_id);
    match db.get(&key).await? {
        Some(data) => Ok(values::decode_counter(&data)?),
        None => Ok(0),
    }
}

async fn load_system_counter(db: &Db, suffix: &[u8]) -> CatalogResult<u64> {
    let key = keys::key_system(suffix);
    match db.get(&key).await? {
        Some(data) => Ok(values::decode_counter(&data)?),
        None => Ok(0),
    }
}

async fn load_format_version(db: &Db) -> CatalogResult<u32> {
    let key = keys::key_system(SYSTEM_CATALOG_FORMAT_VERSION);
    match db.get(&key).await? {
        Some(data) => Ok(values::decode_format_version(&data)?),
        None => Ok(0),
    }
}

async fn count_tag(db: &Db, tag: u8) -> CatalogResult<u64> {
    let prefix = keys::prefix_for_tag(tag);
    let mut count = 0u64;
    let mut iter = db.scan_prefix(&prefix).await?;
    while iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        .is_some()
    {
        count += 1;
    }
    Ok(count)
}
