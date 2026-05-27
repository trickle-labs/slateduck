//! SQLSTATE error mapping.
//!
//! All errors flow through `to_pg_error` for consistent SQLSTATE codes.

use pgwire::error::{ErrorInfo, PgWireError};
use rocklake_catalog::CatalogError;

/// Rocklake unified error type.
#[derive(Debug, thiserror::Error)]
pub enum RocklakeError {
    #[error("catalog error: {0}")]
    Catalog(#[from] CatalogError),

    #[error("SQL dispatch error: {0}")]
    SqlDispatch(#[from] rocklake_sql::SqlDispatchError),

    #[error("writer fenced (SQLSTATE 57P04)")]
    WriterFenced,

    #[error("snapshot out of retention window (SQLSTATE 22023)")]
    SnapshotOutOfRetention,

    #[error("object store error (SQLSTATE 08006): {0}")]
    ObjectStore(String),

    #[error("row not found (SQLSTATE 02000): {0}")]
    NotFound(String),

    #[error("value decode error (SQLSTATE 22P02): {0}")]
    ValueDecode(String),

    #[error("magic mismatch / corruption (SQLSTATE XX001): {0}")]
    Corruption(String),

    #[error("ID counter conflict (SQLSTATE 40001)")]
    CounterConflict,

    #[error("duplicate / PK collision (SQLSTATE 23505): {0}")]
    Duplicate(String),

    #[error("write to read-only replica (SQLSTATE 25006)")]
    ReadOnlyReplica,

    #[error("unsupported feature (SQLSTATE 0A000): {0}")]
    Unsupported(String),

    #[error("permission denied (SQLSTATE 42501): {0}")]
    PermissionDenied(String),

    #[error("catalog not initialized (SQLSTATE 3D000)")]
    CatalogNotInitialized,

    #[error("internal error (SQLSTATE XX000): {0}")]
    Internal(String),

    #[error("transaction batch too large (SQLSTATE 54001)")]
    BatchTooLarge,

    #[error("pgwire error: {0}")]
    PgWire(String),

    #[error("missing required parameter '{name}' (SQLSTATE 22023)")]
    MissingParam { name: String },

    #[error("{message} (SQLSTATE {code})")]
    SqlState { code: String, message: String },
}

impl RocklakeError {
    /// Map to PostgreSQL SQLSTATE code.
    pub fn sqlstate(&self) -> &str {
        match self {
            Self::WriterFenced => "57P04",
            Self::SnapshotOutOfRetention => "22023",
            Self::MissingParam { .. } => "22023",
            Self::ObjectStore(_) => "08006",
            Self::NotFound(_) => "02000",
            Self::ValueDecode(_) => "22P02",
            Self::Corruption(_) => "XX001",
            Self::CounterConflict => "40001",
            Self::Duplicate(_) => "23505",
            Self::ReadOnlyReplica => "25006",
            Self::Unsupported(_) => "0A000",
            Self::PermissionDenied(_) => "42501",
            Self::CatalogNotInitialized => "3D000",
            Self::Internal(_) => "XX000",
            Self::BatchTooLarge => "54001",
            Self::PgWire(_) => "XX000",
            Self::Catalog(e) => catalog_error_sqlstate(e),
            Self::SqlDispatch(_) => "0A000",
            // v0.19: Return the stored code, not a hardcoded "55000".
            Self::SqlState { code, .. } => code.as_str(),
        }
    }

    /// Map to PostgreSQL severity.
    pub fn severity(&self) -> &'static str {
        match self {
            Self::WriterFenced | Self::CatalogNotInitialized => "FATAL",
            _ => "ERROR",
        }
    }

    /// Convert to a PgWire ErrorInfo.
    pub fn to_pg_error_info(&self) -> ErrorInfo {
        ErrorInfo::new(
            self.severity().to_string(),
            self.sqlstate().to_string(),
            self.to_string(),
        )
    }
}

fn catalog_error_sqlstate(e: &CatalogError) -> &'static str {
    match e {
        CatalogError::FormatVersionMismatch { .. } => "0A000",
        CatalogError::NotInitialized => "3D000",
        CatalogError::WriterEpochMismatch => "57P04",
        CatalogError::NotFound(_) => "02000",
        CatalogError::Duplicate(_) => "23505",
        CatalogError::ValueTooLarge { .. } => "54001",
        CatalogError::TransactionConflict(_) => "40001",
        CatalogError::Value(_) => "22P02",
        CatalogError::SnapshotOutOfRetention { .. } => "22023",
        _ => "XX000",
    }
}

impl From<RocklakeError> for PgWireError {
    fn from(e: RocklakeError) -> PgWireError {
        PgWireError::UserError(Box::new(e.to_pg_error_info()))
    }
}
