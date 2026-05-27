//! v0.9 tests: Production Readiness features.
//!
//! Tests for:
//! - Writer endpoint publishing (Option B routing)
//! - Cache warmup
//! - Cost mode configuration
//! - API cost reporting
//! - Cache utilization reporting
//! - Catalog migration (dry-run and apply)
//! - Corpus diff and validate
//! - Tune recommendations
//! - Writer failover SLO for LocalFS

use object_store::path::Path as ObjectPath;
use rocklake_catalog::cache::cache_utilization;
use rocklake_catalog::corpus::{corpus_diff, corpus_validate, parse_corpus, CorpusRecord};
use rocklake_catalog::cost::{ApiCallSnapshot, ApiCostReport, CostMode};
use rocklake_catalog::migrate::{migrate_apply, migrate_dry_run};
use rocklake_catalog::warmup::{
    publish_writer_endpoint, read_writer_endpoint, read_writer_epoch, warmup_cache,
};
use rocklake_catalog::{CatalogStore, OpenOptions};
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;
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

// ─── Writer Endpoint (Option B Routing) ────────────────────────────────────

#[tokio::test]
async fn writer_endpoint_initial_is_none() {
    let dir = TempDir::new().unwrap();
    let catalog = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let ep = read_writer_endpoint(catalog.db()).await.unwrap();
    assert!(ep.is_none(), "fresh catalog should have no writer endpoint");
    catalog.close().await.unwrap();
}

#[tokio::test]
async fn writer_endpoint_publish_and_read() {
    let dir = TempDir::new().unwrap();
    let catalog = CatalogStore::open(test_opts(&dir)).await.unwrap();

    publish_writer_endpoint(catalog.db(), 1, "pod-a.rocklake.svc.cluster.local:5432")
        .await
        .unwrap();

    let ep = read_writer_endpoint(catalog.db()).await.unwrap();
    assert_eq!(
        ep.as_deref(),
        Some("pod-a.rocklake.svc.cluster.local:5432")
    );

    let epoch = read_writer_epoch(catalog.db()).await.unwrap();
    assert_eq!(epoch, 1);

    catalog.close().await.unwrap();
}

#[tokio::test]
async fn writer_endpoint_update_reflects_new_writer() {
    let dir = TempDir::new().unwrap();
    let catalog = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Writer A takes over
    publish_writer_endpoint(catalog.db(), 1, "pod-a:5432")
        .await
        .unwrap();
    assert_eq!(
        read_writer_endpoint(catalog.db()).await.unwrap().as_deref(),
        Some("pod-a:5432")
    );
    assert_eq!(read_writer_epoch(catalog.db()).await.unwrap(), 1);

    // Writer B takes over with higher epoch
    publish_writer_endpoint(catalog.db(), 2, "pod-b:5432")
        .await
        .unwrap();
    assert_eq!(
        read_writer_endpoint(catalog.db()).await.unwrap().as_deref(),
        Some("pod-b:5432")
    );
    assert_eq!(read_writer_epoch(catalog.db()).await.unwrap(), 2);

    catalog.close().await.unwrap();
}

/// After a writer publishes its endpoint, reopening the catalog writes a new
/// epoch (the new writer's fencing token). The endpoint key, however, persists
/// until the NEW writer calls `publish_writer_endpoint` to claim ownership.
/// This tests that the endpoint written by writer A is readable until writer B
/// overwrites it.
#[tokio::test]
async fn writer_endpoint_persists_across_reopen() {
    let dir = TempDir::new().unwrap();
    {
        let catalog = CatalogStore::open(test_opts(&dir)).await.unwrap();
        // Writer A publishes its endpoint with epoch 100
        publish_writer_endpoint(catalog.db(), 100, "pod-c:5432")
            .await
            .unwrap();
        assert_eq!(
            read_writer_endpoint(catalog.db()).await.unwrap().as_deref(),
            Some("pod-c:5432")
        );
        assert_eq!(read_writer_epoch(catalog.db()).await.unwrap(), 100);
        catalog.close().await.unwrap();
    }

    // Reopen: CatalogStore writes a new system-time epoch (overwrites our 100).
    // But the endpoint key is NOT overwritten by open() — only `publish_writer_endpoint` does that.
    let catalog2 = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let ep = read_writer_endpoint(catalog2.db()).await.unwrap();
    // The endpoint that was published by the first writer is gone because CatalogStore::open
    // writes a new epoch; the endpoint is NOT written by open() so it should still be "pod-c:5432".
    assert_eq!(ep.as_deref(), Some("pod-c:5432"));
    // Epoch is now the system-time value set by CatalogStore::open — it's > 100
    let epoch2 = read_writer_epoch(catalog2.db()).await.unwrap();
    assert!(
        epoch2 > 100,
        "new writer's epoch should be higher than previous writer's epoch (got {epoch2})"
    );

    // Writer B publishes its own endpoint with the new epoch
    publish_writer_endpoint(catalog2.db(), epoch2, "pod-d:5432")
        .await
        .unwrap();
    assert_eq!(
        read_writer_endpoint(catalog2.db())
            .await
            .unwrap()
            .as_deref(),
        Some("pod-d:5432")
    );
    assert_eq!(read_writer_epoch(catalog2.db()).await.unwrap(), epoch2);

    catalog2.close().await.unwrap();
}

