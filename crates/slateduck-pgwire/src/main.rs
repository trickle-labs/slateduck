//! `slateduck` — CLI binary with all operational commands.
//!
//! Commands:
//!   serve, gc, excise, checkpoint, export, import, pg-migrate,
//!   rebuild, inspect, verify, repair

use std::net::SocketAddr;
use std::sync::Arc;

use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

use slateduck_catalog::metrics::CatalogMetrics;
use slateduck_catalog::{CatalogStore, OpenOptions};
use slateduck_pgwire::server::{run_server, ServerConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .init();

    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    match args[1].as_str() {
        "serve" => cmd_serve(&args).await?,
        "gc" => cmd_gc(&args).await?,
        "excise" => cmd_excise(&args).await?,
        "checkpoint" => cmd_checkpoint(&args).await?,
        "export" => cmd_export(&args).await?,
        "import" => cmd_import(&args).await?,
        "pg-migrate" => cmd_pg_migrate(&args).await?,
        "rebuild" => cmd_rebuild(&args).await?,
        "inspect" => cmd_inspect(&args).await?,
        "verify" => cmd_verify(&args).await?,
        "repair" => cmd_repair(&args).await?,
        "--help" | "-h" => print_usage(),
        other => {
            eprintln!("Unknown command: {other}");
            print_usage();
            std::process::exit(1);
        }
    }

    Ok(())
}

fn print_usage() {
    eprintln!(
        r#"Usage: slateduck <command> [options]

Commands:
  serve                        Start PG-Wire sidecar
  gc plan|apply                Visibility GC (advance retain-from)
  excise plan|apply            Physical excision of old facts
  checkpoint create|list|restore  Manage catalog checkpoints
  export                       NDJSON export of catalog
  import                       Import catalog from NDJSON
  pg-migrate                   Convert NDJSON to PostgreSQL INSERTs
  rebuild                      Rebuild catalog from Parquet files
  inspect snapshot --latest    Show current catalog state
  verify catalog|data-files    Verify catalog integrity
  repair --dry-run|--apply     Repair catalog issues

Options:
  --catalog <path>             Catalog path (required for most commands)
  --help, -h                   Show this help
"#
    );
}

// ─── serve ─────────────────────────────────────────────────────────────────

async fn cmd_serve(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let config = parse_serve_args(args)?;
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

    // Start metrics server if port specified
    let metrics = Arc::new(CatalogMetrics::new(config.max_sessions as u64));
    if let Some(metrics_port) = config.metrics_port {
        let m = metrics.clone();
        tokio::spawn(async move {
            if let Err(e) = slateduck_catalog::metrics::start_metrics_server(m, metrics_port).await
            {
                tracing::error!("Metrics server error: {e}");
            }
        });
    }

    let server_config = ServerConfig {
        bind_addr: config.bind_addr,
        max_sessions: config.max_sessions,
        max_active_scans: 25,
        tls: slateduck_pgwire::server::TlsConfig {
            cert_path: config.tls_cert,
            key_path: config.tls_key,
        },
        auth: slateduck_pgwire::server::AuthConfig {
            username: config.auth_username,
            password: config.auth_password,
        },
    };

    run_server(server_config, catalog).await?;
    Ok(())
}

struct ServeConfig {
    catalog_url: String,
    bind_addr: SocketAddr,
    max_sessions: usize,
    metrics_port: Option<u16>,
    tls_cert: Option<String>,
    tls_key: Option<String>,
    auth_username: Option<String>,
    auth_password: Option<String>,
}

