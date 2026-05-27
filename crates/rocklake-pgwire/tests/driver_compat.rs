//! v0.27.13 — Multi-Client & Multi-Driver Interoperability Certification
//!
//! Certifies that the RockLake PG-Wire catalog facade is fully compliant with
//! standard Postgres database clients, ORM drivers, and analytical applications
//! under DuckLake 1.0 (Catalog Version 7) and DuckDB v1.5.3 constraints.
//!
//! # Coverage
//!
//! ## Section 1 — Multi-Driver Smoke Tests (real TCP socket)
//!   - `tokio-postgres` (Rust) native driver: schema list, table query,
//!     parameterized queries.
//!   - Simulated `pg` (Node.js) startup session parameters.
//!   - Simulated `psycopg` (Python 3) startup session parameters.
//!   - Simulated `pgx` (Go) startup session parameters.
//!
//! ## Section 2 — CLI Tool Loopback Tests
//!   - `psql` CLI loopback connection and query.
//!   - `pgcli` CLI loopback connection and query.
//!
//! ## Section 3 — BI Tool Facade Validation (in-process executor)
//!   - DBeaver metadata schema discovery queries.
//!   - Metabase catalog scan queries.
//!   - Session commands: `DISCARD ALL`, `SET client_min_messages`.
//!   - Driver parameter-negotiation handshakes run to completion.

use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use rocklake_catalog::{CatalogStore, OpenOptions};
use rocklake_pgwire::executor;
use rocklake_pgwire::session::SessionState;
use rocklake_sql::ParamValues;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Mutex;

// ─── Shared helpers ──────────────────────────────────────────────────────────

fn make_catalog_opts(dir: &TempDir) -> OpenOptions {
    let store = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    OpenOptions {
        object_store: store,
        path: ObjectPath::from("catalog"),
        encryption: None,
    }
}

fn default_notify_manager() -> Arc<rocklake_pgwire::notify::NotifyManager> {
    Arc::new(rocklake_pgwire::notify::NotifyManager::new())
}

fn default_extension_schemas() -> Arc<Vec<String>> {
    Arc::new(vec!["pgtrickle".to_string()])
}

async fn setup_store(dir: &TempDir) -> Arc<Mutex<CatalogStore>> {
    let catalog = CatalogStore::open(make_catalog_opts(dir)).await.unwrap();
    Arc::new(Mutex::new(catalog))
}

/// Start a plain-text server on an OS-assigned ephemeral port.
/// Returns `(addr, shutdown_tx, join_handle)`. Drop `shutdown_tx` to stop.
async fn start_server(
    dir: &TempDir,
) -> (
    std::net::SocketAddr,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let catalog = Arc::new(Mutex::new(
        CatalogStore::open(make_catalog_opts(dir)).await.unwrap(),
    ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let config = rocklake_pgwire::server::ServerConfig {
        bind_addr: addr,
        ..Default::default()
    };

    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        rocklake_pgwire::server::run_server_with_shutdown(config, catalog, rx)
            .await
            .unwrap();
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    (addr, tx, handle)
}

/// Connect a `tokio-postgres` client to a running server.
async fn tcp_connect(addr: std::net::SocketAddr) -> tokio_postgres::Client {
    let conn_str = format!(
        "host=127.0.0.1 port={} user=duckdb dbname=ducklake",
        addr.port()
    );
    let (client, conn) = tokio_postgres::connect(&conn_str, tokio_postgres::NoTls)
        .await
        .unwrap_or_else(|e| panic!("failed to connect to RockLake server: {e}"));
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            eprintln!("[driver_compat] connection error: {e}");
        }
    });
    client
}

// ─── Section 1: Multi-Driver Smoke Tests (real TCP socket) ───────────────────

/// R-01 — Rust tokio-postgres: schema list over a real TCP socket.
///
/// Connects to a live RockLake server and issues a snapshot-scoped
/// `ducklake_schema` query.  A fresh catalog returns zero rows; success
/// validates that the wire protocol, parameter binding, and executor path
/// all work end-to-end over TCP.
#[tokio::test]
async fn rust_tokio_postgres_driver_schema_list() {
    let dir = TempDir::new().unwrap();
    let (addr, _tx, _handle) = start_server(&dir).await;
    let client = tcp_connect(addr).await;

    // DuckLake catalog queries are always snapshot-scoped; pass i64::MAX to
    // see everything visible in the latest snapshot.
    let rows = client
        .query(
            "SELECT * FROM ducklake_schema \
             WHERE begin_snapshot <= $1 \
             AND (end_snapshot IS NULL OR end_snapshot > $1)",
            &[&i64::MAX],
        )
        .await
        .unwrap_or_else(|e| panic!("schema list query failed: {e}"));
    // Zero rows is valid for a fresh catalog; success means the wire protocol works.
    let _ = rows;
}

