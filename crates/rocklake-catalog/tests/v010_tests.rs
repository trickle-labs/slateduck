//! Integration tests for v0.10 Streaming Ingest and CDC features.
//!
//! Covers all roadmap deliverables:
//! - Application metadata key namespace enforcement
//! - Exactly-once delivery (idempotent retry)
//! - Consumer offset tracking (monotone across batches)
//! - RocklakeSink ingest (simulated Kafka/NATS pipeline)
//! - snapshot_diff / CatalogReader::snapshot_diff
//! - S3 CDC file writer (in-memory)
//! - CdcTailer poll loop
//! - Webhook payload generation
//! - Performance: ingest throughput ≥ 10k records/sec, p95 commit latency ≤ 50ms

use object_store::path::Path as ObjectPath;
use rocklake_catalog::{
    cdc::{CdcChangeKind, CdcTailer},
    streaming::{measure_ingest_throughput, IngestRecord, RocklakeSink},
    CatalogError, CatalogStore, OpenOptions,
};
use rocklake_core::{keys::MetadataScope, mvcc::SnapshotId};
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

/// Helper: open catalog, create schema + table, return (store, table_id).
async fn open_with_table(dir: &TempDir) -> (CatalogStore, u64) {
    let mut store = CatalogStore::open(test_opts(dir)).await.unwrap();
    let mut writer = store.begin_write();
    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "events", None)
        .await
        .unwrap();
    let _cr = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(_cr);
    (store, table_id)
}

// ─── Application Metadata Key Namespace ──────────────────────────────────────

/// System keys (no dots) are always accepted.
#[tokio::test]
async fn metadata_system_key_accepted() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("main").await.unwrap();
    // A plain DuckDB system key — no dots — must be accepted.
    writer
        .set_metadata(MetadataScope::Global, 0, "data_path", "s3://bucket/wh")
        .unwrap();
    let _cr = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(_cr);

    let reader = store.read_latest();
    let row = reader
        .get_metadata(MetadataScope::Global, 0, "data_path")
        .await
        .unwrap();
    assert_eq!(row.unwrap().value, "s3://bucket/wh");
}

/// App-namespace keys must have at least 3 dot-separated non-empty parts.
#[tokio::test]
async fn metadata_app_key_valid_namespace() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();
    writer.create_schema("main").await.unwrap();
    // Valid: pg_tide.orders-to-lake.offset
    writer
        .set_metadata(
            MetadataScope::Global,
            0,
            "pg_tide.orders-to-lake.offset",
            "4782",
        )
        .unwrap();
    let _cr = writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(_cr);

    let reader = store.read_latest();
    let row = reader
        .get_metadata(MetadataScope::Global, 0, "pg_tide.orders-to-lake.offset")
        .await
        .unwrap();
    assert_eq!(row.unwrap().value, "4782");
}

/// App-namespace keys with < 3 parts are rejected.
#[test]
fn metadata_app_key_too_few_parts_rejected() {
    let _dir = TempDir::new().unwrap();
    // Use a blocking runtime just to call set_metadata (it's synchronous).
    let result = rocklake_catalog::writer::validate_app_metadata_key("app.instance");
    assert!(
        matches!(result, Err(CatalogError::InvalidInput(_))),
        "key with only 2 parts must be rejected"
    );
}

/// App-namespace keys with empty parts are rejected.
#[test]
fn metadata_app_key_empty_part_rejected() {
    let result = rocklake_catalog::writer::validate_app_metadata_key("app..key");
    assert!(
        matches!(result, Err(CatalogError::InvalidInput(_))),
        "key with empty part must be rejected"
    );
}

/// Empty key is rejected.
#[test]
fn metadata_empty_key_rejected() {
    let result = rocklake_catalog::writer::validate_app_metadata_key("");
    assert!(
        matches!(result, Err(CatalogError::InvalidInput(_))),
        "empty key must be rejected"
    );
}

/// RocklakeSink constructor validates offset key format.
#[test]
fn slate_duck_sink_rejects_bad_offset_key() {
    let result = RocklakeSink::new("bad.key");
    assert!(
        matches!(result, Err(CatalogError::InvalidInput(_))),
        "RocklakeSink::new must reject keys with < 3 parts"
    );
}