fn parse_serve_args(args: &[String]) -> Result<ServeConfig, String> {
    let mut catalog_url = String::new();
    let mut bind_addr: SocketAddr = "0.0.0.0:5432".parse().unwrap();
    let mut max_sessions = 50;
    let mut metrics_port = None;
    let mut tls_cert = None;
    let mut tls_key = None;
    let mut auth_username = None;
    let mut auth_password = None;

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
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
            "--metrics-port" => {
                i += 1;
                metrics_port = Some(
                    args.get(i)
                        .ok_or("--metrics-port requires a value")?
                        .parse()
                        .map_err(|e| format!("invalid metrics-port: {e}"))?,
                );
            }
            "--encryption-key" => {
                i += 1;
                let _key = args.get(i).ok_or("--encryption-key requires a value")?;
            }
            "--tls-cert" => {
                i += 1;
                tls_cert = Some(args.get(i).cloned().ok_or("--tls-cert requires a value")?);
            }
            "--tls-key" => {
                i += 1;
                tls_key = Some(args.get(i).cloned().ok_or("--tls-key requires a value")?);
            }
            "--username" => {
                i += 1;
                auth_username = Some(args.get(i).cloned().ok_or("--username requires a value")?);
            }
            "--password" => {
                i += 1;
                auth_password = Some(args.get(i).cloned().ok_or("--password requires a value")?);
            }
            "--help" | "-h" => {
                eprintln!("Usage: slateduck serve --catalog <path> [--bind <addr>] [--max-sessions <n>] [--metrics-port <port>] [--tls-cert <path>] [--tls-key <path>] [--username <user>] [--password <pass>]");
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
        return Err("--catalog is required".to_string());
    }

    Ok(ServeConfig {
        catalog_url,
        bind_addr,
        max_sessions,
        metrics_port,
        tls_cert,
        tls_key,
        auth_username,
        auth_password,
    })
}

// ─── gc ────────────────────────────────────────────────────────────────────

async fn cmd_gc(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let subcmd = args.get(2).map(|s| s.as_str()).unwrap_or("plan");
    let catalog_url = extract_catalog_arg(args, 3)?;
    let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
    let db = slatedb::Db::open(catalog_path, object_store).await?;

    let retention_days = extract_numeric_arg(args, "--retention-days").unwrap_or(30);

    match subcmd {
        "plan" => {
            let plan = slateduck_catalog::gc::gc_plan(&db, retention_days).await?;
            println!("GC Plan:");
            println!("  Current retain-from: {}", plan.current_retain_from);
            println!("  Proposed retain-from: {}", plan.proposed_retain_from);
            println!("  Snapshots affected: {}", plan.snapshots_affected);
            if !plan.pinned_snapshots.is_empty() {
                println!("  Pinned snapshots: {:?}", plan.pinned_snapshots);
            }
        }
        "apply" => {
            let plan = slateduck_catalog::gc::gc_plan(&db, retention_days).await?;
            let result = slateduck_catalog::gc::gc_apply(&db, plan.proposed_retain_from).await?;
            println!("GC Applied:");
            println!("  Previous retain-from: {}", result.previous_retain_from);
            println!("  New retain-from: {}", result.new_retain_from);
            println!("  Snapshots hidden: {}", result.snapshots_hidden);
        }
        _ => {
            eprintln!("Usage: slateduck gc [plan|apply] --catalog <path> [--retention-days <n>]");
            std::process::exit(1);
        }
    }

    db.close().await?;
    Ok(())
}

// ─── excise ────────────────────────────────────────────────────────────────

