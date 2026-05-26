//! Classifier tests for v0.27.4: DuckDB 1.5.x postgres scanner compatibility.
//!
//! Tests the new SQL statement kinds added to handle the DuckDB 1.5.x postgres
//! scanner initialization sequence:
//!   Step 1 — `DISCARD ALL` (connection pool reset)
//!   Step 2 — `SELECT to_regclass('duckdb_secrets')` (secret storage check)
//!   Step 3 — `SELECT EXISTS(... information_schema.tables ...)` (fallback check)
//!   Step 4 — `SELECT pg_database_size(current_database())` (database size)
//!   Step 5 — Multi-statement catalog scan (pg_namespace / pg_class / pg_enum / ...)

use slateduck_sql::{classify_statement, StatementKind};

// ─── Step 1: DISCARD ALL ─────────────────────────────────────────────────────

#[test]
fn classify_discard_all() {
    let kind = classify_statement("DISCARD ALL").unwrap();
    assert_eq!(
        kind,
        StatementKind::DiscardAll,
        "DISCARD ALL must classify as DiscardAll"
    );
}

#[test]
fn classify_discard_sequences() {
    let kind = classify_statement("DISCARD SEQUENCES").unwrap();
    assert_eq!(
        kind,
        StatementKind::DiscardAll,
        "DISCARD SEQUENCES must classify as DiscardAll"
    );
}

#[test]
fn classify_discard_plans() {
    let kind = classify_statement("DISCARD PLANS").unwrap();
    assert_eq!(
        kind,
        StatementKind::DiscardAll,
        "DISCARD PLANS must classify as DiscardAll"
    );
}

#[test]
fn classify_discard_temp() {
    let kind = classify_statement("DISCARD TEMP").unwrap();
    assert_eq!(
        kind,
        StatementKind::DiscardAll,
        "DISCARD TEMP must classify as DiscardAll"
    );
}

// ─── Step 2: SELECT to_regclass ───────────────────────────────────────────────

#[test]
fn classify_select_to_regclass_duckdb_secrets() {
    let sql = "SELECT to_regclass('duckdb_secrets')";
    let kind = classify_statement(sql).unwrap();
    assert_eq!(
        kind,
        StatementKind::SelectToRegclass,
        "SELECT to_regclass('duckdb_secrets') must classify as SelectToRegclass"
    );
}

#[test]
fn classify_select_to_regclass_any_arg() {
    let sql = "SELECT to_regclass('some_other_table')";
    let kind = classify_statement(sql).unwrap();
    assert_eq!(kind, StatementKind::SelectToRegclass);
}

// ─── Step 3: SELECT EXISTS(... information_schema.tables ...) ─────────────────

#[test]
fn classify_select_exists_info_schema() {
    let sql = "SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = 'duckdb_secrets')";
    let kind = classify_statement(sql).unwrap();
    assert_eq!(
        kind,
        StatementKind::SelectExistsInfoSchema,
        "SELECT EXISTS(... information_schema.tables ...) must classify as SelectExistsInfoSchema"
    );
}

// ─── Step 4: SELECT pg_database_size ─────────────────────────────────────────

#[test]
fn classify_select_pg_database_size() {
    let sql = "SELECT pg_database_size(current_database())";
    let kind = classify_statement(sql).unwrap();
    assert_eq!(
        kind,
        StatementKind::SelectPgDatabaseSize,
        "SELECT pg_database_size(current_database()) must classify as SelectPgDatabaseSize"
    );
}

// ─── Step 5: Multi-statement catalog scan ─────────────────────────────────────

