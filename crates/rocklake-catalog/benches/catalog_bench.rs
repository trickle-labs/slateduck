//! v0.42.0 TPC-H Catalog Benchmark Suite.
//!
//! Measures p50/p95/p99/p99.9 for key catalog operations on LocalFS.
//! Results are recorded in benchmarks/v0.42-catalog-bench.json.
//!
//! Coverage:
//! - `get_current_snapshot()` — warm-cache and cold-process variants
//! - `list_data_files(table)` at 10², 10⁴, 10⁵ file counts
//! - `describe_table` at 1, 50, 100, 500 columns
//! - `create_snapshot` at 1, 10, 100, 1 000 file additions
//! - `prune_files` with a single typed column predicate at 10⁵ files
//! - Concurrent reader throughput at 1, 4, 16 concurrent reader tasks

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use object_store::path::Path as ObjectPath;
use rocklake_catalog::writer::stats::FileColumnStatsInput;
use rocklake_catalog::{CatalogStore, CommitResult, OpenOptions};
use rocklake_core::mvcc::SnapshotId;
use rocklake_core::types::DuckLakeType;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::runtime::Runtime;

// ─── helpers ──────────────────────────────────────────────────────────────────

fn open_catalog(dir: &TempDir) -> CatalogStore {
    let rt = Runtime::new().unwrap();
    let path = dir.path().to_str().unwrap().to_string();
    let store = Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&path).unwrap());
    let opts = OpenOptions {
        object_store: store,
        path: ObjectPath::from("catalog"),
        encryption: None,
    };
    rt.block_on(CatalogStore::open(opts)).unwrap()
}

/// Build a catalog with `n_files` data files and `n_cols` columns, returning
/// the snapshot id produced at commit.
fn setup_catalog_n_cols_n_files(
    rt: &Runtime,
    n_cols: usize,
    n_files: usize,
) -> (CatalogStore, TempDir, u64, u64, CommitResult) {
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

    let col_types = ["INTEGER", "BIGINT", "DOUBLE", "VARCHAR", "BOOLEAN"];
    for c in 0..n_cols {
        rt.block_on(writer.add_column(
            table_id,
            &format!("col_{c}"),
            col_types[c % col_types.len()],
            c as u64,
            false,
            None,
        ))
        .unwrap();
    }

    for i in 0..n_files {
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
        .block_on(writer.create_snapshot(Some("bench"), Some("setup")))
        .unwrap();

    (catalog, dir, schema_id, table_id, snap)
}

/// Convenience: build catalog with 1 column and `n_files` data files plus
/// per-file column stats for pruning tests.
fn setup_catalog_with_stats(
    rt: &Runtime,
    n_files: usize,
) -> (CatalogStore, TempDir, u64, u64, u64, CommitResult) {
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
        .block_on(writer.create_table(schema_id, "stats_table", None))
        .unwrap();
    let col_id = rt
        .block_on(writer.add_column(table_id, "value", "INTEGER", 0, false, None))
        .unwrap();

    for i in 0..n_files {
        let file_id = rt
            .block_on(writer.register_data_file(
                table_id,
                &format!("data/part-{i:08}.parquet"),
                "parquet",
                10000,
                500000,
            ))
            .unwrap();
        let range_start = (i * 1000) as i64;
        rt.block_on(writer.upsert_file_column_stats(FileColumnStatsInput {
            table_id,
            column_id: col_id,
            data_file_id: file_id,
            contains_null: false,
            min_value: Some(&range_start.to_string()),
            max_value: Some(&(range_start + 999).to_string()),
            contains_nan: false,
            column_size_bytes: None,
            value_count: None,
            null_count: None,
            extra_stats: None,
        }))
        .unwrap();
    }

    let snap = rt
        .block_on(writer.create_snapshot(Some("bench"), Some("stats-setup")))
        .unwrap();

    (catalog, dir, schema_id, table_id, col_id, snap)
}

// ─── benchmark: get_current_snapshot (warm-cache) ─────────────────────────────

fn bench_get_current_snapshot_warm(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (catalog, _dir, _, _, snap) = setup_catalog_n_cols_n_files(&rt, 1, 100);

    c.bench_function("get_current_snapshot_warm", |b| {
        b.iter(|| {
            rt.block_on(async {
                let reader = catalog.read_at(snap).unwrap();
                reader.get_snapshot().await.unwrap();
            });
        });
    });

    rt.block_on(catalog.close()).unwrap();
}

// ─── benchmark: get_current_snapshot (cold — fresh catalog open each iter) ────

fn bench_get_current_snapshot_cold(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    // Build the catalog once; open a fresh handle each iteration to simulate
    // cold-process behaviour (no in-process block-cache warming).
    let dir = {
        let (cat, dir, _, _, _) = setup_catalog_n_cols_n_files(&rt, 1, 100);
        rt.block_on(cat.close()).unwrap();
        dir
    };

    c.bench_function("get_current_snapshot_cold", |b| {
        b.iter(|| {
            rt.block_on(async {
                let catalog = open_catalog(&dir);
                let reader = catalog.read_at(SnapshotId::new(1)).unwrap();
                reader.get_snapshot().await.unwrap();
                catalog.close().await.unwrap();
            });
        });
    });
}

// ─── benchmark: list_data_files at varying file counts ────────────────────────

fn bench_list_data_files(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("list_data_files");

    for &n in &[100usize, 10_000, 100_000] {
        let (catalog, _dir, _, table_id, snap) = setup_catalog_n_cols_n_files(&rt, 1, n);

        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                rt.block_on(async {
                    let reader = catalog.read_at(snap).unwrap();
                    reader.list_data_files(table_id).await.unwrap();
                });
            });
        });

        rt.block_on(catalog.close()).unwrap();
    }
    group.finish();
}

