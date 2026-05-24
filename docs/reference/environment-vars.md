# Environment Variables Reference

This page provides the complete configuration reference for SlateDuck. Every environment variable recognized by the system is documented here: its purpose, type, default value, valid range, and examples. Configuration is the primary way operators control SlateDuck's behavior in production — from specifying where catalog data is stored, to tuning performance parameters, to enabling TLS encryption.

SlateDuck follows the twelve-factor app methodology: configuration comes from the environment, not from configuration files. This makes deployment straightforward across diverse environments (Docker, Kubernetes, systemd, Lambda) without needing to manage config file paths or formats.

## Precedence Rules

When the same option is specified in multiple places, the highest-priority source wins:

1. **Command-line flags** (highest priority): `--storage`, `--bind`, etc.
2. **Environment variables**: `SLATEDUCK_STORAGE`, `SLATEDUCK_BIND`, etc.
3. **Compiled defaults** (lowest priority): built into the binary

Command-line flags always override environment variables. This allows operators to temporarily override configuration without modifying the deployment environment.

---

## Storage Configuration

These variables control where SlateDuck stores catalog data and how it authenticates to object storage providers.

### SLATEDUCK_STORAGE

The object storage path for the catalog. This is the most important configuration variable — it determines where all catalog data is persisted.

| Aspect | Detail |
|--------|--------|
| **Required** | Yes (or `--storage` flag) |
| **Type** | URI string |
| **CLI equivalent** | `--catalog <path>` |

**Valid formats:**

```bash
# Local filesystem (development only)
SLATEDUCK_STORAGE=./my-catalog
SLATEDUCK_STORAGE=/var/lib/slateduck/catalog

# Amazon S3
SLATEDUCK_STORAGE=s3://my-bucket/catalog/

# S3 Express One Zone
SLATEDUCK_STORAGE=s3://my-bucket--usw2-az1--x-s3/catalog/

# Google Cloud Storage
SLATEDUCK_STORAGE=gs://my-bucket/catalog/

# Azure Blob Storage
SLATEDUCK_STORAGE=az://my-container/catalog/
```

**Important:** The path should be dedicated to this SlateDuck instance. Do not share a storage path between multiple independent catalogs.

---

### AWS_REGION

The AWS region for S3 access. Required when using S3 storage (unless the region can be inferred from the bucket name or endpoint).

| Aspect | Detail |
|--------|--------|
| **Required** | For S3 (when not inferable) |
| **Default** | `us-east-1` |
| **Type** | AWS region string |

```bash
AWS_REGION=eu-west-1
AWS_REGION=ap-southeast-1
```

### AWS_ENDPOINT_URL

Custom endpoint URL for S3-compatible storage services (MinIO, Ceph, DigitalOcean Spaces, Cloudflare R2, etc.).

| Aspect | Detail |
|--------|--------|
| **Required** | Only for non-AWS S3-compatible services |
| **Default** | AWS standard endpoints |
| **Type** | URL string |

```bash
# MinIO (local development)
AWS_ENDPOINT_URL=http://localhost:9000

# Cloudflare R2
AWS_ENDPOINT_URL=https://ACCOUNT_ID.r2.cloudflarestorage.com

# DigitalOcean Spaces
AWS_ENDPOINT_URL=https://nyc3.digitaloceanspaces.com
```

### AWS_ACCESS_KEY_ID

Static AWS access key for authentication. Prefer IAM roles (instance profiles, IRSA, ECS task roles) over static credentials in production.

| Aspect | Detail |
|--------|--------|
| **Required** | Only for static credential authentication |
| **Default** | None (uses IAM role chain) |
| **Type** | AWS access key string |
| **Security** | Sensitive — do not commit to version control |

### AWS_SECRET_ACCESS_KEY

Static AWS secret key. Always used in conjunction with `AWS_ACCESS_KEY_ID`.

| Aspect | Detail |
|--------|--------|
| **Required** | With AWS_ACCESS_KEY_ID |
| **Default** | None |
| **Type** | AWS secret key string |
| **Security** | Sensitive — do not commit to version control |

### AWS_SESSION_TOKEN