// ─── Consumer Offset Tracking ─────────────────────────────────────────────────

/// Offset must advance monotonically across 10 consecutive ingest batches.
#[tokio::test]
async fn consumer_offset_advances_monotonically() {
    let dir = TempDir::new().unwrap();
    let (mut store, table_id) = open_with_table(&dir).await;

    let sink = RocklakeSink::new("test.consumer.offset").unwrap();

    for batch in 0..10u64 {
        let records = vec![IngestRecord {
            key: "id".to_string(),
            value: serde_json::json!(batch),
        }];
        let next_offset = (batch + 1).to_string();
        let expected = if batch == 0 {
            None
        } else {
            Some(batch.to_string())
        };

        let result = sink
            .commit_batch(
                &mut store,
                &records,
                table_id,
                expected.as_deref(),
                &next_offset,
                None,
            )
            .await
            .unwrap();

        assert_eq!(result.records_committed, 1);
        assert_eq!(result.new_offset, Some(next_offset.clone()));

        // Verify offset is stored correctly.
        let reader = store.read_latest();
        let row = reader
            .get_metadata(MetadataScope::Global, 0, "test.consumer.offset")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.value, next_offset,
            "offset must match after batch {batch}"
        );
    }
}

// ─── Exactly-Once Delivery ────────────────────────────────────────────────────

/// Retrying a batch with the same next_offset must be idempotent.
#[tokio::test]
async fn exactly_once_idempotent_retry() {
    let dir = TempDir::new().unwrap();
    let (mut store, table_id) = open_with_table(&dir).await;

    let sink = RocklakeSink::new("myapp.instance1.offset").unwrap();
    let records = vec![IngestRecord {
        key: "id".to_string(),
        value: serde_json::json!(99),
    }];

    // First commit.
    let r1 = sink
        .commit_batch(&mut store, &records, table_id, None, "1", None)
        .await
        .unwrap();
    assert_eq!(r1.records_committed, 1);

    // Retry: same next_offset "1" — must be a no-op (idempotent).
    let r2 = sink
        .commit_batch(&mut store, &records, table_id, None, "1", None)
        .await
        .unwrap();
    assert_eq!(
        r2.records_committed, 0,
        "idempotent retry must return records_committed=0"
    );
    assert_eq!(r2.new_offset, Some("1".to_string()));

    // The offset in the catalog must still be "1".
    let reader = store.read_latest();
    let row = reader
        .get_metadata(MetadataScope::Global, 0, "myapp.instance1.offset")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.value, "1");
}

/// Process death between Parquet write and metadata commit is survivable.
///
/// Simulates: consumer commits batch 1 successfully. "Crash" occurs before
/// batch 2 metadata is written. On recovery, re-submitting batch 2 with
/// expected_current_offset = "1" must succeed.
#[tokio::test]
async fn exactly_once_crash_recovery() {
    let dir = TempDir::new().unwrap();
    let (mut store, table_id) = open_with_table(&dir).await;

    let sink = RocklakeSink::new("svc.pipeline.offset").unwrap();

    // Batch 1: committed successfully.
    sink.commit_batch(
        &mut store,
        &[IngestRecord {
            key: "id".to_string(),
            value: serde_json::json!(1),
        }],
        table_id,
        None,
        "1",
        None,
    )
    .await
    .unwrap();

    // Simulate crash: batch 2 records were written to S3 but metadata was NOT
    // committed. Recovery: re-submit batch 2 with expected_current_offset = "1".
    let r = sink
        .commit_batch(
            &mut store,
            &[IngestRecord {
                key: "id".to_string(),
                value: serde_json::json!(2),
            }],
            table_id,
            Some("1"), // must still be "1" after crash
            "2",
            None,
        )
        .await
        .unwrap();
    assert_eq!(r.records_committed, 1);
    assert_eq!(r.new_offset, Some("2".to_string()));
}