// ─── benchmark: describe_table at varying column counts ───────────────────────

fn bench_describe_table(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("describe_table");

    for &n_cols in &[1usize, 50, 100, 500] {
        let (catalog, _dir, _, table_id, snap) = setup_catalog_n_cols_n_files(&rt, n_cols, 10);

        group.bench_with_input(BenchmarkId::from_parameter(n_cols), &n_cols, |b, _| {
            b.iter(|| {
                rt.block_on(async {
                    let reader = catalog.read_at(snap).unwrap();
                    reader.describe_table(table_id).await.unwrap();
                });
            });
        });

        rt.block_on(catalog.close()).unwrap();
    }
    group.finish();
}

// ─── benchmark: create_snapshot at varying file-addition counts ───────────────

fn bench_create_snapshot(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("create_snapshot");

    for &n_additions in &[1usize, 10, 100, 1_000] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap().to_string();
        let store = Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&path).unwrap());
        let opts = OpenOptions {
            object_store: store,
            path: ObjectPath::from("catalog"),
            encryption: None,
        };

        let mut catalog = rt.block_on(CatalogStore::open(opts)).unwrap();
        let schema_id = {
            let mut writer = catalog.begin_write();
            let sid = rt.block_on(writer.create_schema("main")).unwrap();
            let tid = rt.block_on(writer.create_table(sid, "t", None)).unwrap();
            let _ = rt.block_on(writer.create_snapshot(None, None)).unwrap();
            let _ = tid;
            sid
        };

        // Pre-create the table id we will add files to.
        let table_id = {
            let reader = catalog.read_at(SnapshotId::new(1)).unwrap();
            let tables = rt.block_on(reader.list_tables(schema_id)).unwrap();
            tables[0].table_id
        };

        group.bench_with_input(
            BenchmarkId::from_parameter(n_additions),
            &n_additions,
            |b, &na| {
                b.iter(|| {
                    let mut writer = catalog.begin_write();
                    for j in 0..na {
                        rt.block_on(writer.register_data_file(
                            table_id,
                            &format!("data/bench-{j:06}.parquet"),
                            "parquet",
                            1000,
                            50000,
                        ))
                        .unwrap();
                    }
                    let _ = rt.block_on(writer.create_snapshot(None, None)).unwrap();
                });
            },
        );

        rt.block_on(catalog.close()).unwrap();
    }
    group.finish();
}