#[test]
fn classify_pg_catalog_scan_multi_statement() {
    let sql = "BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ;\n\
               SELECT oid, nspname FROM pg_namespace ORDER BY oid;\n\
               SELECT pg_namespace.oid AS namespace_id, relname, relpages, attname, \
               pg_type.typname type_name, atttypmod type_modifier, pg_attribute.attndims ndim, \
               attnum, pg_attribute.attnotnull AS notnull, NULL constraint_id, \
               NULL constraint_type, NULL constraint_key \
               FROM pg_class JOIN pg_namespace ON relnamespace = pg_namespace.oid \
               JOIN pg_attribute ON pg_class.oid=pg_attribute.attrelid \
               JOIN pg_type ON atttypid=pg_type.oid \
               WHERE attnum > 0 AND relkind IN ('r', 'v', 'm', 'f', 'p') \
               UNION ALL SELECT pg_namespace.oid AS namespace_id, relname, NULL relpages, \
               NULL attname, NULL type_name, NULL type_modifier, NULL ndim, NULL attnum, \
               NULL AS notnull, pg_constraint.oid AS constraint_id, contype AS constraint_type, \
               conkey AS constraint_key FROM pg_class JOIN pg_namespace ON relnamespace = pg_namespace.oid \
               JOIN pg_constraint ON (pg_class.oid=pg_constraint.conrelid) \
               WHERE relkind IN ('r', 'v', 'm', 'f', 'p') AND contype IN ('p', 'u') \
               ORDER BY namespace_id, relname, attnum, constraint_id;\n\
               SELECT n.oid, enumtypid, typname, enumlabel FROM pg_enum e \
               JOIN pg_type t ON e.enumtypid = t.oid \
               JOIN pg_namespace AS n ON (typnamespace=n.oid) \
               ORDER BY n.oid, enumtypid, enumsortorder;\n\
               SELECT n.oid, t.typrelid AS id, t.typname as type, pg_attribute.attname, sub_type.typname \
               FROM pg_type t JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
               JOIN pg_class ON pg_class.oid = t.typrelid \
               JOIN pg_attribute ON attrelid=t.typrelid \
               JOIN pg_type sub_type ON (pg_attribute.atttypid=sub_type.oid) \
               WHERE pg_class.relkind = 'c' AND t.typtype='c' \
               ORDER BY n.oid, t.oid, attrelid, attnum;\n\
               SELECT pg_namespace.oid, tablename, indexname FROM pg_indexes \
               JOIN pg_namespace ON (schemaname=nspname) ORDER BY pg_namespace.oid;\n\
               ROLLBACK;";
    let kind = classify_statement(sql).unwrap();
    assert_eq!(
        kind,
        StatementKind::PgCatalogScan,
        "Multi-statement pg_namespace/pg_class catalog scan must classify as PgCatalogScan"
    );
}

#[test]
fn classify_pg_catalog_scan_detects_pg_namespace_alone() {
    // Even a simplified version with just pg_namespace and pg_class should detect PgCatalogScan.
    let sql = "BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ; SELECT oid FROM pg_namespace; SELECT * FROM pg_class; ROLLBACK;";
    let kind = classify_statement(sql).unwrap();
    assert_eq!(kind, StatementKind::PgCatalogScan);
}

// ─── v0.27.5: COPY FROM STDIN (binary) support ───────────────────────────────

#[test]
fn classify_copy_from_stdin_ducklake_metadata() {
    let sql = r#"COPY "public"."ducklake_metadata" FROM STDIN (FORMAT binary)"#;
    let kind = classify_statement(sql).unwrap();
    match kind {
        StatementKind::CopyFromStdin { table } => {
            assert_eq!(table, "ducklake_metadata");
        }
        _ => panic!("Expected CopyFromStdin, got {:?}", kind),
    }
}

#[test]
fn classify_copy_from_stdin_ducklake_snapshot() {
    let sql = r#"COPY "public"."ducklake_snapshot" FROM STDIN (FORMAT binary)"#;
    let kind = classify_statement(sql).unwrap();
    match kind {
        StatementKind::CopyFromStdin { table } => {
            assert_eq!(table, "ducklake_snapshot");
        }
        _ => panic!("Expected CopyFromStdin, got {:?}", kind),
    }
}

#[test]
fn classify_copy_from_stdin_ducklake_inlined() {
    let sql = r#"COPY "public"."ducklake_inlined_data_tables" FROM STDIN (FORMAT binary)"#;
    let kind = classify_statement(sql).unwrap();
    match kind {
        StatementKind::CopyFromStdin { table } => {
            assert_eq!(table, "ducklake_inlined_data_tables");
        }
        _ => panic!("Expected CopyFromStdin, got {:?}", kind),
    }
}

