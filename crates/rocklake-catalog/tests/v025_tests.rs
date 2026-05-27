//! v0.25 conformance tests: DuckLake v1.0 SQL Catalog Facade.
//!
//! Covers all phases from the v0.25 roadmap:
//!   Phase 1 -- Schema UUID and path fields
//!   Phase 2 -- Table UUID and path fields
//!   Phase 3 -- Extended column model (nested/default metadata)
//!   Phase 4 -- View with UUID, dialect, column_aliases
//!   Phase 5 -- Macro with UUID, impl dialect, parameter type hints
//!   Phase 6 -- Metadata scope/scope_id round-trip
//!   Phase 7 -- Reader: list_all_views, list_all_macros, list_all_metadata

use object_store::path::Path as ObjectPath;
use rocklake_catalog::{CatalogStore, OpenOptions};
use rocklake_core::keys::MetadataScope;
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

// --- Phase 1: Schema UUID and path fields ------------------------------------

#[test]
fn schema_row_has_v025_fields() {
    let row = SchemaRow {
        schema_id: 1,
        schema_name: "myschema".to_string(),
        begin_snapshot: 1,
        end_snapshot: None,
        schema_uuid: Some("uuid-1234".to_string()),
        path: Some("s3://bucket/schemas/myschema".to_string()),
        path_is_relative: Some(false),
    };
    assert_eq!(row.schema_uuid.as_deref(), Some("uuid-1234"));
    assert_eq!(row.path.as_deref(), Some("s3://bucket/schemas/myschema"));
    assert_eq!(row.path_is_relative, Some(false));
}

#[tokio::test]
async fn create_schema_auto_generates_uuid() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut w = store.begin_write();
    let schema_id = w.create_schema("test_schema").await.unwrap();
    let snap = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let schemas = reader.list_schemas().await.unwrap();
    let schema = schemas.iter().find(|s| s.schema_id == schema_id).unwrap();
    assert!(
        schema.schema_uuid.is_some(),
        "create_schema must auto-generate schema_uuid"
    );
    let uuid_str = schema.schema_uuid.as_ref().unwrap();
    assert_eq!(
        uuid_str.len(),
        36,
        "schema_uuid must be a UUID string (len 36)"
    );
}

// --- Phase 2: Table UUID and path fields -------------------------------------

#[test]
fn table_row_has_v025_fields() {
    let row = TableRow {
        table_id: 1,
        schema_id: 1,
        table_name: "events".to_string(),
        path: Some("s3://bucket/events".to_string()),
        begin_snapshot: 1,
        end_snapshot: None,
        table_uuid: Some("table-uuid-5678".to_string()),
        path_is_relative: Some(true),
    };
    assert_eq!(row.path.as_deref(), Some("s3://bucket/events"));
    assert_eq!(row.table_uuid.as_deref(), Some("table-uuid-5678"));
    assert_eq!(row.path_is_relative, Some(true));
}

#[tokio::test]
async fn create_table_auto_generates_uuid() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut w = store.begin_write();
    let schema_id = w.create_schema("s1").await.unwrap();
    let table_id = w.create_table(schema_id, "my_table", None).await.unwrap();
    let snap = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let tables = reader.list_tables(schema_id).await.unwrap();
    let table = tables.iter().find(|t| t.table_id == table_id).unwrap();
    assert!(
        table.table_uuid.is_some(),
        "create_table must auto-generate table_uuid"
    );
    let uuid_str = table.table_uuid.as_ref().unwrap();
    assert_eq!(
        uuid_str.len(),
        36,
        "table_uuid must be a UUID string (len 36)"
    );
}

// --- Phase 3: Extended column model ------------------------------------------