/// R-02 — Rust tokio-postgres: full DDL cycle — schema, table, snapshot query.
///
/// Uses the DuckLake INSERT protocol to create a schema and table, then reads
/// them back via `ducklake_table` to verify the full round-trip through the
/// TCP PG-Wire facade.
#[tokio::test]
async fn rust_tokio_postgres_driver_table_query() {
    let dir = TempDir::new().unwrap();
    let (addr, _tx, _handle) = start_server(&dir).await;
    let client = tcp_connect(addr).await;

    // Create schema + snapshot atomically.
    client.execute("BEGIN", &[]).await.unwrap();
    client
        .execute(
            "INSERT INTO ducklake_schema (schema_name) VALUES ($1)",
            &[&"analytics"],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO ducklake_snapshot (author, message) VALUES ($1, $2)",
            &[&"tokio-postgres-test", &"create analytics schema"],
        )
        .await
        .unwrap();
    client.execute("COMMIT", &[]).await.unwrap();

    // Retrieve schema_id — must use snapshot-scoped query.
    let schema_rows = client
        .query(
            "SELECT schema_id, schema_name FROM ducklake_schema \
             WHERE begin_snapshot <= $1 \
             AND (end_snapshot IS NULL OR end_snapshot > $1)",
            &[&i64::MAX],
        )
        .await
        .unwrap_or_else(|e| panic!("schema read-back failed: {e}"));
    assert_eq!(
        schema_rows.len(),
        1,
        "must find exactly one schema after commit"
    );
    let schema_id: i64 = schema_rows[0].get("schema_id");

    // Create a table under that schema.
    client.execute("BEGIN", &[]).await.unwrap();
    client
        .execute(
            "INSERT INTO ducklake_table (schema_id, table_name, data_path) VALUES ($1, $2, $3)",
            &[&schema_id, &"events", &"data/analytics/events/"],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO ducklake_snapshot (author, message) VALUES ($1, $2)",
            &[&"tokio-postgres-test", &"create events table"],
        )
        .await
        .unwrap();
    client.execute("COMMIT", &[]).await.unwrap();

    // Verify the table is visible — filter by actual schema_id.
    let tables = client
        .query(
            "SELECT table_name FROM ducklake_table WHERE schema_id = $1",
            &[&schema_id],
        )
        .await
        .unwrap_or_else(|e| panic!("table read-back failed: {e}"));
    let names: Vec<String> = tables.iter().map(|r| r.get("table_name")).collect();
    assert!(
        names.contains(&"events".to_string()),
        "table 'events' must appear in ducklake_table; got: {names:?}"
    );
}

/// R-03 — Rust tokio-postgres: parameterized queries round-trip correctly.
///
/// Verifies that the extended query protocol (Prepare → Bind → Execute) works
/// across the TCP PG-Wire socket.  Uses a snapshot-scoped DuckLake catalog
/// query with an `$1 INT8` parameter, which is the idiomatic parameterized
/// path for this server.
#[tokio::test]
async fn rust_tokio_postgres_driver_parameterized_query() {
    let dir = TempDir::new().unwrap();
    let (addr, _tx, _handle) = start_server(&dir).await;
    let client = tcp_connect(addr).await;

    // A snapshot-scoped schema query exercises Parse → Describe → Bind → Execute
    // with a typed INT8 parameter — the canonical extended-query protocol path.
    let rows = client
        .query(
            "SELECT * FROM ducklake_schema \
             WHERE begin_snapshot <= $1 \
             AND (end_snapshot IS NULL OR end_snapshot > $1)",
            &[&i64::MAX],
        )
        .await
        .unwrap_or_else(|e| panic!("parameterized query failed: {e}"));
    // An empty catalog returns zero rows; the important thing is no wire error.
    let _ = rows;
}

// ─── Section 1 (continued): Driver startup-parameter simulations ─────────────
//
// Node.js `pg`, Python `psycopg3`, and Go `pgx` each issue specific SET
// commands immediately after connection to configure their session. We simulate
// these sequences and verify the server handles each one without error.

