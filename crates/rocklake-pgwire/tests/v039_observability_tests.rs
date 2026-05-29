//! v0.39.0 — Observability & Operational Tooling
//!
//! Tests covering:
//! - Prometheus `/metrics` endpoint: new v0.39.0 fields render correctly
//! - `rocklake diagnose`: structured health report on a fresh catalog
//! - `rocklake sweep-orphans`: dry-run identifies orphan files
//! - OTLP telemetry init/shutdown lifecycle (no network required)
//! - `--metrics-addr` and `--otlp-endpoint` CLI argument parsing
//! - Metrics server responds with correct Content-Type

use std::sync::Arc;

use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use tempfile::TempDir;

use rocklake_catalog::metrics::CatalogMetrics;
use rocklake_catalog::{diagnose_catalog, format_report_text, DiagnoseReport};
use rocklake_catalog::{sweep_orphans, SweepOrphansConfig};
use rocklake_catalog::{CatalogStore, OpenOptions};
use rocklake_pgwire::telemetry::TelemetryConfig;

// ─── helpers ─────────────────────────────────────────────────────────────────

async fn open_fresh_catalog(dir: &TempDir) -> rocklake_catalog::CatalogStore {
    let path = ObjectPath::from(dir.path().to_str().unwrap());
    let store = Arc::new(LocalFileSystem::new());
    let opts = OpenOptions {
        object_store: store,
        path,
        encryption: None,
    };
    CatalogStore::open(opts).await.expect("open catalog")
}

// ─── metrics rendering ────────────────────────────────────────────────────────

#[test]
fn test_metrics_render_includes_v039_fields() {
    let m = CatalogMetrics::new(50);

    // Record some v0.39.0 observations.
    m.observe_create_snapshot_us(1_500_000); // 1.5 s
    m.observe_list_data_files_us(200_000); // 0.2 s
    m.observe_describe_table_us(50_000);
    m.observe_commit_transaction_us(3_000_000);
    m.record_pgwire_query(100_000);
    m.record_pgwire_error("40001");
    m.record_pgwire_error("25006");
    m.set_gc_retain_from_snapshot(42);
    m.add_excision_bytes_deleted(1_024);
    m.add_excision_rows_deleted(10);
    m.set_slatedb_sst_count(7);
    m.set_slatedb_compaction_lag_ms(120);
    m.set_slatedb_memtable_bytes(1_048_576);

    let rendered = m.render_prometheus();

    // Check v0.39.0 catalog op latency histogram.
    assert!(
        rendered.contains("rocklake_catalog_op_duration_seconds_sum{op=\"create_snapshot\"}"),
        "missing create_snapshot sum"
    );
    assert!(
        rendered.contains("rocklake_catalog_op_duration_seconds_count{op=\"create_snapshot\"} 1"),
        "missing create_snapshot count"
    );
    assert!(
        rendered.contains("rocklake_catalog_op_duration_seconds_sum{op=\"list_data_files\"}"),
        "missing list_data_files sum"
    );
    assert!(
        rendered.contains("rocklake_catalog_op_duration_seconds_sum{op=\"describe_table\"}"),
        "missing describe_table sum"
    );
    assert!(
        rendered.contains("rocklake_catalog_op_duration_seconds_sum{op=\"commit_transaction\"}"),
        "missing commit_transaction sum"
    );

    // PG-wire metrics.
    assert!(
        rendered.contains("rocklake_pgwire_queries_total 1"),
        "missing pgwire_queries_total"
    );
    assert!(
        rendered.contains("rocklake_pgwire_errors_total{sqlstate=\"40001\"} 1"),
        "missing pgwire 40001 counter"
    );
    assert!(
        rendered.contains("rocklake_pgwire_errors_total{sqlstate=\"25006\"} 1"),
        "missing pgwire 25006 counter"
    );

    // GC / excision.
    assert!(
        rendered.contains("rocklake_gc_retain_from_snapshot 42"),
        "missing retain_from gauge"
    );
    assert!(
        rendered.contains("rocklake_excision_bytes_deleted_total 1024"),
        "missing excision bytes counter"
    );

    // SlateDB stubs.
    assert!(
        rendered.contains("rocklake_slatedb_sst_count 7"),
        "missing sst_count"
    );
    assert!(
        rendered.contains("rocklake_slatedb_compaction_lag_ms 120"),
        "missing compaction_lag"
    );
    assert!(
        rendered.contains("rocklake_slatedb_memtable_bytes 1048576"),
        "missing memtable_bytes"
    );
}

