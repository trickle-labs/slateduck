//! RockLake Catalog: DuckLake catalog operations backed by SlateDB.

#![deny(missing_docs)]

pub mod audit;
pub mod cache;
pub mod cdc;
pub mod checkpoint;
pub mod cleanup;
pub mod corpus;
pub mod cost;
pub mod diagnose;
pub mod encryption;
pub mod error;
pub mod excise;
pub mod export;
pub mod extension;
pub mod gc;
pub mod init;
pub mod inspect;
pub mod key_migration;
pub mod lease;
pub mod manifest;
pub mod metrics;
pub mod migrate;
pub mod partition;
pub mod performance;
pub mod reader;
pub mod repair;
pub mod store;
pub mod streaming;
pub mod sweep;
pub mod verify;
pub mod wal;
pub mod warmup;
pub mod writer;

pub use audit::{AuditChange, AuditEntry};
pub use cache::{cache_utilization, CacheStats};
pub use cdc::{CdcChangeKind, CdcEvent, CdcSnapshot, CdcTailer, WebhookPayload};
pub use corpus::{corpus_diff, corpus_validate, parse_corpus, CorpusRecord, ValidateResult};
pub use cost::{tune_for_cost_target, ApiCostReport, CostMode};
pub use diagnose::{
    diagnose_catalog, format_report_text, DiagnoseReport, DiagnosticFinding, FindingSeverity,
};
pub use encryption::{EncryptionConfig, EncryptionError};
pub use error::{CatalogError, CatalogResult};
pub use extension::{
    create_extension_table, delete_extension_rows, insert_extension_row, is_registered_extension,
    resolve_extension_id, select_extension_rows, EXTENSION_PGTRICKLE,
};
pub use lease::{hold_snapshot, list_active_leases, minimum_leased_snapshot, release_snapshot};
pub use metrics::CatalogMetrics;
pub use migrate::{migrate_apply, migrate_dry_run, MigrateDryRunResult, MigrateResult};
pub use partition::{CatalogRegistry, DatasetEntry, PartitionedWriter};
pub use performance::{BenchmarkReport, HotKeyState, SlateDbTuning};
pub use reader::{CatalogReader, SnapshotDiff};
pub use store::{CatalogStore, OpenOptions};
pub use streaming::{measure_ingest_throughput, IngestRecord, IngestResult, RockLakeSink};
pub use sweep::{sweep_orphans, SweepOrphansConfig, SweepResult};
pub use warmup::{publish_writer_endpoint, read_writer_endpoint, warmup_cache, WarmupResult};
pub use writer::{
    next_rowid_range,
    snapshot::CommitResult,
    stats::{FileColumnStatsInput, FileVariantStatsInput},
    validate_app_metadata_key, CatalogWriter,
};
