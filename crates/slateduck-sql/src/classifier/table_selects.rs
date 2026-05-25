//! Table select classifiers and string identifier helpers.

use super::StatementKind;

pub(super) fn find_as_keyword(s: &str) -> Option<usize> {
    use sqlparser::dialect::PostgreSqlDialect;
    use sqlparser::tokenizer::{Token, Tokenizer};

    let dialect = PostgreSqlDialect {};
    let tokens = Tokenizer::new(&dialect, s).tokenize().ok()?;

    // Track byte offset through the token stream.
    let mut byte_offset = 0usize;
    let s_bytes = s.as_bytes();

    for tok in &tokens {
        // Advance byte_offset past whitespace.
        while byte_offset < s.len()
            && (s_bytes[byte_offset] == b' '
                || s_bytes[byte_offset] == b'\t'
                || s_bytes[byte_offset] == b'\n'
                || s_bytes[byte_offset] == b'\r')
        {
            byte_offset += 1;
        }

        match tok {
            Token::Word(w) if w.value.eq_ignore_ascii_case("AS") && w.quote_style.is_none() => {
                // Ensure there is a non-AS token after this one (so AS is not
                // the last token) — a trailing AS is not a valid alias marker.
                return Some(byte_offset);
            }
            _ => {
                // Skip past this token's characters.
                let tok_str = tok.to_string();
                byte_offset += tok_str.len();
            }
        }
    }

    None
}

/// Split "schema.name" or just "name" from a name fragment.
/// Handles double-quoted identifiers correctly.
/// Returns `(schema, name)`.
pub(super) fn split_qualified_name(s: &str) -> (Option<String>, String) {
    // Collect the first identifier, honouring double-quote delimiters.
    let s = s.trim();
    let (first_ident, rest_after_first) = take_sql_identifier(s);

    let rest = rest_after_first.trim_start();
    if let Some(after_dot) = rest.strip_prefix('.') {
        // schema.name — take the second identifier.
        let rest2 = after_dot.trim_start();
        let (second_ident, _) = take_sql_identifier(rest2);
        (Some(first_ident), second_ident)
    } else {
        (None, first_ident)
    }
}

/// Extract the first SQL identifier from `s`, stripping double-quote delimiters
/// when present.  Returns `(identifier, remainder_of_s_after_identifier)`.
pub(super) fn take_sql_identifier(s: &str) -> (String, &str) {
    if let Some(after_quote) = s.strip_prefix('"') {
        // Quoted identifier: scan until the closing '"', doubling "" as escape.
        let mut result = String::new();
        let mut chars = after_quote.char_indices();
        let mut end_byte = 1;
        loop {
            match chars.next() {
                None => break,
                Some((i, '"')) => {
                    end_byte = 1 + i + '"'.len_utf8();
                    // Peek for escaped double-quote ""
                    if s[end_byte..].starts_with('"') {
                        result.push('"');
                        chars.next(); // consume second "
                        end_byte += 1;
                    } else {
                        break;
                    }
                }
                Some((_, c)) => result.push(c),
            }
        }
        (result, &s[end_byte..])
    } else {
        // Unquoted identifier: alphanumeric + underscore.
        let token: String = s
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let len = token.len();
        (token, &s[len..])
    }
}

