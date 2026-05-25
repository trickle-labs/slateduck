//! SQL dispatch errors.

/// Errors from SQL dispatch operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SqlDispatchError {
    #[error("unsupported SQL statement (SQLSTATE 0A000): {0}")]
    Unsupported(String),
    #[error("SQL parse error: {0}")]
    ParseError(String),
    #[error("missing parameter ${0}")]
    MissingParam(usize),
    #[error("type mismatch for parameter ${idx}: expected {expected}, got {actual}")]
    TypeMismatch {
        idx: usize,
        expected: &'static str,
        actual: String,
    },
    /// SQLSTATE 42602: invalid channel name for LISTEN/UNLISTEN.
    #[error("invalid channel name '{channel}' (SQLSTATE 42602): {reason}")]
    InvalidChannelName { channel: String, reason: String },
}
