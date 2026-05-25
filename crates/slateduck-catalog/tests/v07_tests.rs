//! v0.7 tests: Performance & Ecosystem features.

use object_store::path::Path as ObjectPath;
use slateduck_catalog::partition::{CatalogRegistry, DatasetEntry, PartitionedWriter};
use slateduck_catalog::performance::{
    read_hot_key, read_packed_metadata, read_secondary_index, write_hot_key, write_packed_metadata,
    write_secondary_index, HotKeyState, SlateDbTuning,
};
use slateduck_catalog::{CatalogStore, OpenOptions};
use slateduck_core::mvcc::SnapshotId;
use slateduck_core::rows::{
    ColumnRow, PackedTableMetadata, PartitionInfoRow, SortInfoRow, TableStatsRow,
};
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

fn test_opts_with_subpath(dir: &TempDir, subpath: &str) -> OpenOptions {
    let path = dir.path().to_str().unwrap().to_string();
    let store = Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&path).unwrap());
    OpenOptions {
        object_store: store,
        path: ObjectPath::from(subpath),
        encryption: None,
    }
}

// ─── Hot Key Tests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn hot_key_read_returns_none_initially() {
    let dir = TempDir::new().unwrap();
    let store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let result = read_hot_key(store.db()).await.unwrap();
    assert!(result.is_none());
    store.close().await.unwrap();
}

#[tokio::test]
async fn hot_key_write_and_read() {
    let dir = TempDir::new().unwrap();
    let store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let state = HotKeyState {
        current_snapshot_id: 42,
        table_file_counts: vec![(1, 100), (2, 50), (3, 200)],
    };

    write_hot_key(store.db(), &state).await.unwrap();

    let loaded = read_hot_key(store.db()).await.unwrap().unwrap();
    assert_eq!(loaded.current_snapshot_id, 42);
    assert_eq!(loaded.table_file_counts.len(), 3);
    assert_eq!(loaded.table_file_counts[0], (1, 100));
    assert_eq!(loaded.table_file_counts[1], (2, 50));
    assert_eq!(loaded.table_file_counts[2], (3, 200));

    store.close().await.unwrap();
}

#[tokio::test]
async fn hot_key_update_overwrites() {
    let dir = TempDir::new().unwrap();
    let store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let state1 = HotKeyState {
        current_snapshot_id: 1,
        table_file_counts: vec![(1, 10)],
    };
    write_hot_key(store.db(), &state1).await.unwrap();

    let state2 = HotKeyState {
        current_snapshot_id: 5,
        table_file_counts: vec![(1, 50), (2, 30)],
    };
    write_hot_key(store.db(), &state2).await.unwrap();

    let loaded = read_hot_key(store.db()).await.unwrap().unwrap();
    assert_eq!(loaded.current_snapshot_id, 5);
    assert_eq!(loaded.table_file_counts.len(), 2);

    store.close().await.unwrap();
}

// ─── Secondary Index Tests ─────────────────────────────────────────────────

#[tokio::test]
async fn secondary_index_empty_returns_empty_vec() {
    let dir = TempDir::new().unwrap();
    let store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let entries = read_secondary_index(store.db(), 1, 1).await.unwrap();
    assert!(entries.is_empty());

    store.close().await.unwrap();
}

#[tokio::test]
async fn secondary_index_write_and_read() {
    let dir = TempDir::new().unwrap();
    let store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    write_secondary_index(store.db(), 1, 100, 1001, "data/file1.parquet")
        .await
        .unwrap();
    write_secondary_index(store.db(), 1, 100, 1002, "data/file2.parquet")
        .await
        .unwrap();
    write_secondary_index(store.db(), 1, 200, 2001, "data/other.parquet")
        .await
        .unwrap();

    // Read files for snapshot 1, table 100
    let entries = read_secondary_index(store.db(), 1, 100).await.unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].data_file_id, 1001);
    assert_eq!(entries[0].path, "data/file1.parquet");
    assert_eq!(entries[1].data_file_id, 1002);
    assert_eq!(entries[1].path, "data/file2.parquet");

    // Read files for snapshot 1, table 200
    let entries = read_secondary_index(store.db(), 1, 200).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].data_file_id, 2001);

    // Read files for nonexistent snapshot
    let entries = read_secondary_index(store.db(), 99, 100).await.unwrap();
    assert!(entries.is_empty());

    store.close().await.unwrap();
}

