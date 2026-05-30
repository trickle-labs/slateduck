//! `rocklake` — CLI binary with all operational commands.
//!
//! Commands:
//!   serve, gc, excise, checkpoint, export, import, pg-migrate,
//!   rebuild, inspect, verify, repair,
//!   warmup, migrate, corpus, tune,
//!   migrate-from-ducklake, export-catalog,
//!   diagnose, sweep-orphans
//!
//! Run `rocklake --help` or `rocklake <command> --help` for full usage.

mod cli;

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use clap::{CommandFactory as _, Parser as _};
use clap_complete::generate;
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

use rocklake_catalog::metrics::CatalogMetrics;
use rocklake_catalog::{CatalogStore, OpenOptions};
use rocklake_pgwire::server::{run_server, ServerConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive(
                "info"
                    .parse()
                    .unwrap_or_else(|_| tracing_subscriber::filter::LevelFilter::INFO.into()),
            ),
        )
        .init();

    // Parse with clap; this provides --help (exit 0), --version, and shell
    // completions.  Fall back to the legacy hand-rolled dispatcher for full
    // backward compatibility.
    let parsed = cli::Cli::try_parse();
    match parsed {
        Ok(cli_args) => dispatch_clap(cli_args).await?,
        Err(e) => {
            // If clap emits --help / --version messages, honour its exit code.
            if e.use_stderr() {
                // Unknown command or bad flag — fall through to legacy dispatch
                // for backward compatibility with scripts that use the old
                // positional command style.
                legacy_dispatch().await?;
            } else {
                // --help or --version: print and exit with the code clap chose.
                e.print()?;
                std::process::exit(0);
            }
        }
    }

    Ok(())
}

