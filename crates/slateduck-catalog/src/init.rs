//! Catalog initialization: safe `open_or_create` using transactions.

use slatedb::{Db, DbTransaction, IsolationLevel};
use slateduck_core::counters::CounterCache;
use slateduck_core::keys;
use slateduck_core::tags::*;
use slateduck_core::values;

use crate::error::{CatalogError, CatalogResult};

/// Initialize a fresh catalog in SlateDB if not already initialized.
/// Uses `DbTransaction` with `SerializableSnapshot` isolation.
/// Two concurrent first-connections produce exactly one coherent initial metadata set.
pub async fn initialize_catalog(db: &Db) -> CatalogResult<CounterCache> {
    let tx = db
        .begin(IsolationLevel::SerializableSnapshot)
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

    // Check if catalog is already initialized
    let format_key = keys::key_system(SYSTEM_CATALOG_FORMAT_VERSION);
    let existing = tx
        .get(&format_key)
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

    if let Some(data) = existing {
        // Already initialized — verify format version
        let version = values::decode_format_version(&data)?;
        if version != CATALOG_FORMAT_VERSION {
            return Err(CatalogError::FormatVersionMismatch {
                expected: CATALOG_FORMAT_VERSION,
                actual: version,
            });
        }

        // Load counters
        let cache = load_counters(&tx).await?;
        tx.rollback();
        return Ok(cache);
    }

    // Initialize catalog format version
    tx.put(
        format_key,
        values::encode_format_version(CATALOG_FORMAT_VERSION),
    )
    .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

    // Initialize counters: all start at 1
    let initial_snapshot_id = 1u64;
    let initial_catalog_id = 1u64;
    let initial_file_id = 1u64;

    tx.put(
        keys::key_counter(COUNTER_NEXT_SNAPSHOT_ID),
        values::encode_counter(initial_snapshot_id),
    )
    .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

    tx.put(
        keys::key_counter(COUNTER_NEXT_CATALOG_ID),
        values::encode_counter(initial_catalog_id),
    )
    .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

    tx.put(
        keys::key_counter(COUNTER_NEXT_FILE_ID),
        values::encode_counter(initial_file_id),
    )
    .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

    // Initialize retain-from to 0 (infinite retention)
    tx.put(
        keys::key_system(SYSTEM_RETAIN_FROM),
        values::encode_counter(0),
    )
    .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

    // Commit the initialization
    tx.commit()
        .await
        .map_err(|e| CatalogError::TransactionConflict(e.to_string()))?;

    Ok(CounterCache::new(
        initial_snapshot_id,
        initial_catalog_id,
        initial_file_id,
    ))
}

/// Load counter values from SlateDB within a transaction.
async fn load_counters(tx: &DbTransaction) -> CatalogResult<CounterCache> {
    let snap_data = tx
        .get(&keys::key_counter(COUNTER_NEXT_SNAPSHOT_ID))
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        .ok_or(CatalogError::NotInitialized)?;
    let next_snapshot_id = values::decode_counter(&snap_data)?;

    let cat_data = tx
        .get(&keys::key_counter(COUNTER_NEXT_CATALOG_ID))
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        .ok_or(CatalogError::NotInitialized)?;
    let next_catalog_id = values::decode_counter(&cat_data)?;

    let file_data = tx
        .get(&keys::key_counter(COUNTER_NEXT_FILE_ID))
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        .ok_or(CatalogError::NotInitialized)?;
    let next_file_id = values::decode_counter(&file_data)?;

    Ok(CounterCache::new(
        next_snapshot_id,
        next_catalog_id,
        next_file_id,
    ))
}

/// Load counter values from the database directly (non-transactional read).
pub async fn load_counters_from_db(db: &Db) -> CatalogResult<CounterCache> {
    let snap_data = db
        .get(&keys::key_counter(COUNTER_NEXT_SNAPSHOT_ID))
        .await?
        .ok_or(CatalogError::NotInitialized)?;
    let next_snapshot_id = values::decode_counter(&snap_data)?;

    let cat_data = db
        .get(&keys::key_counter(COUNTER_NEXT_CATALOG_ID))
        .await?
        .ok_or(CatalogError::NotInitialized)?;
    let next_catalog_id = values::decode_counter(&cat_data)?;

    let file_data = db
        .get(&keys::key_counter(COUNTER_NEXT_FILE_ID))
        .await?
        .ok_or(CatalogError::NotInitialized)?;
    let next_file_id = values::decode_counter(&file_data)?;

    Ok(CounterCache::new(
        next_snapshot_id,
        next_catalog_id,
        next_file_id,
    ))
}

/// Verify the catalog format version matches expectations.
pub async fn verify_format_version(db: &Db) -> CatalogResult<()> {
    let format_key = keys::key_system(SYSTEM_CATALOG_FORMAT_VERSION);
    let data = db
        .get(&format_key)
        .await?
        .ok_or(CatalogError::NotInitialized)?;

    let version = values::decode_format_version(&data)?;
    if version != CATALOG_FORMAT_VERSION {
        return Err(CatalogError::FormatVersionMismatch {
            expected: CATALOG_FORMAT_VERSION,
            actual: version,
        });
    }
    Ok(())
}
