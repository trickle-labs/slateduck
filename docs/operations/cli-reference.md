# CLI Reference

SlateDuck is operated entirely through its command-line interface. The binary accepts a top-level command followed by any sub-commands and options. This page documents every command, sub-command, flag, and environment variable, along with examples showing typical usage.

The guiding principle of the CLI design is that destructive or irreversible operations always require an explicit `apply` sub-command (or `--apply` flag) after a `plan` phase that shows what would happen. You can never accidentally delete or irrevocably modify catalog data with a single command — there is always a preview step.

## Global Options

These options apply to all commands:

| Option | Description | Default |
|--------|-------------|---------|
| `--catalog <path>` | Catalog path (required for most commands) | None — must be provided |
| `--help`, `-h` | Show help text and exit | — |

The `--catalog` option accepts the following path formats:

| Format | Backend | Example |
|--------|---------|---------|
| `s3://<bucket>/<prefix>/` | AWS S3 | `s3://my-bucket/catalog/` |
| `s3express://<bucket>/<prefix>/` | S3 Express One Zone | `s3express://my-bucket--use1-az4--x-s3/catalog/` |
| `gs://<bucket>/<prefix>/` | Google Cloud Storage | `gs://my-bucket/catalog/` |
| `az://<account>/<container>/<prefix>/` | Azure Blob Storage | `az://mylakehouse/catalog/` |
| `file://<absolute-path>` | Local filesystem | `file:///tmp/my-catalog` |
| `<relative-path>` | Local filesystem | `./my-catalog` |

## Commands

### `serve` — Start the PG-Wire Sidecar

Starts SlateDuck as a PostgreSQL wire protocol server. DuckDB clients connect to this server using the `ducklake` extension's `ATTACH` syntax. This is the primary command for all production deployments.

```bash
slateduck serve --catalog <path> [options]
```

**Options:**

| Option | Description | Default |
|--------|-------------|---------|
| `--bind <addr:port>` | Address and port to listen on | `127.0.0.1:5432` |
| `--max-sessions <n>` | Maximum number of concurrent DuckDB sessions | `64` |
| `--read-only` | Start in read-only mode (refuse all write operations) | Off |
| `--auth-user <user>` | Require this PostgreSQL username for connections | None |
| `--auth-password <pass>` | Require this password for connections (insecure; prefer env var) | None |
| `--tls-cert <path>` | Path to TLS certificate file | None |
| `--tls-key <path>` | Path to TLS private key file | None |
| `--metrics-bind <addr:port>` | Bind address for Prometheus `/metrics` endpoint | None |
| `--s3-path-style` | Use path-style S3 addressing (required for MinIO) | Off |
| `--s3-endpoint <url>` | Override S3 endpoint URL (for MinIO or S3-compatible services) | AWS default |

**Examples:**

```bash
# Local development
slateduck serve --catalog ./my-catalog --bind 127.0.0.1:5432

# Production with S3 and metrics
slateduck serve \
  --catalog s3://my-bucket/catalog/ \
  --bind 0.0.0.0:5432 \
  --metrics-bind 0.0.0.0:9090 \
  --max-sessions 128

# Read-only replica
slateduck serve \
  --catalog s3://my-bucket/catalog/ \
  --bind 0.0.0.0:5432 \
  --read-only

# With TLS and authentication
slateduck serve \
  --catalog s3://my-bucket/catalog/ \
  --bind 0.0.0.0:5432 \
  --tls-cert /etc/ssl/certs/slateduck.crt \
  --tls-key /etc/ssl/private/slateduck.key \
  --auth-user ducklake

# MinIO local development
slateduck serve \
  --catalog s3://my-bucket/catalog/ \
  --s3-endpoint http://localhost:9000 \
  --s3-path-style \
  --bind 127.0.0.1:5432
```

**Startup sequence:**

When `serve` starts, it:
1. Opens the SlateDB instance at the catalog path
2. Verifies or initializes the catalog format version
3. Establishes the writer epoch (fences any previous writer if needed)
4. Loads counter state (next snapshot ID, catalog ID, file ID)
5. Starts the TCP listener
6. Logs "Ready to accept connections"

If the catalog is new (no SST files exist at the path), SlateDuck initializes it with the current format version and counters at zero. If the catalog exists, SlateDuck reads the existing state and resumes from where it left off.

---

### `gc` — Visibility Garbage Collection

Advances the `retain-from` horizon, making snapshots older than the specified threshold query-inaccessible. This does not physically delete any data — it only gates visibility. Physical deletion requires the `excise` command.

```bash
slateduck gc plan|apply --catalog <path> [options]
```

#### `gc plan`

Shows what GC would do without making any changes. Always run `plan` before `apply`.

