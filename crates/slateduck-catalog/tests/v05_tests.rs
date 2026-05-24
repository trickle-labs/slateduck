//! Integration tests for v0.5 Native Extension features.
//!
//! Tests: FFI C ABI, Phase 6 catalog operations (views, macros, tags,
//! file variant stats, files scheduled for deletion), and Strategy C equivalence.

use object_store::path::Path as ObjectPath;
use slateduck_catalog::{CatalogStore, OpenOptions};
use slateduck_core::mvcc::SnapshotId;
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

// ─── Phase 6: View Tests ───────────────────────────────────────────────────

#[tokio::test]
async fn create_and_list_views() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let view_id = writer
        .create_view(
            schema_id,
            "active_users",
            "SELECT * FROM users WHERE active = true",
        )
        .await
        .unwrap();
    let _snap = writer.create_snapshot(None, None).await.unwrap();

    let reader = store.read_at(SnapshotId::new(1)).unwrap();
    let views = reader.list_views(schema_id).await.unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].view_id, view_id);
    assert_eq!(views[0].view_name, "active_users");
    assert_eq!(views[0].sql, "SELECT * FROM users WHERE active = true");

    store.close().await.unwrap();
}

#[tokio::test]
async fn drop_view_makes_invisible() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let view_id = writer
        .create_view(schema_id, "temp_view", "SELECT 1")
        .await
        .unwrap();
    let snap1 = writer.create_snapshot(None, None).await.unwrap();

    let _begin_snap = writer.schema_version(); // Get the begin_snapshot of the view
    writer
        .drop_view(schema_id, view_id, snap1.as_u64()) // view was created at snapshot 1's peek
        .await
        .unwrap();
    let snap2 = writer.create_snapshot(None, None).await.unwrap();

    // Visible at snapshot 1
    let reader1 = store.read_at(snap1).unwrap();
    let views1 = reader1.list_views(schema_id).await.unwrap();
    assert_eq!(views1.len(), 1);

    // Not visible at snapshot 2
    let reader2 = store.read_at(snap2).unwrap();
    let views2 = reader2.list_views(schema_id).await.unwrap();
    assert_eq!(views2.len(), 0);

    store.close().await.unwrap();
}

#[tokio::test]
async fn view_creation_increments_schema_version() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let snap1 = writer.create_snapshot(None, None).await.unwrap();

    writer
        .create_view(schema_id, "v1", "SELECT 1")
        .await
        .unwrap();
    let snap2 = writer.create_snapshot(None, None).await.unwrap();

    let reader1 = store.read_at(snap1).unwrap();
    let s1 = reader1.get_snapshot().await.unwrap().unwrap();

    let reader2 = store.read_at(snap2).unwrap();
    let s2 = reader2.get_snapshot().await.unwrap().unwrap();

    assert!(s2.schema_version > s1.schema_version);

    store.close().await.unwrap();
}

// ─── Phase 6: Macro Tests ──────────────────────────────────────────────────

#[tokio::test]
async fn create_and_list_macros() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let macro_id = writer
        .create_macro(schema_id, "add_one", "scalar")
        .await
        .unwrap();
    let _snap = writer.create_snapshot(None, None).await.unwrap();

    let reader = store.read_at(SnapshotId::new(1)).unwrap();
    let macros = reader.list_macros(schema_id).await.unwrap();
    assert_eq!(macros.len(), 1);
    assert_eq!(macros[0].macro_id, macro_id);
    assert_eq!(macros[0].macro_name, "add_one");
    assert_eq!(macros[0].macro_type, "scalar");

    store.close().await.unwrap();
}

#[tokio::test]
async fn macro_with_impl_and_params() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let macro_id = writer
        .create_macro(schema_id, "increment", "scalar")
        .await
        .unwrap();
    let impl_id = writer.add_macro_impl(macro_id, "x + 1").await.unwrap();
    writer
        .add_macro_parameter(macro_id, impl_id, 1, "x", "INTEGER", None)
        .await
        .unwrap();
    let _snap = writer.create_snapshot(None, None).await.unwrap();

    let reader = store.read_at(SnapshotId::new(1)).unwrap();
    let impls = reader.list_macro_impls(macro_id).await.unwrap();
    assert_eq!(impls.len(), 1);
    assert_eq!(impls[0].definition, "x + 1");

    let params = reader
        .list_macro_parameters(macro_id, impl_id)
        .await
        .unwrap();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].parameter_name, "x");
    assert_eq!(params[0].parameter_type, "INTEGER");

    store.close().await.unwrap();
}

