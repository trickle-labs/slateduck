//! Worker configuration and CLI option parsing.

use clap::Parser;

/// Configuration for an IVM worker process.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Unique worker identifier (e.g. hostname + PID).
    pub worker_id: String,
    /// Path (or URL) to the SlateDB store to connect to.
    pub store_path: String,
    /// Shard lease duration in milliseconds.
    pub lease_duration_ms: u64,
    /// Poll interval between input snapshot scans in milliseconds.
    pub poll_interval_ms: u64,
    /// Maximum number of input rows to process per tick.
    pub max_rows_per_tick: usize,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            worker_id: default_worker_id(),
            store_path: String::new(),
            lease_duration_ms: 30_000,
            poll_interval_ms: 500,
            max_rows_per_tick: 10_000,
        }
    }
}

fn default_worker_id() -> String {
    format!("{}-{}", hostname(), std::process::id())
}

fn hostname() -> String {
    std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string())
}

/// Command-line arguments for the `slateduck-ivm` binary.
#[derive(Parser, Debug)]
#[command(
    name = "slateduck-ivm",
    about = "SlateDuck incremental view maintenance worker"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

/// Available subcommands.
#[derive(clap::Subcommand, Debug)]
pub enum Commands {
    /// Start the IVM worker and begin maintaining configured materialized views.
    Serve(ServeArgs),
    /// Print the current IVM status for all matviews in the catalog.
    Status(StatusArgs),
}

/// Arguments for the `serve` subcommand.
#[derive(clap::Args, Debug)]
pub struct ServeArgs {
    /// Path or URL to the SlateDB store.
    #[arg(long, env = "SLATEDUCK_STORE")]
    pub store: String,

    /// Worker ID. Defaults to `hostname-pid`.
    #[arg(long)]
    pub worker_id: Option<String>,

    /// Shard lease duration in milliseconds.
    #[arg(long, default_value = "30000")]
    pub lease_duration_ms: u64,

    /// Input poll interval in milliseconds.
    #[arg(long, default_value = "500")]
    pub poll_interval_ms: u64,

    /// Maximum rows processed per tick.
    #[arg(long, default_value = "10000")]
    pub max_rows_per_tick: usize,
}

/// Arguments for the `status` subcommand.
#[derive(clap::Args, Debug)]
pub struct StatusArgs {
    /// Path or URL to the SlateDB store.
    #[arg(long, env = "SLATEDUCK_STORE")]
    pub store: String,
}