Temporary session token for AWS STS credentials (assumed roles, federated access).

| Aspect | Detail |
|--------|--------|
| **Required** | Only with temporary credentials |
| **Default** | None |
| **Type** | AWS session token string |

### GOOGLE_APPLICATION_CREDENTIALS

Path to a GCS service account JSON key file. Used when running outside GCP (where Application Default Credentials are not available).

| Aspect | Detail |
|--------|--------|
| **Required** | For GCS when outside GCP |
| **Default** | Uses Application Default Credentials |
| **Type** | File path |

```bash
GOOGLE_APPLICATION_CREDENTIALS=/etc/slateduck/gcs-key.json
```

### AZURE_STORAGE_ACCOUNT

Azure storage account name. Required for Azure Blob Storage.

| Aspect | Detail |
|--------|--------|
| **Required** | For Azure storage |
| **Default** | None |
| **Type** | Account name string |

### AZURE_STORAGE_ACCESS_KEY

Azure storage account access key.

| Aspect | Detail |
|--------|--------|
| **Required** | For Azure (when not using managed identity) |
| **Default** | None |
| **Type** | Base64-encoded key |
| **Security** | Sensitive |

---

## Server Configuration

These variables control the network listener, session management, and access control.

### SLATEDUCK_BIND

The address and port SlateDuck listens on for PostgreSQL wire protocol connections.

| Aspect | Detail |
|--------|--------|
| **Required** | No |
| **Default** | `127.0.0.1:5432` |
| **Type** | `host:port` string |
| **CLI equivalent** | `--bind <addr>` |

```bash
# Listen on all interfaces (required in Docker/K8s)
SLATEDUCK_BIND=0.0.0.0:5432

# Non-standard port (to avoid conflict with local PostgreSQL)
SLATEDUCK_BIND=127.0.0.1:5433

# IPv6
SLATEDUCK_BIND=[::]:5432
```

**Security note:** Binding to `0.0.0.0` exposes the service to the network. Ensure network-level access control (security groups, network policies) is in place.

### SLATEDUCK_MAX_SESSIONS

Maximum number of concurrent client connections. Connections beyond this limit receive an immediate error.

| Aspect | Detail |
|--------|--------|
| **Required** | No |
| **Default** | `64` |
| **Type** | Positive integer |
| **Valid range** | 1 – 10000 |

```bash
SLATEDUCK_MAX_SESSIONS=128
```

**Sizing guidance:** Each session uses minimal memory (a few KB for session state). The limit protects against connection exhaustion, not memory exhaustion. Set based on your expected peak concurrent connections plus 20% headroom.

### SLATEDUCK_PASSWORD

When set, requires clients to authenticate with this password during the PostgreSQL startup handshake.

| Aspect | Detail |
|--------|--------|
| **Required** | No (unauthenticated by default) |
| **Default** | None (no authentication required) |
| **Type** | String |
| **Security** | Sensitive — use secrets management in production |

```bash
SLATEDUCK_PASSWORD=my-secure-password
```

**Authentication flow:** Uses PostgreSQL's cleartext password authentication. For production, combine with TLS to protect the password in transit.

### SLATEDUCK_REQUIRE_TLS

When true, rejects connections that do not use TLS encryption.

| Aspect | Detail |
|--------|--------|
| **Required** | No |
| **Default** | `false` |
| **Type** | Boolean (`true` / `false`) |

```bash
SLATEDUCK_REQUIRE_TLS=true
```

Requires `SLATEDUCK_TLS_CERT` and `SLATEDUCK_TLS_KEY` to also be set.

### SLATEDUCK_READ_ONLY

When true, the instance rejects all write operations (DDL, data file registration, etc.). Useful for read replicas or maintenance windows.

| Aspect | Detail |
|--------|--------|
| **Required** | No |
| **Default** | `false` |
| **Type** | Boolean |

```bash
SLATEDUCK_READ_ONLY=true
```

Write attempts return SQLSTATE `25006` (read_only_sql_transaction).

---

## TLS Configuration

### SLATEDUCK_TLS_CERT

Path to the TLS certificate file in PEM format. Required to enable TLS.