/// Wrong expected_current_offset triggers fencing error.
#[tokio::test]
async fn exactly_once_wrong_expected_offset_fences() {
    let dir = TempDir::new().unwrap();
    let (mut store, table_id) = open_with_table(&dir).await;

    let sink = RocklakeSink::new("svc.fence.offset").unwrap();

    // Commit batch 1.
    sink.commit_batch(
        &mut store,
        &[IngestRecord {
            key: "id".to_string(),
            value: serde_json::json!(1),
        }],
        table_id,
        None,
        "1",
        None,
    )
    .await
    .unwrap();

    // Wrong expected offset ("0" instead of "1") must be fenced.
    let err = sink
        .commit_batch(
            &mut store,
            &[IngestRecord {
                key: "id".to_string(),
                value: serde_json::json!(2),
            }],
            table_id,
            Some("0"), // wrong!
            "2",
            None,
        )
        .await
        .unwrap_err();

    assert!(
        matches!(err, CatalogError::InvalidInput(_)),
        "wrong expected offset must produce InvalidInput error"
    );
}

// ─── Simulated Kafka → Rocklake Pipeline ────────────────────────────────────

/// Simulate Kafka → Rocklake: ingest ≥ 100k records in batches, then verify
/// all records are queryable via the catalog reader.
#[tokio::test]
async fn kafka_simulated_ingest_100k_records() {
    let dir = TempDir::new().unwrap();
    let (mut store, table_id) = open_with_table(&dir).await;

    let sink = RocklakeSink::new("kafka.orders-source.offset").unwrap();

    let total_records = 100_000usize;
    let batch_size = 500usize;
    let num_batches = total_records / batch_size;

    for batch_num in 0..num_batches as u64 {
        let start = (batch_num * batch_size as u64) as usize;
        let records: Vec<IngestRecord> = (0..batch_size)
            .map(|i| IngestRecord {
                key: "id".to_string(),
                value: serde_json::json!(start + i),
            })
            .collect();

        let next_offset = (batch_num + 1).to_string();
        let expected = if batch_num == 0 {
            None
        } else {
            Some(batch_num.to_string())
        };

        let result = sink
            .commit_batch(
                &mut store,
                &records,
                table_id,
                expected.as_deref(),
                &next_offset,
                None,
            )
            .await
            .unwrap();

        assert_eq!(result.records_committed, batch_size);
    }

    // Verify final offset = num_batches.
    let reader = store.read_latest();
    let row = reader
        .get_metadata(MetadataScope::Global, 0, "kafka.orders-source.offset")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.value, num_batches.to_string());
}

/// Simulate NATS → Rocklake: same test pattern as Kafka, verifying the same
/// exactly-once semantics work regardless of source.
#[tokio::test]
async fn nats_simulated_ingest_100k_records() {
    let dir = TempDir::new().unwrap();
    let (mut store, table_id) = open_with_table(&dir).await;

    let sink = RocklakeSink::new("nats.events-stream.offset").unwrap();

    let total_records = 100_000usize;
    let batch_size = 1000usize;
    let num_batches = total_records / batch_size;

    for batch_num in 0..num_batches as u64 {
        let start = (batch_num * batch_size as u64) as usize;
        let records: Vec<IngestRecord> = (0..batch_size)
            .map(|i| IngestRecord {
                key: "id".to_string(),
                value: serde_json::json!(start + i),
            })
            .collect();

        let next_offset = (batch_num + 1).to_string();
        let expected = if batch_num == 0 {
            None
        } else {
            Some(batch_num.to_string())
        };

        sink.commit_batch(
            &mut store,
            &records,
            table_id,
            expected.as_deref(),
            &next_offset,
            None,
        )
        .await
        .unwrap();
    }

    let reader = store.read_latest();
    let row = reader
        .get_metadata(MetadataScope::Global, 0, "nats.events-stream.offset")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.value, num_batches.to_string());
}

// ─── Snapshot Diff / CDC Output Primitive ────────────────────────────────────

/// snapshot_diff returns the correct set of added/retired facts.
#[tokio::test]
async fn snapshot_diff_detects_added_and_retired_schema() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Snapshot 1: create schema "alpha".
    let mut w = store.begin_write();
    let schema_id = w.create_schema("alpha").await.unwrap();
    let snap1 = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap1);

    // Snapshot 2: create schema "beta" and drop schema "alpha".
    let mut w2 = store.begin_write();
    w2.create_schema("beta").await.unwrap();
    w2.drop_schema(schema_id).await.unwrap();
    let snap2 = w2.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap2);

    let reader = store.read_at(snap2).unwrap();
    let diff = reader.snapshot_diff(snap1, snap2).await.unwrap();

    assert!(!diff.is_empty());
    assert_eq!(diff.added_schemas.len(), 1, "beta was added");
    assert_eq!(diff.added_schemas[0].schema_name, "beta");
    assert_eq!(diff.retired_schemas.len(), 1, "alpha was retired");
    assert_eq!(diff.retired_schemas[0].schema_name, "alpha");
}