pub(super) fn classify_table_select_with_query(
    table_name: &str,
    query: &sqlparser::ast::Query,
    select: &sqlparser::ast::Select,
) -> StatementKind {
    let lower = table_name.to_lowercase();
    match lower.as_str() {
        "ducklake_snapshot" => classify_snapshot_select(query, select),
        "ducklake_schema" => StatementKind::SelectSchemas,
        "ducklake_table" => StatementKind::SelectTables,
        "ducklake_column" => StatementKind::SelectColumns,
        "ducklake_data_file" => classify_data_file_select(query),
        "ducklake_delete_file" => StatementKind::SelectDeleteFiles,
        "ducklake_file_column_stats" => StatementKind::SelectFileColumnStats,
        "ducklake_table_stats" => StatementKind::SelectTableStats,
        "ducklake_metadata" => StatementKind::SelectMetadata,
        "ducklake_inlined_data_tables" => StatementKind::SelectInlinedData,
        "ducklake_view" => StatementKind::SelectViews,
        "ducklake_macro" => StatementKind::SelectMacros,
        s if s.starts_with("pg_catalog.pg_type") || s == "pg_type" => StatementKind::SelectPgType,
        s if s.starts_with("ducklake_inlined_") => StatementKind::SelectInlinedRows,
        // Virtual catalog schema: slateduck_catalog.{table}
        s if s.starts_with("slateduck_catalog.") => {
            let table_name = s
                .strip_prefix("slateduck_catalog.")
                .unwrap_or(s)
                .to_string();
            StatementKind::VirtualCatalogScan { table_name }
        }
        // Extension schemas (e.g., pgtrickle.pgt_ducklake_provenance)
        s if s.contains('.') && !s.starts_with("pg_catalog") && !s.starts_with("ducklake_") => {
            // Use split_qualified_name to handle quoted identifiers (e.g., "My Schema".tbl).
            let (schema_opt, tbl) = split_qualified_name(table_name);
            if let Some(schema) = schema_opt {
                StatementKind::SelectExtensionTable {
                    schema_name: schema.to_lowercase(),
                    table_name: tbl.to_lowercase(),
                }
            } else {
                StatementKind::Unsupported(format!("SELECT from {s}"))
            }
        }
        _ => StatementKind::Unsupported(format!("SELECT from {table_name}")),
    }
}

/// Classify SELECT on ducklake_snapshot — detect ASC LIMIT 1 and WHERE snapshot_id > $1 patterns.
pub(super) fn classify_snapshot_select(
    query: &sqlparser::ast::Query,
    select: &sqlparser::ast::Select,
) -> StatementKind {
    // Check for ORDER BY snapshot_id ASC LIMIT 1 → SelectFirstSnapshot
    if has_order_by_asc_limit_1(query) {
        return StatementKind::SelectFirstSnapshot;
    }

    // Check for max(snapshot_id) ... WHERE snapshot_id > $1 → SelectMaxSnapshotAfter
    if has_where_snapshot_gt(select) {
        return StatementKind::SelectMaxSnapshotAfter;
    }

    StatementKind::SelectMaxSnapshot
}

/// Classify SELECT on ducklake_data_file — detect parameterized LIMIT.
pub(super) fn classify_data_file_select(query: &sqlparser::ast::Query) -> StatementKind {
    if has_parameterized_limit(query) {
        return StatementKind::SelectDataFilesWithLimit;
    }
    StatementKind::SelectDataFiles
}

/// Check if query has ORDER BY ... ASC LIMIT 1.
pub(super) fn has_order_by_asc_limit_1(query: &sqlparser::ast::Query) -> bool {
    if query.order_by.is_some() {
        if let Some(ref limit) = query.limit {
            let limit_str = limit.to_string();
            if limit_str == "1" {
                return true;
            }
        }
    }
    false
}

/// Check if the SELECT has a WHERE clause with `snapshot_id > $N`.
pub(super) fn has_where_snapshot_gt(select: &sqlparser::ast::Select) -> bool {
    if let Some(ref selection) = select.selection {
        let sel_str = selection.to_string().to_lowercase();
        if sel_str.contains("snapshot_id") && sel_str.contains(">") {
            return true;
        }
    }
    false
}

/// Check if query has a parameterized LIMIT ($N).
pub(super) fn has_parameterized_limit(query: &sqlparser::ast::Query) -> bool {
    if let Some(ref limit) = query.limit {
        let limit_str = limit.to_string();
        if limit_str.starts_with('$') {
            return true;
        }
    }
    false
}