#[test]
fn column_row_has_v025_fields() {
    let row = ColumnRow {
        column_id: 1,
        table_id: 1,
        column_name: "struct_field".to_string(),
        data_type: "STRUCT(x INT, y INT)".to_string(),
        column_index: 0,
        begin_snapshot: 1,
        end_snapshot: None,
        default_value: None,
        is_nullable: true,
        initial_default: Some("NULL".to_string()),
        default_value_type: Some("sql".to_string()),
        default_value_dialect: Some("duckdb".to_string()),
        parent_column: Some(0),
    };
    assert_eq!(row.initial_default.as_deref(), Some("NULL"));
    assert_eq!(row.default_value_type.as_deref(), Some("sql"));
    assert_eq!(row.default_value_dialect.as_deref(), Some("duckdb"));
    assert_eq!(row.parent_column, Some(0));
}

#[tokio::test]
async fn add_column_with_opts_round_trips() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut w = store.begin_write();
    let schema_id = w.create_schema("s1").await.unwrap();
    let table_id = w.create_table(schema_id, "t1", None).await.unwrap();
    let col_id = w
        .add_column_with_opts(
            table_id,
            "amount",
            "DECIMAL(18,2)",
            0,
            false,
            Some("0.00"),
            Some("0.00"),
            Some("sql"),
            Some("duckdb"),
            None,
        )
        .await
        .unwrap();
    let snap = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let (_, cols) = reader.describe_table(table_id).await.unwrap().unwrap();
    let col = cols.iter().find(|c| c.column_id == col_id).unwrap();
    assert_eq!(col.initial_default.as_deref(), Some("0.00"));
    assert_eq!(col.default_value_type.as_deref(), Some("sql"));
    assert_eq!(col.default_value_dialect.as_deref(), Some("duckdb"));
    assert!(col.parent_column.is_none());
}

#[tokio::test]
async fn nested_column_parent_column_round_trips() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut w = store.begin_write();
    let schema_id = w.create_schema("s1").await.unwrap();
    let table_id = w.create_table(schema_id, "t1", None).await.unwrap();
    let parent_id = w
        .add_column(table_id, "rec", "STRUCT(a INT)", 0, false, None)
        .await
        .unwrap();
    let child_id = w
        .add_column_with_opts(
            table_id,
            "a",
            "INT",
            0,
            false,
            None,
            None,
            None,
            None,
            Some(parent_id),
        )
        .await
        .unwrap();
    let snap = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let (_, cols) = reader.describe_table(table_id).await.unwrap().unwrap();
    let child = cols.iter().find(|c| c.column_id == child_id).unwrap();
    assert_eq!(child.parent_column, Some(parent_id));
}

// --- Phase 4: View with UUID, dialect, column_aliases ------------------------

#[test]
fn view_row_has_v025_fields() {
    let row = ViewRow {
        view_id: 1,
        schema_id: 1,
        view_name: "my_view".to_string(),
        sql: "SELECT 1".to_string(),
        begin_snapshot: 1,
        end_snapshot: None,
        view_uuid: Some("view-uuid-abc".to_string()),
        dialect: Some("duckdb".to_string()),
        column_aliases: Some("[\"col1\",\"col2\"]".to_string()),
    };
    assert_eq!(row.view_uuid.as_deref(), Some("view-uuid-abc"));
    assert_eq!(row.dialect.as_deref(), Some("duckdb"));
    assert!(row.column_aliases.is_some());
}

#[tokio::test]
async fn create_view_with_opts_round_trips() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut w = store.begin_write();
    let schema_id = w.create_schema("s1").await.unwrap();
    let view_id = w
        .create_view_with_opts(
            schema_id,
            "my_view",
            "SELECT 42 AS x",
            None,
            Some("duckdb"),
            Some("[\"x\"]"),
        )
        .await
        .unwrap();
    let snap = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let views = reader.list_views(schema_id).await.unwrap();
    let view = views.iter().find(|v| v.view_id == view_id).unwrap();
    assert!(view.view_uuid.is_some(), "view_uuid must be auto-generated");
    assert_eq!(view.dialect.as_deref(), Some("duckdb"));
    assert_eq!(view.column_aliases.as_deref(), Some("[\"x\"]"));
}