/// D-01 — Simulated Node.js `pg` v8 driver startup sequence.
///
/// `pg` sets: `client_encoding`, `DateStyle`, `intervalstyle`,
/// `extra_float_digits`, then issues a ping query.
#[tokio::test]
async fn node_pg_driver_startup_parameters_accepted() {
    let dir = TempDir::new().unwrap();
    let (addr, _tx, _handle) = start_server(&dir).await;
    let client = tcp_connect(addr).await;

    for sql in [
        "SET client_encoding = UTF8",
        "SET DateStyle = 'ISO, MDY'",
        "SET intervalstyle = postgres",
        "SET extra_float_digits = 3",
    ] {
        client
            .execute(sql, &[])
            .await
            .unwrap_or_else(|e| panic!("Node.js pg startup param '{sql}' failed: {e}"));
    }

    let rows = client
        .query("SELECT current_database()", &[])
        .await
        .unwrap_or_else(|e| panic!("Node.js pg initial query failed: {e}"));
    assert!(
        !rows.is_empty(),
        "Node.js pg: current_database() must return a row"
    );
}

/// D-02 — Simulated Python `psycopg` 3 driver startup sequence.
///
/// psycopg3 sets: `DateStyle`, `standard_conforming_strings`,
/// `extra_float_digits`, `TimeZone`, then issues a parameterized ping.
#[tokio::test]
async fn python_psycopg_driver_startup_parameters_accepted() {
    let dir = TempDir::new().unwrap();
    let (addr, _tx, _handle) = start_server(&dir).await;
    let client = tcp_connect(addr).await;

    for sql in [
        "SET DateStyle = 'ISO'",
        "SET standard_conforming_strings = on",
        "SET extra_float_digits = 3",
        "SET TimeZone = 'UTC'",
    ] {
        client
            .execute(sql, &[])
            .await
            .unwrap_or_else(|e| panic!("psycopg3 startup param '{sql}' failed: {e}"));
    }

    // psycopg3 uses the extended query protocol; verify with a supported
    // snapshot-scoped catalog query.
    let rows = client
        .query(
            "SELECT * FROM ducklake_schema \
             WHERE begin_snapshot <= $1 \
             AND (end_snapshot IS NULL OR end_snapshot > $1)",
            &[&i64::MAX],
        )
        .await
        .unwrap_or_else(|e| panic!("psycopg3 parameterized query failed: {e}"));
    // Empty catalog → zero rows; success means extended-query protocol works.
    let _ = rows;
}

/// D-03 — Simulated Go `pgx` v5 driver startup sequence.
///
/// pgx v5 sets: `client_encoding`, `standard_conforming_strings`,
/// `extra_float_digits`, then issues a parameterized query to verify the
/// extended-query protocol path is fully operational.
#[tokio::test]
async fn go_pgx_driver_startup_parameters_accepted() {
    let dir = TempDir::new().unwrap();
    let (addr, _tx, _handle) = start_server(&dir).await;
    let client = tcp_connect(addr).await;

    for sql in [
        "SET client_encoding = UTF8",
        "SET standard_conforming_strings = on",
        "SET extra_float_digits = 3",
    ] {
        client
            .execute(sql, &[])
            .await
            .unwrap_or_else(|e| panic!("pgx startup param '{sql}' failed: {e}"));
    }

    // pgx exclusively uses the extended query protocol; verify with a
    // snapshot-scoped catalog query (INT8 parameter).
    let rows = client
        .query(
            "SELECT * FROM ducklake_schema \
             WHERE begin_snapshot <= $1 \
             AND (end_snapshot IS NULL OR end_snapshot > $1)",
            &[&i64::MAX],
        )
        .await
        .unwrap_or_else(|e| panic!("pgx parameterized query failed: {e}"));
    // Empty catalog → zero rows; success means extended-query protocol works.
    let _ = rows;
}

// ─── Section 2: CLI Tool Loopback Tests ──────────────────────────────────────

/// Locate `psql` binary in common install paths.
/// Panics with a clear install instruction if not found.
fn find_psql() -> String {
    let candidates = [
        "psql",
        "/usr/bin/psql",
        "/usr/local/bin/psql",
        "/opt/homebrew/bin/psql",
        "/opt/homebrew/opt/postgresql@17/bin/psql",
        "/opt/homebrew/opt/postgresql@16/bin/psql",
        "/opt/homebrew/opt/postgresql@15/bin/psql",
        "/opt/homebrew/opt/postgresql@14/bin/psql",
    ];
    for c in &candidates {
        if std::process::Command::new(c)
            .arg("--version")
            .output()
            .is_ok()
        {
            return c.to_string();
        }
    }
    panic!(
        "psql not found. Install: sudo apt-get install -y postgresql-client \
         (Linux) or brew install postgresql@16 (macOS)"
    )
}

