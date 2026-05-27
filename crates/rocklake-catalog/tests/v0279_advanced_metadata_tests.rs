//! v0.27.9 advanced metadata validation tests.
//!
//! Covers all tasks from the v0.27.9 roadmap:
//!   1.  view_lifecycle_create_read_drop
//!   2.  macro_lifecycle_create_read_drop
//!   3.  tag_lifecycle_set_read_remove
//!   4.  column_tag_lifecycle_set_read_remove
//!   5.  sort_info_lifecycle_add_read
//!   6.  drop_table_cascades_sort_info
//!   7.  drop_table_cascades_tags_and_column_tags
//!   8.  alter_table_add_column_time_travel
//!   9.  alter_table_drop_column_time_travel
//!  10.  alter_table_rename_column
//!  11.  drop_table_full_cascade_all_types
//!  12.  describe_table_at_snapshot_before_after_alter

use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use rocklake_catalog::{CatalogStore, OpenOptions};
use std::sync::Arc;
use tempfile::TempDir;

fn test_opts(dir: &TempDir) -> OpenOptions {
    let path = dir.path().to_str().unwrap().to_string();
    let store = Arc::new(LocalFileSystem::new_with_prefix(&path).unwrap());
    OpenOptions {
        object_store: store,
        path: ObjectPath::from("catalog"),
        encryption: None,
    }
}

// ─── 1. View lifecycle ───────────────────────────────────────────────────────

#[tokio::test]
async fn view_lifecycle_create_read_drop() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let schema_id = {
        let mut w = store.begin_write();
        let sid = w.create_schema("main").await.unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        sid
    };

    // Create view.
    let view_id = {
        let mut w = store.begin_write();
        let vid = w
            .create_view(schema_id, "my_view", "SELECT 1 AS n")
            .await
            .unwrap();
        let cr = w.create_snapshot(Some("create_view"), None).await.unwrap();
        store.commit_writer(cr);
        vid
    };

    // Read view back.
    let views = {
        let reader = store.read_latest();
        reader.list_views(schema_id).await.unwrap()
    };
    assert_eq!(views.len(), 1, "should see 1 view");
    assert_eq!(views[0].view_id, view_id);
    assert_eq!(views[0].sql, "SELECT 1 AS n");
    let view_begin = views[0].begin_snapshot;

    // Drop view.
    {
        let mut w = store.begin_write();
        w.drop_view(schema_id, view_id, view_begin).await.unwrap();
        let cr = w.create_snapshot(Some("drop_view"), None).await.unwrap();
        store.commit_writer(cr);
    }

    // View must no longer be visible.
    let views2 = {
        let reader2 = store.read_latest();
        reader2.list_views(schema_id).await.unwrap()
    };
    assert!(views2.is_empty(), "view should be retired after drop");
}

// ─── 2. Macro lifecycle ──────────────────────────────────────────────────────

#[tokio::test]
async fn macro_lifecycle_create_read_drop() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let schema_id = {
        let mut w = store.begin_write();
        let sid = w.create_schema("main").await.unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        sid
    };

    // Create macro.
    let macro_id = {
        let mut w = store.begin_write();
        let mid = w
            .create_macro(schema_id, "add_one", "scalar")
            .await
            .unwrap();
        let impl_id = w.add_macro_impl(mid, "SELECT $1 + 1").await.unwrap();
        w.add_macro_parameter(mid, impl_id, 1, "p1", "INTEGER", None)
            .await
            .unwrap();
        let cr = w.create_snapshot(Some("create_macro"), None).await.unwrap();
        store.commit_writer(cr);
        mid
    };

    // macro_id was captured above; also capture impl_id for reading params
    // Read macro back.
    let (macro_begin, impl_body, param_name) = {
        let reader = store.read_latest();
        let macros = reader.list_macros(schema_id).await.unwrap();
        assert_eq!(macros.len(), 1, "should see 1 macro");
        assert_eq!(macros[0].macro_id, macro_id);
        assert_eq!(macros[0].macro_name, "add_one");
        let impls = reader.list_macro_impls(macro_id).await.unwrap();
        assert_eq!(impls.len(), 1, "should see 1 macro impl");
        let impl_id = impls[0].impl_id;
        let params = reader
            .list_macro_parameters(macro_id, impl_id)
            .await
            .unwrap();
        assert_eq!(params.len(), 1, "should see 1 macro parameter");
        (
            macros[0].begin_snapshot,
            impls[0].sql.clone(),
            params[0].parameter_name.clone(),
        )
    };
    assert_eq!(impl_body, "SELECT $1 + 1");
    assert_eq!(param_name, "p1");

    // Drop macro.
    {
        let mut w = store.begin_write();
        w.drop_macro(schema_id, macro_id, macro_begin)
            .await
            .unwrap();
        let cr = w.create_snapshot(Some("drop_macro"), None).await.unwrap();
        store.commit_writer(cr);
    }

    let macros3 = {
        let reader3 = store.read_latest();
        reader3.list_macros(schema_id).await.unwrap()
    };
    assert!(macros3.is_empty(), "macro should be retired after drop");
}