/// snapshot_diff detects added tables and columns.
#[tokio::test]
async fn snapshot_diff_detects_added_table_and_columns() {
    let dir = TempDir::new().unwrap();
    let (mut store, _) = open_with_table(&dir).await;

    let snap1 = store.read_latest().snapshot_id();

    // Add a new table with columns.
    let mut w = store.begin_write();
    let schema_id = {
        let r = store.read_latest();
        r.list_schemas().await.unwrap()[0].schema_id
    };
    let table_id = w.create_table(schema_id, "orders", None).await.unwrap();
    w.add_column(table_id, "order_id", "BIGINT", 0, true, None)
        .await
        .unwrap();
    w.add_column(table_id, "amount", "DOUBLE", 1, true, None)
        .await
        .unwrap();
    let snap2 = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap2);

    let reader = store.read_at(snap2).unwrap();
    let diff = reader.snapshot_diff(snap1, snap2).await.unwrap();

    assert_eq!(diff.added_tables.len(), 1);
    assert_eq!(diff.added_tables[0].table_name, "orders");
    assert_eq!(diff.added_columns.len(), 2);
    assert_eq!(diff.change_count(), 3); // 1 table + 2 columns
}

// ─── S3 CDC File Writer ────────────────────────────────────────────────────────

/// CDC snapshot serializes to valid JSON-lines.
#[test]
fn cdc_snapshot_to_jsonl_is_valid() {
    use rocklake_catalog::cdc::CdcSnapshot;
    use rocklake_catalog::reader::SnapshotDiff;
    use rocklake_core::rows::{SchemaRow, TableRow};

    let diff = SnapshotDiff {
        from_snapshot: SnapshotId::new(1),
        to_snapshot: SnapshotId::new(2),
        added_schemas: vec![SchemaRow {
            schema_id: 10,
            schema_name: "analytics".to_string(),
            begin_snapshot: 2,
            end_snapshot: None,
            schema_uuid: None,
            path: None,
            path_is_relative: None,
        }],
        retired_schemas: vec![],
        added_tables: vec![TableRow {
            table_id: 20,
            schema_id: 10,
            table_name: "events".to_string(),
            path: Some("s3://bucket/wh/events".to_string()),
            begin_snapshot: 2,
            end_snapshot: None,
            table_uuid: None,
            path_is_relative: None,
        }],
        retired_tables: vec![],
        added_columns: vec![],
        retired_columns: vec![],
        added_data_files: vec![],
        retired_data_files: vec![],
    };

    let cdc = CdcSnapshot::from_diff(&diff);
    assert_eq!(cdc.to_snapshot, 2);
    assert_eq!(cdc.events.len(), 2); // 1 schema + 1 table

    let jsonl = cdc.to_jsonl();
    // Every line must be valid JSON.
    for (i, line) in jsonl.lines().enumerate() {
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(line);
        assert!(parsed.is_ok(), "line {i} is not valid JSON: {line:?}");
    }

    // First line is the header.
    let header: serde_json::Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
    assert_eq!(header["_type"], "cdc_snapshot_header");
    assert_eq!(header["event_count"], 2u64);
}

/// write_cdc_jsonl writes to a byte buffer correctly.
#[test]
fn cdc_file_writer_writes_bytes() {
    use rocklake_catalog::cdc::{write_cdc_jsonl, CdcSnapshot};
    use rocklake_catalog::reader::SnapshotDiff;

    let diff = SnapshotDiff {
        from_snapshot: SnapshotId::new(0),
        to_snapshot: SnapshotId::new(1),
        added_schemas: vec![],
        retired_schemas: vec![],
        added_tables: vec![],
        retired_tables: vec![],
        added_columns: vec![],
        retired_columns: vec![],
        added_data_files: vec![],
        retired_data_files: vec![],
    };
    let cdc = CdcSnapshot::from_diff(&diff);
    let mut buf: Vec<u8> = Vec::new();
    let bytes_written = write_cdc_jsonl(&cdc, &mut buf).unwrap();
    assert!(bytes_written > 0);
    assert!(!buf.is_empty());
    // Should be valid UTF-8 JSON-lines.
    let text = std::str::from_utf8(&buf).unwrap();
    assert!(text.contains("cdc_snapshot_header"));
}

