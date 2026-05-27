//! Pre-parser classifiers for LISTEN/UNLISTEN that sqlparser-rs cannot handle directly.

use crate::error::SqlDispatchError;

use super::StatementKind;

/// Validate a PostgreSQL channel name.
///
/// Rules: 1–63 characters; starts with a letter or underscore; remaining
/// characters are letters, digits, or underscores.  Returns `Err` with
/// SQLSTATE 42602 message for invalid names.
pub(super) fn validate_channel_name(channel: &str) -> Result<(), SqlDispatchError> {
    if channel.is_empty() || channel.len() > 63 {
        return Err(SqlDispatchError::InvalidChannelName {
            channel: channel.to_string(),
            reason: "channel name must be 1–63 characters".to_string(),
        });
    }
    let mut chars = channel.chars();
    let first = chars.next().unwrap();
    if !first.is_alphabetic() && first != '_' {
        return Err(SqlDispatchError::InvalidChannelName {
            channel: channel.to_string(),
            reason: "channel name must start with a letter or underscore".to_string(),
        });
    }
    for c in chars {
        if !c.is_alphanumeric() && c != '_' {
            return Err(SqlDispatchError::InvalidChannelName {
                channel: channel.to_string(),
                reason: format!("invalid character '{c}' in channel name"),
            });
        }
    }
    Ok(())
}

/// Pre-parse LISTEN/UNLISTEN which are non-standard keywords in many dialects.
/// Returns `Err` with SQLSTATE 42602 for invalid channel names.
pub(super) fn classify_listen_prefix(sql: &str) -> Option<Result<StatementKind, SqlDispatchError>> {
    let upper = sql.trim().to_uppercase();
    let trimmed = sql.trim();

    if upper.starts_with("LISTEN ") {
        let channel = trimmed["LISTEN ".len()..]
            .trim()
            .trim_end_matches(';')
            .trim()
            .to_string();
        return Some(validate_channel_name(&channel).map(|_| StatementKind::Listen { channel }));
    }

    if upper.starts_with("UNLISTEN ") {
        let channel = trimmed["UNLISTEN ".len()..]
            .trim()
            .trim_end_matches(';')
            .trim()
            .to_string();
        return Some(validate_channel_name(&channel).map(|_| StatementKind::Unlisten { channel }));
    }

    None
}
