//! Clap-derived CLI structures for `rocklake`.
//!
//! This module defines every subcommand and flag using `clap` derive macros,
//! providing:
//! - Typed, validated argument parsing
//! - `--help` output for the top-level CLI and every subcommand (exit 0)
//! - Shell completion generation via `clap_complete`
//!
//! The module is deliberately kept separate from the command implementations
//! so that the structs can be unit-tested independently.

use clap::{ArgAction, Parser, Subcommand};

/// RockLake — serverless lakehouse catalog backed by SlateDB.
///
/// Run `rocklake <COMMAND> --help` for command-specific options.
#[derive(Debug, Parser)]
#[command(
    name = "rocklake",
    version,
    author,
    about = "RockLake: serverless lakehouse catalog backed by SlateDB",
    long_about = None,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Start the PG-Wire sidecar server.
    Serve(Box<ServeArgs>),

    /// Visibility GC — advance the retain-from watermark.
    #[command(subcommand)]
    Gc(GcSubcommand),

    /// Physical excision of catalog facts before a snapshot.
    #[command(subcommand)]
    Excise(ExciseSubcommand),

    /// Manage catalog checkpoints.
    #[command(subcommand)]
    Checkpoint(CheckpointSubcommand),

    /// Export catalog to NDJSON.
    Export(ExportArgs),

    /// Import catalog from NDJSON.
    Import(ImportArgs),

    /// Convert NDJSON export to PostgreSQL INSERT statements.
    #[command(name = "pg-migrate")]
    PgMigrate(PgMigrateArgs),

    /// Rebuild catalog by scanning Parquet files in object storage.
    Rebuild(RebuildArgs),

    /// Inspect catalog state (snapshot, API costs, cache utilisation).
    #[command(subcommand)]
    Inspect(InspectSubcommand),

    /// Verify catalog integrity.
    #[command(subcommand)]
    Verify(VerifySubcommand),

    /// Repair catalog issues.
    Repair(RepairArgs),

    /// Warm up the block cache before serving.
    Warmup(WarmupArgs),

    /// Migrate catalog to the current format version.
    Migrate(MigrateArgs),

    /// Wire-corpus operations (diff and validate).
    #[command(subcommand)]
    Corpus(CorpusSubcommand),

    /// Output recommended settings for a target cost.
    Tune(TuneArgs),

    /// Migrate from an existing DuckLake catalog into RockLake.
    #[command(name = "migrate-from-ducklake")]
    MigrateFromDucklake(MigrateFromDucklakeArgs),

    /// Export all 28+ DuckLake catalog tables to NDJSON.
    #[command(name = "export-catalog")]
    ExportCatalog(ExportCatalogArgs),

    /// Structured catalog health diagnostic report.
    Diagnose(DiagnoseArgs),

    /// Identify (and optionally delete) orphan Parquet files.
    #[command(name = "sweep-orphans")]
    SweepOrphans(SweepOrphansArgs),

    /// Generate shell completion scripts.
    Completions(CompletionsArgs),
}

// ─── serve ─────────────────────────────────────────────────────────────────

/// Options for `rocklake serve`.
#[derive(Debug, Parser)]
pub struct ServeArgs {
    /// Catalog URL (`file:///…`, `s3://…`, `gs://…`, `az://…`).
    #[arg(short = 'c', long, env = "ROCKLAKE_CATALOG")]
    pub catalog: String,

    /// Bind address for the PG-Wire listener.
    #[arg(short = 'b', long, default_value = "0.0.0.0:5432")]
    pub bind: String,

    /// Maximum concurrent sessions.
    #[arg(long, default_value = "50")]
    pub max_sessions: usize,

    /// Port for the Prometheus `/metrics` HTTP endpoint.
    #[arg(long)]
    pub metrics_port: Option<u16>,

    /// HTTP path for the metrics endpoint.
    #[arg(long, env = "ROCKLAKE_METRICS_PATH", default_value = "/metrics")]
    pub metrics_path: String,

    /// Path to TLS certificate file.
    #[arg(long)]
    pub tls_cert: Option<String>,