| Aspect | Detail |
|--------|--------|
| **Required** | For TLS |
| **Default** | None (TLS disabled) |
| **Type** | File path |

```bash
SLATEDUCK_TLS_CERT=/etc/slateduck/tls/server.crt
```

The certificate file should contain the full chain (server certificate followed by intermediate certificates).

### SLATEDUCK_TLS_KEY

Path to the TLS private key file in PEM format.

| Aspect | Detail |
|--------|--------|
| **Required** | With SLATEDUCK_TLS_CERT |
| **Default** | None |
| **Type** | File path |
| **Security** | Sensitive — restrict file permissions (chmod 600) |

```bash
SLATEDUCK_TLS_KEY=/etc/slateduck/tls/server.key
```

---

## Logging Configuration

### RUST_LOG

Controls log output verbosity using the `tracing` subscriber's filter syntax. Supports per-crate granularity.

| Aspect | Detail |
|--------|--------|
| **Required** | No |
| **Default** | `info` |
| **Type** | Filter string |

```bash
# Basic levels
RUST_LOG=info          # Default: operational messages
RUST_LOG=debug         # Verbose: includes internal state
RUST_LOG=warn          # Quiet: only warnings and errors

# Per-crate filtering
RUST_LOG=slateduck_pgwire=debug,slateduck_catalog=info
RUST_LOG=slateduck_catalog::gc=trace
RUST_LOG=info,slateduck_pgwire::handler=debug
```

**Common debugging configurations:**

| Scenario | Filter |
|----------|--------|
| Protocol debugging | `slateduck_pgwire=debug` |
| Storage performance | `slateduck_catalog=debug,object_store=debug` |
| MVCC issues | `slateduck_catalog::reader=trace` |
| GC behavior | `slateduck_catalog::gc=debug` |
| Everything | `debug` (very verbose) |

### SLATEDUCK_LOG_FORMAT

Controls the output format of log messages.

| Aspect | Detail |
|--------|--------|
| **Required** | No |
| **Default** | `text` |
| **Type** | `text` or `json` |

```bash
# Human-readable (for terminal/development)
SLATEDUCK_LOG_FORMAT=text
# Output: 2025-01-15T10:30:00Z INFO slateduck_pgwire: new session from 192.168.1.5:4321

# Structured JSON (for log aggregation: Datadog, Loki, CloudWatch)
SLATEDUCK_LOG_FORMAT=json
# Output: {"timestamp":"2025-01-15T10:30:00Z","level":"INFO","target":"slateduck_pgwire","message":"new session","client":"192.168.1.5:4321"}
```

---

## Performance Tuning

These variables control SlateDuck's internal performance behavior. The defaults are appropriate for most workloads. Change them only after benchmarking demonstrates a benefit.

### SLATEDUCK_HOT_KEY_CACHE

Enables or disables the hot key cache (in-memory caching of frequently-accessed system keys like counters and epoch).

| Aspect | Detail |
|--------|--------|
| **Required** | No |
| **Default** | `true` |
| **Type** | Boolean |

```bash
SLATEDUCK_HOT_KEY_CACHE=false  # Disable for debugging cache issues
```

Disabling this forces every read to go to SlateDB (and potentially object storage). Useful only for debugging.

### SLATEDUCK_BATCH_SIZE

Maximum number of key-value mutations allowed in a single write batch. Prevents unbounded memory usage from very large transactions.

| Aspect | Detail |
|--------|--------|
| **Required** | No |
| **Default** | `1000` |
| **Type** | Positive integer |
| **Valid range** | 1 – 100000 |

```bash
SLATEDUCK_BATCH_SIZE=5000  # For bulk operations (many columns per table)
```

### SLATEDUCK_PREFETCH_DEPTH

Number of SST blocks to prefetch during prefix scans. Higher values improve sequential read performance but use more memory.

| Aspect | Detail |
|--------|--------|
| **Required** | No |
| **Default** | `4` |
| **Type** | Positive integer |
| **Valid range** | 1 – 64 |

```bash
SLATEDUCK_PREFETCH_DEPTH=8  # For high-latency storage (cross-region S3)
```