#[tokio::test]
async fn drop_macro_makes_invisible() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let macro_id = writer
        .create_macro(schema_id, "temp_macro", "table")
        .await
        .unwrap();
    let snap1 = writer.create_snapshot(None, None).await.unwrap();

    writer
        .drop_macro(schema_id, macro_id, snap1.as_u64())
        .await
        .unwrap();
    let snap2 = writer.create_snapshot(None, None).await.unwrap();

    let reader1 = store.read_at(snap1).unwrap();
    assert_eq!(reader1.list_macros(schema_id).await.unwrap().len(), 1);

    let reader2 = store.read_at(snap2).unwrap();
    assert_eq!(reader2.list_macros(schema_id).await.unwrap().len(), 0);

    store.close().await.unwrap();
}

// ─── Phase 6: Tag Tests ────────────────────────────────────────────────────

#[tokio::test]
async fn set_and_list_tags() {
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
    let _snap = writer.create_snapshot(None, None).await.unwrap();

    let reader = store.read_at(SnapshotId::new(1)).unwrap();
    let tags = reader.list_tags(table_id).await.unwrap();
    assert_eq!(tags.len(), 2);

    let owner_tag = tags.iter().find(|t| t.tag_key == "owner").unwrap();
    assert_eq!(owner_tag.tag_value, "data-team");

    let ret_tag = tags.iter().find(|t| t.tag_key == "retention").unwrap();
    assert_eq!(ret_tag.tag_value, "90d");

    store.close().await.unwrap();
}

#[tokio::test]
async fn remove_tag_makes_invisible() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "t", None).await.unwrap();

    writer.set_tag(table_id, "env", "prod").await.unwrap();
    let snap1 = writer.create_snapshot(None, None).await.unwrap();

    // Remove the tag — need the begin_snapshot from when it was set
    let reader1 = store.read_at(snap1).unwrap();
    let tags = reader1.list_tags(table_id).await.unwrap();
    let tag_begin = tags[0].begin_snapshot;

    writer.remove_tag(table_id, "env", tag_begin).await.unwrap();
    let snap2 = writer.create_snapshot(None, None).await.unwrap();

    let reader2 = store.read_at(snap2).unwrap();
    let tags2 = reader2.list_tags(table_id).await.unwrap();
    assert_eq!(tags2.len(), 0);

    store.close().await.unwrap();
}

#[tokio::test]
async fn set_and_list_column_tags() {
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
    let _snap = writer.create_snapshot(None, None).await.unwrap();

    let reader = store.read_at(SnapshotId::new(1)).unwrap();
    let tags = reader.list_column_tags(table_id, col_id).await.unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].tag_key, "pii");
    assert_eq!(tags[0].tag_value, "true");

    store.close().await.unwrap();
}

#[tokio::test]
async fn remove_column_tag_makes_invisible() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "t", None).await.unwrap();
    let col_id = writer
        .add_column(table_id, "c", "INT", 0, false, None)
        .await
        .unwrap();

    writer
        .set_column_tag(table_id, col_id, "deprecated", "yes")
        .await
        .unwrap();
    let snap1 = writer.create_snapshot(None, None).await.unwrap();

    let reader1 = store.read_at(snap1).unwrap();
    let tags = reader1.list_column_tags(table_id, col_id).await.unwrap();
    let tag_begin = tags[0].begin_snapshot;

    writer
        .remove_column_tag(table_id, col_id, "deprecated", tag_begin)
        .await
        .unwrap();
    let snap2 = writer.create_snapshot(None, None).await.unwrap();

    let reader2 = store.read_at(snap2).unwrap();
    let tags2 = reader2.list_column_tags(table_id, col_id).await.unwrap();
    assert_eq!(tags2.len(), 0);

    store.close().await.unwrap();
}