// --- Phase 5: Macro with UUID, impl dialect, parameter type hints ------------

#[test]
fn macro_row_has_v025_uuid_field() {
    let row = MacroRow {
        macro_id: 1,
        schema_id: 1,
        macro_name: "my_macro".to_string(),
        macro_type: "scalar".to_string(),
        begin_snapshot: 1,
        end_snapshot: None,
        macro_uuid: Some("macro-uuid-def".to_string()),
    };
    assert_eq!(row.macro_uuid.as_deref(), Some("macro-uuid-def"));
}

#[test]
fn macro_impl_row_has_v025_fields() {
    let row = MacroImplRow {
        impl_id: 1,
        macro_id: 1,
        sql: "x + 1".to_string(),
        dialect: Some("duckdb".to_string()),
        impl_type: Some("scalar".to_string()),
    };
    assert_eq!(row.sql, "x + 1");
    assert_eq!(row.dialect.as_deref(), Some("duckdb"));
    assert_eq!(row.impl_type.as_deref(), Some("scalar"));
}

#[tokio::test]
async fn create_macro_auto_generates_uuid() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut w = store.begin_write();
    let schema_id = w.create_schema("s1").await.unwrap();
    let macro_id = w
        .create_macro(schema_id, "add_one", "scalar")
        .await
        .unwrap();
    let snap = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let macros = reader.list_macros(schema_id).await.unwrap();
    let m = macros.iter().find(|m| m.macro_id == macro_id).unwrap();
    assert!(
        m.macro_uuid.is_some(),
        "create_macro must auto-generate macro_uuid"
    );
    assert_eq!(m.macro_uuid.as_ref().unwrap().len(), 36);
}

#[tokio::test]
async fn add_macro_impl_with_opts_round_trips() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut w = store.begin_write();
    let schema_id = w.create_schema("s1").await.unwrap();
    let macro_id = w.create_macro(schema_id, "my_fn", "scalar").await.unwrap();
    let impl_id = w
        .add_macro_impl_with_opts(macro_id, "x * 2", Some("duckdb"), Some("scalar"))
        .await
        .unwrap();
    let snap = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let impls = reader.list_macro_impls(macro_id).await.unwrap();
    let impl_row = impls.iter().find(|i| i.impl_id == impl_id).unwrap();
    assert_eq!(impl_row.sql, "x * 2");
    assert_eq!(impl_row.dialect.as_deref(), Some("duckdb"));
    assert_eq!(impl_row.impl_type.as_deref(), Some("scalar"));
}

// --- Phase 6: Metadata scope/scope_id round-trip -----------------------------

#[test]
fn metadata_row_has_v025_scope_fields() {
    let row = MetadataRow {
        key: "my.key".to_string(),
        value: "my_value".to_string(),
        scope: Some("table".to_string()),
        scope_id: Some(42),
    };
    assert_eq!(row.scope.as_deref(), Some("table"));
    assert_eq!(row.scope_id, Some(42));
}

#[tokio::test]
async fn set_metadata_global_round_trips() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut w = store.begin_write();
    w.set_metadata(MetadataScope::Global, 0, "app.v025.version", "1.0")
        .unwrap();
    let snap = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let rows = reader.list_all_metadata().await.unwrap();
    let row = rows
        .iter()
        .find(|r| r.key == "app.v025.version")
        .expect("global metadata must be found");
    assert_eq!(row.value, "1.0");
    assert_eq!(row.scope.as_deref(), Some("global"));
    assert_eq!(row.scope_id, Some(0));
}

#[tokio::test]
async fn set_metadata_table_scope_round_trips() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut w = store.begin_write();
    let schema_id = w.create_schema("s1").await.unwrap();
    let table_id = w.create_table(schema_id, "t1", None).await.unwrap();
    w.set_metadata(MetadataScope::Table, table_id, "table.owner.alice", "alice")
        .unwrap();
    let snap = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let rows = reader.list_all_metadata().await.unwrap();
    let row = rows
        .iter()
        .find(|r| r.key == "table.owner.alice")
        .expect("table metadata must be found");
    assert_eq!(row.value, "alice");
    assert_eq!(row.scope.as_deref(), Some("table"));
    assert_eq!(row.scope_id, Some(table_id));
}