#[tokio::test]
async fn secondary_index_multiple_snapshots() {
    let dir = TempDir::new().unwrap();
    let store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Snapshot 1: add 2 files to table 1
    write_secondary_index(store.db(), 1, 1, 10, "a.parquet")
        .await
        .unwrap();
    write_secondary_index(store.db(), 1, 1, 11, "b.parquet")
        .await
        .unwrap();

    // Snapshot 2: add 1 more file to table 1
    write_secondary_index(store.db(), 2, 1, 12, "c.parquet")
        .await
        .unwrap();

    let snap1 = read_secondary_index(store.db(), 1, 1).await.unwrap();
    assert_eq!(snap1.len(), 2);

    let snap2 = read_secondary_index(store.db(), 2, 1).await.unwrap();
    assert_eq!(snap2.len(), 1);

    store.close().await.unwrap();
}

// ─── Packed Metadata Tests ─────────────────────────────────────────────────

#[tokio::test]
async fn packed_metadata_read_returns_none_initially() {
    let dir = TempDir::new().unwrap();
    let store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let result = read_packed_metadata(store.db(), 1).await.unwrap();
    assert!(result.is_none());

    store.close().await.unwrap();
}

#[tokio::test]
async fn packed_metadata_write_and_read() {
    let dir = TempDir::new().unwrap();
    let store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let metadata = PackedTableMetadata {
        table_id: 42,
        columns: vec![
            ColumnRow {
                column_id: 1,
                table_id: 42,
                column_name: "id".to_string(),
                data_type: "INTEGER".to_string(),
                column_index: 0,
                begin_snapshot: 1,
                end_snapshot: None,
                default_value: None,
                is_nullable: false,
            },
            ColumnRow {
                column_id: 2,
                table_id: 42,
                column_name: "name".to_string(),
                data_type: "VARCHAR".to_string(),
                column_index: 1,
                begin_snapshot: 1,
                end_snapshot: None,
                default_value: None,
                is_nullable: true,
            },
        ],
        partition_info: vec![PartitionInfoRow {
            partition_id: 1,
            table_id: 42,
            begin_snapshot: 1,
            end_snapshot: None,
        }],
        sort_info: vec![SortInfoRow {
            sort_id: 1,
            table_id: 42,
            begin_snapshot: 1,
            end_snapshot: None,
        }],
        table_stats: Some(TableStatsRow {
            table_id: 42,
            record_count: 1000,
            file_count: 5,
            file_size_bytes: 50000,
            next_row_id: None,
        }),
        schema_version: 3,
    };

    write_packed_metadata(store.db(), &metadata).await.unwrap();

    let loaded = read_packed_metadata(store.db(), 42).await.unwrap().unwrap();
    assert_eq!(loaded.table_id, 42);
    assert_eq!(loaded.columns.len(), 2);
    assert_eq!(loaded.columns[0].column_name, "id");
    assert_eq!(loaded.columns[1].column_name, "name");
    assert_eq!(loaded.partition_info.len(), 1);
    assert_eq!(loaded.sort_info.len(), 1);
    assert_eq!(loaded.table_stats.unwrap().record_count, 1000);
    assert_eq!(loaded.schema_version, 3);

    store.close().await.unwrap();
}

// ─── SlateDB Tuning Tests ──────────────────────────────────────────────────

#[test]
fn slatedb_tuning_default() {
    let tuning = SlateDbTuning::default();
    assert_eq!(tuning.block_size, 4096);
    assert!(tuning.bloom_filter_enabled);
    assert_eq!(tuning.bloom_filter_fp_rate, 0.01);
    assert_eq!(tuning.l0_sst_count_threshold, 4);
    assert_eq!(tuning.max_write_batch_bytes, 64 * 1024 * 1024);
    assert_eq!(tuning.compaction_aggressiveness, 5);
}

#[test]
fn slatedb_tuning_high_ingest() {
    let tuning = SlateDbTuning::high_ingest();
    assert_eq!(tuning.l0_sst_count_threshold, 2);
    assert_eq!(tuning.compaction_aggressiveness, 8);
}

