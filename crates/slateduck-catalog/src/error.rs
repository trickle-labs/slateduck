//! Catalog store errors.

use slateduck_core::keys::KeyError;
use slateduck_core::values::ValueError;

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
    TypeCompare(#[from] slateduck_core::types::TypeCompareError),
    #[error("transaction conflict: {0}")]
    TransactionConflict(String),
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
}

impl From<slatedb::Error> for CatalogError {
    fn from(e: slatedb::Error) -> Self {
        Self::SlateDb(e.to_string())
    }
}

pub type CatalogResult<T> = Result<T, CatalogError>;