    /// Path to TLS private key file.
    #[arg(long)]
    pub tls_key: Option<String>,

    /// Require TLS for all connections.
    #[arg(long, action = ArgAction::SetTrue)]
    pub tls_required: bool,

    /// Username for PG-Wire authentication.
    #[arg(long, env = "ROCKLAKE_AUTH_USER")]
    pub auth_user: Option<String>,

    /// Password for PG-Wire authentication.
    #[arg(long, env = "ROCKLAKE_AUTH_PASSWORD")]
    pub auth_password: Option<String>,

    /// Serving mode: `writer` (accepts writes) or `reader` (read-only).
    #[arg(long, default_value = "writer", value_parser = ["writer", "reader"])]
    pub mode: String,

    /// Enable read-only mode (alias for `--mode reader`).
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "mode")]
    pub read_only: bool,

    /// Cost/latency preset.
    #[arg(long, default_value = "balanced", value_parser = ["conservative", "balanced", "latency"])]
    pub cost_mode: String,

    /// S3-compatible endpoint URL (e.g. for MinIO).
    #[arg(long)]
    pub s3_endpoint: Option<String>,

    /// Use S3 path-style addressing.
    #[arg(long, action = ArgAction::SetTrue)]
    pub s3_path_style: bool,

    /// AES-256 encryption key (64 hex digits).
    #[arg(long)]
    pub encryption_key: Option<String>,

    /// Also start a DataFusion PG-Wire listener on this port.
    #[arg(long)]
    pub datafusion_pg_wire: Option<u16>,

    /// Comma-separated allowed extension schema names.
    #[arg(long, env = "ROCKLAKE_EXTENSION_SCHEMAS", value_delimiter = ',')]
    pub extension_schemas: Vec<String>,

    /// OTLP HTTP endpoint for OpenTelemetry tracing.
    #[arg(long, env = "ROCKLAKE_OTLP_ENDPOINT")]
    pub otlp_endpoint: Option<String>,

    /// Close idle connections after this many seconds (default: 60).
    #[arg(long, default_value = "60")]
    pub idle_connection_timeout: u64,

    /// Maximum seconds to wait for in-flight queries during SIGTERM drain (default: 30).
    #[arg(long, default_value = "30")]
    pub drain_timeout: u64,

    /// Capacity of the DataFusion AsyncBridge channel (default: 256).
    #[arg(long, default_value = "256")]
    pub datafusion_bridge_queue_depth: usize,
}

// ─── gc ────────────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum GcSubcommand {
    /// Show the GC plan without applying it.
    Plan(GcArgs),
    /// Apply the GC plan (advance retain-from).
    Apply(GcArgs),
}

#[derive(Debug, Parser)]
pub struct GcArgs {
    /// Catalog URL.
    #[arg(short = 'c', long, env = "ROCKLAKE_CATALOG")]
    pub catalog: String,

    /// Retention period in days (snapshots older than this are eligible for GC).
    #[arg(long, default_value = "30")]
    pub retention_days: u64,
}

// ─── excise ────────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum ExciseSubcommand {
    /// Show the excision plan without deleting anything.
    Plan(ExciseArgs),
    /// Apply the excision plan (physically delete old catalog facts).
    Apply(ExciseArgs),
}

#[derive(Debug, Parser)]
pub struct ExciseArgs {
    /// Catalog URL.
    #[arg(short = 'c', long, env = "ROCKLAKE_CATALOG")]
    pub catalog: String,

    /// Delete facts for all snapshots strictly before this ID.
    #[arg(long)]
    pub before: u64,
}

// ─── checkpoint ────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum CheckpointSubcommand {
    /// Create a new catalog checkpoint.
    Create(CheckpointCreateArgs),
    /// List existing checkpoints.
    List(CheckpointListArgs),
    /// Restore catalog to a saved checkpoint.
    Restore(CheckpointRestoreArgs),
}

#[derive(Debug, Parser)]
pub struct CheckpointCreateArgs {
    /// Catalog URL.
    #[arg(short = 'c', long, env = "ROCKLAKE_CATALOG")]
    pub catalog: String,