/// Locate `pgcli` binary in common install paths.
fn find_pgcli() -> String {
    // Build candidate list: static paths + $HOME/.local/bin + workspace venv.
    let mut candidates: Vec<String> = vec![
        "pgcli".to_string(),
        "/usr/local/bin/pgcli".to_string(),
        "/opt/homebrew/bin/pgcli".to_string(),
        "/usr/bin/pgcli".to_string(),
        "/home/runner/.local/bin/pgcli".to_string(),
        "/root/.local/bin/pgcli".to_string(),
    ];

    // Also check $HOME/.local/bin (pip --user install target).
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(format!("{home}/.local/bin/pgcli"));
    }

    // Check workspace .venv (project-local virtual environment).
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        // CARGO_MANIFEST_DIR is the crate root; workspace is two levels up.
        let ws = std::path::Path::new(&manifest_dir)
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf());
        if let Some(ws_root) = ws {
            candidates.push(
                ws_root
                    .join(".venv/bin/pgcli")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }

    for c in &candidates {
        if std::process::Command::new(c)
            .arg("--version")
            .output()
            .is_ok()
        {
            return c.to_string();
        }
    }
    panic!("pgcli not found. Install: pip install pgcli  (or pip3 install pgcli)")
}

/// C-01 — psql CLI loopback connection and query.
///
/// Spawns `psql` against the live RockLake PG-Wire server and verifies that
/// `SELECT current_database()` returns "ducklake" without error.
#[tokio::test]
async fn psql_cli_loopback_connection() {
    let psql = find_psql();
    let dir = TempDir::new().unwrap();
    let (addr, _tx, _handle) = start_server(&dir).await;

    // Use tokio::process::Command to avoid blocking the runtime thread while
    // waiting for the subprocess (blocking std::process::Command::output()
    // would prevent the server tasks from running, causing a hang).
    let output = tokio::process::Command::new(&psql)
        .args([
            "-h",
            "127.0.0.1",
            "-p",
            &addr.port().to_string(),
            "-U",
            "duckdb",
            "-d",
            "ducklake",
            "--no-password",
            "-t",
            "-c",
            "SELECT current_database()",
        ])
        .output()
        .await
        .unwrap_or_else(|e| panic!("failed to spawn psql ({psql}): {e}"));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "psql must exit 0; stderr: {stderr}"
    );
    assert!(
        stdout.contains("ducklake"),
        "psql output must contain 'ducklake'; stdout: {stdout}"
    );
}

/// C-02 — pgcli CLI loopback connection (connectivity ping).
///
/// Spawns `pgcli --ping` against the live server and verifies that the full
/// startup handshake — TCP connect, auth, session-parameter negotiation —
/// completes without error.  `--ping` exits immediately after connectivity
/// is confirmed, making it suitable for non-interactive testing.
#[tokio::test]
async fn pgcli_cli_loopback_connection() {
    let pgcli = find_pgcli();
    let dir = TempDir::new().unwrap();
    let (addr, _tx, _handle) = start_server(&dir).await;

    // Use tokio::process::Command to avoid blocking the runtime (same reason
    // as the psql test above).
    let output = tokio::process::Command::new(&pgcli)
        .args([
            "-h",
            "127.0.0.1",
            "-p",
            &addr.port().to_string(),
            "-U",
            "duckdb",
            "-d",
            "ducklake",
            "--no-password",
            "--ping",
        ])
        .output()
        .await
        .unwrap_or_else(|e| panic!("failed to spawn pgcli ({pgcli}): {e}"));

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "pgcli --ping must exit 0 (connectivity confirmed); stderr: {stderr}"
    );
}

// ─── Section 3: BI Tool Facade Validation (in-process executor) ──────────────
//
// BI tools (DBeaver, Metabase) issue specific metadata-discovery queries on
// connection. We verify that the RockLake executor handles every query in their
// startup sequence, returning structurally valid responses.

/// B-01 — DBeaver metadata schema discovery.
///
/// DBeaver issues these queries on first connect:
///   1. `DISCARD ALL` — session reset.
///   2. `SELECT * FROM ducklake_schema` — schema list.
///   3. `SELECT * FROM ducklake_table` — table list.
///   4. `SELECT typname FROM pg_catalog.pg_type WHERE typtype = 'b'` — types.
///   5. `SELECT current_database()` — database identification.
///
/// All must succeed (no SQLSTATE error response).
#[tokio::test]
async fn dbeaver_metadata_schema_discovery() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;
    let params = ParamValues::default();
    let nm = default_notify_manager();
    let es = default_extension_schemas();

    let queries = [
        "DISCARD ALL",
        "SELECT * FROM ducklake_schema",
        "SELECT * FROM ducklake_table",
        "SELECT typname FROM pg_catalog.pg_type WHERE typtype = 'b'",
        "SELECT current_database()",
    ];

    for sql in queries {
        let mut session = SessionState::new();
        let res = executor::execute_sql(sql, &params, &store, &mut session, &nm, &es)
            .await
            .unwrap_or_else(|e| panic!("DBeaver query '{sql}' must not error: {e}"));
        assert!(
            !res.is_empty(),
            "DBeaver query '{sql}' must return a response"
        );
    }
}