#[test]
fn slatedb_tuning_read_heavy() {
    let tuning = SlateDbTuning::read_heavy();
    assert_eq!(tuning.block_size, 8192);
    assert_eq!(tuning.bloom_filter_fp_rate, 0.005);
    assert_eq!(tuning.l0_sst_count_threshold, 8);
    assert_eq!(tuning.compaction_aggressiveness, 3);
}

// ─── Multi-Writer Partitioning Tests ───────────────────────────────────────

#[tokio::test]
async fn catalog_registry_register_and_list_datasets() {
    let dir = TempDir::new().unwrap();
    let opts = test_opts_with_subpath(&dir, "registry");

    let mut registry = CatalogRegistry::open(opts).await.unwrap();

    let entry1 = DatasetEntry {
        name: "sales".to_string(),
        catalog_path: "catalogs/sales".to_string(),
        description: Some("Sales dataset".to_string()),
        created_at: "2025-01-01T00:00:00Z".to_string(),
    };

    let entry2 = DatasetEntry {
        name: "users".to_string(),
        catalog_path: "catalogs/users".to_string(),
        description: None,
        created_at: "2025-01-02T00:00:00Z".to_string(),
    };

    registry.register_dataset(&entry1).await.unwrap();
    registry.register_dataset(&entry2).await.unwrap();

    let datasets = registry.list_datasets().await.unwrap();
    assert_eq!(datasets.len(), 2);

    let sales = registry.get_dataset("sales").await.unwrap().unwrap();
    assert_eq!(sales.name, "sales");
    assert_eq!(sales.catalog_path, "catalogs/sales");
    assert_eq!(sales.description, Some("Sales dataset".to_string()));

    registry.close().await.unwrap();
}

#[tokio::test]
async fn catalog_registry_unregister_dataset() {
    let dir = TempDir::new().unwrap();
    let opts = test_opts_with_subpath(&dir, "registry");

    let mut registry = CatalogRegistry::open(opts).await.unwrap();

    let entry = DatasetEntry {
        name: "temp".to_string(),
        catalog_path: "catalogs/temp".to_string(),
        description: None,
        created_at: "2025-01-01T00:00:00Z".to_string(),
    };

    registry.register_dataset(&entry).await.unwrap();
    registry.unregister_dataset("temp").await.unwrap();

    let result = registry.get_dataset("temp").await.unwrap();
    assert!(result.is_none());

    registry.close().await.unwrap();
}

#[tokio::test]
async fn partitioned_writer_open_multiple_datasets() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap().to_string();
    let store = Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&path).unwrap());

    let mut writer = PartitionedWriter::new(store, ObjectPath::from("multi"));

    // Open two dataset catalogs
    let ds1 = writer.open_dataset("dataset_a").await.unwrap();
    let mut w1 = ds1.begin_write();
    let _schema_id = w1.create_schema("main").await.unwrap();
    w1.create_snapshot(None, None).await.unwrap();

    let ds2 = writer.open_dataset("dataset_b").await.unwrap();
    let mut w2 = ds2.begin_write();
    let _schema_id_b = w2.create_schema("analytics").await.unwrap();
    w2.create_snapshot(None, None).await.unwrap();

    // Verify isolation: each dataset has its own schema
    let ds1_ref = writer.open_dataset("dataset_a").await.unwrap();
    let r1 = ds1_ref.read_at(SnapshotId::new(1)).unwrap();
    let schemas_a = r1.list_schemas().await.unwrap();
    assert_eq!(schemas_a.len(), 1);
    assert_eq!(schemas_a[0].schema_name, "main");

    let ds2_ref = writer.open_dataset("dataset_b").await.unwrap();
    let r2 = ds2_ref.read_at(SnapshotId::new(1)).unwrap();
    let schemas_b = r2.list_schemas().await.unwrap();
    assert_eq!(schemas_b.len(), 1);
    assert_eq!(schemas_b[0].schema_name, "analytics");

    assert_eq!(writer.open_datasets().len(), 2);

    writer.close_all().await.unwrap();
}