// ─── Phase 6: File Variant Stats Tests ─────────────────────────────────────

#[tokio::test]
async fn upsert_and_list_file_variant_stats() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "events", None)
        .await
        .unwrap();
    let col_id = writer
        .add_column(table_id, "payload", "JSON", 0, true, None)
        .await
        .unwrap();
    let file_id = writer
        .register_data_file(table_id, "f1.parquet", "parquet", 100, 5000)
        .await
        .unwrap();

    writer
        .upsert_file_variant_stats(
            table_id,
            col_id,
            "$.name",
            file_id,
            Some("alice"),
            Some("zoe"),
        )
        .await
        .unwrap();
    writer
        .upsert_file_variant_stats(table_id, col_id, "$.age", file_id, Some("18"), Some("99"))
        .await
        .unwrap();
    let _snap = writer.create_snapshot(None, None).await.unwrap();

    let reader = store.read_at(SnapshotId::new(1)).unwrap();
    let stats = reader
        .list_file_variant_stats(table_id, col_id)
        .await
        .unwrap();
    assert_eq!(stats.len(), 2);

    let name_stat = stats.iter().find(|s| s.variant_path == "$.name").unwrap();
    assert_eq!(name_stat.min_value.as_deref(), Some("alice"));
    assert_eq!(name_stat.max_value.as_deref(), Some("zoe"));

    store.close().await.unwrap();
}

// ─── Phase 6: Files Scheduled for Deletion Tests ───────────────────────────

#[tokio::test]
async fn schedule_and_list_file_deletion() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "t", None).await.unwrap();
    let file_id = writer
        .register_data_file(table_id, "old.parquet", "parquet", 10, 500)
        .await
        .unwrap();

    writer
        .schedule_file_deletion(file_id, "old.parquet", "data")
        .await
        .unwrap();
    let _snap = writer.create_snapshot(None, None).await.unwrap();

    let reader = store.read_at(SnapshotId::new(1)).unwrap();
    let scheduled = reader.list_files_scheduled_for_deletion().await.unwrap();
    assert_eq!(scheduled.len(), 1);
    assert_eq!(scheduled[0].data_file_id, file_id);
    assert_eq!(scheduled[0].path, "old.parquet");
    assert_eq!(scheduled[0].file_type, "data");

    // Remove from schedule
    let schedule_start = scheduled[0].schedule_start;
    writer
        .remove_scheduled_deletion(schedule_start, file_id)
        .await
        .unwrap();

    // After removal, list is empty
    let reader2 = store.read_latest();
    let scheduled2 = reader2.list_files_scheduled_for_deletion().await.unwrap();
    assert_eq!(scheduled2.len(), 0);

    store.close().await.unwrap();
}

// ─── FFI C ABI Tests ───────────────────────────────────────────────────────

#[test]
fn ffi_abi_version() {
    assert_eq!(slateduck_ffi::slateduck_abi_version(), 5_000);
}

#[test]
fn ffi_open_close_roundtrip() {
    use std::ffi::CString;

    let dir = TempDir::new().unwrap();
    let path = CString::new(dir.path().to_str().unwrap()).unwrap();
    let mut err = slateduck_ffi::SlateduckError {
        code: 0,
        message: std::ptr::null_mut(),
    };

    let catalog = slateduck_ffi::slateduck_open(path.as_ptr(), &mut err);
    assert!(!catalog.is_null(), "open failed: code={}", err.code);
    assert_eq!(err.code, 0);

    let snap = slateduck_ffi::slateduck_get_current_snapshot(catalog, &mut err);
    assert_eq!(err.code, 0);
    assert_eq!(snap.snapshot_id, 0);

    slateduck_ffi::slateduck_close(catalog);
}