```bash
slateduck gc plan --catalog s3://my-bucket/catalog/ --retain-days 30
```

Output:
```
GC Plan
=======
Current retain-from:     snapshot 0 (no retention set)
Current latest snapshot: snapshot 1247
Proposed retain-from:    snapshot 938 (30 days ago: 2024-03-15T00:00:00Z)

Snapshots that would become inaccessible: 938
Estimated storage freed by subsequent excision: 124 MB

No changes made. Run 'gc apply' to proceed.
```

#### `gc apply`

Applies the retention policy:

```bash
slateduck gc apply --catalog s3://my-bucket/catalog/ --retain-days 30
```

**Options for both `plan` and `apply`:**

| Option | Description |
|--------|-------------|
| `--retain-days <n>` | Keep history for the last N days |
| `--retain-snapshots <n>` | Keep the last N snapshots |
| `--retain-from <snapshot-id>` | Set retain-from to a specific snapshot ID |

You must provide exactly one of `--retain-days`, `--retain-snapshots`, or `--retain-from`.

**Notes:**
- GC is idempotent: running `apply` multiple times with the same arguments is safe.
- GC only advances `retain-from` forward, never backward. You cannot undo a GC operation by re-running with a more permissive threshold.
- Pinned snapshots (see `checkpoint`) are never made inaccessible by GC, regardless of the retain-from setting.

---

### `excise` — Physical Data Deletion

Physically deletes superseded catalog entries whose `end_snapshot` is before the current `retain-from` horizon. This is an irreversible operation. An audit entry is written to the `0xFF | "excised"` system key recording what was deleted, when, and by whom.

```bash
slateduck excise plan|apply --catalog <path> [options]
```

#### `excise plan`

Shows what excision would delete:

```bash
slateduck excise plan --catalog s3://my-bucket/catalog/
```

Output:
```
Excision Plan
=============
Retain-from: snapshot 938
Eligible for excision: 12,847 rows across 9 catalog tables
Estimated storage reduction: 124 MB (SST space reclaimed after compaction)

Tables with eligible rows:
  ducklake_table:                  42 rows
  ducklake_column:                 387 rows
  ducklake_data_file:              11,903 rows
  ducklake_data_file_column_stats: 515 rows

Audit entry will be written to sys/excised.

Run 'excise apply' to proceed. This operation is irreversible.
```

#### `excise apply`

Performs the excision:

```bash
slateduck excise apply --catalog s3://my-bucket/catalog/
```

!!! warning "Irreversible"
    Excised data cannot be recovered. The `excise apply` command will ask for confirmation before proceeding unless `--yes` is passed. Ensure you have a recent backup or export before running excise in production.

**Options:**

| Option | Description |
|--------|-------------|
| `--yes` | Skip confirmation prompt |
| `--dry-run` | Alias for `plan` (deprecated; use `plan` instead) |

---

### `checkpoint` — Catalog Checkpoints

Manages named checkpoints — persistent pointers to specific snapshot IDs that prevent GC from advancing past them. Use checkpoints to preserve the state of the catalog at a significant point in time (a quarterly close, a schema migration milestone, an audit snapshot).

```bash
slateduck checkpoint create|list|restore --catalog <path> [options]
```

#### `checkpoint create`

Creates a named checkpoint at the current snapshot (or a specified snapshot):

```bash
# Checkpoint at the current snapshot
slateduck checkpoint create \
  --catalog s3://my-bucket/catalog/ \
  --name "Q2-2024-close" \
  --description "End of Q2 2024 reporting period"

# Checkpoint at a specific snapshot ID
slateduck checkpoint create \
  --catalog s3://my-bucket/catalog/ \
  --name "pre-migration" \
  --snapshot-id 1200
```

#### `checkpoint list`

Lists all existing checkpoints:

```bash
slateduck checkpoint list --catalog s3://my-bucket/catalog/
```

Output:
```
Name              Snapshot ID  Created                    Description
pre-migration     1200         2024-06-14T09:15:00Z       Before schema v2 migration
Q2-2024-close     1247         2024-06-30T23:59:59Z       End of Q2 2024 reporting period
annual-audit-2023 842          2024-01-01T00:00:00Z       Annual audit snapshot
```

#### `checkpoint restore`

Rolls the catalog forward to a checkpoint state. This does not rewrite history — it creates a new snapshot whose contents match the checkpoint snapshot:

```bash
slateduck checkpoint restore \
  --catalog s3://my-bucket/catalog/ \
  --name "pre-migration"
```

---

### `export` — NDJSON Catalog Export

Exports the full catalog (or a subset) to NDJSON (Newline-Delimited JSON) format. Each line of the output file is a self-contained JSON object representing one catalog row. The export is point-in-time: it captures the catalog state at a specific snapshot and is not affected by concurrent writes.

