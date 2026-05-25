//! Classifier tests for v0.21: quoted identifiers, AS edge cases, invalid LISTEN channels.

use slateduck_sql::error::SqlDispatchError;
use slateduck_sql::{classify_statement, StatementKind};

// ─── Quoted identifier tests ─────────────────────────────────────────────────

#[test]
fn classify_quoted_schema_name() {
    // "My Schema".my_table should be recognised as a SELECT from an
    // extension schema table, not mis-parsed as a plain table.
    let sql = r#"SELECT * FROM "My Schema".my_table"#;
    let kind = classify_statement(sql).unwrap();
    // The classifier should recognise the dot-separated qualified name and
    // return a SelectExtensionTable variant.
    assert!(
        matches!(kind, StatementKind::SelectExtensionTable { .. }),
        "expected SelectExtensionTable, got {kind:?}"
    );
}

#[test]
fn classify_quoted_name_with_embedded_dot() {
    // A quoted identifier that contains a dot inside the quotes should NOT be
    // split at that dot — the whole quoted token is one identifier.
    let sql = r#"SELECT * FROM "schema.with.dots".tbl"#;
    let kind = classify_statement(sql).unwrap();
    // The outer schema name is "schema.with.dots" (a single identifier), the
    // table name is "tbl".  This should produce an extension-table SELECT.
    assert!(
        matches!(kind, StatementKind::SelectExtensionTable { ref schema_name, .. }
            if schema_name == "schema.with.dots"),
        "expected SelectExtensionTable with quoted schema, got {kind:?}"
    );
}

// ─── AS keyword edge cases ───────────────────────────────────────────────────

#[test]
fn classify_ivm_create_as_no_surrounding_spaces() {
    // "CREATE INCREMENTAL MATERIALIZED VIEW v1 AS SELECT …"
    // The AS keyword should be detected even without surrounding spaces.
    let sql = "CREATE INCREMENTAL MATERIALIZED VIEW v1 AS(SELECT region, COUNT(*) AS cnt FROM sales GROUP BY region)";
    let kind = classify_statement(sql).unwrap();
    assert!(
        matches!(kind, StatementKind::CreateIncrementalMatview { .. }),
        "expected CreateIncrementalMatview, got {kind:?}"
    );
}

#[test]
fn classify_ivm_create_as_with_newline() {
    // AS on a new line should still be found.
    let sql = "CREATE INCREMENTAL MATERIALIZED VIEW v1\nAS\nSELECT region, COUNT(*) FROM sales GROUP BY region";
    let kind = classify_statement(sql).unwrap();
    assert!(
        matches!(kind, StatementKind::CreateIncrementalMatview { .. }),
        "expected CreateIncrementalMatview, got {kind:?}"
    );
}

// ─── LISTEN channel validation tests ─────────────────────────────────────────

#[test]
fn classify_listen_valid_channel() {
    let sql = "LISTEN my_channel";
    let kind = classify_statement(sql).unwrap();
    assert_eq!(
        kind,
        StatementKind::Listen {
            channel: "my_channel".to_string()
        }
    );
}

#[test]
fn classify_listen_valid_channel_with_underscores() {
    let sql = "LISTEN slateduck_events_v2";
    let kind = classify_statement(sql).unwrap();
    assert_eq!(
        kind,
        StatementKind::Listen {
            channel: "slateduck_events_v2".to_string()
        }
    );
}

#[test]
fn classify_listen_empty_channel_returns_error() {
    // An empty channel name is invalid per PostgreSQL identifier rules.
    let sql = "LISTEN \"\"";
    let result = classify_statement(sql);
    match result {
        Err(SqlDispatchError::InvalidChannelName { .. }) => {}
        other => panic!("expected InvalidChannelName error, got {other:?}"),
    }
}

#[test]
fn classify_listen_leading_digit_returns_error() {
    // Identifiers must not start with a digit.
    let sql = "LISTEN 3bad_channel";
    let result = classify_statement(sql);
    match result {
        Err(SqlDispatchError::InvalidChannelName { .. }) => {}
        other => panic!("expected InvalidChannelName error, got {other:?}"),
    }
}

#[test]
fn classify_listen_too_long_channel_returns_error() {
    // PostgreSQL max identifier length is 63 bytes.
    let long_name = "a".repeat(64);
    let sql = format!("LISTEN {long_name}");
    let result = classify_statement(&sql);
    match result {
        Err(SqlDispatchError::InvalidChannelName { .. }) => {}
        other => panic!("expected InvalidChannelName error, got {other:?}"),
    }
}

#[test]
fn classify_listen_max_length_valid() {
    // Exactly 63 characters is valid.
    let name = "a".repeat(63);
    let sql = format!("LISTEN {name}");
    let kind = classify_statement(&sql).unwrap();
    assert_eq!(kind, StatementKind::Listen { channel: name });
}

#[test]
fn classify_unlisten_valid_channel() {
    let sql = "UNLISTEN my_channel";
    let kind = classify_statement(sql).unwrap();
    assert_eq!(
        kind,
        StatementKind::Unlisten {
            channel: "my_channel".to_string()
        }
    );
}
