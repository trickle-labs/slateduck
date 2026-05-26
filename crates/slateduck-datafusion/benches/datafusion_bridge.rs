//! N-05: DataFusion AsyncBridge benchmark.
//!
//! Measures the overhead of `AsyncBridge::run_sync` after replacing the
//! per-call `std::thread::spawn` approach with a single persistent background
//! thread.  Run with:
//!   cargo bench -p slateduck-datafusion

use criterion::{criterion_group, criterion_main, Criterion};
use datafusion::catalog::CatalogProvider;
use object_store::path::Path as ObjectPath;
use slateduck_catalog::{CatalogStore, OpenOptions};
use slateduck_core::mvcc::SnapshotId;
use slateduck_datafusion::SlateDuckCatalogProvider;
use std::sync::Arc;
use tempfile::TempDir;

fn setup_catalog() -> (TempDir, CatalogStore) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap().to_string();
    let store = Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&path).unwrap());
    let opts = OpenOptions {
        object_store: store,
        path: ObjectPath::from("catalog"),
        encryption: None,
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut catalog = rt.block_on(CatalogStore::open(opts)).unwrap();
    rt.block_on(async {
        let mut writer = catalog.begin_write();
        let sid = writer.create_schema("main").await.unwrap();
        writer.create_table(sid, "events", None).await.unwrap();
        writer.create_table(sid, "orders", None).await.unwrap();
        let cr = writer.create_snapshot(None, None).await.unwrap();
        catalog.commit_writer(cr);
    });
    (dir, catalog)
}

fn bench_schema_names(c: &mut Criterion) {
    let (_dir, store) = setup_catalog();
    let provider = SlateDuckCatalogProvider::new(store, Some(SnapshotId::new(1)));

    c.bench_function("async_bridge_schema_names", |b| {
        b.iter(|| {
            let names = provider.schema_names();
            assert_eq!(names.len(), 1);
        })
    });
}

fn bench_table_names(c: &mut Criterion) {
    let (_dir, store) = setup_catalog();
    let provider = SlateDuckCatalogProvider::new(store, Some(SnapshotId::new(1)));

    c.bench_function("async_bridge_table_names", |b| {
        b.iter(|| {
            let schema_prov = provider.schema("main").unwrap();
            let tables = schema_prov.table_names();
            assert_eq!(tables.len(), 2);
        })
    });
}

criterion_group!(benches, bench_schema_names, bench_table_names);
criterion_main!(benches);