```bash
slateduck export --catalog <path> [options]
```

**Options:**

| Option | Description |
|--------|-------------|
| `--output <path>` | Output file path (default: stdout) |
| `--at-snapshot <id>` | Export at a specific snapshot ID |
| `--at-time <ISO8601>` | Export at the snapshot closest to a specific time |
| `--schema <name>` | Export only the specified schema (repeatable) |
| `--table <schema.table>` | Export only the specified table (repeatable) |

**Examples:**

```bash
# Export entire catalog to a file
slateduck export \
  --catalog s3://my-bucket/catalog/ \
  --output catalog-backup.ndjson

# Export at a specific snapshot
slateduck export \
  --catalog s3://my-bucket/catalog/ \
  --at-snapshot 1000 \
  --output catalog-at-1000.ndjson

# Export at a specific time
slateduck export \
  --catalog s3://my-bucket/catalog/ \
  --at-time "2024-06-30T23:59:59Z" \
  --output q2-close.ndjson

# Export only the analytics schema
slateduck export \
  --catalog s3://my-bucket/catalog/ \
  --schema analytics \
  --output analytics.ndjson

# Export to stdout and pipe to gzip
slateduck export --catalog s3://my-bucket/catalog/ | gzip > catalog.ndjson.gz
```

---

### `import` — Import Catalog from NDJSON

Imports a catalog from an NDJSON export file. Used for migration between backends (e.g., PostgreSQL to SlateDuck), disaster recovery, or seeding a new catalog from an export.

```bash
slateduck import --catalog <path> --input <file>
```

**Options:**

| Option | Description |
|--------|-------------|
| `--input <path>` | Input NDJSON file (required) |
| `--merge` | Merge into existing catalog (default: fail if catalog is not empty) |
| `--dry-run` | Validate the import file without writing |

**Example:**

```bash
# Import into a new catalog
slateduck import \
  --catalog s3://new-bucket/catalog/ \
  --input catalog-backup.ndjson

# Validate before importing
slateduck import \
  --catalog s3://new-bucket/catalog/ \
  --input catalog-backup.ndjson \
  --dry-run
```

---

### `pg-migrate` — Convert NDJSON to PostgreSQL INSERTs

Converts an NDJSON catalog export to SQL `INSERT` statements that can be executed against a PostgreSQL database. This is the migration path from SlateDuck to a PostgreSQL-backed DuckLake catalog.

```bash
slateduck pg-migrate --input <ndjson-file> --output <sql-file>
```

**Example:**

```bash
slateduck pg-migrate \
  --input catalog-export.ndjson \
  --output catalog-inserts.sql

# Apply to PostgreSQL
psql -h pg-host -U ducklake -d catalog_db -f catalog-inserts.sql
```

---

### `rebuild` — Rebuild Catalog from Parquet Files

Scans Parquet files in a data bucket and reconstructs catalog entries from the file metadata. Used when the catalog has been lost or corrupted but the underlying Parquet files are intact.

```bash
slateduck rebuild --catalog <path> --data-path <data-uri>
```

!!! warning
    Rebuild cannot recover schema history, view definitions, custom statistics, or MVCC version history — only the current set of data files. For anything more, use `import` from a backup.

---

### `inspect` — Inspect Catalog State

Shows the current state of the catalog in human-readable or JSON format. Useful for debugging, monitoring, and understanding what is in the catalog.

```bash
slateduck inspect snapshot --latest --catalog <path> [options]
```

**Options:**

| Option | Description |
|--------|-------------|
| `--latest` | Show the latest snapshot state |
| `--at-snapshot <id>` | Show state at a specific snapshot |
| `--format json` | Output as JSON instead of human-readable table |
| `--key <key>` | Inspect a specific raw key |
| `--prefix <prefix>` | Inspect all keys under a prefix |

**Example output:**

```
Catalog: s3://my-bucket/catalog/
Format version: 1
Latest snapshot: 1247 (2024-06-30T23:59:57Z)
Writer epoch: 7
Retain-from: snapshot 938 (2024-03-31T00:00:00Z)

Schemas: 3
  analytics (id=1): 4 tables
  marketing (id=2): 2 tables
  staging (id=3): 1 table

Total data files: 10,847
Total registered rows: ~2.1 billion
Catalog size (SST): ~47 MB
```

---

### `verify` — Verify Catalog Integrity

Checks the catalog for internal consistency and optionally verifies that referenced data files exist in object storage.

```bash
slateduck verify catalog|data-files --catalog <path> [options]
```

#### `verify catalog`

Checks catalog-internal consistency: key format validity, MVCC invariants (no row with end_snapshot ≤ begin_snapshot), counter consistency, and format version correctness:

```bash
slateduck verify catalog --catalog s3://my-bucket/catalog/
```

#### `verify data-files`

Checks that every data file registered in the catalog exists in object storage (does HEAD requests for each registered path):

```bash
slateduck verify data-files --catalog s3://my-bucket/catalog/
```

This can be slow for catalogs with many files. Use `--sample <n>` to verify a random sample instead:

```bash
slateduck verify data-files --catalog s3://my-bucket/catalog/ --sample 1000
```

---

### `repair` — Repair Catalog Issues

Applies automatic fixes for detectable catalog problems. Always run with `--dry-run` first to see what would be changed.

```bash
slateduck repair --dry-run|--apply --catalog <path>
```

**Options:**

| Option | Description |
|--------|-------------|
| `--dry-run` | Show what would be repaired without making changes |
| `--apply` | Apply the repairs |

Current auto-repairable issues:
- Orphaned column entries (columns whose table no longer exists)
- Stale secondary index entries (index points to a data file that was excised)
- Counter desync (counters behind the highest-observed ID)

Issues that require manual intervention:
- Missing data files (listed in catalog but absent from storage)
- Corrupted SST files (require SlateDB-level repair)
- Format version mismatches (require migration)

---

## Environment Variables

All command-line flags have corresponding environment variables. Environment variables take lower priority than command-line flags.

| Environment Variable | Flag Equivalent | Description |
|---------------------|-----------------|-------------|
| `SLATEDUCK_CATALOG` | `--catalog` | Catalog path |
| `SLATEDUCK_BIND` | `--bind` | Bind address |
| `SLATEDUCK_MAX_SESSIONS` | `--max-sessions` | Maximum concurrent sessions |
| `SLATEDUCK_READ_ONLY` | `--read-only` | Enable read-only mode (`true`/`false`) |
| `SLATEDUCK_AUTH_USER` | `--auth-user` | Required authentication username |
| `SLATEDUCK_AUTH_PASSWORD` | `--auth-password` | Authentication password |
| `SLATEDUCK_TLS_CERT` | `--tls-cert` | TLS certificate path |
| `SLATEDUCK_TLS_KEY` | `--tls-key` | TLS private key path |
| `SLATEDUCK_METRICS_BIND` | `--metrics-bind` | Metrics endpoint address |
| `SLATEDUCK_S3_PATH_STYLE` | `--s3-path-style` | Enable path-style S3 (`true`/`false`) |
| `SLATEDUCK_S3_ENDPOINT` | `--s3-endpoint` | Override S3 endpoint URL |
| `SLATEDUCK_CACHE_SIZE_MB` | `--cache-size-mb` | SlateDB block cache size in MiB |
| `SLATEDUCK_LOG_LEVEL` | `--log-level` | Log level (`error`/`warn`/`info`/`debug`/`trace`) |
| `SLATEDUCK_LOG_FORMAT` | `--log-format` | Log format (`text`/`json`) |

Object-store credentials are configured through the standard SDK environment variables for each provider:

**AWS S3:**
- `AWS_REGION`
- `AWS_ACCESS_KEY_ID`
- `AWS_SECRET_ACCESS_KEY`
- `AWS_SESSION_TOKEN` (for assumed roles)
- `AWS_ENDPOINT_URL` (for S3-compatible services)

**Google Cloud Storage:**
- `GOOGLE_APPLICATION_CREDENTIALS` (path to service account key)
- `GOOGLE_CLOUD_PROJECT`

**Azure Blob Storage:**
- `AZURE_STORAGE_ACCOUNT_NAME`
- `AZURE_CLIENT_ID`
- `AZURE_CLIENT_SECRET`
- `AZURE_TENANT_ID`
- `AZURE_STORAGE_CONNECTION_STRING` (for development only)
- `AZURE_USE_MANAGED_IDENTITY` (`true` for Managed Identity)

## Exit Codes

| Exit Code | Meaning |
|-----------|---------|
| `0` | Success |
| `1` | General error (see stderr for details) |
| `2` | Invalid arguments or missing required options |
| `3` | Catalog not found or could not be opened |
| `4` | Authentication error (wrong credentials or permissions) |
| `5` | Storage backend unavailable |
| `6` | Catalog format version mismatch (upgrade needed) |
| `7` | Writer lock conflict (another writer is active) |

## Further Reading

- **[Configuration Reference](configuration.md)** — All configuration options in depth
- **[Garbage Collection](garbage-collection.md)** — How GC and retention policies work
- **[Excision](excision.md)** — When and how to use physical data deletion
- **[Checkpoints](../operations/backup-restore.md)** — Using checkpoints for backup and recovery
- **[Export and Import](export.md)** — NDJSON format and migration workflows
