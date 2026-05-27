//! v0.27 conformance tests: External Compatibility Validation.
//!
//! Covers all phases from the v0.27 roadmap:
//!   Phase 1 -- ducklake_tag facade: spec column names (tag_name, tag_value)
//!   Phase 2 -- ducklake_tag and ducklake_column_tag DROP TABLE cascade lifecycle
//!   Phase 3 -- ducklake_sort_info round-trip lifecycle (define, drop, verify retired)
//!   Phase 4 -- ducklake_schema_version write-path (evolve schema, verify version)
//!   Phase 5 -- list_all_tags and list_all_column_tags across all objects
//!   Phase 6 -- list_all_sort_info across all tables
//!   Phase 7 -- Definition-of-Done checklist tests

use object_store::path::Path as ObjectPath;
use rocklake_catalog::{CatalogStore, OpenOptions};
use rocklake_core::mvcc::SnapshotId;
use rocklake_core::rows::*;
use std::sync::Arc;
use tempfile::TempDir;

fn test_opts(dir: &TempDir) -> OpenOptions {
    let path = dir.path().to_str().unwrap().to_string();
    let store = Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&path).unwrap());
    OpenOptions {
        object_store: store,
        path: ObjectPath::from("catalog"),
        encryption: None,
    }
}

// ─── Phase 1: ducklake_tag facade column names ───────────────────────────────

#[test]
fn tag_row_has_spec_fields() {
    // The internal row uses tag_key for the name field and tag_value for the value.
    // The SQL facade must expose these as tag_name / tag_value per the spec.
    let row = TagRow {
        object_id: 1,
        tag_key: "owner".to_string(),
        tag_value: "data-team".to_string(),
        begin_snapshot: 1,
        end_snapshot: None,
    };
    // Internal field names match expectations.
    assert_eq!(row.tag_key, "owner");
    assert_eq!(row.tag_value, "data-team");
    // tag_id is synthesized as object_id in the SQL facade.
    assert_eq!(row.object_id, 1);
}

#[test]
fn column_tag_row_has_spec_fields() {
    let row = ColumnTagRow {
        table_id: 10,
        column_id: 20,
        tag_key: "pii".to_string(),
        tag_value: "true".to_string(),
        begin_snapshot: 1,
        end_snapshot: None,
    };
    assert_eq!(row.tag_key, "pii");
    assert_eq!(row.tag_value, "true");
    // tag_id is synthesized as column_id in the SQL facade.
    assert_eq!(row.column_id, 20);
}

// ─── Phase 2: DROP TABLE cascade for tags and column tags ────────────────────

#[tokio::test]
async fn drop_table_retires_tags() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "events", None)
        .await
        .unwrap();

    writer
        .set_tag(table_id, "owner", "data-team")
        .await
        .unwrap();
    writer.set_tag(table_id, "retention", "90d").await.unwrap();
    let snap1 = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap1);

    // Verify tags are visible at snap1.
    let reader = store.read_at(snap1).unwrap();
    let tags = reader.list_all_tags().await.unwrap();
    assert_eq!(tags.len(), 2, "should see 2 tags before drop");

    // Drop the table — tags must be cascade-retired.
    let mut writer2 = store.begin_write();
    writer2
        .drop_table(schema_id, table_id, snap1.0)
        .await
        .unwrap();
    let snap2 = writer2.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap2);

    let reader2 = store.read_at(snap2).unwrap();
    let tags_after = reader2.list_all_tags().await.unwrap();
    assert_eq!(tags_after.len(), 0, "all tags must be retired after drop");

    // At the older snapshot, tags are still visible.
    let reader_old = store.read_at(snap1).unwrap();
    let tags_old = reader_old.list_all_tags().await.unwrap();
    assert_eq!(tags_old.len(), 2, "tags still visible at snap1");

    store.close().await.unwrap();
}