#[tokio::test]
async fn partitioned_writer_concurrent_independent_writes() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap().to_string();
    let store = Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&path).unwrap());

    // Writer 1 works on dataset_x
    let mut writer1 = PartitionedWriter::new(store.clone(), ObjectPath::from("concurrent"));
    let ds_x = writer1.open_dataset("x").await.unwrap();
    let mut wx = ds_x.begin_write();
    wx.create_schema("schema_x").await.unwrap();
    wx.create_snapshot(None, None).await.unwrap();

    // Writer 2 works on dataset_y (no contention with writer 1)
    let mut writer2 = PartitionedWriter::new(store.clone(), ObjectPath::from("concurrent"));
    let ds_y = writer2.open_dataset("y").await.unwrap();
    let mut wy = ds_y.begin_write();
    wy.create_schema("schema_y").await.unwrap();
    wy.create_snapshot(None, None).await.unwrap();

    // Verify both are independent
    let ds_x_read = writer1.open_dataset("x").await.unwrap();
    let rx = ds_x_read.read_at(SnapshotId::new(1)).unwrap();
    let schemas_x = rx.list_schemas().await.unwrap();
    assert_eq!(schemas_x[0].schema_name, "schema_x");

    let ds_y_read = writer2.open_dataset("y").await.unwrap();
    let ry = ds_y_read.read_at(SnapshotId::new(1)).unwrap();
    let schemas_y = ry.list_schemas().await.unwrap();
    assert_eq!(schemas_y[0].schema_name, "schema_y");

    writer1.close_all().await.unwrap();
    writer2.close_all().await.unwrap();
}

// ─── Benchmark Report Tests ────────────────────────────────────────────────

#[test]
fn benchmark_report_serialization() {
    use slateduck_catalog::performance::{BenchmarkMeasurement, BenchmarkReport, ComparisonEntry};

    let report = BenchmarkReport {
        version: "0.7.0".to_string(),
        timestamp: "2025-05-23T00:00:00Z".to_string(),
        storage: "LocalFileSystem".to_string(),
        measurements: vec![
            BenchmarkMeasurement {
                operation: "list_data_files_10k".to_string(),
                p50_us: 450,
                p95_us: 1800,
                p99_us: 4500,
            },
            BenchmarkMeasurement {
                operation: "create_snapshot_1_file".to_string(),
                p50_us: 180,
                p95_us: 900,
                p99_us: 2700,
            },
            BenchmarkMeasurement {
                operation: "cold_start_read".to_string(),
                p50_us: 45,
                p95_us: 180,
                p99_us: 450,
            },
        ],
        comparison_vs_baseline: vec![
            ComparisonEntry {
                operation: "list_data_files_100".to_string(),
                baseline_p50_us: 500,
                current_p50_us: 450,
                ratio: 0.9,
            },
            ComparisonEntry {
                operation: "create_snapshot".to_string(),
                baseline_p50_us: 200,
                current_p50_us: 180,
                ratio: 0.9,
            },
        ],
    };

    let json = serde_json::to_string_pretty(&report).unwrap();
    let parsed: BenchmarkReport = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.version, "0.7.0");
    assert_eq!(parsed.measurements.len(), 3);
    assert_eq!(parsed.comparison_vs_baseline.len(), 2);
    assert_eq!(parsed.comparison_vs_baseline[0].ratio, 0.9);
}

// ─── Integration: Hot Key + Snapshot Creation ──────────────────────────────

#[tokio::test]
async fn hot_key_updated_after_snapshot() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Create some data
    let mut writer = store.begin_write();
    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "events", None)
        .await
        .unwrap();
    writer
        .register_data_file(table_id, "file1.parquet", "parquet", 1000, 50000)
        .await
        .unwrap();
    writer
        .register_data_file(table_id, "file2.parquet", "parquet", 2000, 100000)
        .await
        .unwrap();
    let snap = writer.create_snapshot(None, None).await.unwrap();

    // Write hot key with current state
    let hot_state = HotKeyState {
        current_snapshot_id: snap.as_u64(),
        table_file_counts: vec![(table_id, 2)],
    };
    write_hot_key(store.db(), &hot_state).await.unwrap();

    // Simulate cold start: read hot key
    let loaded = read_hot_key(store.db()).await.unwrap().unwrap();
    assert_eq!(loaded.current_snapshot_id, snap.as_u64());
    assert_eq!(loaded.table_file_counts[0], (table_id, 2));

    store.close().await.unwrap();
}