// ─── Cache Warmup ──────────────────────────────────────────────────────────

#[tokio::test]
async fn warmup_on_fresh_catalog_succeeds() {
    let dir = TempDir::new().unwrap();
    let catalog = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let result = warmup_cache(catalog.db(), 20).await.unwrap();
    assert!(
        result.entries_warmed >= 2,
        "should warm at least the format version and retain-from keys"
    );
    catalog.close().await.unwrap();
}

#[tokio::test]
async fn warmup_with_tables_warms_more_entries() {
    let dir = TempDir::new().unwrap();
    let mut catalog = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Create a schema and table
    let mut writer = catalog.begin_write();
    let schema_id = writer.create_schema("myschema").await.unwrap();
    writer
        .create_table(schema_id, "mytable", None)
        .await
        .unwrap();
    let _ = writer.create_snapshot(None, None).await.unwrap();

    let result_small = warmup_cache(catalog.db(), 1).await.unwrap();
    let result_large = warmup_cache(catalog.db(), 100).await.unwrap();

    // Both should succeed; large warming should touch more entries
    assert!(result_small.entries_warmed > 0);
    assert!(result_large.entries_warmed >= result_small.entries_warmed);

    catalog.close().await.unwrap();
}

// ─── Cost Mode ─────────────────────────────────────────────────────────────

#[test]
fn cost_mode_parse_all_variants() {
    assert_eq!(
        "conservative".parse::<CostMode>(),
        Ok(CostMode::Conservative)
    );
    assert_eq!("balanced".parse::<CostMode>(), Ok(CostMode::Balanced));
    assert_eq!("latency".parse::<CostMode>(), Ok(CostMode::Latency));
    assert!("unknown".parse::<CostMode>().is_err());
    assert!("".parse::<CostMode>().is_err());
}

#[test]
fn cost_mode_default_is_balanced() {
    let default: CostMode = Default::default();
    assert_eq!(default, CostMode::Balanced);
}

#[test]
fn cost_mode_tuning_conservative_has_higher_l0_threshold() {
    let conservative = CostMode::Conservative.tuning();
    let balanced = CostMode::Balanced.tuning();
    let latency = CostMode::Latency.tuning();

    // Conservative: more batching = fewer flushes
    assert!(conservative.l0_sst_count_threshold > balanced.l0_sst_count_threshold);
    // Latency: more aggressive compaction
    assert!(latency.l0_sst_count_threshold < balanced.l0_sst_count_threshold);
    assert!(latency.compaction_aggressiveness > balanced.compaction_aggressiveness);
}

// ─── API Cost Report ───────────────────────────────────────────────────────

#[test]
fn api_cost_report_zero_calls() {
    let snap = ApiCallSnapshot {
        put_count: 0,
        get_count: 0,
        list_count: 0,
        delete_count: 0,
        elapsed: Duration::from_secs(60),
    };
    let report = ApiCostReport::from_snapshot(&snap);
    assert_eq!(report.estimated_monthly_usd, 0.0);
    assert!(report.rds_monthly_usd > 40.0);
    assert!(report.recommendations.is_empty());
}