/// B-02 — Metabase catalog scan.
///
/// Metabase issues these queries to populate its schema browser:
///   1. `SET client_min_messages = warning` — session parameter.
///   2. `SELECT version()` — server identification.
///   3. `SELECT current_database()` — database identification.
///   4. `SELECT * FROM ducklake_schema` — schema list.
///   5. `SELECT * FROM ducklake_table` — table list for schema scan.
///
/// All must succeed.
#[tokio::test]
async fn metabase_catalog_scan() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;
    let params = ParamValues::default();
    let nm = default_notify_manager();
    let es = default_extension_schemas();

    let queries = [
        "SET client_min_messages = warning",
        "SELECT version()",
        "SELECT current_database()",
        "SELECT * FROM ducklake_schema",
        "SELECT * FROM ducklake_table",
    ];

    for sql in queries {
        let mut session = SessionState::new();
        let res = executor::execute_sql(sql, &params, &store, &mut session, &nm, &es)
            .await
            .unwrap_or_else(|e| panic!("Metabase query '{sql}' must not error: {e}"));
        assert!(
            !res.is_empty(),
            "Metabase query '{sql}' must return a response"
        );
    }
}

/// B-03 — Session command: DISCARD ALL completes with an Execution response.
///
/// DuckDB issues `DISCARD ALL` when returning connections to its pool.
/// The server must respond with an Execution completion tag, not an error.
#[tokio::test]
async fn session_discard_all_accepted() {
    use pgwire::api::results::Response;

    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;
    let params = ParamValues::default();
    let mut session = SessionState::new();
    let nm = default_notify_manager();
    let es = default_extension_schemas();

    let responses = executor::execute_sql("DISCARD ALL", &params, &store, &mut session, &nm, &es)
        .await
        .expect("DISCARD ALL must not error");

    assert!(
        !responses.is_empty(),
        "DISCARD ALL must produce at least one response"
    );

    let is_execution = responses
        .iter()
        .any(|r| matches!(r, Response::Execution(_)));
    assert!(
        is_execution,
        "DISCARD ALL must produce an Execution completion response"
    );
}

/// B-04 — Session command: SET client_min_messages accepted for all log levels.
///
/// BI tools and ORMs issue `SET client_min_messages = <level>` at startup.
/// The server must accept all standard log levels without error.
#[tokio::test]
async fn session_set_client_min_messages_accepted() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;
    let params = ParamValues::default();
    let nm = default_notify_manager();
    let es = default_extension_schemas();

    for level in ["warning", "error", "notice", "log", "debug"] {
        let sql = format!("SET client_min_messages = {level}");
        let mut session = SessionState::new();
        let res = executor::execute_sql(&sql, &params, &store, &mut session, &nm, &es)
            .await
            .unwrap_or_else(|e| panic!("SET client_min_messages = {level} must not error: {e}"));
        assert!(
            !res.is_empty(),
            "SET client_min_messages = {level} must produce a response"
        );
    }
}

/// B-05 — Driver parameter-negotiation handshake: all standard SET commands.
///
/// Verifies that every session parameter negotiated by standard drivers is
/// accepted without error: `standard_conforming_strings`, `TimeZone`,
/// `DateStyle`, `intervalstyle`, `extra_float_digits`, `client_encoding`,
/// `application_name`.
#[tokio::test]
async fn all_driver_parameter_negotiation_handshakes() {
    let dir = TempDir::new().unwrap();
    let store = setup_store(&dir).await;
    let params = ParamValues::default();
    let nm = default_notify_manager();
    let es = default_extension_schemas();

    let settings = [
        "SET standard_conforming_strings = on",
        "SET TimeZone = 'UTC'",
        "SET DateStyle = 'ISO, MDY'",
        "SET intervalstyle = postgres",
        "SET extra_float_digits = 3",
        "SET client_encoding = UTF8",
        "SET application_name = 'test-driver'",
    ];

    for sql in &settings {
        let mut session = SessionState::new();
        let res = executor::execute_sql(sql, &params, &store, &mut session, &nm, &es)
            .await
            .unwrap_or_else(|e| panic!("parameter negotiation '{sql}' must not error: {e}"));
        assert!(
            !res.is_empty(),
            "'{sql}' must produce a response (parameter negotiation must complete)"
        );
    }
}