// ─── benchmark: prune_files at 10⁵ files ──────────────────────────────────────

fn bench_prune_files_100k(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (catalog, _dir, _, table_id, col_id, snap) = setup_catalog_with_stats(&rt, 100_000);

    let col_type = DuckLakeType::Integer {
        signed: true,
        width_bits: 32,
    };

    c.bench_function("prune_files_100k", |b| {
        b.iter(|| {
            rt.block_on(async {
                let reader = catalog.read_at(snap).unwrap();
                reader
                    .prune_files(table_id, col_id, "50_000_000", &col_type)
                    .await
                    .unwrap();
            });
        });
    });

    rt.block_on(catalog.close()).unwrap();
}

// ─── benchmark: concurrent reader throughput at 1 / 4 / 16 tasks ─────────────

fn bench_concurrent_readers(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (catalog, _dir, _, table_id, snap) = setup_catalog_n_cols_n_files(&rt, 10, 1_000);
    let catalog = Arc::new(catalog);

    let mut group = c.benchmark_group("concurrent_readers");

    for &parallelism in &[1usize, 4, 16] {
        group.bench_with_input(
            BenchmarkId::from_parameter(parallelism),
            &parallelism,
            |b, &p| {
                b.iter(|| {
                    let catalog = Arc::clone(&catalog);
                    rt.block_on(async move {
                        let mut handles = Vec::with_capacity(p);
                        for _ in 0..p {
                            let cat = Arc::clone(&catalog);
                            handles.push(tokio::spawn(async move {
                                let reader = cat.read_at(snap).unwrap();
                                reader.list_data_files(table_id).await.unwrap();
                            }));
                        }
                        for h in handles {
                            h.await.unwrap();
                        }
                    });
                });
            },
        );
    }
    group.finish();

    // Close the catalog once all benchmarks are done.
    // SAFETY: Arc has exactly one strong reference at this point (p=16 group
    // finished). Use try_unwrap to avoid a potential deadlock.
    match Arc::try_unwrap(catalog) {
        Ok(c) => {
            rt.block_on(c.close()).unwrap();
        }
        Err(_) => { /* other refs still alive — just drop */ }
    }
}

// ─── legacy single-function groups (retained for baseline regression) ─────────

fn bench_list_data_files_legacy(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (catalog, _dir, _, table_id, snap) = setup_catalog_n_cols_n_files(&rt, 1, 100);

    c.bench_function("list_data_files_100", |b| {
        b.iter(|| {
            rt.block_on(async {
                let reader = catalog.read_at(snap).unwrap();
                reader.list_data_files(table_id).await.unwrap();
            });
        });
    });

    rt.block_on(catalog.close()).unwrap();
}

fn bench_prune_files_legacy(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (catalog, _dir, _, table_id, col_id, snap) = setup_catalog_with_stats(&rt, 100);

    let col_type = DuckLakeType::Integer {
        signed: true,
        width_bits: 32,
    };

    c.bench_function("prune_files_100", |b| {
        b.iter(|| {
            rt.block_on(async {
                let reader = catalog.read_at(snap).unwrap();
                reader
                    .prune_files(table_id, col_id, "5000", &col_type)
                    .await
                    .unwrap();
            });
        });
    });

    rt.block_on(catalog.close()).unwrap();
}

fn bench_list_data_files_10k(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (catalog, _dir, _, table_id, snap) = setup_catalog_n_cols_n_files(&rt, 1, 10_000);

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
    let (catalog, _dir, _, table_id, snap) = setup_catalog_n_cols_n_files(&rt, 1, 100_000);

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
    bench_get_current_snapshot_warm,
    bench_get_current_snapshot_cold,
    bench_list_data_files,
    bench_describe_table,
    bench_create_snapshot,
    bench_prune_files_100k,
    bench_concurrent_readers,
    // Legacy single-function benchmarks retained for regression tracking.
    bench_list_data_files_legacy,
    bench_prune_files_legacy,
    bench_list_data_files_10k,
    bench_list_data_files_100k,
);
criterion_main!(benches);