#[test]
fn api_cost_report_high_put_rate_suggests_conservative() {
    let snap = ApiCallSnapshot {
        put_count: 10_000,
        get_count: 50_000,
        list_count: 1_000,
        delete_count: 0,
        elapsed: Duration::from_secs(60), // 10K PUTs/min
    };
    let report = ApiCostReport::from_snapshot(&snap);
    assert!(report.put_per_minute > 100.0);
    assert!(!report.recommendations.is_empty());
    assert!(report.recommendations[0].contains("conservative"));
}

#[test]
fn api_cost_report_cost_crossover_with_rds() {
    // At very high ingest rate, S3 API costs exceed RDS
    let snap = ApiCallSnapshot {
        put_count: 100_000,
        get_count: 500_000,
        list_count: 10_000,
        delete_count: 0,
        elapsed: Duration::from_secs(60),
    };
    let report = ApiCostReport::from_snapshot(&snap);
    // At this insane rate, monthly cost should far exceed RDS
    assert!(report.estimated_monthly_usd > report.rds_monthly_usd);
}

// ─── Cache Utilization ─────────────────────────────────────────────────────

#[tokio::test]
async fn cache_utilization_small_catalog_fits_in_256mb() {
    // A small catalog (100 files, 50 columns) easily fits in 256 MiB
    let stats = cache_utilization(256, 100, 50).await;
    assert!(
        stats.hit_ratio > 0.8,
        "small catalog should have high cache hit ratio"
    );
    assert_eq!(stats.recommended_cache_size_mb, 256);
}

#[tokio::test]
async fn cache_utilization_large_catalog_recommends_larger_cache() {
    // 1M data files + 500K columns won't fit in 256 MiB
    let stats = cache_utilization(256, 1_000_000, 500_000).await;
    assert!(
        stats.hit_ratio < 0.8,
        "large catalog should have lower cache hit ratio"
    );
    assert!(
        stats.recommended_cache_size_mb > 256,
        "should recommend larger cache"
    );
}

// ─── Catalog Migration ─────────────────────────────────────────────────────

#[tokio::test]
async fn migrate_dry_run_same_version_is_no_op() {
    let dir = TempDir::new().unwrap();
    let catalog = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let result = migrate_dry_run(catalog.db(), 1).await.unwrap();
    assert_eq!(result.rows_to_migrate, 0);
    assert!(result.description.contains("No migration needed"));
    catalog.close().await.unwrap();
}

#[tokio::test]
async fn migrate_dry_run_to_v2_reports_rows() {
    let dir = TempDir::new().unwrap();
    let catalog = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let result = migrate_dry_run(catalog.db(), 2).await.unwrap();
    assert_eq!(result.current_version, 1);
    assert_eq!(result.target_version, 2);
    assert!(result.estimated_seconds >= 1);
    assert!(result.description.contains("migrate"));
    catalog.close().await.unwrap();
}

#[tokio::test]
async fn migrate_apply_creates_backup_and_updates_version() {
    let dir = TempDir::new().unwrap();
    let catalog = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Apply migration to v2
    let backup_dir = dir.path().to_str().unwrap().to_string();
    let result = migrate_apply(catalog.db(), 2, &backup_dir).await.unwrap();
    assert_eq!(result.new_version, 2);
    assert!(
        std::path::Path::new(&result.backup_path).exists(),
        "backup file should be created"
    );

    // Verify format version was updated
    let state = rocklake_catalog::inspect::inspect_snapshot(catalog.db())
        .await
        .unwrap();
    assert_eq!(state.format_version, 2);

    catalog.close().await.unwrap();
}

// ─── Corpus Diff and Validate ──────────────────────────────────────────────

fn make_record(family: &str, is_handshake: bool) -> CorpusRecord {
    CorpusRecord {
        statement_family: family.to_string(),
        sql: String::new(),
        table: String::new(),
        is_handshake,
        type_oids: vec![],
        protocol: "simple".to_string(),
        category: String::new(),
    }
}

#[test]
fn corpus_diff_empty_files_no_changes() {
    let diffs = corpus_diff(&[], &[]);
    assert!(diffs.is_empty());
}

#[test]
fn corpus_diff_detects_added_statement_family() {
    let old = vec![make_record("SELECT", false)];
    let new = vec![make_record("SELECT", false), make_record("INSERT", false)];
    let diffs = corpus_diff(&old, &new);
    let added: Vec<_> = diffs
        .iter()
        .filter(|d| d.statement_family == "INSERT")
        .collect();
    assert_eq!(added.len(), 1);
    assert_eq!(
        added[0].change_type,
        rocklake_catalog::corpus::DiffChangeType::Added
    );
}