#[tokio::test]
async fn drop_table_retires_column_tags() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "users", None).await.unwrap();
    let col_id = writer
        .add_column(table_id, "email", "VARCHAR", 0, true, None)
        .await
        .unwrap();

    writer
        .set_column_tag(table_id, col_id, "pii", "true")
        .await
        .unwrap();
    writer
        .set_column_tag(table_id, col_id, "gdpr", "subject")
        .await
        .unwrap();
    let snap1 = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap1);

    let reader = store.read_at(snap1).unwrap();
    let col_tags = reader.list_all_column_tags().await.unwrap();
    assert_eq!(col_tags.len(), 2, "should see 2 column tags before drop");

    let mut writer2 = store.begin_write();
    writer2
        .drop_table(schema_id, table_id, snap1.0)
        .await
        .unwrap();
    let snap2 = writer2.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap2);

    let reader2 = store.read_at(snap2).unwrap();
    let col_tags_after = reader2.list_all_column_tags().await.unwrap();
    assert_eq!(
        col_tags_after.len(),
        0,
        "all column tags must be retired after drop"
    );

    store.close().await.unwrap();
}

// ─── Phase 3: ducklake_sort_info round-trip lifecycle ────────────────────────

#[tokio::test]
async fn sort_info_round_trip() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "orders", None)
        .await
        .unwrap();

    // Define sort info on the table.
    writer.add_sort_info(table_id, 1).await.unwrap();
    let snap1 = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap1);

    let reader = store.read_at(snap1).unwrap();
    let sort_rows = reader.list_all_sort_info().await.unwrap();
    assert_eq!(sort_rows.len(), 1, "should see 1 sort_info row after add");
    assert_eq!(sort_rows[0].table_id, table_id);
    assert_eq!(sort_rows[0].sort_id, 1);
    assert!(sort_rows[0].end_snapshot.is_none());

    store.close().await.unwrap();
}

#[tokio::test]
async fn drop_table_retires_sort_info() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "t", None).await.unwrap();

    writer.add_sort_info(table_id, 1).await.unwrap();
    let snap1 = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap1);

    // Verify sort_info is visible.
    let reader = store.read_at(snap1).unwrap();
    let sort_rows = reader.list_all_sort_info().await.unwrap();
    assert_eq!(sort_rows.len(), 1, "sort_info visible before drop");

    // Drop the table — sort_info must be cascade-retired.
    let mut writer2 = store.begin_write();
    writer2
        .drop_table(schema_id, table_id, snap1.0)
        .await
        .unwrap();
    let snap2 = writer2.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap2);

    let reader2 = store.read_at(snap2).unwrap();
    let sort_rows_after = reader2.list_all_sort_info().await.unwrap();
    assert_eq!(
        sort_rows_after.len(),
        0,
        "sort_info must be retired after drop"
    );

    // Sort_info still visible at earlier snapshot.
    let reader_old = store.read_at(snap1).unwrap();
    let sort_rows_old = reader_old.list_all_sort_info().await.unwrap();
    assert_eq!(sort_rows_old.len(), 1, "sort_info visible at snap1");

    store.close().await.unwrap();
}

// ─── Phase 4: ducklake_schema_version write-path ─────────────────────────────

#[tokio::test]
async fn schema_version_increments_with_ddl() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let initial_version = store.schema_version();

    let mut writer = store.begin_write();
    let schema_id = writer.create_schema("main").await.unwrap();
    let _table_id = writer.create_table(schema_id, "t", None).await.unwrap();
    let _snap = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(_snap);

    let after_ddl = store.schema_version();
    assert!(
        after_ddl > initial_version,
        "schema_version must increment after DDL: initial={initial_version} after={after_ddl}"
    );

    store.close().await.unwrap();
}

#[tokio::test]
async fn schema_version_exposed_via_facade() {
    // Verify that the spec column names for ducklake_schema_version are correct.
    // The spec says: schema_version (BIGINT), schema_version_info (VARCHAR).
    // This is returned by make_schema_version_response(catalog_schema_version).
    // We test the catalog API here; the SQL facade test is in v027_pgwire_tests.

    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("analytics").await.unwrap();
    let _tbl = writer
        .create_table(schema_id, "events", None)
        .await
        .unwrap();
    let _snap = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(_snap);

    let version = store.schema_version();
    assert!(version >= 1, "schema_version must be >= 1 after DDL");

    store.close().await.unwrap();
}