#[test]
fn classify_copy_from_stdin_unquoted_schema() {
    let sql = "COPY public.ducklake_table FROM STDIN (FORMAT binary)";
    let kind = classify_statement(sql).unwrap();
    match kind {
        StatementKind::CopyFromStdin { table } => {
            assert_eq!(table, "ducklake_table");
        }
        _ => panic!("Expected CopyFromStdin, got {:?}", kind),
    }
}

#[test]
fn classify_copy_from_stdin_case_insensitive() {
    let sql = r#"copy "public"."ducklake_schema" from stdin (format binary)"#;
    let kind = classify_statement(sql).unwrap();
    match kind {
        StatementKind::CopyFromStdin { table } => {
            assert_eq!(table, "ducklake_schema");
        }
        _ => panic!("Expected CopyFromStdin, got {:?}", kind),
    }
}

#[test]
fn classify_copy_from_stdin_with_column_list() {
    // DuckDB sends COPY with column list for some tables
    let sql = r#"COPY "public"."ducklake_metadata" ("key", "value") FROM STDIN (FORMAT BINARY)"#;
    let kind = classify_statement(sql).unwrap();
    match kind {
        StatementKind::CopyFromStdin { table } => {
            assert_eq!(table, "ducklake_metadata");
        }
        _ => panic!("Expected CopyFromStdin, got {:?}", kind),
    }
}

#[test]
fn classify_copy_from_stdin_non_ducklake_table_is_unsupported() {
    // Non-ducklake tables should not be handled by the fast-path classifier.
    // The classifier returns None for non-ducklake tables, letting sqlparser
    // handle them. sqlparser will fail to parse COPY...FROM STDIN, resulting
    // in a parse error (which is correct for unsupported operations).
    let sql = r#"COPY "public"."my_regular_table" FROM STDIN (FORMAT binary)"#;
    let result = classify_statement(sql);
    // Either it parses to Unsupported OR fails to parse (both acceptable)
    match result {
        Ok(StatementKind::Unsupported(_)) => {} // expected
        Err(_) => {}                            // also acceptable (parse error)
        other => panic!("Expected Unsupported or parse error, got {:?}", other),
    }
}

#[test]
fn classify_duckdb_derived_snapshot_max_select() {
    let sql = r#"SELECT "max" FROM (
        SELECT snapshot_id, schema_version, next_catalog_id, next_file_id
        FROM "public".ducklake_snapshot
        WHERE snapshot_id = (SELECT MAX(snapshot_id) FROM "public".ducklake_snapshot)
    ) AS __unnamed_subquery"#;

    let kind = classify_statement(sql).unwrap();
    assert_eq!(
        kind,
        StatementKind::SelectMaxSnapshot,
        "DuckDB derived snapshot max query must classify as SelectMaxSnapshot"
    );
}

#[test]
fn classify_full_latest_snapshot_tuple_select() {
    let sql = r#"SELECT snapshot_id, schema_version, next_catalog_id, next_file_id
        FROM "public".ducklake_snapshot
        WHERE snapshot_id = (SELECT MAX(snapshot_id) FROM "public".ducklake_snapshot)"#;

    let kind = classify_statement(sql).unwrap();
    assert_eq!(
        kind,
        StatementKind::SelectLatestSnapshotInfo,
        "Full DuckLake latest snapshot tuple must classify as SelectLatestSnapshotInfo"
    );
}

#[test]
fn classify_derived_latest_snapshot_tuple_select() {
    let sql = r#"SELECT "snapshot_id", "schema_version", "next_catalog_id", "next_file_id"
        FROM (SELECT snapshot_id, schema_version, next_catalog_id, next_file_id
              FROM "public".ducklake_snapshot
              WHERE snapshot_id = (SELECT MAX(snapshot_id) FROM "public".ducklake_snapshot))
        AS __unnamed_subquery"#;

    let kind = classify_statement(sql).unwrap();
    assert_eq!(
        kind,
        StatementKind::SelectLatestSnapshotInfo,
        "Derived DuckDB latest snapshot tuple must classify as SelectLatestSnapshotInfo"
    );
}