// ─── 3. Tag lifecycle ────────────────────────────────────────────────────────

#[tokio::test]
async fn tag_lifecycle_set_read_remove() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (schema_id, table_id) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("main").await.unwrap();
        let tid = w.create_table(sid, "events", None).await.unwrap();
        w.set_tag(tid, "owner", "data-team").await.unwrap();
        w.set_tag(tid, "env", "prod").await.unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        (sid, tid)
    };

    // Tags visible.
    let (owner_begin,) = {
        let reader = store.read_latest();
        let tags = reader.list_tags(table_id).await.unwrap();
        assert_eq!(tags.len(), 2, "should see 2 live tags");
        let owner = tags.iter().find(|t| t.tag_key == "owner").unwrap().clone();
        (owner.begin_snapshot,)
    };
    let _ = schema_id;

    // Remove one tag.
    {
        let mut w2 = store.begin_write();
        w2.remove_tag(table_id, "owner", owner_begin).await.unwrap();
        let cr2 = w2.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr2);
    }

    let tags3 = {
        let reader3 = store.read_latest();
        reader3.list_tags(table_id).await.unwrap()
    };
    assert_eq!(tags3.len(), 1, "only 1 tag should remain after remove");
    assert_eq!(tags3[0].tag_key, "env");
}

// ─── 4. Column tag lifecycle ─────────────────────────────────────────────────

#[tokio::test]
async fn column_tag_lifecycle_set_read_remove() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (table_id, column_id) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("main").await.unwrap();
        let tid = w.create_table(sid, "users", None).await.unwrap();
        let cid = w
            .add_column(tid, "email", "VARCHAR", 0, false, None)
            .await
            .unwrap();
        w.set_column_tag(tid, cid, "pii", "true").await.unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        (tid, cid)
    };

    // Column tag visible.
    let pii_begin = {
        let reader = store.read_latest();
        let ctags = reader.list_column_tags(table_id, column_id).await.unwrap();
        assert_eq!(ctags.len(), 1, "should see 1 column tag");
        assert_eq!(ctags[0].tag_key, "pii");
        assert_eq!(ctags[0].tag_value, "true");
        ctags[0].begin_snapshot
    };

    // Remove column tag.
    {
        let mut w2 = store.begin_write();
        w2.remove_column_tag(table_id, column_id, "pii", pii_begin)
            .await
            .unwrap();
        let cr2 = w2.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr2);
    }

    let ctags3 = {
        let reader3 = store.read_latest();
        reader3.list_column_tags(table_id, column_id).await.unwrap()
    };
    assert!(
        ctags3.is_empty(),
        "column tag should be retired after remove"
    );
}

// ─── 5. Sort info lifecycle ───────────────────────────────────────────────────

#[tokio::test]
async fn sort_info_lifecycle_add_read() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let table_id = {
        let mut w = store.begin_write();
        let sid = w.create_schema("main").await.unwrap();
        let tid = w.create_table(sid, "events", None).await.unwrap();
        // add_sort_info(table_id, sort_id): sort_id is caller-assigned
        w.add_sort_info(tid, 1).await.unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        tid
    };

    let sorts = {
        let reader = store.read_latest();
        reader.list_all_sort_info().await.unwrap()
    };
    assert_eq!(sorts.len(), 1, "should see 1 sort info row");
    assert_eq!(sorts[0].sort_id, 1);
    assert_eq!(sorts[0].table_id, table_id);
    assert!(sorts[0].end_snapshot.is_none(), "sort info should be live");
}