/// Dispatch based on a successfully-parsed clap CLI.
async fn dispatch_clap(cli: cli::Cli) -> Result<(), Box<dyn std::error::Error>> {
    use cli::Commands;

    // Build a synthetic args vec for the legacy handlers so we don't have to
    // rewrite every handler right now.  Each branch translates the typed clap
    // struct back into the `--flag value` format the handlers expect.
    match cli.command {
        Commands::Completions(args) => {
            let mut cmd = cli::Cli::command();
            generate(args.shell, &mut cmd, "rocklake", &mut io::stdout());
            return Ok(());
        }
        Commands::Serve(a) => {
            let mut synthetic = vec!["rocklake".to_string(), "serve".to_string()];
            synthetic.extend(["--catalog".to_string(), a.catalog]);
            synthetic.extend(["--bind".to_string(), a.bind]);
            synthetic.extend(["--max-sessions".to_string(), a.max_sessions.to_string()]);
            if let Some(p) = a.metrics_port {
                synthetic.extend(["--metrics-port".to_string(), p.to_string()]);
            }
            synthetic.extend(["--metrics-path".to_string(), a.metrics_path]);
            if let Some(c) = a.tls_cert {
                synthetic.extend(["--tls-cert".to_string(), c]);
            }
            if let Some(k) = a.tls_key {
                synthetic.extend(["--tls-key".to_string(), k]);
            }
            if a.tls_required {
                synthetic.push("--tls-required".to_string());
            }
            if let Some(u) = a.auth_user {
                synthetic.extend(["--auth-user".to_string(), u]);
            }
            if let Some(p) = a.auth_password {
                synthetic.extend(["--auth-password".to_string(), p]);
            }
            let mode = if a.read_only {
                "reader".to_string()
            } else {
                a.mode
            };
            synthetic.extend(["--mode".to_string(), mode]);
            synthetic.extend(["--cost-mode".to_string(), a.cost_mode]);
            if let Some(e) = a.s3_endpoint {
                synthetic.extend(["--s3-endpoint".to_string(), e]);
            }
            if a.s3_path_style {
                synthetic.push("--s3-path-style".to_string());
            }
            if let Some(k) = a.encryption_key {
                synthetic.extend(["--encryption-key".to_string(), k]);
            }
            if let Some(p) = a.datafusion_pg_wire {
                synthetic.extend(["--datafusion-pg-wire".to_string(), p.to_string()]);
            }
            if !a.extension_schemas.is_empty() {
                synthetic.extend([
                    "--extension-schemas".to_string(),
                    a.extension_schemas.join(","),
                ]);
            }
            if let Some(o) = a.otlp_endpoint {
                synthetic.extend(["--otlp-endpoint".to_string(), o]);
            }
            synthetic.extend([
                "--idle-connection-timeout".to_string(),
                a.idle_connection_timeout.to_string(),
            ]);
            synthetic.extend(["--drain-timeout".to_string(), a.drain_timeout.to_string()]);
            synthetic.extend([
                "--datafusion-bridge-queue-depth".to_string(),
                a.datafusion_bridge_queue_depth.to_string(),
            ]);
            cmd_serve(&synthetic).await?;
        }
        Commands::Gc(sub) => {
            use cli::GcSubcommand;
            let (subcmd, args) = match sub {
                GcSubcommand::Plan(a) => ("plan", a),
                GcSubcommand::Apply(a) => ("apply", a),
            };
            let synthetic = vec![
                "rocklake".to_string(),
                "gc".to_string(),
                subcmd.to_string(),
                "--catalog".to_string(),
                args.catalog,
                "--retention-days".to_string(),
                args.retention_days.to_string(),
            ];
            cmd_gc(&synthetic).await?;
        }
        Commands::Excise(sub) => {
            use cli::ExciseSubcommand;
            let (subcmd, args) = match sub {
                ExciseSubcommand::Plan(a) => ("plan", a),
                ExciseSubcommand::Apply(a) => ("apply", a),
            };
            let synthetic = vec![
                "rocklake".to_string(),
                "excise".to_string(),
                subcmd.to_string(),
                "--catalog".to_string(),
                args.catalog,
                "--before".to_string(),
                args.before.to_string(),
            ];
            cmd_excise(&synthetic).await?;
        }
        Commands::Checkpoint(sub) => {
            use cli::CheckpointSubcommand;
            match sub {
                CheckpointSubcommand::Create(a) => {
                    let mut s = vec![
                        "rocklake".to_string(),
                        "checkpoint".to_string(),
                        "create".to_string(),
                        "--catalog".to_string(),
                        a.catalog,
                    ];
                    if let Some(l) = a.label {
                        s.extend(["--label".to_string(), l]);
                    }
                    cmd_checkpoint(&s).await?;
                }
                CheckpointSubcommand::List(a) => {
                    let s = vec![
                        "rocklake".to_string(),
                        "checkpoint".to_string(),
                        "list".to_string(),
                        "--catalog".to_string(),
                        a.catalog,
                    ];
                    cmd_checkpoint(&s).await?;
                }
                CheckpointSubcommand::Restore(a) => {
                    let s = vec![
                        "rocklake".to_string(),
                        "checkpoint".to_string(),
                        "restore".to_string(),
                        "--catalog".to_string(),
                        a.catalog,
                        "--id".to_string(),
                        a.id.to_string(),
                    ];
                    cmd_checkpoint(&s).await?;
                }
            }
        }
        Commands::Export(a) => {
            let mut s = vec![
                "rocklake".to_string(),
                "export".to_string(),
                a.catalog,
                "--output".to_string(),
                a.output,
            ];
            if let Some(id) = a.snapshot_id {
                s.extend(["--snapshot-id".to_string(), id.to_string()]);
            }
            cmd_export(&s).await?;
        }
        Commands::Import(a) => {
            let s = vec![
                "rocklake".to_string(),
                "import".to_string(),
                a.catalog,
                "--input".to_string(),
                a.input,
            ];
            cmd_import(&s).await?;
        }
        Commands::PgMigrate(a) => {
            let s = vec![
                "rocklake".to_string(),
                "pg-migrate".to_string(),
                "--input".to_string(),
                a.input,
            ];
            cmd_pg_migrate(&s).await?;
        }
        Commands::Rebuild(a) => {
            let mut s = vec![
                "rocklake".to_string(),
                "rebuild".to_string(),
                "--catalog".to_string(),
                a.catalog,
            ];
            if let Some(d) = a.data_root {
                s.extend(["--data-root".to_string(), d]);
            }
            if let Some(e) = a.s3_endpoint {
                s.extend(["--s3-endpoint".to_string(), e]);
            }
            if a.s3_path_style {
                s.push("--s3-path-style".to_string());
            }
            cmd_rebuild(&s).await?;
        }
        Commands::Inspect(sub) => {
            use cli::InspectSubcommand;
            let (subcmd, args) = match sub {
                InspectSubcommand::Snapshot(a) => ("snapshot", a),
                InspectSubcommand::ApiCosts(a) => ("api-costs", a),
                InspectSubcommand::CacheUtilization(a) => ("cache-utilization", a),
            };
            let s = vec![
                "rocklake".to_string(),
                "inspect".to_string(),
                subcmd.to_string(),
                "--catalog".to_string(),
                args.catalog,
            ];
            cmd_inspect(&s).await?;
        }
        Commands::Verify(sub) => {
            use cli::VerifySubcommand;
            let (subcmd, args) = match sub {
                VerifySubcommand::Catalog(a) => ("catalog", a),
                VerifySubcommand::DataFiles(a) => ("data-files", a),
            };
            let s = vec![
                "rocklake".to_string(),
                "verify".to_string(),
                subcmd.to_string(),
                "--catalog".to_string(),
                args.catalog,
            ];
            cmd_verify(&s).await?;
        }
        Commands::Repair(a) => {
            let mut s = vec![
                "rocklake".to_string(),
                "repair".to_string(),
                "--catalog".to_string(),
                a.catalog,
            ];
            if a.dry_run {
                s.push("--dry-run".to_string());
            }
            if a.apply {
                s.push("--apply".to_string());
            }
            cmd_repair(&s).await?;
        }
        Commands::Warmup(a) => {
            let mut s = vec![
                "rocklake".to_string(),
                "warmup".to_string(),
                "--catalog".to_string(),
                a.catalog,
            ];
            if let Some(t) = a.tables {
                s.extend(["--tables".to_string(), t.to_string()]);
            }
            cmd_warmup(&s).await?;
        }
        Commands::Migrate(a) => {
            let mut s = vec![
                "rocklake".to_string(),
                "migrate".to_string(),
                "--catalog".to_string(),
                a.catalog,
            ];
            if a.dry_run {
                s.push("--dry-run".to_string());
            }
            if a.apply {
                s.push("--apply".to_string());
            }
            cmd_migrate(&s).await?;
        }
        Commands::Corpus(sub) => {
            use cli::CorpusSubcommand;
            match sub {
                CorpusSubcommand::Diff(a) => {
                    let s = vec![
                        "rocklake".to_string(),
                        "corpus".to_string(),
                        "diff".to_string(),
                        a.left,
                        a.right,
                    ];
                    cmd_corpus(&s).await?;
                }
                CorpusSubcommand::Validate(a) => {
                    let s = vec![
                        "rocklake".to_string(),
                        "corpus".to_string(),
                        "validate".to_string(),
                        "--catalog".to_string(),
                        a.catalog,
                        a.corpus,
                    ];
                    cmd_corpus(&s).await?;
                }
            }
        }
        Commands::Tune(a) => {
            let mut s = vec![
                "rocklake".to_string(),
                "tune".to_string(),
                "--catalog".to_string(),
                a.catalog,
            ];
            if let Some(c) = a.target_cost_usd {
                s.extend(["--target-cost-usd".to_string(), c.to_string()]);
            }
            cmd_tune(&s).await?;
        }
        Commands::MigrateFromDucklake(a) => {
            let mut s = vec![
                "rocklake".to_string(),
                "migrate-from-ducklake".to_string(),
                "--source".to_string(),
                a.source,
                "--catalog".to_string(),
                a.catalog,
            ];
            if a.dry_run {
                s.push("--dry-run".to_string());
            }
            for v in a.accept_versions {
                s.extend(["--accept-version".to_string(), v]);
            }
            cmd_migrate_from_ducklake(&s).await?;
        }
        Commands::ExportCatalog(a) => {
            let mut s = vec![
                "rocklake".to_string(),
                "export-catalog".to_string(),
                "--catalog".to_string(),
                a.catalog,
                "--out".to_string(),
                a.out,
            ];
            if let Some(id) = a.at_snapshot {
                s.extend(["--at-snapshot".to_string(), id.to_string()]);
            }
            cmd_export_catalog(&s).await?;
        }
        Commands::Diagnose(a) => {
            let mut s = vec![
                "rocklake".to_string(),
                "diagnose".to_string(),
                "--catalog".to_string(),
                a.catalog,
            ];
            if a.json {
                s.push("--json".to_string());
            }
            if let Some(d) = a.data_root {
                s.extend(["--data-root".to_string(), d]);
            }
            cmd_diagnose(&s).await?;
        }
        Commands::SweepOrphans(a) => {
            let mut s = vec![
                "rocklake".to_string(),
                "sweep-orphans".to_string(),
                "--catalog".to_string(),
                a.catalog,
                "--data-root".to_string(),
                a.data_root,
                "--grace-period-hours".to_string(),
                a.grace_period_hours.to_string(),
            ];
            if a.apply {
                s.push("--apply".to_string());
            }
            cmd_sweep_orphans(&s).await?;
        }
    }
    Ok(())
}

