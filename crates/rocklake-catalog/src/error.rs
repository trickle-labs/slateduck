//! Catalog store errors.

#![allow(missing_docs)]

use rocklake_core::keys::KeyError;
use rocklake_core::values::ValueError;

/// Errors that can occur in catalog operations.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("SlateDB error: {0}")]
    SlateDb(String),
    #[error("key error: {0}")]
    Key(#[from] KeyError),
    #[error("value error: {0}")]
    Value(#[from] ValueError),
    #[error("catalog format version mismatch: expected {expected}, got {actual} (SQLSTATE 0A000)")]
    FormatVersionMismatch { expected: u32, actual: u32 },
    #[error("catalog not initialized")]
    NotInitialized,
    #[error("writer epoch mismatch: another writer is active (SQLSTATE 57P04)")]
    WriterEpochMismatch,
    #[error("entity not found: {0}")]
    NotFound(String),
    #[error("duplicate entity: {0}")]
    Duplicate(String),
    #[error("value too large: {size} bytes exceeds 64 MiB limit (SQLSTATE 54001)")]
    ValueTooLarge { size: usize },
    #[error("type comparison error: {0}")]
    TypeCompare(#[from] rocklake_core::types::TypeCompareError),
    #[error("transaction conflict: {0}")]
    TransactionConflict(String),
    /// Transient object-store error (network glitch, throttling, 503).
    /// Operations that fail with this variant may be safely retried.
    #[error("transient object-store error: {0}")]
    ObjectStoreTransient(String),
    /// Permanent object-store error (permission denied, bucket not found, 404).
    /// Retrying is unlikely to help.
    #[error("permanent object-store error: {0}")]
    ObjectStorePermanent(String),
    /// Catalog data corruption detected (checksum mismatch, unexpected format).
    #[error("catalog corruption: {0}")]
    Corruption(String),
    #[error("pinned snapshot {pinned_snapshot} blocks retain-from advancement to {requested_retain_from}")]
    PinnedSnapshotBlocks {
        pinned_snapshot: u64,
        requested_retain_from: u64,
    },
    #[error("excision unsafe: retain-from ({retain_from}) has not been advanced past before_snapshot ({before_snapshot})")]
    ExcisionUnsafe {
        retain_from: u64,
        before_snapshot: u64,
    },
    #[error("repair refused: {0}")]
    RepairRefused(String),
    #[error("snapshot {requested} is below the retention floor {retain_from} (SQLSTATE 22023)")]
    SnapshotOutOfRetention { requested: u64, retain_from: u64 },
    #[error("import error at line {line} (table {table}): {message}")]
    Import {
        line: usize,
        table: String,
        message: String,
    },
    #[error("internal error: {0}")]
    Internal(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("generation mismatch: expected {expected}, actual {actual} (CAS conflict)")]
    GenerationMismatch { expected: u64, actual: u64 },
    #[error("unsupported DuckLake catalog version {version}: {message} (SQLSTATE 0A000)")]
    UnsupportedDuckLakeVersion { version: u64, message: String },
    #[error("migration source error: {0}")]
    MigrationSource(String),
}

impl From<slatedb::Error> for CatalogError {
    fn from(e: slatedb::Error) -> Self {
        classify_slatedb_error(e)
    }
}

/// Classify a SlateDB error into the appropriate `CatalogError` variant.
///
/// - Transient errors (network issues, throttling, temporary failures) map to
///   `ObjectStoreTransient` so callers can retry safely.
/// - Permanent errors (permissions, missing resources) map to
///   `ObjectStorePermanent`.
/// - Checksum / format errors map to `Corruption`.
/// - Write conflicts map to `TransactionConflict`.
/// - Everything else maps to `SlateDb(String)` for backward compatibility.
fn classify_slatedb_error(e: slatedb::Error) -> CatalogError {
    let msg = e.to_string();
    let lower = msg.to_ascii_lowercase();

    // Transaction conflicts → TransactionConflict
    if lower.contains("conflict") || lower.contains("concurrent") || lower.contains("cas") {
        return CatalogError::TransactionConflict(msg);
    }

    // Checksum / corruption → Corruption
    if lower.contains("checksum") || lower.contains("corrupt") || lower.contains("invalid data") {
        return CatalogError::Corruption(msg);
    }

    // Transient object-store errors (network, throttling, 5xx)
    if lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("connection")
        || lower.contains("throttl")
        || lower.contains("service unavailable")
        || lower.contains("503")
        || lower.contains("temporary")
        || lower.contains("retry")
    {
        return CatalogError::ObjectStoreTransient(msg);
    }

    // Permanent object-store errors (auth, missing bucket, 403/404)
    if lower.contains("permission")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("not found")
        || lower.contains("404")
        || lower.contains("403")
        || lower.contains("no such")
    {
        return CatalogError::ObjectStorePermanent(msg);
    }

    CatalogError::SlateDb(msg)
}

/// Whether this error is safe to retry.
pub fn is_transient(e: &CatalogError) -> bool {
    matches!(e, CatalogError::ObjectStoreTransient(_))
}

/// Retry an async operation up to `max_attempts` times, retrying only on
/// `ObjectStoreTransient` errors.  Returns the last error if all attempts fail.
///
/// # Examples
///
/// ```no_run
/// # async fn example() -> rocklake_catalog::error::CatalogResult<()> {
/// use rocklake_catalog::error::with_transient_retry;
///
/// let result = with_transient_retry(3, || async {
///     // some catalog operation
///     Ok(())
/// }).await;
/// # result
/// # }
/// ```
pub async fn with_transient_retry<F, Fut, T>(max_attempts: u32, f: F) -> CatalogResult<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = CatalogResult<T>>,
{
    let mut last_err = None;
    for attempt in 0..max_attempts {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if is_transient(&e) => {
                tracing::warn!(
                    attempt = attempt + 1,
                    max = max_attempts,
                    error = %e,
                    "transient catalog error — retrying"
                );
                last_err = Some(e);
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_err.expect("max_attempts must be > 0"))
}

pub type CatalogResult<T> = Result<T, CatalogError>;