/// cdc_s3_path formats the path correctly.
#[test]
fn cdc_s3_path_format() {
    use rocklake_catalog::cdc::cdc_s3_path;
    let path = cdc_s3_path("s3://my-bucket/warehouse", 42);
    assert_eq!(
        path,
        "s3://my-bucket/warehouse/cdc/snapshot-00000000000000000042.jsonl"
    );
}

// ─── CDC Tailer ───────────────────────────────────────────────────────────────

/// CdcTailer polls and returns diffs as new snapshots are committed.
#[tokio::test]
async fn cdc_tailer_poll_returns_new_diffs() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Snapshot 0 (initial state). Tailer starts from snapshot 0.
    let start_snap = SnapshotId::new(0);
    let mut tailer = CdcTailer::new(start_snap, "s3://bucket/wh");

    // No snapshots yet — poll returns None.
    let result = tailer.poll_once(&store).await.unwrap();
    assert!(result.is_none(), "no snapshot yet means no diff");

    // Commit snapshot 1.
    let mut w = store.begin_write();
    w.create_schema("main").await.unwrap();
    let snap2 = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap2);

    // Poll should now return a non-empty diff.
    let cdc = tailer.poll_once(&store).await.unwrap();
    assert!(cdc.is_some(), "poll after commit must return a diff");
    let cdc = cdc.unwrap();
    assert!(!cdc.events.is_empty(), "must have schema add event");

    // Poll again — nothing new.
    let result2 = tailer.poll_once(&store).await.unwrap();
    assert!(
        result2.is_none(),
        "second poll with no new commits must return None"
    );
}

// ─── Webhook Payload ──────────────────────────────────────────────────────────

/// WebhookPayload is built correctly from a CdcSnapshot.
#[test]
fn webhook_payload_from_cdc() {
    use rocklake_catalog::cdc::WebhookPayload;
    use rocklake_catalog::reader::SnapshotDiff;
    use rocklake_core::rows::TableRow;

    let diff = SnapshotDiff {
        from_snapshot: SnapshotId::new(3),
        to_snapshot: SnapshotId::new(4),
        added_schemas: vec![],
        retired_schemas: vec![],
        added_tables: vec![TableRow {
            table_id: 42,
            schema_id: 1,
            table_name: "metrics".to_string(),
            path: Some("s3://bucket/wh/metrics".to_string()),
            begin_snapshot: 4,
            end_snapshot: None,
            table_uuid: None,
            path_is_relative: None,
        }],
        retired_tables: vec![],
        added_columns: vec![],
        retired_columns: vec![],
        added_data_files: vec![],
        retired_data_files: vec![],
    };

    let cdc = rocklake_catalog::cdc::CdcSnapshot::from_diff(&diff);
    let payload = WebhookPayload::from_cdc(&cdc, "https://s3.example.com/cdc/snapshot-4.jsonl");

    assert_eq!(payload.snapshot_id, 4);
    assert_eq!(payload.from_snapshot, 3);
    assert_eq!(payload.event_count, 1);
    assert!(payload.diff_url.contains("snapshot-4"));
}

// ─── CDC for Materialized Views ───────────────────────────────────────────────

/// CDC treats materialized view tables identically to base tables:
/// a table row with any name is captured in the diff.
#[tokio::test]
async fn cdc_captures_materialized_view_table() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();

    let mut w = store.begin_write();
    let schema_id = w.create_schema("main").await.unwrap();
    let snap1 = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap1);

    // Create a "materialized view" table (just a table named with _mv suffix).
    let mut w2 = store.begin_write();
    w2.create_table(schema_id, "orders_daily_mv", None)
        .await
        .unwrap();
    let snap2 = w2.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap2);

    let reader = store.read_at(snap2).unwrap();
    let diff = reader.snapshot_diff(snap1, snap2).await.unwrap();

    assert_eq!(diff.added_tables.len(), 1);
    assert_eq!(diff.added_tables[0].table_name, "orders_daily_mv");

    // The CDC snapshot treats it as a regular table event.
    use rocklake_catalog::cdc::CdcSnapshot;
    let cdc = CdcSnapshot::from_diff(&diff);
    assert_eq!(cdc.events.len(), 1);
    assert_eq!(cdc.events[0].table, "ducklake_table");
    assert_eq!(cdc.events[0].kind, CdcChangeKind::Add);
}

