# Configuration

Rocklake is configured through a combination of command-line flags, environment variables, and (optionally) a TOML configuration file. The design follows the twelve-factor app methodology: configuration that varies between environments lives in the environment, not in code. Sensible defaults mean that most deployments need only a storage path and a bind address to get started.

This page documents every available configuration option, explains the precedence rules, and provides guidance on which options matter for which deployment scenarios.

## Configuration Precedence

When the same option is specified in multiple places, higher-precedence sources override lower ones:

1. **Command-line flags** — highest priority, ideal for one-off overrides and debugging
2. **Environment variables** — standard for container deployments and CI/CD
3. **Configuration file** (`rocklake.toml`) — structured, version-controllable settings
4. **Compiled defaults** — lowest priority, documented below

This means you can set baseline configuration in a TOML file, override per-environment values with environment variables, and further override for testing with command-line flags. There are no surprises — the most specific source always wins.

## Command-Line Flags

The full set of command-line flags:

```bash
rocklake serve [FLAGS] [OPTIONS]
```

### Required

| Flag | Description |
|------|-------------|
| `--catalog <path>` | Object storage path for the catalog. See [path formats](#object-storage-path-format) below. |

### Server Options

| Flag | Default | Description |
|------|---------|-------------|
| `--bind ADDR:PORT` | `127.0.0.1:5432` | Network address and port to listen on. Use `0.0.0.0:5432` to listen on all interfaces. |
| `--max-sessions N` | `64` | Maximum number of concurrent client sessions. Each session consumes ~1 MB. |
| `--read-only` | `false` | Disable all write operations. The server refuses DDL/DML and acts as a read replica. |
| `--log-level LEVEL` | `info` | Logging verbosity: `error`, `warn`, `info`, `debug`, `trace`. |
| `--log-format FORMAT` | `text` | Log output format: `text` (human-friendly) or `json` (machine-parseable). |

### TLS Options

| Flag | Default | Description |
|------|---------|-------------|
| `--tls-cert PATH` | (none) | Path to PEM-encoded TLS certificate file. If specified, `--tls-key` must also be provided. |
| `--tls-key PATH` | (none) | Path to PEM-encoded TLS private key file. |
| `--tls-ca PATH` | (none) | Path to PEM-encoded CA certificate for mutual TLS (client certificate verification). |

When TLS is configured, the server requires all connections to use TLS. There is no mixed-mode listener — either all connections are encrypted or none are. If you need both, run two Rocklake instances on different ports.

### Authentication Options

| Flag | Default | Description |
|------|---------|-------------|
| `--auth-user NAME` | (none) | If set, require this username during PostgreSQL authentication. |
| `--auth-password SECRET` | (none) | If set, require this password. Prefer `ROCKLAKE_PASSWORD` env var to avoid shell history exposure. |

### Performance Tuning

| Flag | Default | Description |
|------|---------|-------------|
| `--hot-key-cache BOOL` | `true` | Enable caching of frequently-read keys in memory. Reduces object storage reads for catalog metadata. |
| `--batch-size N` | `1000` | Maximum number of key-value pairs in a single write batch. Larger batches reduce round trips but increase commit latency. |
| `--prefetch-depth N` | `4` | Number of SST data blocks to prefetch during sequential scans. Higher values improve scan throughput at the cost of memory. |
| `--compaction-interval SECS` | `300` | Seconds between background compaction checks. Set to `0` to disable automatic compaction (manual only). |

## Environment Variables

Environment variables provide the same configuration surface as command-line flags, plus additional provider-specific settings.

### Core Rocklake Variables

| Variable | Equivalent Flag | Description |
|----------|----------------|-------------|
| `ROCKLAKE_CATALOG` | `--catalog` | Object storage path |
| `ROCKLAKE_BIND` | `--bind` | Listen address and port |
| `ROCKLAKE_MAX_SESSIONS` | `--max-sessions` | Concurrent session limit |
| `ROCKLAKE_READ_ONLY` | `--read-only` | Read-only mode (`true`/`false`) |
| `ROCKLAKE_TLS_CERT` | `--tls-cert` | TLS certificate path |
| `ROCKLAKE_TLS_KEY` | `--tls-key` | TLS private key path |
| `ROCKLAKE_TLS_CA` | `--tls-ca` | Mutual TLS CA certificate |
| `ROCKLAKE_AUTH_USER` | `--auth-user` | Required username |
| `ROCKLAKE_PASSWORD` | `--auth-password` | Required password (preferred over flag) |
| `ROCKLAKE_LOG_LEVEL` | `--log-level` | Log verbosity |
| `ROCKLAKE_LOG_FORMAT` | `--log-format` | Log format |
| `ROCKLAKE_HOT_KEY_CACHE` | `--hot-key-cache` | Hot key cache toggle |
| `ROCKLAKE_BATCH_SIZE` | `--batch-size` | Write batch size |
| `ROCKLAKE_PREFETCH_DEPTH` | `--prefetch-depth` | Scan prefetch depth |
| `ROCKLAKE_COMPACTION_INTERVAL` | `--compaction-interval` | Compaction check interval |

### AWS / S3 Variables

| Variable | Description |
|----------|-------------|
| `AWS_REGION` | AWS region for S3 access (e.g., `us-east-1`) |
| `AWS_ACCESS_KEY_ID` | Static access key (prefer IAM roles in production) |
| `AWS_SECRET_ACCESS_KEY` | Static secret key |
| `AWS_SESSION_TOKEN` | Temporary session token (for assumed roles) |
| `AWS_ENDPOINT_URL` | Custom S3-compatible endpoint URL (MinIO, R2, Tigris, LocalStack) |
| `AWS_S3_EXPRESS` | Set to `true` to enable S3 Express One Zone optimizations |
| `AWS_PROFILE` | Named profile from `~/.aws/config` |

### Google Cloud Storage Variables

| Variable | Description |
|----------|-------------|
| `GOOGLE_APPLICATION_CREDENTIALS` | Path to service account JSON key file |
| `GOOGLE_CLOUD_PROJECT` | GCP project ID (for billing/quota) |

### Azure Blob Storage Variables

| Variable | Description |
|----------|-------------|
| `AZURE_STORAGE_ACCOUNT` | Storage account name |
| `AZURE_STORAGE_KEY` | Storage account access key |
| `AZURE_TENANT_ID` | Azure AD tenant for service principal auth |
| `AZURE_CLIENT_ID` | Service principal client ID |
| `AZURE_CLIENT_SECRET` | Service principal client secret |
| `AZURE_STORAGE_CONNECTION_STRING` | Full connection string (alternative to individual variables) |

### Logging Variables

| Variable | Description |
|----------|-------------|
| `RUST_LOG` | Fine-grained log filter (e.g., `rocklake_catalog=debug,rocklake_pgwire=info`) |
| `RUST_LOG_STYLE` | Terminal color support: `auto`, `always`, `never` |

## Configuration File (TOML)

For complex deployments, you can use a TOML configuration file. By default, Rocklake looks for `rocklake.toml` in the current directory. Override with the `--config` flag or `ROCKLAKE_CONFIG` environment variable:

```bash
rocklake --config /etc/rocklake/rocklake.toml
```

Example configuration file:

```toml
# /etc/rocklake/rocklake.toml

[server]
catalog = "s3://my-lakehouse-bucket/catalog/"
bind = "0.0.0.0:5432"
max_sessions = 100
read_only = false

[tls]
cert = "/etc/rocklake/tls/cert.pem"
key = "/etc/rocklake/tls/key.pem"
# ca = "/etc/rocklake/tls/ca.pem"  # Uncomment for mutual TLS

[auth]
user = "ducklake"
# Password should come from ROCKLAKE_PASSWORD env var

[logging]
level = "info"
format = "json"

[performance]
hot_key_cache = true
batch_size = 1000
prefetch_depth = 4
compaction_interval = 300
```

The TOML file uses the same names as environment variables but with dots replaced by section headers. Boolean values use `true`/`false` (not quoted strings).

## Object Storage Path Format

The `--catalog` flag (or `ROCKLAKE_CATALOG` variable) accepts several path formats:

| Format | Example | Provider |
|--------|---------|----------|
| `s3://bucket/prefix/` | `s3://my-data/catalog/` | AWS S3, S3 Express One Zone |
| `s3://bucket/prefix/` | `s3://my-data/catalog/` | S3-compatible (MinIO, R2, Tigris) with `AWS_ENDPOINT_URL` |
| `gs://bucket/prefix/` | `gs://my-data/catalog/` | Google Cloud Storage |
| `az://container/prefix/` | `az://data/catalog/` | Azure Blob Storage |
| `./relative/path/` | `./my-catalog/` | Local filesystem (relative) |
| `/absolute/path/` | `/var/data/catalog/` | Local filesystem (absolute) |

The trailing slash is optional but recommended for clarity. The specified path becomes the root of the SlateDB instance — all WAL segments, sorted string tables (SSTs), manifests, and compacted files live under this prefix.

### Path Layout Within Storage

Once Rocklake starts writing to a storage path, the internal layout is:

```
s3://my-bucket/catalog/
├── manifest/           # SlateDB manifest files
├── wal/               # Write-ahead log segments  
├── compacted/         # Compacted SST files
└── sst/               # Sorted string table files
```

Do not manually modify files under this prefix. Rocklake manages this layout exclusively through SlateDB's compaction and garbage collection.

## Deployment-Specific Recipes

### Local Development (Minimal)

```bash
rocklake serve --catalog ./dev-catalog --bind 127.0.0.1:5432
```

No environment variables needed. Data stored as local files.

### Docker / Kubernetes (Environment-Driven)

```bash
# All configuration via environment
export ROCKLAKE_CATALOG=s3://production-bucket/catalog/
export ROCKLAKE_BIND=0.0.0.0:5432
export ROCKLAKE_PASSWORD=secure-random-password
export ROCKLAKE_LOG_FORMAT=json
export AWS_REGION=us-east-1
rocklake serve
```

### Read Replica (Read-Only)

```bash
rocklake serve --catalog s3://production-bucket/catalog/ --read-only --bind 0.0.0.0:5432
```

The server refuses any DDL or DML statements. Multiple read-only instances can connect to the same storage path concurrently.

### High-Security (Mutual TLS + Auth)

```bash
rocklake serve \
    --catalog s3://sensitive-bucket/catalog/ \
    --bind 0.0.0.0:5432 \
    --tls-cert /etc/rocklake/tls/server-cert.pem \
    --tls-key /etc/rocklake/tls/server-key.pem \
    --tls-ca /etc/rocklake/tls/client-ca.pem \
    --auth-user ducklake \
    --max-sessions 20
```

### S3-Compatible (MinIO)

```bash
export AWS_ENDPOINT_URL=http://minio.internal:9000
export AWS_ACCESS_KEY_ID=minioadmin
export AWS_SECRET_ACCESS_KEY=minioadmin
export ROCKLAKE_S3_PATH_STYLE=true
rocklake serve --catalog s3://my-bucket/catalog/ --bind 0.0.0.0:5432
```

## Validation and Diagnostics

On startup, Rocklake validates all configuration and reports errors clearly:

```
ERROR: --catalog is required but not set
ERROR: --tls-cert specified without --tls-key
ERROR: Cannot access storage path s3://bucket/path/ — Access Denied
```

Use `--log-level debug` to see the full resolved configuration (with secrets masked) at startup:

```
INFO  Configuration resolved:
INFO    catalog: s3://my-bucket/catalog/
INFO    bind: 0.0.0.0:5432
INFO    max_sessions: 100
INFO    read_only: false
INFO    tls: enabled (cert: /etc/rocklake/tls/cert.pem)
INFO    auth: enabled (user: ducklake)
INFO    hot_key_cache: true
INFO    batch_size: 1000
```

## Performance Tuning Reference

Most deployments will work well with the defaults, but these settings can have a significant impact in specific scenarios.

### Write Batch Size (`--batch-size`)

Rocklake accumulates catalog mutations in a batch before committing them to SlateDB. Larger batches mean fewer round trips to object storage per transaction, which reduces latency for write-heavy workloads where many rows change per transaction.

However, larger batches also consume more memory during the commit phase. The default of 1000 is appropriate for most DuckLake workloads where a typical `INSERT` operation registers a small number of Parquet files. If you are running bulk import operations that register tens of thousands of files in a single transaction, consider increasing this to 5000–10000. If you are on memory-constrained hardware, reducing it to 250–500 reduces peak memory use.

```bash
# For bulk import workloads
rocklake serve --catalog s3://my-bucket/catalog/ --batch-size 5000

# For memory-constrained hosts (< 512 MB available)
rocklake serve --catalog s3://my-bucket/catalog/ --batch-size 250
```

### Hot Key Cache (`--hot-key-cache`)

The hot key cache keeps the most recently read catalog keys in memory. Since DuckDB's `ducklake` extension reads the same schema, table list, and column list at the start of every query session, these keys are accessed far more frequently than individual data file entries.

With the cache enabled (the default), repeated reads of frequently accessed keys avoid object storage round trips. The cache is bounded by the block cache size in SlateDB — it does not grow unboundedly. On memory-constrained deployments where every megabyte counts, disable the cache with `--hot-key-cache false` to release that memory, accepting higher read latency.

### Session Limit (`--max-sessions`)

Each concurrent session uses approximately 1 MB of memory for its connection state, read views, and query parsing buffers. The default limit of 64 supports up to 64 simultaneous DuckDB connections at ~64 MB peak session memory.

For deployments with many short-lived connections (such as dashboards with per-query connections), you may want to increase this. For deployments where each connection is held open for hours (batch jobs, ETL pipelines), the default is generous — you rarely need more than a handful of concurrent long-lived connections.

```bash
# High-concurrency analytical dashboard
rocklake serve --catalog s3://my-bucket/catalog/ --max-sessions 256

# Single ETL pipeline, minimize resource use
rocklake serve --catalog s3://my-bucket/catalog/ --max-sessions 4
```

### Prefetch Depth (`--prefetch-depth`)

During SST scans (such as listing all data files for a table, or scanning snapshot history), Rocklake prefetches ahead by this many data blocks. Higher values use more memory but reduce sequential scan latency by overlapping I/O with processing.

At the default of 4, this is invisible for most workloads. If you are running the `export` command frequently over large catalogs, try `--prefetch-depth 8` or `--prefetch-depth 16` to speed up the sequential passes.

### Compaction (`--compaction-interval`)

SlateDB compacts SST files in the background to reduce read amplification and garbage-collect superseded versions. By default, Rocklake triggers a compaction check every 300 seconds (5 minutes).

Compaction is a background operation — it does not block reads or writes. However, it does consume CPU and generate object storage I/O. On cost-sensitive deployments (where you are paying per S3 API call), you may want to increase the interval to 1800 or 3600 seconds. On write-heavy deployments that generate many small SSTs, you may want to decrease it to 60–120 seconds.

Setting `--compaction-interval 0` disables automatic compaction entirely. You are then responsible for triggering compaction manually. This is not recommended for production deployments.

## Security Configuration Reference

### Password Handling

Avoid passing passwords on the command line. Shell history, process listings (`ps aux`), and container logs can all expose command-line arguments. Use the `ROCKLAKE_AUTH_PASSWORD` environment variable or the TOML file's `[auth]` section instead:

```bash
# Avoid this (exposed in ps, history)
rocklake serve --auth-password my-secret-pass

# Prefer this
export ROCKLAKE_AUTH_PASSWORD=my-secret-pass
rocklake serve --catalog s3://my-bucket/catalog/

# Or use a secrets manager
export ROCKLAKE_AUTH_PASSWORD=$(aws secretsmanager get-secret-value \
  --secret-id rocklake/auth-password --query SecretString --output text)
rocklake serve --catalog s3://my-bucket/catalog/
```

### Mutual TLS

When `--tls-ca` is configured, Rocklake requires every connecting client to present a certificate signed by the specified CA. This means clients can only connect if they have a valid client certificate — password authentication becomes a second factor rather than the only factor.

Mutual TLS is the recommended security configuration for production deployments that are exposed beyond a trusted private network. DuckDB supports client certificates via connection string parameters:

```sql
ATTACH 'host=rocklake.example.com port=5432 sslmode=verify-full sslcert=client.crt sslkey=client.key sslrootcert=ca.crt'
  AS my_lakehouse (TYPE ducklake);
```

### Read-Only Mode

The `--read-only` flag is useful for secondary endpoints that should serve queries without accepting schema changes or data modifications. Read-only instances can connect to the same catalog as the primary read-write instance simultaneously — SlateDB's snapshot isolation ensures they see a consistent view.

One pattern is to expose two endpoints: a read-write endpoint for DuckDB sessions that need to write, and a read-only endpoint for all query-only workloads. This limits the blast radius if a query-only client's credentials are compromised — they can read data but cannot corrupt the catalog.

```bash
# Primary: read-write
rocklake serve --catalog s3://my-bucket/catalog/ --bind 10.0.0.1:5432

# Secondary: read-only (separate IP/port, same catalog)
rocklake serve --catalog s3://my-bucket/catalog/ --bind 10.0.0.2:5432 --read-only
```

## Logging Configuration

### Log Levels

Rocklake supports five log levels in increasing verbosity:

| Level | What Is Logged |
|-------|---------------|
| `error` | Unrecoverable errors only |
| `warn` | Potentially concerning conditions (degraded performance, retried operations) |
| `info` | Normal operational events (startup, shutdown, session open/close, GC runs) |
| `debug` | Detailed operational information (individual SQL statements, key access patterns) |
| `trace` | Extremely verbose low-level tracing (SlateDB I/O, key encoding/decoding) |

`info` is the right default for most production deployments. Use `debug` when troubleshooting unexpected behavior. Use `trace` only when diagnosing performance problems or suspected bugs in key encoding — at trace level, log output can overwhelm disk I/O.

### Log Format

`--log-format text` (the default) produces human-friendly color-highlighted output:

```
2024-06-30T12:01:00.123Z  INFO rocklake_pgwire: New session from 10.0.1.5:49123
2024-06-30T12:01:00.124Z  INFO rocklake_catalog: Query plan: ListTables schema=analytics
2024-06-30T12:01:00.231Z  INFO rocklake_pgwire: Session closed after 107ms (3 queries)
```

`--log-format json` produces structured JSON, one event per line:

```json
{"timestamp":"2024-06-30T12:01:00.123Z","level":"INFO","target":"rocklake_pgwire","message":"New session","client_addr":"10.0.1.5:49123"}
```

JSON format is strongly recommended for container deployments where logs are collected by a log aggregation system (Datadog, Grafana Loki, Elastic Stack). JSON events are machine-parseable without regex extraction, and structured fields like `client_addr` and `session_id` remain searchable.

### Fine-Grained Filtering

The `RUST_LOG` environment variable provides per-crate log level control:

```bash
# Debug the PG wire layer only, keep catalog at info
RUST_LOG=rocklake_pgwire=debug,rocklake_catalog=info

# Suppress SlateDB noise, debug Rocklake
RUST_LOG=slatedb=warn,rocklake=debug

# Full trace for key encoding
RUST_LOG=rocklake_core=trace,rocklake_catalog=debug
```

`RUST_LOG` takes precedence over `--log-level` for the specific crates it targets. You can use both together: set `--log-level warn` for low baseline noise, and override specific crates with `RUST_LOG` as needed.

## Further Reading

- **[Binary Deployment](binary.md)** — Running as a standalone process with systemd
- **[Docker Deployment](docker.md)** — Container configuration patterns
- **[TLS and Authentication](tls.md)** — Certificate management details
- **[Networking](networking.md)** — Firewall and load balancer configuration
- **[CLI Reference](../operations/cli-reference.md)** — Full list of commands and options