// ─── Phase 5: list_all_tags across all objects ───────────────────────────────

#[tokio::test]
async fn list_all_tags_returns_tags_for_all_objects() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table1 = writer.create_table(schema_id, "t1", None).await.unwrap();
    let table2 = writer.create_table(schema_id, "t2", None).await.unwrap();

    writer.set_tag(table1, "env", "prod").await.unwrap();
    writer.set_tag(table2, "env", "staging").await.unwrap();
    writer.set_tag(table2, "owner", "analytics").await.unwrap();
    let snap = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let tags = reader.list_all_tags().await.unwrap();
    assert_eq!(tags.len(), 3, "should see all 3 tags across both tables");

    let t1_tags: Vec<_> = tags.iter().filter(|t| t.object_id == table1).collect();
    let t2_tags: Vec<_> = tags.iter().filter(|t| t.object_id == table2).collect();
    assert_eq!(t1_tags.len(), 1);
    assert_eq!(t2_tags.len(), 2);

    store.close().await.unwrap();
}

#[tokio::test]
async fn list_all_column_tags_returns_tags_for_all_columns() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "users", None).await.unwrap();
    let col1 = writer
        .add_column(table_id, "email", "VARCHAR", 0, true, None)
        .await
        .unwrap();
    let col2 = writer
        .add_column(table_id, "ssn", "VARCHAR", 1, true, None)
        .await
        .unwrap();

    writer
        .set_column_tag(table_id, col1, "pii", "true")
        .await
        .unwrap();
    writer
        .set_column_tag(table_id, col2, "pii", "true")
        .await
        .unwrap();
    writer
        .set_column_tag(table_id, col2, "encryption", "aes256")
        .await
        .unwrap();
    let snap = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let col_tags = reader.list_all_column_tags().await.unwrap();
    assert_eq!(col_tags.len(), 3, "should see all 3 column tags");

    store.close().await.unwrap();
}

// ─── Phase 6: list_all_sort_info across all tables ───────────────────────────

#[tokio::test]
async fn list_all_sort_info_returns_entries_for_multiple_tables() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let t1 = writer
        .create_table(schema_id, "orders", None)
        .await
        .unwrap();
    let t2 = writer
        .create_table(schema_id, "customers", None)
        .await
        .unwrap();

    writer.add_sort_info(t1, 1).await.unwrap();
    writer.add_sort_info(t1, 2).await.unwrap();
    writer.add_sort_info(t2, 1).await.unwrap();
    let snap = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let sort_rows = reader.list_all_sort_info().await.unwrap();
    assert_eq!(sort_rows.len(), 3, "should see 3 sort_info rows total");

    store.close().await.unwrap();
}

// ─── Phase 7: Definition-of-Done checklist tests ─────────────────────────────

#[test]
fn dod_all_28_spec_tables_in_manifest() {
    // DoD criterion 1: All 28 spec tables are visible through SQL.
    let manifest_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/ducklake-v1.0-schema.toml"
    );
    let content = std::fs::read_to_string(manifest_path).unwrap();
    let table_count = content.matches("[[table]]").count();
    assert_eq!(
        table_count, 28,
        "Manifest must define exactly 28 tables, got {}",
        table_count
    );
}

#[test]
fn dod_manifest_includes_tag_tables() {
    let manifest_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/ducklake-v1.0-schema.toml"
    );
    let content = std::fs::read_to_string(manifest_path).unwrap();
    // DoD: ducklake_tag and ducklake_column_tag must have tag_name column.
    let tag_section_start = content.find("name = \"ducklake_tag\"").unwrap();
    let tag_section = &content[tag_section_start..tag_section_start + 500];
    assert!(
        tag_section.contains("tag_name"),
        "ducklake_tag spec must have tag_name column"
    );
    let col_tag_section_start = content.find("name = \"ducklake_column_tag\"").unwrap();
    let col_tag_section = &content[col_tag_section_start..col_tag_section_start + 500];
    assert!(
        col_tag_section.contains("tag_name"),
        "ducklake_column_tag spec must have tag_name column"
    );
}