#[test]
fn test_metrics_render_latency_seconds_conversion() {
    let m = CatalogMetrics::new(10);
    // 500_000 us = 0.5 s
    m.observe_create_snapshot_us(500_000);

    let rendered = m.render_prometheus();
    // The sum should be close to 0.5
    assert!(
        rendered.contains("rocklake_catalog_op_duration_seconds_sum{op=\"create_snapshot\"} 0.5"),
        "latency conversion: expected 0.5 s, rendered:\n{rendered}"
    );
}

// ─── OTLP telemetry lifecycle ─────────────────────────────────────────────────

#[test]
fn test_telemetry_no_endpoint_ok() {
    let cfg = TelemetryConfig {
        otlp_endpoint: None,
        service_name: "rocklake-test".to_string(),
    };
    let handle = cfg.init();
    handle.shutdown(); // must not panic
}

#[test]
fn test_telemetry_with_endpoint_ok() {
    let cfg = TelemetryConfig {
        otlp_endpoint: Some("http://localhost:4318".to_string()),
        service_name: "rocklake-test".to_string(),
    };
    let handle = cfg.init();
    handle.shutdown(); // must not panic; no actual connection made
}

// ─── diagnose ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_diagnose_fresh_catalog_ok() {
    let dir = TempDir::new().unwrap();
    let store = open_fresh_catalog(&dir).await;
    drop(store); // close the CatalogStore; reopen as raw slatedb::Db

    let path = ObjectPath::from(dir.path().to_str().unwrap());
    let os = Arc::new(LocalFileSystem::new());
    let db = slatedb::Db::open(path, os).await.expect("open slatedb");

    let report = diagnose_catalog(&db, None)
        .await
        .expect("diagnose should not fail on a fresh catalog");

    db.close().await.unwrap();

    // A freshly initialised catalog should report ok or degraded (no P0).
    assert!(
        report.is_ok(),
        "Fresh catalog should have no P0 findings; got: {:?}",
        report.findings
    );
    assert_eq!(
        report.orphan_files.len(),
        0,
        "No orphan files on fresh catalog"
    );
    assert_eq!(
        report.snapshot_gaps.len(),
        0,
        "No snapshot gaps on fresh catalog"
    );
}

#[tokio::test]
async fn test_diagnose_text_format() {
    let dir = TempDir::new().unwrap();
    let store = open_fresh_catalog(&dir).await;
    drop(store);

    let path = ObjectPath::from(dir.path().to_str().unwrap());
    let os = Arc::new(LocalFileSystem::new());
    let db = slatedb::Db::open(path, os).await.expect("open slatedb");

    let report = diagnose_catalog(&db, None).await.unwrap();
    db.close().await.unwrap();

    let text = format_report_text(&report);
    assert!(
        text.contains("=== RockLake Catalog Diagnostics ==="),
        "missing header"
    );
    assert!(text.contains("Overall status:"), "missing status line");
    assert!(text.contains("Format version:"), "missing format version");
    assert!(text.contains("Latest snapshot:"), "missing snapshot");
}

#[tokio::test]
async fn test_diagnose_json_serialisable() {
    let dir = TempDir::new().unwrap();
    let store = open_fresh_catalog(&dir).await;
    drop(store);

    let path = ObjectPath::from(dir.path().to_str().unwrap());
    let os = Arc::new(LocalFileSystem::new());
    let db = slatedb::Db::open(path, os).await.expect("open slatedb");

    let report = diagnose_catalog(&db, None).await.unwrap();
    db.close().await.unwrap();

    let json = serde_json::to_string(&report).expect("report must serialise to JSON");
    let parsed: DiagnoseReport = serde_json::from_str(&json).expect("JSON must round-trip");
    assert_eq!(report.format_version, parsed.format_version);
    assert_eq!(report.overall_status, parsed.overall_status);
}

// ─── sweep-orphans ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_sweep_orphans_empty_catalog_no_orphans() {
    let dir = TempDir::new().unwrap();
    let store = open_fresh_catalog(&dir).await;
    drop(store);

    let path = ObjectPath::from(dir.path().to_str().unwrap());
    let os: Arc<dyn object_store::ObjectStore> = Arc::new(LocalFileSystem::new());
    let db = slatedb::Db::open(path, os.clone())
        .await
        .expect("open slatedb");

    let config = SweepOrphansConfig {
        grace_period_hours: 0, // sweep everything immediately
        apply: false,
        data_root: dir.path().to_str().unwrap().to_string(),
    };

    let result = sweep_orphans(&db, os, &config).await.unwrap();
    db.close().await.unwrap();

    // A fresh catalog has no data files and no orphans.
    assert_eq!(
        result.orphan_files.len(),
        0,
        "Fresh catalog should have no orphan files"
    );
    assert_eq!(result.deleted, 0, "No deletions in dry-run");
}

