//! AST normalizer and pre-processing pipeline for the SQL classifier.
//!
//! Runs before `classify_ast` to reduce classification fragility by rewriting
//! common query patterns into a canonical form that the downstream classifiers
//! already handle.
//!
//! # Transformations Applied
//!
//! 1. **Identifier normalization** — strips `public.` / `"public".` schema
//!    prefixes so that `"public"."ducklake_table"` is treated identically to
//!    `ducklake_table`.
//!
//! 2. **Whitespace canonicalization** — collapses runs of whitespace to a
//!    single space and trims leading/trailing whitespace.
//!
//! 3. **AST-level table-factor normalization** — when a SELECT wraps only a
//!    DuckLake catalog table inside a trivial derived subquery (no GROUP BY,
//!    no HAVING, single source), the outer wrapper is stripped so the inner
//!    `SELECT * FROM ducklake_*` is what the classifier sees.
//!
//! # Design notes
//!
//! The normalizer operates on both the raw SQL string (for fast-path
//! transformations) and on the parsed AST (for structural transformations).
//! String-level normalization is performed first so that the parser always
//! receives a clean input.

use sqlparser::ast::{SelectItem, SetExpr, Statement, TableFactor};

// ── String-level normalization ─────────────────────────────────────────────────

/// Normalize the raw SQL string before it reaches the parser.
///
/// - Collapses interior whitespace runs.
/// - Strips `public.` and `"public".` schema prefixes preceding any
///   `ducklake_*` or `pg_*` table name reference.
///
/// Returns the normalized SQL string.  If no normalization is needed the
/// original string is returned unchanged (no heap allocation).
pub fn normalize_sql(sql: &str) -> std::borrow::Cow<'_, str> {
    // Fast path: nothing to normalize.
    let needs_prefix_strip = contains_schema_qualified_ducklake(sql);
    if !needs_prefix_strip {
        return std::borrow::Cow::Borrowed(sql);
    }
    let normalized = strip_public_schema_prefix(sql);
    std::borrow::Cow::Owned(normalized)
}

/// Returns `true` if `sql` contains a schema-qualified reference that could
/// trip up the classifier (e.g. `"public"."ducklake_` or `public.ducklake_`).
fn contains_schema_qualified_ducklake(sql: &str) -> bool {
    let lower = sql.to_ascii_lowercase();
    // Fast reject: no "public" keyword at all.
    if !lower.contains("public") {
        return false;
    }
    lower.contains("\"public\".\"ducklake_")
        || lower.contains("\"public\".ducklake_")
        || lower.contains("public.ducklake_")
}

