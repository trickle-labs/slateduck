//! `slateduck serve` — PG-Wire sidecar binary.
//!
//! Usage:
//!   slateduck serve --catalog <path> --bind <addr>

use std::net::SocketAddr;
use std::sync::Arc;

use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

use slateduck_catalog::{CatalogStore, OpenOptions};
use slateduck_pgwire::server::{run_server, ServerConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .init();

    let args: Vec<String> = std::env::args().collect();
    let config = parse_args(&args)?;

    let (catalog_path, object_store) = resolve_catalog(&config.catalog_url)?;

    let opts = OpenOptions {
        object_store: object_store.clone(),
        path: catalog_path,
    };

    let store = CatalogStore::open(opts)
        .await
        .map_err(|e| format!("Failed to open catalog: {e}"))?;

    tracing::info!("Catalog opened successfully");

    let catalog = Arc::new(Mutex::new(store));

    let server_config = ServerConfig {
        bind_addr: config.bind_addr,
        max_sessions: config.max_sessions,
        max_active_scans: 25,
    };

    run_server(server_config, catalog).await?;
    Ok(())
}

struct CliConfig {
    catalog_url: String,
    bind_addr: SocketAddr,
    max_sessions: usize,
}

fn parse_args(args: &[String]) -> Result<CliConfig, String> {
    let mut catalog_url = String::new();
    let mut bind_addr: SocketAddr = "0.0.0.0:5432".parse().unwrap();
    let mut max_sessions = 50;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "serve" => {} // subcommand, skip
            "--catalog" | "-c" => {
                i += 1;
                catalog_url = args.get(i).cloned().ok_or("--catalog requires a value")?;
            }
            "--bind" | "-b" => {
                i += 1;
                let addr_str = args.get(i).ok_or("--bind requires a value")?;
                bind_addr = addr_str
                    .parse()
                    .map_err(|e| format!("invalid bind address: {e}"))?;
            }
            "--max-sessions" => {
                i += 1;
                max_sessions = args
                    .get(i)
                    .ok_or("--max-sessions requires a value")?
                    .parse()
                    .map_err(|e| format!("invalid max-sessions: {e}"))?;
            }
            "--help" | "-h" => {
                eprintln!(
                    "Usage: slateduck serve --catalog <path> [--bind <addr>] [--max-sessions <n>]"
                );
                std::process::exit(0);
            }
            other => {
                if catalog_url.is_empty() && !other.starts_with('-') {
                    catalog_url = other.to_string();
                }
            }
        }
        i += 1;
    }

    if catalog_url.is_empty() {
        return Err("--catalog is required. Usage: slateduck serve --catalog <path>".to_string());
    }

    Ok(CliConfig {
        catalog_url,
        bind_addr,
        max_sessions,
    })
}

fn resolve_catalog(url: &str) -> Result<(ObjectPath, Arc<dyn object_store::ObjectStore>), String> {
    // For now, support local filesystem paths
    // S3 support will be added in v0.4
    if url.starts_with("s3://") {
        return Err(
            "S3 catalog support requires AWS credentials configuration (coming in v0.4)"
                .to_string(),
        );
    }

    let path = std::path::Path::new(url);
    let canonical = if path.exists() {
        path.canonicalize()
            .map_err(|e| format!("cannot resolve path: {e}"))?
    } else {
        std::fs::create_dir_all(path).map_err(|e| format!("cannot create catalog dir: {e}"))?;
        path.canonicalize()
            .map_err(|e| format!("cannot resolve path: {e}"))?
    };

    let store = Arc::new(
        LocalFileSystem::new_with_prefix(&canonical)
            .map_err(|e| format!("cannot create local object store: {e}"))?,
    );
    let obj_path = ObjectPath::from("");

    Ok((obj_path, store))
}
