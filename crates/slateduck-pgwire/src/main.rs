//! `slateduck serve` — PG-Wire sidecar binary.
//!
//! Usage:
//!   slateduck serve --catalog <path> --bind <addr:port> [--retention-days N]

use std::sync::Arc;

use object_store::local::LocalFileSystem;
use slateduck_catalog::{CatalogStore, OpenOptions};
use slateduck_pgwire::SlateDuckHandler;
use tokio::net::TcpListener;
use tracing::{error, info};

#[derive(Debug)]
struct CliArgs {
    catalog_path: String,
    bind_addr: String,
    retention_days: u32,
}

fn parse_args() -> CliArgs {
    let args: Vec<String> = std::env::args().collect();
    let mut catalog_path = String::new();
    let mut bind_addr = "0.0.0.0:5432".to_string();
    let mut retention_days = 7u32;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "serve" => {} // subcommand, skip
            "--catalog" | "-c" => {
                i += 1;
                if i < args.len() {
                    catalog_path = args[i].clone();
                }
            }
            "--bind" | "-b" => {
                i += 1;
                if i < args.len() {
                    bind_addr = args[i].clone();
                }
            }
            "--retention-days" => {
                i += 1;
                if i < args.len() {
                    retention_days = args[i].parse().unwrap_or(7);
                }
            }
            "--help" | "-h" => {
                eprintln!(
                    "Usage: slateduck serve --catalog <path> --bind <addr:port> [--retention-days N]"
                );
                std::process::exit(0);
            }
            other => {
                if catalog_path.is_empty() {
                    catalog_path = other.to_string();
                }
            }
        }
        i += 1;
    }

    if catalog_path.is_empty() {
        catalog_path = "/tmp/slateduck-catalog".to_string();
    }

    CliArgs {
        catalog_path,
        bind_addr,
        retention_days,
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let args = parse_args();
    info!(
        "SlateDuck v0.3.0 starting: catalog={}, bind={}",
        args.catalog_path, args.bind_addr
    );

    let object_store = Arc::new(LocalFileSystem::new());
    let opts = OpenOptions {
        path: args.catalog_path.clone(),
        object_store,
        retention_days: args.retention_days,
    };

    let store = CatalogStore::open(opts).await?;
    let store = Arc::new(store);

    let listener = TcpListener::bind(&args.bind_addr).await?;
    info!("Listening on {}", args.bind_addr);

    loop {
        let (socket, addr) = listener.accept().await?;
        info!("New connection from {addr}");
        let handler = Arc::new(SlateDuckHandler::new(store.clone()));

        tokio::spawn(async move {
            if let Err(e) = pgwire::tokio::process_socket(socket, None, handler).await {
                error!("Connection error from {addr}: {e}");
            }
        });
    }
}