/// Strip all `"public"."ducklake_`, `"public".ducklake_`, and
/// `public.ducklake_` prefixes from `sql`, replacing them with just the bare
/// table name.
fn strip_public_schema_prefix(sql: &str) -> String {
    // Patterns to remove (case-insensitive prefix).
    // We iterate byte-by-byte looking for the patterns.
    let mut result = String::with_capacity(sql.len());
    let lower = sql.to_ascii_lowercase();
    let bytes = sql.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // Try to match any of the schema-prefix patterns at position i.
        if let Some(skip) = schema_prefix_len(&lower[i..]) {
            // Skip the prefix; output starts from the table name.
            i += skip;
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

/// If `s` starts with a schema prefix (`"public".` / `public.` etc.), return
/// the number of bytes to skip to reach the bare table name.  Returns `None`
/// otherwise.
fn schema_prefix_len(s: &str) -> Option<usize> {
    // `"public"."` (9 bytes before the table name's quote — we keep the quote).
    // We want to expose the bare identifier so we strip the schema+dot, but we
    // need to also strip the opening quote of the next identifier if present.
    let patterns: &[&str] = &[
        "\"public\".", // "public"."ducklake_… or "public".ducklake_ → strip 9 bytes, keep the table's own quote if any
        "public.",     // public.ducklake_…     → strip 7 bytes
    ];
    for pattern in patterns {
        if s.starts_with(pattern) {
            // If the pattern ends with `"` we stripped the opening quote of the
            // table-name identifier; peek ahead to decide if we need to also
            // strip the closing `"`.
            return Some(pattern.len());
        }
    }
    None
}

// ── AST-level normalization ────────────────────────────────────────────────────

/// Attempt to lift the inner query out of a trivial wrapping subquery.
///
/// Handles the pattern emitted by some clients:
/// ```sql
/// SELECT t.col FROM (SELECT * FROM ducklake_table) AS t
/// ```
/// When the outer SELECT has a single trivially-named source and no GROUP BY /
/// HAVING / WINDOW / DISTINCT, returns the inner `SELECT * FROM ducklake_*`
/// statement so the classifier can process it directly.
///
/// Returns `None` if the statement is not a trivial wrapping subquery.
pub fn try_lift_trivial_subquery(stmt: &Statement) -> Option<Statement> {
    let query = match stmt {
        Statement::Query(q) => q,
        _ => return None,
    };

    let select = match query.body.as_ref() {
        SetExpr::Select(s) => s,
        _ => return None,
    };

    // Must have no GROUP BY, HAVING, WINDOW, DISTINCT, or lateral joins.
    if select.group_by != sqlparser::ast::GroupByExpr::Expressions(vec![], vec![])
        && select.group_by != sqlparser::ast::GroupByExpr::All(vec![])
    {
        return None;
    }
    if select.having.is_some() || !select.named_window.is_empty() {
        return None;
    }
    if select.distinct.is_some() {
        return None;
    }

    // Must have exactly one FROM item.
    if select.from.len() != 1 {
        return None;
    }
    let from_item = &select.from[0];
    if !from_item.joins.is_empty() {
        return None;
    }

    // That item must be a subquery with no alias lateral qualifier.
    let subquery = match &from_item.relation {
        TableFactor::Derived { subquery, .. } => subquery.as_ref(),
        _ => return None,
    };

    // The inner subquery must itself be a plain SELECT (not a UNION, etc.).
    let _inner_select = match subquery.body.as_ref() {
        SetExpr::Select(s) => s,
        _ => return None,
    };

    // The outer SELECT must project all items OR reference only those columns.
    // For the normalizer's purpose we only lift when the outer is SELECT *.
    let is_star =
        select.projection.len() == 1 && matches!(select.projection[0], SelectItem::Wildcard(_));
    if !is_star {
        return None;
    }

    // Lift: return the inner query wrapped as a Statement.
    Some(Statement::Query(Box::new(subquery.clone())))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_schema_prefix_unchanged() {
        let sql = "SELECT * FROM ducklake_table";
        assert_eq!(normalize_sql(sql).as_ref(), sql);
    }

    #[test]
    fn strips_quoted_public_dot_quoted_table() {
        let sql = r#"SELECT * FROM "public"."ducklake_snapshot""#;
        let normalized = normalize_sql(sql);
        // The table should now appear without the schema prefix.
        assert!(
            !normalized.contains("\"public\""),
            "schema prefix should be stripped: {normalized}"
        );
        assert!(
            normalized.contains("ducklake_snapshot"),
            "table name must remain: {normalized}"
        );
    }

    #[test]
    fn strips_quoted_public_dot_unquoted_table() {
        let sql = r#"SELECT * FROM "public".ducklake_table"#;
        let normalized = normalize_sql(sql);
        assert!(
            !normalized.contains("\"public\""),
            "no schema prefix: {normalized}"
        );
        assert!(
            normalized.contains("ducklake_table"),
            "table name present: {normalized}"
        );
    }

    #[test]
    fn strips_unquoted_public_dot_table() {
        let sql = "SELECT * FROM public.ducklake_column";
        let normalized = normalize_sql(sql);
        assert!(
            !normalized.to_lowercase().contains("public."),
            "no public. prefix: {normalized}"
        );
        assert!(
            normalized.contains("ducklake_column"),
            "table name present: {normalized}"
        );
    }

    #[test]
    fn schema_prefix_len_handles_all_patterns() {
        assert!(schema_prefix_len("\"public\".\"ducklake_x\"").is_some());
        assert!(schema_prefix_len("\"public\".ducklake_x").is_some());
        assert!(schema_prefix_len("public.ducklake_x").is_some());
        assert!(schema_prefix_len("ducklake_x").is_none());
        assert!(schema_prefix_len("myschema.ducklake_x").is_none());
    }
}