#[test]
fn ffi_list_operations_empty_catalog() {
    use std::ffi::CString;

    let dir = TempDir::new().unwrap();
    let path = CString::new(dir.path().to_str().unwrap()).unwrap();
    let mut err = slateduck_ffi::SlateduckError {
        code: 0,
        message: std::ptr::null_mut(),
    };

    let catalog = slateduck_ffi::slateduck_open(path.as_ptr(), &mut err);
    assert!(!catalog.is_null());

    // List schemas (empty)
    let schemas = slateduck_ffi::slateduck_list_schemas(catalog, 1, &mut err);
    assert_eq!(err.code, 0);
    assert_eq!(schemas.count, 0);

    // List tables (empty)
    let tables = slateduck_ffi::slateduck_list_tables(catalog, 1, 1, &mut err);
    assert_eq!(err.code, 0);
    assert_eq!(tables.count, 0);

    // List files (empty)
    let files = slateduck_ffi::slateduck_list_data_files(catalog, 1, 1, &mut err);
    assert_eq!(err.code, 0);
    assert_eq!(files.count, 0);

    slateduck_ffi::slateduck_close(catalog);
}

// ─── Strategy B/C Equivalence Test ─────────────────────────────────────────

#[tokio::test]
async fn strategy_b_c_equivalence() {
    // Create a catalog via the Rust API (Strategy B path)
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "orders", Some("data/orders/"))
        .await
        .unwrap();
    writer
        .add_column(table_id, "id", "INTEGER", 0, false, None)
        .await
        .unwrap();
    writer
        .add_column(table_id, "amount", "DECIMAL(10,2)", 1, true, None)
        .await
        .unwrap();
    let file_id = writer
        .register_data_file(
            table_id,
            "data/orders/part-0001.parquet",
            "parquet",
            1000,
            50000,
        )
        .await
        .unwrap();
    let _snap = writer.create_snapshot(None, None).await.unwrap();
    store.close().await.unwrap();

    // Now read via FFI (Strategy C path) in a blocking context.
    // We must spawn a blocking task to avoid nested runtime panics.
    let dir_path = dir.path().to_str().unwrap().to_string();
    let (ffi_schema_id, ffi_table_name, ffi_col_count, ffi_file_id_result, ffi_snap_id) =
        tokio::task::spawn_blocking(move || {
            use std::ffi::CString;
            let path = CString::new(dir_path.as_str()).unwrap();
            let mut err = slateduck_ffi::SlateduckError {
                code: 0,
                message: std::ptr::null_mut(),
            };

            let catalog = slateduck_ffi::slateduck_open(path.as_ptr(), &mut err);
            assert!(!catalog.is_null(), "FFI open failed");

            // Get snapshot
            let snap = slateduck_ffi::slateduck_get_current_snapshot(catalog, &mut err);
            assert_eq!(err.code, 0);
            let snap_id = snap.snapshot_id;

            // List schemas
            let schemas = slateduck_ffi::slateduck_list_schemas(catalog, 1, &mut err);
            assert_eq!(err.code, 0);
            assert_eq!(schemas.count, 1);
            let _schema_name = unsafe {
                std::ffi::CStr::from_ptr((*schemas.schemas).schema_name)
                    .to_str()
                    .unwrap()
                    .to_string()
            };
            let ffi_sid = unsafe { (*schemas.schemas).schema_id };

            // List tables
            let tables = slateduck_ffi::slateduck_list_tables(catalog, ffi_sid, 1, &mut err);
            assert_eq!(err.code, 0);
            assert_eq!(tables.count, 1);
            let tname = unsafe {
                std::ffi::CStr::from_ptr((*tables.tables).table_name)
                    .to_str()
                    .unwrap()
                    .to_string()
            };

            // Describe table
            let table_id_val = unsafe { (*tables.tables).table_id };
            let columns =
                slateduck_ffi::slateduck_describe_table(catalog, table_id_val, 1, &mut err);
            assert_eq!(err.code, 0);
            let col_count = columns.count;

            // List data files
            let files =
                slateduck_ffi::slateduck_list_data_files(catalog, table_id_val, 1, &mut err);
            assert_eq!(err.code, 0);
            assert_eq!(files.count, 1);
            let ffi_fid = unsafe { (*files.files).data_file_id };

            // Free and close
            slateduck_ffi::slateduck_schema_list_free(&mut slateduck_ffi::SlateduckSchemaList {
                schemas: schemas.schemas,
                count: schemas.count,
            });
            slateduck_ffi::slateduck_table_list_free(&mut slateduck_ffi::SlateduckTableList {
                tables: tables.tables,
                count: tables.count,
            });
            slateduck_ffi::slateduck_column_list_free(&mut slateduck_ffi::SlateduckColumnList {
                columns: columns.columns,
                count: columns.count,
            });
            slateduck_ffi::slateduck_file_list_free(&mut slateduck_ffi::SlateduckFileList {
                files: files.files,
                count: files.count,
            });
            slateduck_ffi::slateduck_close(catalog);

            (ffi_sid, tname, col_count, ffi_fid, snap_id)
        })
        .await
        .unwrap();

    // Verify Strategy B and Strategy C produce identical results
    assert_eq!(ffi_schema_id, schema_id);
    assert_eq!(ffi_table_name, "orders");
    assert_eq!(ffi_col_count, 2);
    assert_eq!(ffi_file_id_result, file_id);
    assert_eq!(ffi_snap_id, 1);
}

