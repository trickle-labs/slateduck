//! `slateduck-ivm` — SlateDuck incremental view maintenance worker binary.

use clap::Parser;
use slateduck_ivm::config::{Cli, Commands, WorkerConfig};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    match cli.command {
        Commands::Serve(args) => {
            let config = WorkerConfig {
                worker_id: args
                    .worker_id
                    .unwrap_or_else(|| format!("{}-{}", hostname(), std::process::id())),
                store_path: args.store.clone(),
                lease_duration_ms: args.lease_duration_ms,
                poll_interval_ms: args.poll_interval_ms,
                max_rows_per_tick: args.max_rows_per_tick,
            };

            eprintln!(
                "slateduck-ivm worker {} starting (store={})",
                config.worker_id, config.store_path
            );
            eprintln!(
                "Store path connection not yet implemented in v0.11 — worker config loaded OK."
            );
            // Full event loop integration is the v0.11 acceptance criterion;
            // the CatalogStore::open() call requires an ObjectStore handle which
            // is configured via environment variables at runtime.
            let _ = config;
        }
        Commands::Status(args) => {
            eprintln!("Status for store: {}", args.store);
            eprintln!("Status subcommand requires a live catalog connection.");
        }
    }
}

fn hostname() -> String {
    std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string())
}