    /// Human-readable label for the checkpoint.
    #[arg(long)]
    pub label: Option<String>,
}

#[derive(Debug, Parser)]
pub struct CheckpointListArgs {
    /// Catalog URL.
    #[arg(short = 'c', long, env = "ROCKLAKE_CATALOG")]
    pub catalog: String,
}

#[derive(Debug, Parser)]
pub struct CheckpointRestoreArgs {
    /// Catalog URL.
    #[arg(short = 'c', long, env = "ROCKLAKE_CATALOG")]
    pub catalog: String,

    /// ID of the checkpoint to restore.
    #[arg(long)]
    pub id: u64,
}

// ─── export ────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
pub struct ExportArgs {
    /// Catalog URL.
    #[arg(env = "ROCKLAKE_CATALOG")]
    pub catalog: String,

    /// Output file path.
    #[arg(long, default_value = "catalog.ndjson")]
    pub output: String,

    /// Export only this snapshot ID (default: latest).
    #[arg(long)]
    pub snapshot_id: Option<u64>,
}

// ─── import ────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
pub struct ImportArgs {
    /// Catalog URL.
    #[arg(env = "ROCKLAKE_CATALOG")]
    pub catalog: String,

    /// Input NDJSON file path.
    #[arg(long)]
    pub input: String,
}

// ─── pg-migrate ────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
pub struct PgMigrateArgs {
    /// Input NDJSON export file.
    #[arg(long)]
    pub input: String,
}

// ─── rebuild ───────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
pub struct RebuildArgs {
    /// Catalog URL.
    #[arg(short = 'c', long, env = "ROCKLAKE_CATALOG")]
    pub catalog: String,

    /// Object-store root containing the Parquet data files.
    #[arg(long)]
    pub data_root: Option<String>,

    /// S3-compatible endpoint URL.
    #[arg(long)]
    pub s3_endpoint: Option<String>,

    /// Use S3 path-style addressing.
    #[arg(long, action = ArgAction::SetTrue)]
    pub s3_path_style: bool,
}

// ─── inspect ───────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum InspectSubcommand {
    /// Show the current snapshot metadata.
    Snapshot(InspectArgs),
    /// Show per-operation API cost estimates.
    #[command(name = "api-costs")]
    ApiCosts(InspectArgs),
    /// Show block-cache utilisation statistics.
    #[command(name = "cache-utilization")]
    CacheUtilization(InspectArgs),
}

#[derive(Debug, Parser)]
pub struct InspectArgs {
    /// Catalog URL.
    #[arg(short = 'c', long, env = "ROCKLAKE_CATALOG")]
    pub catalog: String,
}

// ─── verify ────────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum VerifySubcommand {
    /// Verify catalog key-value integrity.
    Catalog(VerifyArgs),
    /// Verify that all registered data files are accessible.
    #[command(name = "data-files")]
    DataFiles(VerifyArgs),
}

#[derive(Debug, Parser)]
pub struct VerifyArgs {
    /// Catalog URL.
    #[arg(short = 'c', long, env = "ROCKLAKE_CATALOG")]
    pub catalog: String,
}

// ─── repair ────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
pub struct RepairArgs {
    /// Catalog URL.
    #[arg(short = 'c', long, env = "ROCKLAKE_CATALOG")]
    pub catalog: String,

    /// Preview repairs without applying them.
    #[arg(long, action = ArgAction::SetTrue)]
    pub dry_run: bool,

    /// Apply repairs.
    #[arg(long, action = ArgAction::SetTrue)]
    pub apply: bool,
}

// ─── warmup ────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
pub struct WarmupArgs {
    /// Catalog URL.
    #[arg(short = 'c', long, env = "ROCKLAKE_CATALOG")]
    pub catalog: String,

    /// Number of tables to warm up (default: all).
    #[arg(long)]
    pub tables: Option<u64>,
}

// ─── migrate ───────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
pub struct MigrateArgs {
    /// Catalog URL.
    #[arg(short = 'c', long, env = "ROCKLAKE_CATALOG")]
    pub catalog: String,

