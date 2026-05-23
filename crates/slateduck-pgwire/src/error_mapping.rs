//! SQLSTATE error mapping.
//!
//! All errors flow through `to_pg_error` which maps SlateDuckError
//! to a PostgreSQL ErrorResponse with proper SQLSTATE codes.

use pgwire::error::ErrorInfo;
use slateduck_core::SlateDuckError;
use slateduck_sql::DispatchError;

/// Map a SlateDuckError to a PostgreSQL ErrorInfo.
pub fn to_pg_error(err: &SlateDuckError) -> ErrorInfo {
    match err {
        SlateDuckError::WriterFenced => {
            ErrorInfo::new("FATAL".to_string(), "57P04".to_string(), err.to_string())
        }
        SlateDuckError::FormatVersionMismatch { .. } => {
            ErrorInfo::new("ERROR".to_string(), "0A000".to_string(), err.to_string())
        }
        SlateDuckError::CatalogNotInitialized => {
            ErrorInfo::new("FATAL".to_string(), "3D000".to_string(), err.to_string())
        }
        SlateDuckError::TransactionConflict(_) => {
            ErrorInfo::new("ERROR".to_string(), "40001".to_string(), err.to_string())
        }
        SlateDuckError::ValueTooLarge { .. } => {
            ErrorInfo::new("ERROR".to_string(), "54001".to_string(), err.to_string())
        }
        SlateDuckError::FeatureNotSupported(_) => {
            ErrorInfo::new("ERROR".to_string(), "0A000".to_string(), err.to_string())
        }
        SlateDuckError::MagicMismatch(_) => {
            ErrorInfo::new("ERROR".to_string(), "XX001".to_string(), err.to_string())
        }
        SlateDuckError::UnknownEncodingVersion(_) => {
            ErrorInfo::new("ERROR".to_string(), "22P02".to_string(), err.to_string())
        }
        SlateDuckError::Encoding(_) => {
            ErrorInfo::new("ERROR".to_string(), "22P02".to_string(), err.to_string())
        }
        SlateDuckError::ObjectStore(_) => {
            ErrorInfo::new("ERROR".to_string(), "08006".to_string(), err.to_string())
        }
        SlateDuckError::SlateDb(msg) => {
            if msg.contains("fenced") || msg.contains("Fenced") {
                ErrorInfo::new("FATAL".to_string(), "57P04".to_string(), msg.clone())
            } else {
                ErrorInfo::new("ERROR".to_string(), "XX000".to_string(), msg.clone())
            }
        }
        SlateDuckError::UnknownTag(_) => {
            ErrorInfo::new("ERROR".to_string(), "XX001".to_string(), err.to_string())
        }
        SlateDuckError::Internal(_) => {
            ErrorInfo::new("ERROR".to_string(), "XX000".to_string(), err.to_string())
        }
    }
}

/// Map a DispatchError to a PostgreSQL ErrorInfo.
pub fn dispatch_to_pg_error(err: &DispatchError) -> ErrorInfo {
    match err {
        DispatchError::Unsupported(msg) => ErrorInfo::new(
            "ERROR".to_string(),
            "0A000".to_string(),
            format!("feature not supported: {msg}"),
        ),
        DispatchError::ParseError(msg) => ErrorInfo::new(
            "ERROR".to_string(),
            "42601".to_string(),
            format!("syntax error: {msg}"),
        ),
        DispatchError::InvalidValue(msg) => ErrorInfo::new(
            "ERROR".to_string(),
            "22023".to_string(),
            format!("invalid parameter value: {msg}"),
        ),
    }
}

/// Create a "row not found" error.
pub fn row_not_found_error(msg: &str) -> ErrorInfo {
    ErrorInfo::new("ERROR".to_string(), "02000".to_string(), msg.to_string())
}

/// Create a "duplicate key" error.
pub fn duplicate_key_error(msg: &str) -> ErrorInfo {
    ErrorInfo::new("ERROR".to_string(), "23505".to_string(), msg.to_string())
}

/// Create a "permission denied" error.
pub fn permission_denied_error(msg: &str) -> ErrorInfo {
    ErrorInfo::new("ERROR".to_string(), "42501".to_string(), msg.to_string())
}

/// Create a "read-only" error.
pub fn read_only_error() -> ErrorInfo {
    ErrorInfo::new(
        "ERROR".to_string(),
        "25006".to_string(),
        "cannot execute write operations on a read-only connection".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_fenced_maps_to_57p04() {
        let err = SlateDuckError::WriterFenced;
        let info = to_pg_error(&err);
        assert_eq!(info.code, "57P04");
        assert_eq!(info.severity, "FATAL");
    }

    #[test]
    fn format_version_mismatch_maps_to_0a000() {
        let err = SlateDuckError::FormatVersionMismatch {
            expected: 1,
            actual: 2,
        };
        let info = to_pg_error(&err);
        assert_eq!(info.code, "0A000");
    }

    #[test]
    fn catalog_not_initialized_maps_to_3d000() {
        let err = SlateDuckError::CatalogNotInitialized;
        let info = to_pg_error(&err);
        assert_eq!(info.code, "3D000");
        assert_eq!(info.severity, "FATAL");
    }

    #[test]
    fn transaction_conflict_maps_to_40001() {
        let err = SlateDuckError::TransactionConflict("conflict".to_string());
        let info = to_pg_error(&err);
        assert_eq!(info.code, "40001");
    }

    #[test]
    fn value_too_large_maps_to_54001() {
        let err = SlateDuckError::ValueTooLarge {
            size: 100_000_000,
            limit: 67_108_864,
        };
        let info = to_pg_error(&err);
        assert_eq!(info.code, "54001");
    }

    #[test]
    fn feature_not_supported_maps_to_0a000() {
        let err = SlateDuckError::FeatureNotSupported("foo".to_string());
        let info = to_pg_error(&err);
        assert_eq!(info.code, "0A000");
    }

    #[test]
    fn magic_mismatch_maps_to_xx001() {
        let err = SlateDuckError::MagicMismatch(vec![0, 1, 2, 3]);
        let info = to_pg_error(&err);
        assert_eq!(info.code, "XX001");
    }

    #[test]
    fn dispatch_unsupported_maps_to_0a000() {
        let err = DispatchError::Unsupported("CREATE INDEX".to_string());
        let info = dispatch_to_pg_error(&err);
        assert_eq!(info.code, "0A000");
    }

    #[test]
    fn dispatch_parse_error_maps_to_42601() {
        let err = DispatchError::ParseError("near X".to_string());
        let info = dispatch_to_pg_error(&err);
        assert_eq!(info.code, "42601");
    }

    #[test]
    fn dispatch_invalid_value_maps_to_22023() {
        let err = DispatchError::InvalidValue("bad id".to_string());
        let info = dispatch_to_pg_error(&err);
        assert_eq!(info.code, "22023");
    }
}