// ─── 6. DROP TABLE cascades sort_info ────────────────────────────────────────

#[tokio::test]
async fn drop_table_cascades_sort_info() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (schema_id, table_id, snap1) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("main").await.unwrap();
        let tid = w.create_table(sid, "events", None).await.unwrap();
        w.add_sort_info(tid, 1).await.unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        (sid, tid, cr)
    };

    // Verify sort info present before drop.
    {
        let reader1 = store.read_latest();
        let sorts_before = reader1.list_all_sort_info().await.unwrap();
        assert_eq!(
            sorts_before.len(),
            1,
            "sort info should be visible before drop"
        );
    }

    // Drop table — sort info must be cascade-retired.
    {
        let mut w2 = store.begin_write();
        w2.drop_table(schema_id, table_id, snap1.0).await.unwrap();
        let snap2 = w2.create_snapshot(None, None).await.unwrap();
        store.commit_writer(snap2);
    }

    let sorts_after = {
        let reader2 = store.read_latest();
        reader2.list_all_sort_info().await.unwrap()
    };
    assert!(
        sorts_after.iter().all(|s| s.end_snapshot.is_some()),
        "sort info should be retired after DROP TABLE"
    );
}

// ─── 7. DROP TABLE cascades tags and column tags ──────────────────────────────

#[tokio::test]
async fn drop_table_cascades_tags_and_column_tags() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (schema_id, table_id, snap1) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("main").await.unwrap();
        let tid = w.create_table(sid, "events", None).await.unwrap();
        let cid = w
            .add_column(tid, "email", "VARCHAR", 0, false, None)
            .await
            .unwrap();
        w.set_tag(tid, "owner", "data-team").await.unwrap();
        w.set_column_tag(tid, cid, "pii", "true").await.unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        (sid, tid, cr)
    };

    // Verify tags present.
    {
        let reader1 = store.read_latest();
        let tags_before = reader1.list_all_tags().await.unwrap();
        let ctags_before = reader1.list_all_column_tags().await.unwrap();
        assert_eq!(tags_before.len(), 1, "table tag should exist before drop");
        assert_eq!(ctags_before.len(), 1, "column tag should exist before drop");
    }

    // Drop table.
    {
        let mut w2 = store.begin_write();
        w2.drop_table(schema_id, table_id, snap1.0).await.unwrap();
        let snap2 = w2.create_snapshot(None, None).await.unwrap();
        store.commit_writer(snap2);
    }

    let (tags_after, ctags_after) = {
        let reader2 = store.read_latest();
        let t = reader2.list_all_tags().await.unwrap();
        let ct = reader2.list_all_column_tags().await.unwrap();
        (t, ct)
    };

    assert!(
        tags_after.iter().all(|t| t.end_snapshot.is_some()),
        "all table tags should be retired after DROP TABLE"
    );
    assert!(
        ctags_after.iter().all(|t| t.end_snapshot.is_some()),
        "all column tags should be retired after DROP TABLE"
    );
}

// ─── 8. ALTER TABLE add column time-travel ───────────────────────────────────

#[tokio::test]
async fn alter_table_add_column_time_travel() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (schema_id, table_id, snap1) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("main").await.unwrap();
        let tid = w.create_table(sid, "events", None).await.unwrap();
        w.add_column(tid, "id", "BIGINT", 0, false, None)
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        (sid, tid, cr)
    };
    let _ = schema_id;

    // Add a new column.
    {
        let mut w2 = store.begin_write();
        w2.add_column(table_id, "payload", "VARCHAR", 1, true, None)
            .await
            .unwrap();
        let snap2 = w2.create_snapshot(None, None).await.unwrap();
        store.commit_writer(snap2);
    }

    // At snap1: only 'id' column.
    let cols_old = {
        let reader_old = store.read_at(snap1).unwrap();
        let (_, cols) = reader_old
            .describe_table(table_id)
            .await
            .unwrap()
            .expect("table must exist at snap1");
        cols
    };
    assert_eq!(cols_old.len(), 1, "snap1 should have only 1 column");
    assert_eq!(cols_old[0].column_name, "id");

    // Latest: both columns.
    let cols_new = {
        let reader_new = store.read_latest();
        let (_, cols) = reader_new
            .describe_table(table_id)
            .await
            .unwrap()
            .expect("table must exist at latest");
        cols
    };
    assert_eq!(cols_new.len(), 2, "latest should have 2 columns");
    let names: Vec<&str> = cols_new.iter().map(|c| c.column_name.as_str()).collect();
    assert!(names.contains(&"id"));
    assert!(names.contains(&"payload"));
}