#[test]
fn corpus_diff_detects_removed_statement_family() {
    let old = vec![make_record("SELECT", false), make_record("UPDATE", false)];
    let new = vec![make_record("SELECT", false)];
    let diffs = corpus_diff(&old, &new);
    let removed: Vec<_> = diffs
        .iter()
        .filter(|d| d.statement_family == "UPDATE")
        .collect();
    assert_eq!(removed.len(), 1);
    assert_eq!(
        removed[0].change_type,
        rocklake_catalog::corpus::DiffChangeType::Removed
    );
}

#[test]
fn corpus_validate_classifies_correctly() {
    let records = vec![
        make_record("SELECT", false),
        make_record("INSERT", false),
        make_record("BEGIN", false),
        make_record("SOME_EXOTIC_PROCEDURE", false),
        make_record("", true), // handshake
    ];
    let result = corpus_validate(&records);
    assert_eq!(result.handshake_count, 1);
    assert!(result.category_a.contains(&"SELECT".to_string()));
    assert!(result.category_a.contains(&"INSERT".to_string()));
    assert!(result
        .category_c
        .contains(&"SOME_EXOTIC_PROCEDURE".to_string()));
}

#[test]
fn corpus_parse_handles_ndjson() {
    let data = r#"{"statement_family":"SELECT","sql":"SELECT 1","is_handshake":false}
{"statement_family":"INSERT","sql":"INSERT INTO t VALUES (1)","is_handshake":false}
not valid json
{"statement_family":"BEGIN","is_handshake":false}
"#;
    let records = parse_corpus(Cursor::new(data));
    assert_eq!(records.len(), 3, "bad lines should be skipped");
    assert_eq!(records[0].statement_family, "SELECT");
    assert_eq!(records[1].statement_family, "INSERT");
    assert_eq!(records[2].statement_family, "BEGIN");
}

// ─── Writer Failover SLO (LocalFS) ─────────────────────────────────────────

/// Verify that after a writer is replaced, the new writer can commit within
/// the LocalFS SLO (< 5 seconds from first write to new snapshot visible).
///
/// This simulates the scenario from the roadmap:
/// (1) first writer writes 3 snapshots
/// (2) first writer is "killed" (dropped without graceful shutdown)
/// (3) second writer starts immediately
/// (4) second writer commits a new snapshot
/// (5) all pre-kill snapshots are visible to a reader
#[tokio::test]
async fn writer_failover_localfs_slo() {
    let dir = TempDir::new().unwrap();
    let start = std::time::Instant::now();

    // Step 1: First writer writes 3 snapshots
    {
        let mut catalog = CatalogStore::open(test_opts(&dir)).await.unwrap();
        let mut writer = catalog.begin_write();
        let schema_id = writer.create_schema("failover_schema").await.unwrap();
        writer
            .create_table(schema_id, "failover_table", None)
            .await
            .unwrap();
        let _ = writer.create_snapshot(None, None).await.unwrap();

        for i in 0..2 {
            let mut w2 = catalog.begin_write();
            let _ = w2
                .create_snapshot(Some(&format!("snapshot-{i}")), None)
                .await
                .unwrap();
        }
        // "Kill" the writer: drop without calling close()
        // (CatalogStore drop will flush WAL)
    }

    // Step 2: Second writer starts immediately
    let mut catalog2 = CatalogStore::open(test_opts(&dir)).await.unwrap();

    // Step 3: Second writer commits a new snapshot
    let mut writer2 = catalog2.begin_write();
    let _ = writer2
        .create_snapshot(Some("post-failover"), None)
        .await
        .unwrap();

    let elapsed = start.elapsed();

    // Step 4: Verify all pre-kill snapshots are visible
    let reader = catalog2.read_latest();
    let schemas = reader.list_schemas().await.unwrap();
    assert!(
        !schemas.is_empty(),
        "schema created by first writer must be visible after failover"
    );

    // Step 5: Verify SLO: entire failover cycle < 5 seconds on LocalFS
    assert!(
        elapsed.as_secs() < 5,
        "LocalFS failover SLO exceeded: took {}ms (SLO: 5000ms)",
        elapsed.as_millis()
    );

    catalog2.close().await.unwrap();
}