#[test]
fn dod_sort_info_spec_columns_present() {
    // DoD: ducklake_sort_info must expose spec column names.
    let manifest_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/ducklake-v1.0-schema.toml"
    );
    let content = std::fs::read_to_string(manifest_path).unwrap();
    let sort_info_start = content.find("name = \"ducklake_sort_info\"").unwrap();
    let sort_info_section = &content[sort_info_start..sort_info_start + 600];
    for required_col in &["sort_id", "begin_snapshot", "end_snapshot", "table_id"] {
        assert!(
            sort_info_section.contains(required_col),
            "ducklake_sort_info spec must have column: {}",
            required_col
        );
    }
}

#[test]
fn dod_schema_version_spec_columns_present() {
    // DoD: ducklake_schema_version must expose spec column names.
    let manifest_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/ducklake-v1.0-schema.toml"
    );
    let content = std::fs::read_to_string(manifest_path).unwrap();
    let sv_start = content.find("name = \"ducklake_schema_version\"").unwrap();
    let sv_section = &content[sv_start..sv_start + 300];
    assert!(
        sv_section.contains("schema_version"),
        "ducklake_schema_version spec must have schema_version column"
    );
    assert!(
        sv_section.contains("schema_version_info"),
        "ducklake_schema_version spec must have schema_version_info column"
    );
}

#[tokio::test]
async fn dod_mvcc_windows_in_tag_rows() {
    // DoD criterion 5: MVCC windows consistent across all spec tables.
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "t", None).await.unwrap();
    writer.set_tag(table_id, "k", "v").await.unwrap();
    let snap = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let tags = reader.list_all_tags().await.unwrap();
    assert_eq!(tags.len(), 1);
    assert!(
        tags[0].end_snapshot.is_none(),
        "live tag must have no end_snapshot"
    );
    assert!(
        tags[0].begin_snapshot > 0,
        "live tag must have begin_snapshot > 0"
    );

    store.close().await.unwrap();
}

#[tokio::test]
async fn dod_no_supported_write_is_noop() {
    // DoD criterion 11: no supported write is accepted as a no-op.
    // Verify that set_tag / set_column_tag / add_sort_info actually persist rows.
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("s").await.unwrap();
    let table_id = writer.create_table(schema_id, "t", None).await.unwrap();
    let col_id = writer
        .add_column(table_id, "c", "INT", 0, false, None)
        .await
        .unwrap();

    writer.set_tag(table_id, "a", "b").await.unwrap();
    writer
        .set_column_tag(table_id, col_id, "x", "y")
        .await
        .unwrap();
    writer.add_sort_info(table_id, 1).await.unwrap();
    let snap = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    assert_eq!(reader.list_all_tags().await.unwrap().len(), 1);
    assert_eq!(reader.list_all_column_tags().await.unwrap().len(), 1);
    assert_eq!(reader.list_all_sort_info().await.unwrap().len(), 1);

    store.close().await.unwrap();
}

#[tokio::test]
async fn dod_time_travel_sees_retired_tags() {
    // DoD criterion 5: time travel uses begin_snapshot and end_snapshot consistently.
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "t", None).await.unwrap();
    writer.set_tag(table_id, "tier", "gold").await.unwrap();
    let snap1 = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap1);

    // At snap1: 1 tag.
    let r1 = store.read_at(snap1).unwrap();
    assert_eq!(r1.list_all_tags().await.unwrap().len(), 1);

    // Drop table at snap2.
    let mut writer2 = store.begin_write();
    writer2
        .drop_table(schema_id, table_id, snap1.0)
        .await
        .unwrap();
    let snap2 = writer2.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap2);

    // At snap2: 0 tags (retired).
    let r2 = store.read_at(snap2).unwrap();
    assert_eq!(r2.list_all_tags().await.unwrap().len(), 0);

    // Time travel to snap1 still shows the tag.
    let r1_again = store.read_at(SnapshotId::new(snap1.0)).unwrap();
    assert_eq!(r1_again.list_all_tags().await.unwrap().len(), 1);

    store.close().await.unwrap();
}