// ─── 9. ALTER TABLE drop column time-travel ──────────────────────────────────

#[tokio::test]
async fn alter_table_drop_column_time_travel() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (table_id, col_b, snap1) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("main").await.unwrap();
        let tid = w.create_table(sid, "events", None).await.unwrap();
        w.add_column(tid, "id", "BIGINT", 0, false, None)
            .await
            .unwrap();
        let cb = w
            .add_column(tid, "payload", "VARCHAR", 1, true, None)
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        (tid, cb, cr)
    };

    // Drop 'payload' column.
    {
        let mut w2 = store.begin_write();
        w2.drop_column(table_id, col_b, snap1.0).await.unwrap();
        let snap2 = w2.create_snapshot(None, None).await.unwrap();
        store.commit_writer(snap2);
    }

    // At snap1: both columns visible.
    let cols_old = {
        let reader_old = store.read_at(snap1).unwrap();
        let (_, cols) = reader_old
            .describe_table(table_id)
            .await
            .unwrap()
            .expect("table exists at snap1");
        cols
    };
    assert_eq!(cols_old.len(), 2, "snap1 should show both columns");

    // Latest: only 'id' remains.
    let cols_new = {
        let reader_new = store.read_latest();
        let (_, cols) = reader_new
            .describe_table(table_id)
            .await
            .unwrap()
            .expect("table exists at latest");
        cols
    };
    assert_eq!(cols_new.len(), 1, "latest should have 1 column after drop");
    assert_eq!(cols_new[0].column_name, "id");
}

// ─── 10. ALTER TABLE rename column ───────────────────────────────────────────

#[tokio::test]
async fn alter_table_rename_column() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (table_id, col_id, snap1) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("main").await.unwrap();
        let tid = w.create_table(sid, "users", None).await.unwrap();
        let cid = w
            .add_column(tid, "old_name", "VARCHAR", 0, false, None)
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        (tid, cid, cr)
    };

    // Rename the column: retire old row, insert new row with updated name.
    {
        let mut w2 = store.begin_write();
        let old_row = w2.rename_column(table_id, col_id).await.unwrap();
        w2.insert_renamed_column(old_row, "new_name").await.unwrap();
        let snap2 = w2.create_snapshot(None, None).await.unwrap();
        store.commit_writer(snap2);
    }

    // Latest: column has new name.
    let cols_latest = {
        let reader = store.read_latest();
        let (_, cols) = reader
            .describe_table(table_id)
            .await
            .unwrap()
            .expect("table exists");
        cols
    };
    assert_eq!(cols_latest.len(), 1, "should still have exactly 1 column");
    assert_eq!(
        cols_latest[0].column_name, "new_name",
        "column should be renamed"
    );

    // At snap1: still has old name.
    let cols_snap1 = {
        let reader_old = store.read_at(snap1).unwrap();
        let (_, cols) = reader_old
            .describe_table(table_id)
            .await
            .unwrap()
            .expect("table exists at snap1");
        cols
    };
    assert_eq!(cols_snap1.len(), 1);
    assert_eq!(
        cols_snap1[0].column_name, "old_name",
        "snap1 should still show old name"
    );
}

// ─── 11. DROP TABLE full cascade all types ───────────────────────────────────