    /// Preview migration without writing.
    #[arg(long, action = ArgAction::SetTrue)]
    pub dry_run: bool,

    /// Apply the migration.
    #[arg(long, action = ArgAction::SetTrue)]
    pub apply: bool,
}

// ─── corpus ────────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum CorpusSubcommand {
    /// Diff two wire-corpus files.
    Diff(CorpusDiffArgs),
    /// Validate a wire-corpus against the server.
    Validate(CorpusValidateArgs),
}

#[derive(Debug, Parser)]
pub struct CorpusDiffArgs {
    /// First corpus file.
    pub left: String,
    /// Second corpus file.
    pub right: String,
}

#[derive(Debug, Parser)]
pub struct CorpusValidateArgs {
    /// Corpus file to validate.
    pub corpus: String,
    /// Catalog URL to validate against.
    #[arg(short = 'c', long, env = "ROCKLAKE_CATALOG")]
    pub catalog: String,
}

// ─── tune ──────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
pub struct TuneArgs {
    /// Catalog URL.
    #[arg(short = 'c', long, env = "ROCKLAKE_CATALOG")]
    pub catalog: String,

    /// Target monthly cost in USD.
    #[arg(long)]
    pub target_cost_usd: Option<f64>,
}

// ─── migrate-from-ducklake ─────────────────────────────────────────────────

#[derive(Debug, Parser)]
pub struct MigrateFromDucklakeArgs {
    /// Source: `sqlite:/path/to/catalog.db` or path to an NDJSON dump.
    #[arg(long)]
    pub source: String,

    /// Destination RockLake catalog URL.
    #[arg(short = 'c', long, env = "ROCKLAKE_CATALOG")]
    pub catalog: String,

    /// Preview migration without writing.
    #[arg(long, action = ArgAction::SetTrue)]
    pub dry_run: bool,

    /// Accept DuckLake catalog versions beyond the default (v1.0).
    /// May be specified multiple times.
    #[arg(long = "accept-version", action = ArgAction::Append)]
    pub accept_versions: Vec<String>,
}

// ─── export-catalog ────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
pub struct ExportCatalogArgs {
    /// Catalog URL.
    #[arg(short = 'c', long, env = "ROCKLAKE_CATALOG")]
    pub catalog: String,

    /// Output NDJSON file path.
    #[arg(long, default_value = "catalog-export.ndjson")]
    pub out: String,

    /// Export only this snapshot ID (default: latest).
    #[arg(long)]
    pub at_snapshot: Option<u64>,
}

// ─── diagnose ──────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
pub struct DiagnoseArgs {
    /// Catalog URL.
    #[arg(short = 'c', long, env = "ROCKLAKE_CATALOG")]
    pub catalog: String,

    /// Emit JSON output instead of human-readable text.
    #[arg(long, action = ArgAction::SetTrue)]
    pub json: bool,

    /// Object-store root containing the data files (enables data-file checks).
    #[arg(long)]
    pub data_root: Option<String>,
}

// ─── sweep-orphans ─────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
pub struct SweepOrphansArgs {
    /// Catalog URL.
    #[arg(short = 'c', long, env = "ROCKLAKE_CATALOG")]
    pub catalog: String,

    /// Object-store prefix for data files (e.g. `s3://bucket/data/`).
    #[arg(long)]
    pub data_root: String,

    /// Grace period: files newer than this many hours are never deleted.
    #[arg(long, default_value = "24")]
    pub grace_period_hours: u64,

    /// Delete orphan files (default: dry-run only).
    #[arg(long, action = ArgAction::SetTrue)]
    pub apply: bool,
}

// ─── completions ───────────────────────────────────────────────────────────

/// Generate shell completion scripts for `rocklake`.
///
/// Output the script to stdout and source it in your shell profile:
///
/// ```sh
/// # bash
/// rocklake completions bash >> ~/.bash_completion
/// # zsh
/// rocklake completions zsh > ~/.zfunc/_rocklake
/// # fish
/// rocklake completions fish > ~/.config/fish/completions/rocklake.fish
/// ```
#[derive(Debug, Parser)]
pub struct CompletionsArgs {
    /// Target shell.
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}