#[test]
fn classify_ducklake_snapshot_stats_changes_union() {
    let sql = r#"
SELECT
    snapshot_id,
    schema_version,
    next_catalog_id,
    next_file_id,
    COALESCE((
            SELECT STRING_AGG(changes_made, ',')
            FROM "public".ducklake_snapshot_changes c
            WHERE c.snapshot_id > 0
            ),'') AS changes,
    NULL AS table_id,
    NULL AS column_id,
    NULL AS record_count,
    NULL AS next_row_id,
    NULL AS file_size_bytes,
    NULL AS contains_null,
    NULL AS contains_nan,
    NULL AS min_value,
    NULL AS max_value,
    NULL AS extra_stats
    FROM "public".ducklake_snapshot
    WHERE snapshot_id = (
        SELECT MAX(snapshot_id)
        FROM "public".ducklake_snapshot)
UNION ALL
SELECT
    NULL,
    NULL,
    NULL,
    NULL,
    NULL,
    table_id,
    column_id,
    record_count,
    next_row_id,
    file_size_bytes,
    contains_null,
    contains_nan,
    min_value,
    max_value,
    extra_stats
FROM "public".ducklake_table_stats
LEFT JOIN "public".ducklake_table_column_stats
    USING (table_id)
WHERE record_count IS NOT NULL
    AND file_size_bytes IS NOT NULL
ORDER BY table_id NULLS FIRST;
"#;

    let kind = classify_statement(sql).unwrap();
    assert_eq!(kind, StatementKind::SelectSnapshotStatsAndChanges);
}

// ─── ducklake_macro_impl SELECT (as extracted from COPY TO STDOUT) ───────────

#[test]
fn classify_macro_impl_inner_select() {
    let sql = r#"SELECT "macro_id", "dialect", "sql", "type", "impl_id" FROM "public"."ducklake_macro_impl""#;
    let kind = classify_statement(sql).unwrap();
    assert_eq!(
        kind,
        StatementKind::SelectMacroImpls,
        "SELECT from ducklake_macro_impl must classify as SelectMacroImpls"
    );
}

// ─── derived subquery pattern for non-snapshot ducklake tables ───────────────

#[test]
fn classify_derived_ducklake_table_select() {
    let sql = r#"SELECT "table_id", "begin_snapshot", "end_snapshot", "schema_id", "table_uuid", "table_name", "data_path" FROM (SELECT table_id, begin_snapshot, end_snapshot, schema_id, table_uuid, table_name, data_path FROM "public"."ducklake_table" WHERE "begin_snapshot" <= '1') AS __unnamed_subquery"#;
    let kind = classify_statement(sql).unwrap();
    assert_eq!(
        kind,
        StatementKind::SelectTables,
        "Derived subquery select from ducklake_table must classify as SelectTables"
    );
}

#[test]
fn classify_derived_ducklake_column_select() {
    let sql = r#"SELECT "column_id", "table_id", "column_name" FROM (SELECT column_id, table_id, column_name FROM "public"."ducklake_column" WHERE "begin_snapshot" <= '1') AS __unnamed_subquery"#;
    let kind = classify_statement(sql).unwrap();
    assert_eq!(
        kind,
        StatementKind::SelectColumns,
        "Derived subquery select from ducklake_column must classify as SelectColumns"
    );
}

#[test]
fn classify_dynamic_inlined_row_update() {
    let sql = r#"
WITH deleted_row_list(deleted_row_id) AS (
VALUES (2)
)
UPDATE "public".ducklake_inlined_data_3_3
SET end_snapshot = 7
FROM deleted_row_list
WHERE row_id=deleted_row_id AND end_snapshot IS NULL AND begin_snapshot != 7;
"#;

    let kind = classify_statement(sql).unwrap();
    assert_eq!(kind, StatementKind::UpdateInlinedRowEndSnapshot);
}