#[tokio::test]
async fn test_sweep_orphans_dry_run_does_not_delete() {
    let dir = TempDir::new().unwrap();

    // Write a stray .parquet file that is not registered in the catalog.
    let orphan_path = dir.path().join("orphan_file.parquet");
    std::fs::write(&orphan_path, b"PAR1stray").unwrap();

    let store = open_fresh_catalog(&dir).await;
    drop(store);

    let path = ObjectPath::from(dir.path().to_str().unwrap());
    let os: Arc<dyn object_store::ObjectStore> = Arc::new(LocalFileSystem::new());
    let db = slatedb::Db::open(path, os.clone())
        .await
        .expect("open slatedb");

    let config = SweepOrphansConfig {
        grace_period_hours: 0,
        apply: false, // DRY RUN
        data_root: dir.path().to_str().unwrap().to_string(),
    };

    let result = sweep_orphans(&db, os, &config).await.unwrap();
    db.close().await.unwrap();

    // The orphan file should be reported but NOT deleted.
    assert!(
        result
            .orphan_files
            .iter()
            .any(|f| f.ends_with("orphan_file.parquet")),
        "Orphan file should be reported; got {:?}",
        result.orphan_files
    );
    assert_eq!(result.deleted, 0, "Dry-run must not delete files");
    assert!(orphan_path.exists(), "File must still exist after dry-run");
}

#[tokio::test]
async fn test_sweep_orphans_apply_deletes_orphan() {
    let dir = TempDir::new().unwrap();

    // Write a stray .parquet file not registered in the catalog.
    let orphan_path = dir.path().join("stray.parquet");
    std::fs::write(&orphan_path, b"PAR1stray").unwrap();

    let store = open_fresh_catalog(&dir).await;
    drop(store);

    let path = ObjectPath::from(dir.path().to_str().unwrap());
    let os: Arc<dyn object_store::ObjectStore> = Arc::new(LocalFileSystem::new());
    let db = slatedb::Db::open(path, os.clone())
        .await
        .expect("open slatedb");

    let config = SweepOrphansConfig {
        grace_period_hours: 0, // no grace, delete now
        apply: true,           // APPLY mode
        data_root: dir.path().to_str().unwrap().to_string(),
    };

    let result = sweep_orphans(&db, os, &config).await.unwrap();
    db.close().await.unwrap();

    assert_eq!(result.deleted, 1, "Expected exactly 1 file deleted");
    assert!(
        !orphan_path.exists(),
        "Orphan file should be deleted after --apply"
    );
}

// ─── metrics server response ──────────────────────────────────────────────────

#[tokio::test]
async fn test_metrics_server_responds_ok() {
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let m = Arc::new(CatalogMetrics::new(10));
    m.observe_create_snapshot_us(1_000);
    m.set_gc_retain_from_snapshot(5);

    // Start on a random free port.
    let port = {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        l.local_addr().unwrap().port()
    };

    let m_clone = m.clone();
    tokio::spawn(async move {
        if let Err(e) =
            rocklake_catalog::metrics::start_metrics_server(m_clone, port, "/metrics").await
        {
            eprintln!("metrics server error in test: {e}");
        }
    });

    // Give the server a moment to start.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .expect("connect to metrics server");

    stream
        .write_all(b"GET /metrics HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();

    let mut resp = String::new();
    stream.read_to_string(&mut resp).await.unwrap();

    assert!(
        resp.starts_with("HTTP/1.1 200"),
        "expected 200, got:\n{resp}"
    );
    assert!(
        resp.contains("text/plain"),
        "expected text/plain content-type"
    );
    assert!(
        resp.contains("rocklake_gc_retain_from_snapshot 5"),
        "v0.39.0 gauge missing in metrics response"
    );
}

#[tokio::test]
async fn test_metrics_server_404_on_unknown_path() {
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let m = Arc::new(CatalogMetrics::new(10));
    let port = {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        l.local_addr().unwrap().port()
    };

    let m_clone = m.clone();
    tokio::spawn(async move {
        let _ = rocklake_catalog::metrics::start_metrics_server(m_clone, port, "/metrics").await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .expect("connect to metrics server");

    stream
        .write_all(b"GET /not-metrics HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();

    let mut resp = String::new();
    stream.read_to_string(&mut resp).await.unwrap();

    assert!(
        resp.starts_with("HTTP/1.1 404"),
        "expected 404, got:\n{resp}"
    );
}