async fn cmd_excise(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let subcmd = args.get(2).map(|s| s.as_str()).unwrap_or("plan");
    let catalog_url = extract_catalog_arg(args, 3)?;
    let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
    let db = slatedb::Db::open(catalog_path, object_store).await?;

    let before = extract_numeric_arg(args, "--before")
        .ok_or("--before <snapshot> is required for excise")?;

    match subcmd {
        "plan" => {
            let plan = slateduck_catalog::excise::excise_plan(&db, before).await?;
            println!("Excise Plan:");
            println!("  Before snapshot: {}", plan.before_snapshot);
            println!("  Version rows eligible: {}", plan.version_rows_eligible);
            println!(
                "  Inlined inserts eligible: {}",
                plan.inlined_inserts_eligible
            );
            println!(
                "  Inlined deletes eligible: {}",
                plan.inlined_deletes_eligible
            );
            println!("  Data files eligible: {}", plan.data_files_eligible.len());
            println!("  Safe: {}", if plan.is_safe { "yes" } else { "NO" });
        }
        "apply" => {
            let result = slateduck_catalog::excise::excise_apply(&db, before, "operator").await?;
            println!("Excise Applied:");
            println!("  Keys deleted: {}", result.keys_deleted);
            println!("  Keys failed: {}", result.keys_failed);
            println!("  Audit entry ID: {}", result.audit_entry_id);
        }
        _ => {
            eprintln!("Usage: slateduck excise [plan|apply] --catalog <path> --before <snapshot>");
            std::process::exit(1);
        }
    }

    db.close().await?;
    Ok(())
}

// ─── checkpoint ────────────────────────────────────────────────────────────

async fn cmd_checkpoint(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let subcmd = args.get(2).map(|s| s.as_str()).unwrap_or("list");
    let catalog_url = extract_catalog_arg(args, 3)?;
    let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
    let db = slatedb::Db::open(catalog_path, object_store).await?;

    match subcmd {
        "create" => {
            let label = extract_string_arg(args, "--label");
            let info =
                slateduck_catalog::checkpoint::create_checkpoint(&db, label.as_deref()).await?;
            println!("Checkpoint created:");
            println!("  ID: {}", info.id);
            println!("  Snapshot ID: {}", info.snapshot_id);
            println!("  Created at: {}", info.created_at);
        }
        "list" => {
            let checkpoints = slateduck_catalog::checkpoint::list_checkpoints(&db).await?;
            if checkpoints.is_empty() {
                println!("No checkpoints found.");
            } else {
                println!("{:<20} {:<12} {:<30} Label", "ID", "Snapshot", "Created");
                for cp in checkpoints {
                    println!(
                        "{:<20} {:<12} {:<30} {}",
                        cp.id,
                        cp.snapshot_id,
                        cp.created_at,
                        cp.label.unwrap_or_default()
                    );
                }
            }
        }
        "restore" => {
            let id = extract_numeric_arg(args, "--id")
                .ok_or("--id <checkpoint_id> is required for restore")?;
            let info = slateduck_catalog::checkpoint::restore_checkpoint(&db, id).await?;
            println!("Checkpoint restored:");
            println!("  ID: {}", info.id);
            println!("  Restored to snapshot: {}", info.snapshot_id);
        }
        _ => {
            eprintln!("Usage: slateduck checkpoint [create|list|restore] --catalog <path>");
            std::process::exit(1);
        }
    }

    db.close().await?;
    Ok(())
}

// ─── export ────────────────────────────────────────────────────────────────

async fn cmd_export(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let catalog_url = extract_catalog_arg(args, 2)?;
    let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
    let db = slatedb::Db::open(catalog_path, object_store).await?;

    let output_path =
        extract_string_arg(args, "--output").unwrap_or_else(|| "catalog.ndjson".to_string());
    let snapshot_id = extract_numeric_arg(args, "--snapshot-id");

    let mut file = std::fs::File::create(&output_path)
        .map_err(|e| format!("Cannot create output file: {e}"))?;

    let result = slateduck_catalog::export::export_catalog(&db, snapshot_id, &mut file).await?;
    println!("Export complete:");
    println!("  Rows exported: {}", result.rows_exported);
    println!("  Tables exported: {}", result.tables_exported);
    println!("  Output: {output_path}");

    db.close().await?;
    Ok(())
}

// ─── import ────────────────────────────────────────────────────────────────