### SLATEDUCK_CACHE_SIZE_MB

SlateDB block cache size in megabytes. The block cache stores recently-read SST blocks in memory to avoid repeated object storage fetches.

| Aspect | Detail |
|--------|--------|
| **Required** | No |
| **Default** | `64` |
| **Type** | Positive integer (MB) |
| **Valid range** | 8 – 4096 |

```bash
SLATEDUCK_CACHE_SIZE_MB=256  # For large catalogs (many tables/files)
```

**Sizing guidance:** The working set for most catalogs fits in 64–128 MB. Larger caches help when the catalog has thousands of tables or hundreds of thousands of data files.

### SLATEDUCK_INLINE_THRESHOLD_BYTES

Maximum size of inlined data in catalog values. Files smaller than this threshold may have their content stored directly in the catalog (alongside their metadata) rather than as separate objects.

| Aspect | Detail |
|--------|--------|
| **Required** | No |
| **Default** | `4096` |
| **Type** | Positive integer (bytes) |
| **Valid range** | 0 – 65536 |

```bash
SLATEDUCK_INLINE_THRESHOLD_BYTES=8192  # Inline larger files
SLATEDUCK_INLINE_THRESHOLD_BYTES=0     # Never inline (always reference)
```

---

## Metrics Configuration

### SLATEDUCK_METRICS_BIND

Address and port for the Prometheus metrics HTTP endpoint.

| Aspect | Detail |
|--------|--------|
| **Required** | No |
| **Default** | Disabled (no metrics endpoint) |
| **Type** | `host:port` string |

```bash
SLATEDUCK_METRICS_BIND=0.0.0.0:9090
```

When set, SlateDuck exposes a `/metrics` endpoint in Prometheus exposition format at the specified address.

---

## Health Check Configuration

### SLATEDUCK_HEALTH_BIND

Address and port for the HTTP health check endpoint.

| Aspect | Detail |
|--------|--------|
| **Required** | No |
| **Default** | Disabled (no health endpoint) |
| **Type** | `host:port` string |

```bash
SLATEDUCK_HEALTH_BIND=0.0.0.0:8080
```

When set, exposes `/health/live` (liveness) and `/health/ready` (readiness) endpoints.

---

## Example Configurations

### Local Development

```bash
SLATEDUCK_STORAGE=./dev-catalog
SLATEDUCK_BIND=127.0.0.1:5433
RUST_LOG=debug
SLATEDUCK_LOG_FORMAT=text
```

### Production (AWS S3)

```bash
SLATEDUCK_STORAGE=s3://mycompany-lakehouse/catalog/
AWS_REGION=us-east-1
SLATEDUCK_BIND=0.0.0.0:5432
SLATEDUCK_TLS_CERT=/etc/slateduck/tls/server.crt
SLATEDUCK_TLS_KEY=/etc/slateduck/tls/server.key
SLATEDUCK_REQUIRE_TLS=true
SLATEDUCK_PASSWORD=${SLATEDUCK_PASSWORD_FROM_SECRETS}
SLATEDUCK_MAX_SESSIONS=128
SLATEDUCK_CACHE_SIZE_MB=256
SLATEDUCK_METRICS_BIND=0.0.0.0:9090
SLATEDUCK_HEALTH_BIND=0.0.0.0:8080
RUST_LOG=info
SLATEDUCK_LOG_FORMAT=json
```

### Docker Compose (with MinIO)

```bash
SLATEDUCK_STORAGE=s3://catalog/data/
AWS_ENDPOINT_URL=http://minio:9000
AWS_ACCESS_KEY_ID=minioadmin
AWS_SECRET_ACCESS_KEY=minioadmin
AWS_REGION=us-east-1
SLATEDUCK_BIND=0.0.0.0:5432
RUST_LOG=info
```

## Further Reading

- **[Deployment: Configuration](../deployment/configuration.md)** — Deployment-specific configuration guidance
- **[Performance: Tuning](../performance/tuning.md)** — When and how to adjust performance parameters
- **[Deployment: TLS](../deployment/tls.md)** — TLS setup guide
