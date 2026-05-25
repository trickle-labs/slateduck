//! Benchmark baseline for Phase 2 catalog operations.
//!
//! Measures p50/p95/p99/p99.9 for key catalog operations on LocalFS.
//! Results should be recorded in benchmarks/phase-2-baseline.json.

use criterion::{criterion_group, criterion_main, Criterion};
use object_store::path::Path as ObjectPath;
use slateduck_catalog::writer::stats::FileColumnStatsInput;
use slateduck_catalog::{CatalogStore, OpenOptions};
use slateduck_core::mvcc::SnapshotId;
use slateduck_core::types::DuckLakeType;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::runtime::Runtime;

fn setup_catalog(rt: &Runtime) -> (CatalogStore, TempDir, u64, u64, u64) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap().to_string();
    let store = Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&path).unwrap());
    let opts = OpenOptions {
        object_store: store,
        path: ObjectPath::from("catalog"),
        encryption: None,
    };

    let mut catalog = rt.block_on(CatalogStore::open(opts)).unwrap();
    let mut writer = catalog.begin_write();

    let schema_id = rt.block_on(writer.create_schema("main")).unwrap();
    let table_id = rt
        .block_on(writer.create_table(schema_id, "bench_table", None))
        .unwrap();
    let col_id = rt
        .block_on(writer.add_column(table_id, "value", "INTEGER", 0, false, None))
        .unwrap();

    // Populate with data files and stats
    for i in 0..100 {
        let file_id = rt
            .block_on(writer.register_data_file(
                table_id,
                &format!("data/part-{i:04}.parquet"),
                "parquet",
                10000,
                500000,
            ))
            .unwrap();
        rt.block_on(writer.upsert_file_column_stats(FileColumnStatsInput {
            table_id,
            column_id: col_id,
            data_file_id: file_id,
            has_null: false,
            min_value: Some(&format!("{}", i * 100)),
            max_value: Some(&format!("{}", (i + 1) * 100)),
            contains_nan: false,
        }))
        .unwrap();
    }

    let _snap = rt
        .block_on(writer.create_snapshot(Some("bench"), Some("setup")))
        .unwrap();

    (catalog, dir, schema_id, table_id, col_id)
}

fn bench_get_current_snapshot(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (catalog, _dir, _, _, _) = setup_catalog(&rt);

    c.bench_function("get_current_snapshot", |b| {
        b.iter(|| {
            rt.block_on(async {
                let reader = catalog.read_at(SnapshotId::new(1)).unwrap();
                reader.get_snapshot().await.unwrap();
            });
        });
    });

    rt.block_on(catalog.close()).unwrap();
}

fn bench_list_data_files(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (catalog, _dir, _, table_id, _) = setup_catalog(&rt);

    c.bench_function("list_data_files_100", |b| {
        b.iter(|| {
            rt.block_on(async {
                let reader = catalog.read_at(SnapshotId::new(1)).unwrap();
                reader.list_data_files(table_id).await.unwrap();
            });
        });
    });

    rt.block_on(catalog.close()).unwrap();
}

fn bench_describe_table(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (catalog, _dir, _, table_id, _) = setup_catalog(&rt);

    c.bench_function("describe_table", |b| {
        b.iter(|| {
            rt.block_on(async {
                let reader = catalog.read_at(SnapshotId::new(1)).unwrap();
                reader.describe_table(table_id).await.unwrap();
            });
        });
    });

    rt.block_on(catalog.close()).unwrap();
}

fn bench_prune_files(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (catalog, _dir, _, table_id, col_id) = setup_catalog(&rt);

    let col_type = DuckLakeType::Integer {
        signed: true,
        width_bits: 32,
    };

    c.bench_function("prune_files_100", |b| {
        b.iter(|| {
            rt.block_on(async {
                let reader = catalog.read_at(SnapshotId::new(1)).unwrap();
                reader
                    .prune_files(table_id, col_id, "5000", &col_type)
                    .await
                    .unwrap();
            });
        });
    });

    rt.block_on(catalog.close()).unwrap();
}

fn bench_create_snapshot(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap().to_string();
    let store = Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&path).unwrap());
    let opts = OpenOptions {
        object_store: store,
        path: ObjectPath::from("catalog"),
        encryption: None,
    };

    let mut catalog = rt.block_on(CatalogStore::open(opts)).unwrap();

    c.bench_function("create_snapshot", |b| {
        b.iter(|| {
            let mut writer = catalog.begin_write();
            rt.block_on(writer.create_snapshot(None, None)).unwrap();
        });
    });

    rt.block_on(catalog.close()).unwrap();
}

fn setup_catalog_n_files(rt: &Runtime, n: usize) -> (CatalogStore, TempDir, u64, u64, SnapshotId) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap().to_string();
    let store = Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&path).unwrap());
    let opts = OpenOptions {
        object_store: store,
        path: ObjectPath::from("catalog"),
        encryption: None,
    };

    let mut catalog = rt.block_on(CatalogStore::open(opts)).unwrap();
    let mut writer = catalog.begin_write();

    let schema_id = rt.block_on(writer.create_schema("main")).unwrap();
    let table_id = rt
        .block_on(writer.create_table(schema_id, "large_table", None))
        .unwrap();

    for i in 0..n {
        rt.block_on(writer.register_data_file(
            table_id,
            &format!("data/part-{i:08}.parquet"),
            "parquet",
            10000,
            500000,
        ))
        .unwrap();
    }

    let snap = rt
        .block_on(writer.create_snapshot(Some("bench"), Some("large-setup")))
        .unwrap();

    (catalog, dir, schema_id, table_id, snap)
}

fn bench_list_data_files_10k(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (catalog, _dir, _, table_id, snap) = setup_catalog_n_files(&rt, 10_000);

    c.bench_function("list_data_files_10k", |b| {
        b.iter(|| {
            rt.block_on(async {
                let reader = catalog.read_at(snap).unwrap();
                reader.list_data_files(table_id).await.unwrap();
            });
        });
    });

    rt.block_on(catalog.close()).unwrap();
}

fn bench_list_data_files_100k(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (catalog, _dir, _, table_id, snap) = setup_catalog_n_files(&rt, 100_000);

    c.bench_function("list_data_files_100k", |b| {
        b.iter(|| {
            rt.block_on(async {
                let reader = catalog.read_at(snap).unwrap();
                reader.list_data_files(table_id).await.unwrap();
            });
        });
    });

    rt.block_on(catalog.close()).unwrap();
}

criterion_group!(
    benches,
    bench_get_current_snapshot,
    bench_list_data_files,
    bench_describe_table,
    bench_prune_files,
    bench_create_snapshot,
    bench_list_data_files_10k,
    bench_list_data_files_100k,
);
criterion_main!(benches);