async fn cmd_import(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let catalog_url = extract_catalog_arg(args, 2)?;
    let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
    let db = slatedb::Db::open(catalog_path, object_store).await?;

    let input_path =
        extract_string_arg(args, "--input").ok_or("--input <file> is required for import")?;

    let file =
        std::fs::File::open(&input_path).map_err(|e| format!("Cannot open input file: {e}"))?;
    let reader = std::io::BufReader::new(file);

    let result = slateduck_catalog::export::import_catalog(&db, reader).await?;
    println!("Import complete:");
    println!("  Rows imported: {}", result.rows_imported);
    println!("  Tables imported: {}", result.tables_imported);

    db.close().await?;
    Ok(())
}

// ─── pg-migrate ────────────────────────────────────────────────────────────

async fn cmd_pg_migrate(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let input_path =
        extract_string_arg(args, "--input").ok_or("--input <file> is required for pg-migrate")?;

    let file =
        std::fs::File::open(&input_path).map_err(|e| format!("Cannot open input file: {e}"))?;
    let reader = std::io::BufReader::new(file);

    let mut stdout = std::io::stdout();
    let count = slateduck_catalog::export::pg_migrate(reader, &mut stdout)?;
    eprintln!("Generated {count} INSERT statements.");

    Ok(())
}

// ─── rebuild ───────────────────────────────────────────────────────────────

async fn cmd_rebuild(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let catalog_url = extract_catalog_arg(args, 2)?;
    let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
    let db = slatedb::Db::open(catalog_path, object_store.clone()).await?;

    let data_path =
        extract_string_arg(args, "--data-path").ok_or("--data-path is required for rebuild")?;

    // List Parquet files in the data path
    let data_prefix = ObjectPath::from(data_path.as_str());
    let mut data_paths = Vec::new();

    use futures::TryStreamExt;
    let objects: Vec<_> = object_store
        .list(Some(&data_prefix))
        .try_collect()
        .await
        .unwrap_or_default();

    for obj in objects {
        let path_str = obj.location.to_string();
        if path_str.ends_with(".parquet") {
            data_paths.push(path_str);
        }
    }

    let count = slateduck_catalog::export::rebuild_catalog(&db, &data_paths).await?;
    println!("Rebuild complete: {count} files registered.");

    db.close().await?;
    Ok(())
}

// ─── inspect ───────────────────────────────────────────────────────────────

async fn cmd_inspect(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let catalog_url = extract_catalog_arg(args, 2)?;
    let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
    let db = slatedb::Db::open(catalog_path, object_store).await?;

    let result = slateduck_catalog::inspect::inspect_snapshot(&db).await?;
    println!("Catalog State:");
    println!("  Latest snapshot ID: {}", result.latest_snapshot_id);
    println!("  Schema version: {}", result.schema_version);
    println!("  Snapshot time: {}", result.snapshot_time);
    println!("  Next snapshot ID: {}", result.next_snapshot_id);
    println!("  Next catalog ID: {}", result.next_catalog_id);
    println!("  Next file ID: {}", result.next_file_id);
    println!("  Schemas: {}", result.schema_count);
    println!("  Tables: {}", result.table_count);
    println!("  Columns: {}", result.column_count);
    println!("  Data files: {}", result.data_file_count);
    println!("  Delete files: {}", result.delete_file_count);
    println!("  Retain-from: {}", result.retain_from);
    println!("  Writer epoch: {}", result.writer_epoch);
    println!("  Format version: {}", result.format_version);

    db.close().await?;
    Ok(())
}

// ─── verify ────────────────────────────────────────────────────────────────