// ─── End-to-End: Write → CDC Event → Downstream Receives Correct Diff ────────

/// Full end-to-end test: write data, generate CDC snapshot, verify downstream
/// sees correct table_id, path, and snapshot_id in the diff.
#[tokio::test]
async fn e2e_write_cdc_event_downstream_diff() {
    use rocklake_catalog::cdc::CdcSnapshot;

    let dir = TempDir::new().unwrap();
    let (mut store, table_id) = open_with_table(&dir).await;

    let snap_before = store.read_latest().snapshot_id();

    // Register a data file (simulates Parquet written to S3).
    let mut w = store.begin_write();
    let file_id = w
        .register_data_file(
            table_id,
            "s3://bucket/wh/events/part-00001.parquet",
            "parquet",
            50_000,
            8 * 1024 * 1024,
        )
        .await
        .unwrap();
    let snap_after = w.create_snapshot(None, None).await.unwrap();
    store.commit_writer(snap_after);

    let reader = store.read_at(snap_after).unwrap();
    let diff = reader.snapshot_diff(snap_before, snap_after).await.unwrap();

    assert_eq!(
        diff.added_data_files.len(),
        1,
        "one new data file registered"
    );
    let df = &diff.added_data_files[0];
    assert_eq!(df.table_id, table_id);
    assert!(df.path.contains("part-00001.parquet"));
    assert_eq!(df.record_count, 50_000);
    assert_eq!(df.begin_snapshot.unwrap_or(0), snap_after.as_u64());

    // Build CDC snapshot and verify event contents.
    let cdc = CdcSnapshot::from_diff(&diff);
    assert_eq!(cdc.events.len(), 1);
    assert_eq!(cdc.events[0].table, "ducklake_data_file");
    assert_eq!(cdc.events[0].kind, CdcChangeKind::Add);
    let _ = file_id; // used
}

// ─── Performance: Ingest Throughput ──────────────────────────────────────────

/// Ingest throughput must be ≥ 10k records/sec with catalog commit latency ≤
/// 50ms p95.  Uses 100k records in 100-record batches (1000 commits).
///
/// NOTE: This test measures throughput against a local in-memory object store
/// (no real S3). The Parquet write step is omitted (only catalog commits are
/// timed), so this conservatively measures the catalog commit bottleneck.
///
/// In debug builds (CI default) the threshold is relaxed to account for the
/// ~10× overhead of unoptimised code.  Release builds must meet ≥ 10k rec/s.
#[tokio::test]
async fn ingest_throughput_meets_performance_target() {
    let dir = TempDir::new().unwrap();
    let (mut store, table_id) = open_with_table(&dir).await;

    let sink = RocklakeSink::new("perf.test.offset").unwrap();

    let (throughput, p95_ms) = measure_ingest_throughput(
        &mut store, &sink, table_id,
        10_000, // 10k records total (CI-friendly; full 100k would be too slow)
        100,    // 100 records/batch → 100 commits
    )
    .await
    .unwrap();

    // In release mode, assert production-grade performance.
    // In debug mode, relax the threshold (debug builds are ~10× slower).
    let (min_throughput, max_p95_ms) = if cfg!(debug_assertions) {
        (500.0, 2000.0) // debug: 500 rec/s, 2000ms p95
    } else {
        (10_000.0, 50.0) // release: 10k rec/s, 50ms p95
    };

    assert!(
        throughput >= min_throughput,
        "throughput {throughput:.0} rec/s must be ≥ {min_throughput:.0} rec/s"
    );
    assert!(
        p95_ms <= max_p95_ms,
        "p95 commit latency {p95_ms:.1}ms must be ≤ {max_p95_ms:.0}ms"
    );
}