// --- Phase 7: Reader list_all_{views,macros,metadata} ------------------------

#[tokio::test]
async fn list_all_views_returns_all_views_across_schemas() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut w = store.begin_write();
    let s1 = w.create_schema("s1").await.unwrap();
    let s2 = w.create_schema("s2").await.unwrap();
    w.create_view(s1, "v1", "SELECT 1").await.unwrap();
    w.create_view(s1, "v2", "SELECT 2").await.unwrap();
    w.create_view(s2, "v3", "SELECT 3").await.unwrap();
    let snap = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let views = reader.list_all_views().await.unwrap();
    assert_eq!(views.len(), 3, "list_all_views must return all 3 views");
}

#[tokio::test]
async fn list_all_macros_returns_all_macros_across_schemas() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut w = store.begin_write();
    let s1 = w.create_schema("s1").await.unwrap();
    let s2 = w.create_schema("s2").await.unwrap();
    w.create_macro(s1, "fn1", "scalar").await.unwrap();
    w.create_macro(s2, "fn2", "table").await.unwrap();
    let snap = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let macros = reader.list_all_macros().await.unwrap();
    assert_eq!(macros.len(), 2, "list_all_macros must return all 2 macros");
}

#[tokio::test]
async fn list_all_metadata_returns_all_scope_entries() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut w = store.begin_write();
    let schema_id = w.create_schema("s1").await.unwrap();
    let table_id = w.create_table(schema_id, "t1", None).await.unwrap();
    w.set_metadata(MetadataScope::Global, 0, "global.key.one", "gval")
        .unwrap();
    w.set_metadata(MetadataScope::Schema, schema_id, "schema.key.one", "sval")
        .unwrap();
    w.set_metadata(MetadataScope::Table, table_id, "table.key.one", "tval")
        .unwrap();
    let snap = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap);

    let reader = store.read_at(snap).unwrap();
    let rows = reader.list_all_metadata().await.unwrap();
    assert_eq!(rows.len(), 3, "list_all_metadata must return all 3 entries");

    let global = rows.iter().find(|r| r.key == "global.key.one").unwrap();
    assert_eq!(global.scope.as_deref(), Some("global"));

    let schema_meta = rows.iter().find(|r| r.key == "schema.key.one").unwrap();
    assert_eq!(schema_meta.scope.as_deref(), Some("schema"));
    assert_eq!(schema_meta.scope_id, Some(schema_id));

    let table_meta = rows.iter().find(|r| r.key == "table.key.one").unwrap();
    assert_eq!(table_meta.scope.as_deref(), Some("table"));
    assert_eq!(table_meta.scope_id, Some(table_id));
}

#[tokio::test]
async fn drop_view_is_invisible_in_list_all_views() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut w = store.begin_write();
    let schema_id = w.create_schema("s1").await.unwrap();
    let v1 = w
        .create_view(schema_id, "v_keep", "SELECT 1")
        .await
        .unwrap();
    let v2 = w
        .create_view(schema_id, "v_drop", "SELECT 2")
        .await
        .unwrap();
    let snap1 = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap1);

    let mut w2 = store.begin_write();
    w2.drop_view(schema_id, v2, snap1.as_u64()).await.unwrap();
    let snap2 = w2.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap2);

    let reader = store.read_at(snap2).unwrap();
    let views = reader.list_all_views().await.unwrap();
    assert_eq!(
        views.len(),
        1,
        "dropped view must not appear in list_all_views"
    );
    assert_eq!(views[0].view_id, v1);
    let _ = snap1; // suppress unused warning
}