async fn cmd_verify(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let subcmd = args.get(2).map(|s| s.as_str()).unwrap_or("catalog");
    let catalog_url = extract_catalog_arg(args, 3)?;
    let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
    let db = slatedb::Db::open(catalog_path, object_store.clone()).await?;

    match subcmd {
        "catalog" => {
            let result = slateduck_catalog::verify::verify_catalog(&db).await?;
            println!("Catalog Verification:");
            println!("  Tables checked: {}", result.tables_checked);
            println!("  Rows checked: {}", result.rows_checked);
            if result.errors.is_empty() {
                println!("  Status: OK");
            } else {
                println!("  Errors:");
                for err in &result.errors {
                    println!("    - {err}");
                }
            }
            if !result.warnings.is_empty() {
                println!("  Warnings:");
                for warn in &result.warnings {
                    println!("    - {warn}");
                }
            }
        }
        "data-files" => {
            let result = slateduck_catalog::cleanup::verify_data_files(&db, &object_store).await?;
            println!("Data File Verification:");
            println!("  Files OK: {}", result.files_ok);
            println!("  Files missing: {}", result.files_missing.len());
            println!("  Files error: {}", result.files_error.len());
            println!("  Total checked: {}", result.total_checked);
            if !result.files_missing.is_empty() {
                println!("  Missing files:");
                for path in &result.files_missing {
                    println!("    - {path}");
                }
            }
        }
        _ => {
            eprintln!("Usage: slateduck verify [catalog|data-files] --catalog <path>");
            std::process::exit(1);
        }
    }

    db.close().await?;
    Ok(())
}

// ─── repair ────────────────────────────────────────────────────────────────

async fn cmd_repair(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let catalog_url = extract_catalog_arg(args, 2)?;
    let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
    let db = slatedb::Db::open(catalog_path, object_store).await?;

    let apply = args.iter().any(|a| a == "--apply");

    let plan = slateduck_catalog::repair::repair_plan(&db).await?;

    if plan.is_empty() {
        println!("No repairs needed. Catalog is healthy.");
    } else {
        println!("Repair Plan:");
        for action in &plan.actions {
            println!("  - {action:?}");
        }
        if plan.has_unrecoverable() {
            println!("  UNRECOVERABLE ERRORS (restore from backup):");
            for err in &plan.unrecoverable_errors {
                println!("    - {err}");
            }
        }

        if apply && !plan.has_unrecoverable() {
            let result = slateduck_catalog::repair::repair_apply(&db, &plan).await?;
            println!("Repair Applied:");
            println!("  Actions applied: {}", result.actions_applied);
            println!("  Actions failed: {}", result.actions_failed);
        } else if !apply {
            println!("\nDry run. Use --apply to execute repairs.");
        }
    }

    db.close().await?;
    Ok(())
}

// ─── Helpers ───────────────────────────────────────────────────────────────

fn extract_catalog_arg(args: &[String], start: usize) -> Result<String, String> {
    for (i, arg) in args.iter().enumerate().skip(start) {
        if arg == "--catalog" || arg == "-c" {
            return args
                .get(i + 1)
                .cloned()
                .ok_or_else(|| "--catalog requires a value".to_string());
        }
    }
    for arg in args.iter().skip(start) {
        if !arg.starts_with('-') {
            return Ok(arg.clone());
        }
    }
    Err("--catalog <path> is required".to_string())
}

fn extract_numeric_arg(args: &[String], flag: &str) -> Option<u64> {
    for (i, arg) in args.iter().enumerate() {
        if arg == flag {
            return args.get(i + 1).and_then(|v| v.parse().ok());
        }
    }
    None
}

fn extract_string_arg(args: &[String], flag: &str) -> Option<String> {
    for (i, arg) in args.iter().enumerate() {
        if arg == flag {
            return args.get(i + 1).cloned();
        }
    }
    None
}

fn resolve_catalog(url: &str) -> Result<(ObjectPath, Arc<dyn object_store::ObjectStore>), String> {
    if let Some(without_scheme) = url.strip_prefix("s3://") {
        let (bucket, prefix) = match without_scheme.find('/') {
            Some(idx) => (&without_scheme[..idx], &without_scheme[idx + 1..]),
            None => (without_scheme, ""),
        };

        let store = object_store::aws::AmazonS3Builder::from_env()
            .with_bucket_name(bucket)
            .build()
            .map_err(|e| format!("Failed to create S3 object store: {e}"))?;

        let obj_path = ObjectPath::from(prefix);
        Ok((obj_path, Arc::new(store)))
    } else {
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
}