// ─── All 28 Tables Implemented Test ────────────────────────────────────────

#[tokio::test]
async fn all_28_ducklake_tables_tested() {
    use slateduck_core::tags::{TagStatus, ALL_TAGS};

    // Verify all tags are Live (no more Deferred or Unimplemented for DuckLake tables)
    for desc in ALL_TAGS.iter() {
        if desc.tag <= 0x1C {
            assert_eq!(
                desc.status,
                TagStatus::Live,
                "Table '{}' (tag 0x{:02X}) should be Live but is {:?}",
                desc.name,
                desc.tag,
                desc.status,
            );
        }
    }

    // Exercise operations on each category:
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    // Schemas, tables, columns (already tested above, quick sanity)
    let schema_id = writer.create_schema("test").await.unwrap();
    let table_id = writer.create_table(schema_id, "t1", None).await.unwrap();
    let col_id = writer
        .add_column(table_id, "c1", "INT", 0, false, None)
        .await
        .unwrap();

    // Views
    let _view_id = writer
        .create_view(schema_id, "v1", "SELECT 1")
        .await
        .unwrap();

    // Macros
    let macro_id = writer
        .create_macro(schema_id, "m1", "scalar")
        .await
        .unwrap();
    let impl_id = writer.add_macro_impl(macro_id, "x + 1").await.unwrap();
    writer
        .add_macro_parameter(macro_id, impl_id, 1, "x", "INT", None)
        .await
        .unwrap();

    // Data files and delete files
    let file_id = writer
        .register_data_file(table_id, "f.parquet", "parquet", 100, 5000)
        .await
        .unwrap();
    let _del_id = writer
        .register_delete_file(file_id, "d.parquet", 10, 500)
        .await
        .unwrap();

    // Inlined data
    writer
        .register_inlined_insert(table_id, 1, 0, b"data".to_vec())
        .await
        .unwrap();
    writer
        .register_inlined_delete(table_id, file_id, 0)
        .await
        .unwrap();

    // Stats
    writer
        .update_table_stats(table_id, 100, 1, 5000)
        .await
        .unwrap();
    writer
        .upsert_file_column_stats(
            table_id,
            col_id,
            file_id,
            false,
            Some("1"),
            Some("100"),
            false,
        )
        .await
        .unwrap();

    // File variant stats
    writer
        .upsert_file_variant_stats(table_id, col_id, "$.path", file_id, Some("a"), Some("z"))
        .await
        .unwrap();

    // Tags
    writer.set_tag(table_id, "key", "val").await.unwrap();
    writer
        .set_column_tag(table_id, col_id, "pii", "false")
        .await
        .unwrap();

    // File scheduled for deletion
    writer
        .schedule_file_deletion(file_id, "f.parquet", "data")
        .await
        .unwrap();

    let _snap = writer.create_snapshot(None, None).await.unwrap();

    // Verify catalog
    let result = slateduck_catalog::verify::verify_catalog(store.db())
        .await
        .unwrap();
    assert!(result.is_ok(), "verification errors: {:?}", result.errors);

    store.close().await.unwrap();
}