#[tokio::test]
async fn drop_table_full_cascade_all_types() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (schema_id, table_id, snap1) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("main").await.unwrap();
        let tid = w.create_table(sid, "big_table", None).await.unwrap();
        let cid = w
            .add_column(tid, "ts", "TIMESTAMP", 0, false, None)
            .await
            .unwrap();
        w.set_tag(tid, "team", "platform").await.unwrap();
        w.set_column_tag(tid, cid, "sensitive", "no").await.unwrap();
        w.add_sort_info(tid, 1).await.unwrap();
        w.register_data_file(tid, "s3://b/f.parquet", "parquet", 100, 4096)
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        (sid, tid, cr)
    };

    // Verify all metadata types live before drop.
    {
        let r = store.read_latest();
        assert!(!r.list_all_tags().await.unwrap().is_empty());
        assert!(!r.list_all_column_tags().await.unwrap().is_empty());
        assert!(!r.list_all_sort_info().await.unwrap().is_empty());
        assert!(!r.list_data_files(table_id).await.unwrap().is_empty());
    }

    // Drop table.
    {
        let mut w2 = store.begin_write();
        w2.drop_table(schema_id, table_id, snap1.0).await.unwrap();
        let snap2 = w2.create_snapshot(None, None).await.unwrap();
        store.commit_writer(snap2);
    }

    // After drop: all metadata types retired or empty.
    {
        let r2 = store.read_latest();

        let tables = r2.list_tables(schema_id).await.unwrap();
        assert!(tables.is_empty(), "table should be retired after drop");

        let tags = r2.list_all_tags().await.unwrap();
        assert!(
            tags.iter().all(|t| t.end_snapshot.is_some()),
            "all tags retired after DROP TABLE"
        );

        let ctags = r2.list_all_column_tags().await.unwrap();
        assert!(
            ctags.iter().all(|t| t.end_snapshot.is_some()),
            "all column tags retired after DROP TABLE"
        );

        let sorts = r2.list_all_sort_info().await.unwrap();
        assert!(
            sorts.iter().all(|s| s.end_snapshot.is_some()),
            "all sort info retired after DROP TABLE"
        );

        let files = r2.list_data_files(table_id).await.unwrap();
        assert!(
            files.is_empty(),
            "data files should be retired after DROP TABLE"
        );
    }
}

// ─── 12. Describe table at snapshot before/after alter ───────────────────────

#[tokio::test]
async fn describe_table_at_snapshot_before_after_alter() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let (table_id, snap1) = {
        let mut w = store.begin_write();
        let sid = w.create_schema("main").await.unwrap();
        let tid = w.create_table(sid, "t", None).await.unwrap();
        w.add_column(tid, "a", "INTEGER", 0, false, None)
            .await
            .unwrap();
        w.add_column(tid, "b", "VARCHAR", 1, true, None)
            .await
            .unwrap();
        let cr = w.create_snapshot(None, None).await.unwrap();
        store.commit_writer(cr);
        (tid, cr)
    };

    // Add column c.
    {
        let mut w2 = store.begin_write();
        w2.add_column(table_id, "c", "BOOLEAN", 2, true, None)
            .await
            .unwrap();
        let snap2 = w2.create_snapshot(None, None).await.unwrap();
        store.commit_writer(snap2);
    }

    // At snap1: exactly 2 columns [a, b].
    let cols_snap1 = {
        let reader_snap1 = store.read_at(snap1).unwrap();
        let (_, cols) = reader_snap1
            .describe_table(table_id)
            .await
            .unwrap()
            .expect("table at snap1");
        cols
    };
    let names_snap1: Vec<&str> = cols_snap1.iter().map(|c| c.column_name.as_str()).collect();
    assert_eq!(names_snap1.len(), 2);
    assert!(names_snap1.contains(&"a"));
    assert!(names_snap1.contains(&"b"));
    assert!(!names_snap1.contains(&"c"), "c should not exist at snap1");

    // At latest: 3 columns [a, b, c].
    let cols_latest = {
        let reader_latest = store.read_latest();
        let (_, cols) = reader_latest
            .describe_table(table_id)
            .await
            .unwrap()
            .expect("table at latest");
        cols
    };
    let names_latest: Vec<&str> = cols_latest.iter().map(|c| c.column_name.as_str()).collect();
    assert_eq!(names_latest.len(), 3);
    assert!(names_latest.contains(&"c"), "c should exist at latest");
}
