//! Pre-parser classifiers for IVM DDL and LISTEN/UNLISTEN that sqlparser-rs
//! cannot handle directly.

use crate::error::SqlDispatchError;

use super::table_selects::{find_as_keyword, split_qualified_name};
use super::StatementKind;

/// Fast string-based pre-classifier for IVM DDL statements that sqlparser-rs
/// cannot parse (non-standard keyword combinations like INCREMENTAL).
pub(super) fn classify_ivm_prefix(sql: &str) -> Option<StatementKind> {
    let upper = sql.trim().to_uppercase();
    let trimmed = sql.trim();

    if upper.starts_with("CREATE INCREMENTAL MATERIALIZED VIEW") {
        // Extract "[[schema.]name] AS ..."
        let rest = &trimmed["CREATE INCREMENTAL MATERIALIZED VIEW".len()..].trim_start();
        // Split off the AS clause.
        let (name_part, select_sql) = if let Some(pos) = find_as_keyword(rest) {
            (&rest[..pos].trim(), rest[pos + 2..].trim().to_string())
        } else {
            return Some(StatementKind::Unsupported(
                "CREATE INCREMENTAL MATERIALIZED VIEW missing AS clause".to_string(),
            ));
        };
        let (schema, name) = split_qualified_name(name_part);
        return Some(StatementKind::CreateIncrementalMatview {
            name,
            schema,
            select_sql,
            with_options: Vec::new(),
        });
    }

    if upper.starts_with("DROP INCREMENTAL MATERIALIZED VIEW") {
        let rest = trimmed["DROP INCREMENTAL MATERIALIZED VIEW".len()..].trim_start();
        let (if_exists, name_part) = if rest.to_uppercase().starts_with("IF EXISTS") {
            (true, rest["IF EXISTS".len()..].trim_start())
        } else {
            (false, rest)
        };
        let (schema, name) = split_qualified_name(name_part);
        return Some(StatementKind::DropIncrementalMatview {
            name,
            schema,
            if_exists,
        });
    }

    if upper.starts_with("ALTER INCREMENTAL MATERIALIZED VIEW") {
        let rest = trimmed["ALTER INCREMENTAL MATERIALIZED VIEW".len()..].trim_start();
        // Just capture the name; options parsing is a v0.12 concern.
        let (schema, name) = split_qualified_name(rest);
        return Some(StatementKind::AlterIncrementalMatview {
            name,
            schema,
            options: Vec::new(),
        });
    }

    if upper.starts_with("REFRESH INCREMENTAL MATERIALIZED VIEW") {
        let rest = trimmed["REFRESH INCREMENTAL MATERIALIZED VIEW".len()..].trim_start();
        let name_part = rest
            .trim_end_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '_')
            .trim();
        let (schema, name) = split_qualified_name(name_part);
        return Some(StatementKind::RefreshIncrementalMatviewFull { name, schema });
    }

    if upper.starts_with("SHOW MATERIALIZED VIEWS") {
        return Some(StatementKind::ShowMaterializedViews);
    }

    if upper.starts_with("SHOW MATVIEW SHARDS") {
        let rest = trimmed["SHOW MATVIEW SHARDS".len()..].trim_start();
        let (schema, name) = split_qualified_name(rest);
        return Some(StatementKind::ShowMatviewShards {
            view_name: name,
            schema,
        });
    }

    if upper.starts_with("EXPLAIN MATVIEW") {
        let rest = trimmed["EXPLAIN MATVIEW".len()..].trim_start();
        let (schema, name) = split_qualified_name(rest);
        return Some(StatementKind::ExplainMatview {
            view_name: name,
            schema,
        });
    }

    None
}

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