/// Legacy dispatcher: handles the original positional command-name style for
/// backward compatibility with scripts that do not yet use the clap path.
async fn legacy_dispatch() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        // Delegate to clap for a nice help message.
        cli::Cli::parse_from(["rocklake", "--help"]);
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
        "warmup" => cmd_warmup(&args).await?,
        "migrate" => cmd_migrate(&args).await?,
        "corpus" => cmd_corpus(&args).await?,
        "tune" => cmd_tune(&args).await?,
        "migrate-from-ducklake" => cmd_migrate_from_ducklake(&args).await?,
        "export-catalog" => cmd_export_catalog(&args).await?,
        "diagnose" => cmd_diagnose(&args).await?,
        "sweep-orphans" => cmd_sweep_orphans(&args).await?,
        "--help" | "-h" => {
            cli::Cli::parse_from(["rocklake", "--help"]);
        }
        other => {
            eprintln!("Unknown command: {other}");
            eprintln!("Run 'rocklake --help' for available commands.");
            std::process::exit(1);
        }
    }

    Ok(())
}

// ─── serve ─────────────────────────────────────────────────────────────────

async fn cmd_serve(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let config = parse_serve_args(args)?;

    // v0.39.0: Initialise OTLP tracing if --otlp-endpoint is set.
    let _telemetry = rocklake_pgwire::telemetry::TelemetryConfig {
        otlp_endpoint: config.otlp_endpoint.clone(),
        service_name: "rocklake".to_string(),
    }
    .init();

    let s3_opts = S3Options {
        endpoint: config.s3_endpoint.clone(),
        path_style: config.s3_path_style,
    };
    let (catalog_path, object_store) = resolve_catalog_with_opts(&config.catalog_url, &s3_opts)?;

    let opts = OpenOptions {
        object_store: object_store.clone(),
        path: catalog_path,
        encryption: config
            .encryption_key
            .as_deref()
            .map(rocklake_catalog::EncryptionConfig::from_hex)
            .transpose()
            .map_err(|e| format!("--encryption-key: {e}"))?,
    };

    let store = if config.mode == "reader" {
        // Read-only mode: skip the writer-epoch CAS so that any number of
        // reader replicas can open the same catalog concurrently without
        // contending on the epoch key.
        tracing::info!("Opening catalog in read-only mode (no writer epoch)");
        CatalogStore::open_without_epoch(opts)
            .await
            .map_err(|e| format!("Failed to open catalog (read-only): {e}"))?
    } else {
        CatalogStore::open(opts)
            .await
            .map_err(|e| format!("Failed to open catalog: {e}"))?
    };

    tracing::info!("Catalog opened successfully");
    tracing::info!(
        "Serving mode: {}, cost mode: {:?}, datafusion bridge queue depth: {}",
        config.mode,
        config.cost_mode,
        config.datafusion_bridge_queue_depth,
    );

    let catalog = Arc::new(Mutex::new(store));

    // Start metrics server if port specified
    let metrics = Arc::new(CatalogMetrics::new(config.max_sessions as u64));
    if let Some(metrics_port) = config.metrics_port {
        let m = metrics.clone();
        let mpath = config.metrics_path.clone();
        tokio::spawn(async move {
            if let Err(e) =
                rocklake_catalog::metrics::start_metrics_server(m, metrics_port, &mpath).await
            {
                tracing::error!("Metrics server error: {e}");
            }
        });
    }

    // Background task: sync CDC record-count mismatch counter from rocklake-sql global.
    {
        let m = metrics.clone();
        tokio::spawn(async move {
            loop {
                m.set_cdc_record_count_mismatches(rocklake_sql::cdc_record_count_mismatch_total());
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });
    }

    let server_config = ServerConfig {
        bind_addr: config.bind_addr,
        max_sessions: config.max_sessions,
        max_active_scans: 25,
        tls: rocklake_pgwire::server::TlsConfig {
            cert_path: config.tls_cert,
            key_path: config.tls_key,
            required: config.tls_required,
        },
        auth: rocklake_pgwire::server::AuthConfig {
            username: config.auth_username,
            password: config.auth_password,
            scram_sha256: false,
        },
        extension_schemas: config.extension_schemas.clone(),
        idle_connection_timeout: std::time::Duration::from_secs(
            config.idle_connection_timeout_secs,
        ),
        drain_timeout: std::time::Duration::from_secs(config.drain_timeout_secs),
    };

    // If --datafusion-pg-wire <port> is set, also start a second listener on
    // that port.  DataFusion clients connecting there are routed through the
    // same bounded SQL dispatcher as every other DuckLake client.
    if let Some(df_port) = config.datafusion_pg_wire_port {
        let df_addr: SocketAddr = format!("0.0.0.0:{df_port}")
            .parse()
            .map_err(|e| format!("invalid DataFusion pg-wire address port {df_port}: {e}"))?;
        let df_config = ServerConfig {
            bind_addr: df_addr,
            max_sessions: server_config.max_sessions,
            max_active_scans: server_config.max_active_scans,
            tls: rocklake_pgwire::server::TlsConfig::default(),
            auth: rocklake_pgwire::server::AuthConfig::default(),
            extension_schemas: config.extension_schemas,
            idle_connection_timeout: server_config.idle_connection_timeout,
            drain_timeout: server_config.drain_timeout,
        };
        let df_catalog = catalog.clone();
        tokio::spawn(async move {
            if let Err(e) = run_server(df_config, df_catalog).await {
                tracing::error!("DataFusion pg-wire listener error: {e}");
            }
        });
        tracing::info!("DataFusion pg-wire listener started on port {df_port}");
    }

    run_server(server_config, catalog).await?;
    Ok(())
}

struct ServeConfig {
    catalog_url: String,
    bind_addr: SocketAddr,
    max_sessions: usize,
    metrics_port: Option<u16>,
    /// HTTP path for the metrics endpoint. Default: `/metrics`.
    metrics_path: String,
    tls_cert: Option<String>,
    tls_key: Option<String>,
    tls_required: bool,
    auth_username: Option<String>,
    auth_password: Option<String>,
    /// Serving mode: "writer" (accepts writes) or "reader" (read-only, returns 25006 on writes).
    mode: String,
    /// Cost/latency preset: "conservative", "balanced" (default), or "latency".
    cost_mode: rocklake_catalog::CostMode,
    /// Optional S3-compatible endpoint URL (e.g. for MinIO).
    s3_endpoint: Option<String>,
    /// Use S3 path-style addressing (required for some S3-compatible stores).
    s3_path_style: bool,
    /// Optional AES-256 encryption key (64 hex digits).
    encryption_key: Option<String>,
    /// When set, also listen on this port for DataFusion pg-wire connections.
    /// DataFusion clients connecting on this port are routed through the same
    /// bounded SQL dispatcher as DuckDB/Spark/Trino clients.
    datafusion_pg_wire_port: Option<u16>,
    /// Allowed extension schema names (default: ["pgtrickle"]).
    extension_schemas: Vec<String>,
    /// Optional OTLP HTTP endpoint for OpenTelemetry tracing (e.g. "http://jaeger:4318").
    /// When not set, no spans are exported. Document: docs/operations/monitoring.md.
    otlp_endpoint: Option<String>,
    /// Duration in seconds after which an idle connection is closed (default: 60).
    idle_connection_timeout_secs: u64,
    /// Grace period in seconds for in-flight queries on SIGTERM drain (default: 30).
    drain_timeout_secs: u64,
    /// Capacity of the DataFusion AsyncBridge channel (default: 256).
    datafusion_bridge_queue_depth: usize,
}

fn parse_serve_args(args: &[String]) -> Result<ServeConfig, String> {
    let mut catalog_url = String::new();
    // SAFETY: "0.0.0.0:5432" is a compile-time constant that always parses correctly.
    let mut bind_addr: SocketAddr = "0.0.0.0:5432"
        .parse()
        .expect("default bind address is always valid");
    let mut max_sessions = 50;
    let mut metrics_port = None;
    // Metrics path: read from env first, CLI flag overrides.
    let mut metrics_path: String =
        std::env::var("ROCKLAKE_METRICS_PATH").unwrap_or_else(|_| "/metrics".to_string());
    let mut tls_cert = None;
    let mut tls_key = None;
    let mut tls_required = false;
    // Read auth from env vars first; CLI flags override.
    let mut auth_username: Option<String> = std::env::var("ROCKLAKE_AUTH_USER").ok();
    let mut auth_password: Option<String> = std::env::var("ROCKLAKE_AUTH_PASSWORD").ok();
    // OTLP endpoint: read from env first, CLI flag overrides.
    let mut otlp_endpoint: Option<String> = std::env::var("ROCKLAKE_OTLP_ENDPOINT").ok();
    let mut mode = "writer".to_string();
    let mut cost_mode = rocklake_catalog::CostMode::Balanced;
    let mut s3_endpoint: Option<String> = None;
    let mut s3_path_style = false;
    let mut encryption_key: Option<String> = None;
    let mut datafusion_pg_wire_port: Option<u16> = None;
    // Extension schemas: read from env first (comma-separated), CLI flag overrides.
    let mut extension_schemas: Vec<String> = std::env::var("ROCKLAKE_EXTENSION_SCHEMAS")
        .ok()
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_else(|| vec!["public".to_string(), "pgtrickle".to_string()]);
    let mut idle_connection_timeout_secs: u64 = 60;
    let mut drain_timeout_secs: u64 = 30;
    let mut datafusion_bridge_queue_depth: usize = 256;

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
            "--metrics-bind" => {
                i += 1;
                let bind_str = args.get(i).ok_or("--metrics-bind requires a value")?;
                // Parse as <host:port> and extract port for the metrics server.
                let port: u16 = bind_str
                    .rsplit_once(':')
                    .ok_or("--metrics-bind must be in <host:port> format")
                    .and_then(|(_, p)| p.parse().map_err(|_| "--metrics-bind port is invalid"))?;
                metrics_port = Some(port);
            }
            "--metrics-path" => {
                i += 1;
                metrics_path = args
                    .get(i)
                    .cloned()
                    .ok_or("--metrics-path requires a value")?;
            }
            "--encryption-key" => {
                i += 1;
                encryption_key = Some(
                    args.get(i)
                        .cloned()
                        .ok_or("--encryption-key requires a value")?,
                );
            }
            "--tls-cert" => {
                i += 1;
                tls_cert = Some(args.get(i).cloned().ok_or("--tls-cert requires a value")?);
            }
            "--tls-key" => {
                i += 1;
                tls_key = Some(args.get(i).cloned().ok_or("--tls-key requires a value")?);
            }
            "--tls-required" => {
                tls_required = true;
            }
            // New canonical names for auth flags.
            "--auth-user" => {
                i += 1;
                auth_username = Some(args.get(i).cloned().ok_or("--auth-user requires a value")?);
            }
            "--auth-password" => {
                i += 1;
                auth_password = Some(
                    args.get(i)
                        .cloned()
                        .ok_or("--auth-password requires a value")?,
                );
            }
            // Legacy aliases kept for backward compatibility.
            "--username" => {
                i += 1;
                auth_username = Some(args.get(i).cloned().ok_or("--username requires a value")?);
            }
            "--password" => {
                i += 1;
                auth_password = Some(args.get(i).cloned().ok_or("--password requires a value")?);
            }
            "--mode" => {
                i += 1;
                let m = args.get(i).cloned().ok_or("--mode requires a value")?;
                if m != "writer" && m != "reader" {
                    return Err(format!("--mode must be 'writer' or 'reader', got '{m}'"));
                }
                mode = m;
            }
            "--read-only" => {
                mode = "reader".to_string();
            }
            "--cost-mode" => {
                i += 1;
                let m = args.get(i).ok_or("--cost-mode requires a value")?;
                cost_mode = m.parse::<rocklake_catalog::CostMode>()?;
            }
            "--s3-endpoint" => {
                i += 1;
                s3_endpoint = Some(
                    args.get(i)
                        .cloned()
                        .ok_or("--s3-endpoint requires a value")?,
                );
            }
            "--s3-path-style" => {
                s3_path_style = true;
            }
            "--datafusion-pg-wire" => {
                i += 1;
                datafusion_pg_wire_port = Some(
                    args.get(i)
                        .ok_or("--datafusion-pg-wire requires a port value")?
                        .parse()
                        .map_err(|e| format!("invalid --datafusion-pg-wire port: {e}"))?,
                );
            }
            "--extension-schemas" => {
                i += 1;
                let schemas_str = args
                    .get(i)
                    .cloned()
                    .ok_or("--extension-schemas requires a comma-separated list")?;
                extension_schemas = schemas_str
                    .split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect();
            }
            // v0.39.0: OTLP tracing endpoint.
            "--otlp-endpoint" => {
                i += 1;
                otlp_endpoint = Some(
                    args.get(i)
                        .cloned()
                        .ok_or("--otlp-endpoint requires a URL value")?,
                );
            }
            // v0.39.0: --metrics-addr is a convenient alias for --metrics-bind.
            "--metrics-addr" => {
                i += 1;
                let bind_str = args
                    .get(i)
                    .ok_or("--metrics-addr requires a <host:port> value")?;
                let port: u16 = bind_str
                    .rsplit_once(':')
                    .ok_or("--metrics-addr must be in <host:port> format")
                    .and_then(|(_, p)| p.parse().map_err(|_| "--metrics-addr port is invalid"))?;
                metrics_port = Some(port);
            }
            // v0.47.0: connection management.
            "--idle-connection-timeout" => {
                i += 1;
                idle_connection_timeout_secs = args
                    .get(i)
                    .ok_or("--idle-connection-timeout requires a value")?
                    .parse()
                    .map_err(|e| format!("invalid --idle-connection-timeout: {e}"))?;
            }
            "--drain-timeout" => {
                i += 1;
                drain_timeout_secs = args
                    .get(i)
                    .ok_or("--drain-timeout requires a value")?
                    .parse()
                    .map_err(|e| format!("invalid --drain-timeout: {e}"))?;
            }
            "--datafusion-bridge-queue-depth" => {
                i += 1;
                datafusion_bridge_queue_depth = args
                    .get(i)
                    .ok_or("--datafusion-bridge-queue-depth requires a value")?
                    .parse()
                    .map_err(|e| format!("invalid --datafusion-bridge-queue-depth: {e}"))?;
            }
            "--help" | "-h" => {
                eprintln!(
                    "Usage: rocklake serve --catalog <path> \
                    [--bind <addr>] [--max-sessions <n>] \
                    [--metrics-port <port>] [--metrics-bind <host:port>] [--metrics-path <path>] \
                    [--tls-cert <path>] [--tls-key <path>] [--tls-required] \
                    [--auth-user <user>] [--auth-password <pass>] \
                    [--mode writer|reader] [--read-only] \
                    [--cost-mode conservative|balanced|latency] \
                    [--s3-endpoint <url>] [--s3-path-style] \
                    [--encryption-key <hex>] \
                    [--datafusion-pg-wire <port>] \
                    [--extension-schemas <schema,...>]"
                );
                eprintln!(
                    "\nEnvironment variables:\
                    \n  ROCKLAKE_AUTH_USER           Username for authentication\
                    \n  ROCKLAKE_AUTH_PASSWORD        Password for authentication\
                    \n  ROCKLAKE_EXTENSION_SCHEMAS    Comma-separated allowed extension schema names\
                    \n  ROCKLAKE_METRICS_PATH         HTTP path for the metrics endpoint (default: /metrics)"
                );
                eprintln!(
                    "\nSupported catalog URLs:\
                    \n  s3://bucket/path         Amazon S3 or compatible\
                    \n  gs://bucket/path         Google Cloud Storage\
                    \n  az://container/path      Azure Blob Storage\
                    \n  /local/path              Local filesystem"
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
        return Err("--catalog is required".to_string());
    }

    Ok(ServeConfig {
        catalog_url,
        bind_addr,
        max_sessions,
        metrics_port,
        metrics_path,
        tls_cert,
        tls_key,
        tls_required,
        auth_username,
        auth_password,
        mode,
        cost_mode,
        s3_endpoint,
        s3_path_style,
        encryption_key,
        datafusion_pg_wire_port,
        extension_schemas,
        otlp_endpoint,
        idle_connection_timeout_secs,
        drain_timeout_secs,
        datafusion_bridge_queue_depth,
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
            let plan = rocklake_catalog::gc::gc_plan(&db, retention_days).await?;
            println!("GC Plan:");
            println!("  Current retain-from: {}", plan.current_retain_from);
            println!("  Proposed retain-from: {}", plan.proposed_retain_from);
            println!("  Snapshots affected: {}", plan.snapshots_affected);
            if !plan.pinned_snapshots.is_empty() {
                println!("  Pinned snapshots: {:?}", plan.pinned_snapshots);
            }
        }
        "apply" => {
            let plan = rocklake_catalog::gc::gc_plan(&db, retention_days).await?;
            let result = rocklake_catalog::gc::gc_apply(&db, plan.proposed_retain_from).await?;
            println!("GC Applied:");
            println!("  Previous retain-from: {}", result.previous_retain_from);
            println!("  New retain-from: {}", result.new_retain_from);
            println!("  Snapshots hidden: {}", result.snapshots_hidden);
        }
        _ => {
            eprintln!("Usage: rocklake gc [plan|apply] --catalog <path> [--retention-days <n>]");
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
            let plan = rocklake_catalog::excise::excise_plan(&db, before).await?;
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
            let result = rocklake_catalog::excise::excise_apply(&db, before, "operator").await?;
            println!("Excise Applied:");
            println!("  Keys deleted: {}", result.keys_deleted);
            println!("  Keys failed: {}", result.keys_failed);
            println!("  Audit entry ID: {}", result.audit_entry_id);
        }
        _ => {
            eprintln!("Usage: rocklake excise [plan|apply] --catalog <path> --before <snapshot>");
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
                rocklake_catalog::checkpoint::create_checkpoint(&db, label.as_deref()).await?;
            println!("Checkpoint created:");
            println!("  ID: {}", info.id);
            println!("  Snapshot ID: {}", info.snapshot_id);
            println!("  Created at: {}", info.created_at);
        }
        "list" => {
            let checkpoints = rocklake_catalog::checkpoint::list_checkpoints(&db).await?;
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
            let info = rocklake_catalog::checkpoint::restore_checkpoint(&db, id).await?;
            println!("Checkpoint restored:");
            println!("  ID: {}", info.id);
            println!("  Restored to snapshot: {}", info.snapshot_id);
        }
        _ => {
            eprintln!("Usage: rocklake checkpoint [create|list|restore] --catalog <path>");
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

    let result = rocklake_catalog::export::export_catalog(&db, snapshot_id, &mut file).await?;
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

    let result = rocklake_catalog::export::import_catalog(&db, reader).await?;
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
    let count = rocklake_catalog::export::pg_migrate(reader, &mut stdout)?;
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
        .map_err(|e| format!("Failed to list objects at '{data_path}': {e}"))?;

    for obj in objects {
        let path_str = obj.location.to_string();
        if path_str.ends_with(".parquet") {
            data_paths.push(path_str);
        }
    }

    let count = rocklake_catalog::export::rebuild_catalog(&db, &data_paths).await?;
    println!("Rebuild complete: {count} files registered.");

    db.close().await?;
    Ok(())
}

// ─── inspect ───────────────────────────────────────────────────────────────

async fn cmd_inspect(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let subcmd = args.get(2).map(|s| s.as_str()).unwrap_or("snapshot");

    match subcmd {
        "snapshot" | "--latest" => {
            let catalog_url = extract_catalog_arg(args, 3)?;
            let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
            let db = slatedb::Db::open(catalog_path, object_store).await?;

            let result = rocklake_catalog::inspect::inspect_snapshot(&db).await?;
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
        }
        "api-costs" => {
            let catalog_url = extract_catalog_arg(args, 3)?;
            let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
            let db = slatedb::Db::open(catalog_path, object_store).await?;
            let state = rocklake_catalog::inspect::inspect_snapshot(&db).await?;
            db.close().await?;

            let file_count = state.data_file_count;
            let snap = rocklake_catalog::cost::ApiCallSnapshot {
                put_count: file_count * 3,
                get_count: file_count * 10,
                list_count: file_count / 10 + 1,
                delete_count: 0,
                elapsed: std::time::Duration::from_secs(3600),
            };
            let report = rocklake_catalog::cost::ApiCostReport::from_snapshot(&snap);

            let stream = args.iter().any(|a| a == "--stream");
            if stream {
                println!("Streaming mode: one report per minute. Press Ctrl+C to stop.");
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                loop {
                    interval.tick().await;
                    report.print();
                }
            } else {
                report.print();
            }
        }
        "cache-utilization" => {
            let catalog_url = extract_catalog_arg(args, 3)?;
            let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
            let db = slatedb::Db::open(catalog_path, object_store).await?;
            let state = rocklake_catalog::inspect::inspect_snapshot(&db).await?;
            db.close().await?;

            let cache_size_mb = extract_numeric_arg(args, "--cache-size-mb").unwrap_or(256);
            let stats = rocklake_catalog::cache_utilization(
                cache_size_mb,
                state.data_file_count,
                state.column_count,
            )
            .await;
            stats.print();
        }
        _ => {
            eprintln!(
                "Usage: rocklake inspect [snapshot|api-costs|cache-utilization] --catalog <path>"
            );
            std::process::exit(1);
        }
    }

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
            let result = rocklake_catalog::verify::verify_catalog(&db).await?;
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
            let result = rocklake_catalog::cleanup::verify_data_files(&db, &object_store).await?;
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
            eprintln!("Usage: rocklake verify [catalog|data-files] --catalog <path>");
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

    let plan = rocklake_catalog::repair::repair_plan(&db).await?;

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
            let result = rocklake_catalog::repair::repair_apply(&db, &plan).await?;
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

// ─── warmup ────────────────────────────────────────────────────────────────

async fn cmd_warmup(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let catalog_url = extract_catalog_arg(args, 2)?;
    let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
    let db = slatedb::Db::open(catalog_path, object_store).await?;

    let max_tables = extract_numeric_arg(args, "--tables").unwrap_or(20) as usize;
    let result = rocklake_catalog::warmup_cache(&db, max_tables).await?;

    println!("Cache Warmup Complete:");
    println!("  Entries warmed:   {}", result.entries_warmed);
    println!("  Snapshot loaded:  {}", result.snapshot_loaded);
    println!("  Warmup hit ratio: {:.2}", result.warmup_hit_ratio);

    if result.warmup_hit_ratio >= 0.5 {
        println!("  Status: OK — cache warm for first requests");
    } else {
        println!("  Status: COLD — first requests will pay S3 round-trip latency");
    }

    db.close().await?;
    Ok(())
}

// ─── migrate ───────────────────────────────────────────────────────────────

async fn cmd_migrate(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let catalog_url = extract_catalog_arg(args, 2)?;
    let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
    let db = slatedb::Db::open(catalog_path, object_store).await?;

    let target_version = extract_numeric_arg(args, "--target-version").unwrap_or(2) as u32;
    let apply = args.iter().any(|a| a == "--apply");
    let dry_run = args.iter().any(|a| a == "--dry-run") || !apply;

    if dry_run {
        let result = rocklake_catalog::migrate::migrate_dry_run(&db, target_version).await?;
        println!("Migration Dry Run:");
        println!("  Current version:    {}", result.current_version);
        println!("  Target version:     {}", result.target_version);
        println!("  Rows to migrate:    {}", result.rows_to_migrate);
        println!("  Estimated duration: ~{}s", result.estimated_seconds);
        println!();
        println!("{}", result.description);
        if result.rows_to_migrate > 0 {
            println!();
            println!("Run with --apply to execute the migration.");
        }
    } else {
        let backup_dir =
            extract_string_arg(args, "--backup-dir").unwrap_or_else(|| ".".to_string());
        let result =
            rocklake_catalog::migrate::migrate_apply(&db, target_version, &backup_dir).await?;
        println!("Migration Complete:");
        println!("  Rows migrated:  {}", result.rows_migrated);
        println!("  New version:    {}", result.new_version);
        println!("  Backup written: {}", result.backup_path);
    }

    db.close().await?;
    Ok(())
}

// ─── corpus ────────────────────────────────────────────────────────────────

async fn cmd_corpus(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let subcmd = args.get(2).map(|s| s.as_str()).unwrap_or("validate");

    match subcmd {
        "diff" => {
            let old_path = extract_string_arg(args, "--old")
                .ok_or("--old <file> is required for corpus diff")?;
            let new_path = extract_string_arg(args, "--new")
                .ok_or("--new <file> is required for corpus diff")?;

            let old_file = std::fs::File::open(&old_path)
                .map_err(|e| format!("Cannot open old corpus: {e}"))?;
            let new_file = std::fs::File::open(&new_path)
                .map_err(|e| format!("Cannot open new corpus: {e}"))?;

            let old_records = rocklake_catalog::parse_corpus(std::io::BufReader::new(old_file));
            let new_records = rocklake_catalog::parse_corpus(std::io::BufReader::new(new_file));
            let diffs = rocklake_catalog::corpus_diff(&old_records, &new_records);

            if diffs.is_empty() {
                println!("No differences found between corpus files.");
            } else {
                println!("Corpus Diff ({} changes):", diffs.len());
                for d in &diffs {
                    println!(
                        "  [{:8}] {} — {}",
                        d.change_type, d.statement_family, d.detail
                    );
                }
            }
        }
        "validate" => {
            let corpus_path = extract_string_arg(args, "--corpus")
                .ok_or("--corpus <file> is required for corpus validate")?;

            let path = std::path::Path::new(&corpus_path);
            let mut all_records = Vec::new();
            if path.is_dir() {
                let mut entries: Vec<_> = std::fs::read_dir(path)
                    .map_err(|e| format!("Cannot read corpus directory: {e}"))?
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().map(|x| x == "jsonl").unwrap_or(false))
                    .collect();
                entries.sort_by_key(|e| e.file_name());
                for entry in entries {
                    let file = std::fs::File::open(entry.path())
                        .map_err(|e| format!("Cannot open corpus file: {e}"))?;
                    let mut records = rocklake_catalog::parse_corpus(std::io::BufReader::new(file));
                    all_records.append(&mut records);
                }
            } else {
                let file =
                    std::fs::File::open(path).map_err(|e| format!("Cannot open corpus: {e}"))?;
                all_records = rocklake_catalog::parse_corpus(std::io::BufReader::new(file));
            }
            let result = rocklake_catalog::corpus_validate(&all_records);
            result.print();
        }
        _ => {
            eprintln!("Usage: rocklake corpus [diff|validate] [--old <file>] [--new <file>] [--corpus <file>]");
            std::process::exit(1);
        }
    }

    Ok(())
}

// ─── tune ──────────────────────────────────────────────────────────────────

async fn cmd_tune(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let catalog_url = extract_catalog_arg(args, 2)?;
    let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
    let db = slatedb::Db::open(catalog_path, object_store).await?;
    let state = rocklake_catalog::inspect::inspect_snapshot(&db).await?;
    db.close().await?;

    let target_cost = extract_numeric_arg(args, "--target-cost-usd-per-month")
        .map(|v| v as f64)
        .unwrap_or(50.0);

    // Build a cost report from catalog metadata
    let snap = rocklake_catalog::cost::ApiCallSnapshot {
        put_count: state.data_file_count * 3,
        get_count: state.data_file_count * 10,
        list_count: state.data_file_count / 10 + 1,
        delete_count: 0,
        elapsed: std::time::Duration::from_secs(3600),
    };
    let report = rocklake_catalog::cost::ApiCostReport::from_snapshot(&snap);

    println!("RockLake Tuning Recommendations");
    println!("=================================");
    println!("Target monthly cost: ${target_cost:.2}");
    println!();

    let recs = rocklake_catalog::tune_for_cost_target(target_cost, &report);
    for r in &recs {
        println!("{r}");
    }

    println!();
    println!("Cost Mode Profiles:");
    for mode in [
        rocklake_catalog::CostMode::Conservative,
        rocklake_catalog::CostMode::Balanced,
        rocklake_catalog::CostMode::Latency,
    ] {
        let name = match mode {
            rocklake_catalog::CostMode::Conservative => "conservative",
            rocklake_catalog::CostMode::Balanced => "balanced",
            rocklake_catalog::CostMode::Latency => "latency",
        };
        println!("  --cost-mode={name}");
        println!("    {}", mode.profile_description());
    }

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

/// Options for S3-compatible object store configuration.
#[derive(Default)]
struct S3Options {
    endpoint: Option<String>,
    path_style: bool,
}

fn resolve_catalog(url: &str) -> Result<(ObjectPath, Arc<dyn object_store::ObjectStore>), String> {
    resolve_catalog_with_opts(url, &S3Options::default())
}

fn resolve_catalog_with_opts(
    url: &str,
    s3_opts: &S3Options,
) -> Result<(ObjectPath, Arc<dyn object_store::ObjectStore>), String> {
    if let Some(without_scheme) = url.strip_prefix("s3://") {
        let (bucket, prefix) = match without_scheme.find('/') {
            Some(idx) => (&without_scheme[..idx], &without_scheme[idx + 1..]),
            None => (without_scheme, ""),
        };

        let mut builder = object_store::aws::AmazonS3Builder::from_env().with_bucket_name(bucket);
        if let Some(ref endpoint) = s3_opts.endpoint {
            builder = builder.with_endpoint(endpoint);
        }
        if s3_opts.path_style {
            builder = builder.with_virtual_hosted_style_request(false);
        }
        let store = builder
            .build()
            .map_err(|e| format!("Failed to create S3 object store: {e}"))?;

        let obj_path = ObjectPath::from(prefix);
        Ok((obj_path, Arc::new(store)))
    } else if let Some(without_scheme) = url.strip_prefix("gs://") {
        let (bucket, prefix) = match without_scheme.find('/') {
            Some(idx) => (&without_scheme[..idx], &without_scheme[idx + 1..]),
            None => (without_scheme, ""),
        };

        let store = object_store::gcp::GoogleCloudStorageBuilder::from_env()
            .with_bucket_name(bucket)
            .build()
            .map_err(|e| format!("Failed to create GCS object store: {e}"))?;

        let obj_path = ObjectPath::from(prefix);
        Ok((obj_path, Arc::new(store)))
    } else if let Some(without_scheme) = url
        .strip_prefix("az://")
        .or_else(|| url.strip_prefix("azure://"))
        .or_else(|| url.strip_prefix("abfss://"))
    {
        let (container, prefix) = match without_scheme.find('/') {
            Some(idx) => (&without_scheme[..idx], &without_scheme[idx + 1..]),
            None => (without_scheme, ""),
        };

        let store = object_store::azure::MicrosoftAzureBuilder::from_env()
            .with_container_name(container)
            .build()
            .map_err(|e| format!("Failed to create Azure object store: {e}"))?;

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

// ─── migrate-from-ducklake ─────────────────────────────────────────────────

/// Import an existing DuckLake catalog into RockLake.
///
/// The source can be:
///   - A SQLite DuckLake catalog:  `--source sqlite:/path/to/catalog.db`
///   - An NDJSON dump (legacy):    `--source /path/to/dump.ndjson`
///
/// Use `--accept-version V1_1_DEV_1` to allow migration from a DuckLake v1.1
/// pre-release catalog (catalog_version 8).  By default only v1.0 (version 7)
/// is accepted.
///
/// Use `--dry-run` to inspect the migration plan without writing anything.
///
/// Example:
///   rocklake migrate-from-ducklake --source sqlite:./duck.db --catalog ./my-catalog
///   rocklake migrate-from-ducklake --source dump.ndjson --catalog ./my-catalog
async fn cmd_migrate_from_ducklake(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let source = extract_string_arg(args, "--source")
        .ok_or("--source <file|sqlite:path> is required for migrate-from-ducklake")?;
    let catalog_url = extract_string_arg(args, "--catalog")
        .ok_or("--catalog <path> is required for migrate-from-ducklake")?;
    let dry_run = args.iter().any(|a| a == "--dry-run");

    // Collect all --accept-version tokens.
    let mut accept_versions: Vec<String> = Vec::new();
    {
        let mut i = 0;
        while i < args.len() {
            if args[i] == "--accept-version" {
                if let Some(v) = args.get(i + 1) {
                    accept_versions.push(v.clone());
                }
            }
            i += 1;
        }
    }
    let accept_refs: Vec<&str> = accept_versions.iter().map(|s| s.as_str()).collect();

    println!("migrate-from-ducklake: source={source}, catalog={catalog_url}, dry_run={dry_run}");

    // Open the destination RockLake catalog.
    let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
    let db = slatedb::Db::open(catalog_path, object_store).await?;

    if let Some(sqlite_path) = source.strip_prefix("sqlite:") {
        // ── SQLite DuckLake source ──────────────────────────────────────────
        let mut src =
            rocklake_catalog::migrate_from_ducklake::SqliteDuckLakeSource::open(sqlite_path, None)?;
        let report = rocklake_catalog::migrate_from_ducklake::migrate_from_source(
            &mut src,
            &db,
            &accept_refs,
            dry_run,
        )
        .await?;

        println!(
            "Migration {}:",
            if dry_run { "dry-run" } else { "complete" }
        );
        println!(
            "  Source catalog version: {}",
            report.source_catalog_version
        );
        println!("  Data files:      {}", report.data_file_count);
        println!("  Total migrated:  {}", report.total_migrated());
        println!("  Total skipped:   {}", report.total_skipped());
        if !dry_run {
            println!("  Catalog written to: {catalog_url}");
        }
    } else {
        // ── NDJSON dump source (legacy) ─────────────────────────────────────
        let file =
            std::fs::File::open(&source).map_err(|e| format!("Cannot open source file: {e}"))?;
        let reader = std::io::BufReader::new(file);

        let result = rocklake_catalog::export::import_catalog(&db, reader).await?;

        println!("Migration complete (NDJSON source):");
        println!("  Rows imported:   {}", result.rows_imported);
        println!("  Tables imported: {}", result.tables_imported);
        println!("  Catalog written to: {catalog_url}");
    }

    db.close().await?;
    Ok(())
}

// ─── export-catalog ────────────────────────────────────────────────────────

/// Export all DuckLake catalog tables (28 spec + 4 extension) to a JSON-lines file.
///
/// This produces an interop dump suitable for migration or debugging.
/// Sensitive fields (encryption keys, secrets) are redacted in the output.
///
/// Example:
///   rocklake export-catalog --catalog ./my-catalog --out catalog-dump.ndjson
///   rocklake export-catalog --catalog ./my-catalog --out snap1.ndjson --at-snapshot 1
async fn cmd_export_catalog(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let catalog_url = extract_string_arg(args, "--catalog")
        .ok_or("--catalog <path> is required for export-catalog")?;
    let output_path =
        extract_string_arg(args, "--out").unwrap_or_else(|| "catalog-export.ndjson".to_string());
    // Accept both --at-snapshot (preferred) and --snapshot-id (legacy)
    let snapshot_id = extract_numeric_arg(args, "--at-snapshot")
        .or_else(|| extract_numeric_arg(args, "--snapshot-id"));

    let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
    let db = slatedb::Db::open(catalog_path, object_store).await?;

    let mut file = std::fs::File::create(&output_path)
        .map_err(|e| format!("Cannot create output file {output_path}: {e}"))?;

    let result = rocklake_catalog::export::export_catalog(&db, snapshot_id, &mut file).await?;

    println!("Export complete (28 DuckLake spec + 4 extension catalog tables):");
    println!("  Rows exported:   {}", result.rows_exported);
    println!("  Tables exported: {}", result.tables_exported);
    println!("  Output:          {output_path}");

    db.close().await?;
    Ok(())
}

// ─── diagnose (v0.39.0) ────────────────────────────────────────────────────

/// Run a structured health diagnostic against a catalog.
///
/// Example:
///   rocklake diagnose --catalog ./my-catalog
///   rocklake diagnose --catalog s3://bucket/catalog/ --json
///   rocklake diagnose --catalog ./my-catalog --data-root ./data/
async fn cmd_diagnose(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let catalog_url =
        extract_string_arg(args, "--catalog").ok_or("--catalog <path> is required for diagnose")?;
    let json_output = args.iter().any(|a| a == "--json");
    let data_root = extract_string_arg(args, "--data-root");

    let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
    let db = slatedb::Db::open(catalog_path, object_store.clone()).await?;

    let store_and_root = data_root.map(|root| (object_store, root));

    let report = rocklake_catalog::diagnose_catalog(&db, store_and_root).await?;
    db.close().await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", rocklake_catalog::format_report_text(&report));
    }

    // Exit non-zero if P0 findings are present (suitable for CI gates).
    if !report.is_ok() {
        std::process::exit(1);
    }

    Ok(())
}

// ─── sweep-orphans (v0.39.0) ───────────────────────────────────────────────

/// Identify (and optionally delete) orphan Parquet files in object storage.
///
/// Example:
///   rocklake sweep-orphans --catalog ./my-catalog --data-root ./data/
///   rocklake sweep-orphans --catalog ./my-catalog --data-root s3://bucket/data/ --grace-period-hours 48
///   rocklake sweep-orphans --catalog ./my-catalog --data-root ./data/ --apply
async fn cmd_sweep_orphans(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let catalog_url = extract_string_arg(args, "--catalog")
        .ok_or("--catalog <path> is required for sweep-orphans")?;
    let data_root = extract_string_arg(args, "--data-root")
        .ok_or("--data-root <prefix> is required for sweep-orphans")?;
    let grace_period_hours = extract_numeric_arg(args, "--grace-period-hours").unwrap_or(24);
    let apply = args.iter().any(|a| a == "--apply");

    let (catalog_path, object_store) = resolve_catalog(&catalog_url)?;
    let db = slatedb::Db::open(catalog_path, object_store.clone()).await?;

    let config = rocklake_catalog::SweepOrphansConfig {
        grace_period_hours,
        apply,
        data_root: data_root.clone(),
    };

    let result = rocklake_catalog::sweep_orphans(&db, object_store, &config).await?;
    db.close().await?;

    if apply {
        println!("Sweep complete (--apply mode):");
    } else {
        println!("Sweep complete (dry-run — use --apply to delete):");
    }
    println!("  Data root:          {data_root}");
    println!("  Files scanned:      {}", result.total_scanned);
    println!("  Orphan files found: {}", result.orphan_files.len());
    println!("  Files deleted:      {}", result.deleted);
    println!("  Grace period:       {grace_period_hours}h");

    if !result.orphan_files.is_empty() {
        println!("\nOrphan files:");
        for f in &result.orphan_files {
            println!("  {f}");
        }
    }

    Ok(())
}
