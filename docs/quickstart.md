# SlateDuck Documentation

## Quickstart

### Local Development

```bash
# Build
cargo build --release

# Start the PG-Wire sidecar with a local catalog
slateduck serve --catalog ./my-catalog --bind 0.0.0.0:5432

# Connect DuckDB
duckdb -c "ATTACH 'ducklake:postgres:host=localhost port=5432' AS lake;"
```

### MinIO (Docker)

```bash
# Start MinIO
docker run -d -p 9000:9000 -p 9001:9001 \
  -e MINIO_ROOT_USER=minioadmin \
  -e MINIO_ROOT_PASSWORD=minioadmin \
  minio/minio server /data --console-address :9001

# Create bucket
mc alias set local http://localhost:9000 minioadmin minioadmin
mc mb local/warehouse

# Start SlateDuck
export AWS_ENDPOINT=http://localhost:9000
export AWS_ACCESS_KEY_ID=minioadmin
export AWS_SECRET_ACCESS_KEY=minioadmin
export AWS_ALLOW_HTTP=true

slateduck serve --catalog s3://warehouse/catalogs/main --bind 0.0.0.0:5432 --metrics-port 9090
```

### AWS S3

```bash
# Ensure AWS credentials are configured (env vars, ~/.aws/credentials, or IAM role)
export AWS_REGION=us-east-1

slateduck serve --catalog s3://my-bucket/catalogs/production --bind 0.0.0.0:5432
```

## Architecture

```
┌─────────────┐        ┌──────────────────┐        ┌─────────────┐
│   DuckDB    │◄──PG───│  SlateDuck       │◄──KV───│  SlateDB    │
│  (client)   │  Wire  │  Sidecar         │        │  (on S3)    │
└─────────────┘        └──────────────────┘        └─────────────┘
                              │
                              │ Metrics
                              ▼
                       ┌──────────────┐
                       │  /metrics    │
                       │  (Prometheus)│
                       └──────────────┘
```

**Catalog Plane:** SlateDuck manages the DuckLake catalog in SlateDB (object storage).
Credential separation ensures the sidecar only needs `catalogs/` prefix access.

**Data Plane:** DuckDB reads/writes Parquet files directly using its own credentials
(scoped to the `data/` prefix).

## DuckDB Compatibility

| DuckDB Version | Status | Notes |
|---------------|--------|-------|
| 1.5.2 | Validated | Phase 0 wire corpus baseline |
| 1.5.x | Compatible | Minor version bumps require new corpus |
| 1.6.x+ | Untested | Major version requires full validation |

## Time Travel

SlateDuck supports time travel natively through MVCC snapshots:

```sql
-- Query at a specific snapshot
SELECT * FROM lake.main.my_table AT (SNAPSHOT 42);
```

All committed facts are readable at their original `dl_snapshot_id` by default
(infinite retention). Operators can configure bounded retention:

```bash
# Advance visibility floor to 30 days
slateduck gc apply --catalog <path> --retention-days 30
```

## Troubleshooting

### `slateduck verify catalog`

Checks primary-key uniqueness, foreign-key references, MVCC interval consistency,
and counter monotonicity.

```bash
slateduck verify catalog --catalog ./my-catalog
```

### `slateduck inspect snapshot --latest`

Shows current snapshot, schema version, counters, and file counts.

```bash
slateduck inspect snapshot --latest --catalog ./my-catalog
```

### `slateduck gc plan`

Shows what would happen if `retain-from` is advanced:

```bash
slateduck gc plan --catalog ./my-catalog --retention-days 30
```

## Operational Commands

| Command | Purpose |
|---------|---------|
| `slateduck serve` | Start PG-Wire sidecar |
| `slateduck inspect snapshot --latest` | Current state summary |
| `slateduck verify catalog` | Integrity verification |
| `slateduck verify data-files` | Check all Parquet files exist |
| `slateduck gc plan/apply` | Advance retain-from (no deletion) |
| `slateduck excise plan/apply` | Physical deletion (audited) |
| `slateduck checkpoint create/list/restore` | Backup management |
| `slateduck export` | NDJSON catalog export |
| `slateduck import` | Initialize from NDJSON |
| `slateduck pg-migrate` | Convert to PostgreSQL INSERTs |
| `slateduck rebuild` | Rebuild from Parquet files |
| `slateduck repair` | Fix repairable issues |

## Comparison

| Feature | SlateDuck | PG-backed DuckLake | SQLite-backed DuckLake |
|---------|-----------|-------------------|----------------------|
| Infrastructure | Zero (object store only) | PostgreSQL server | Local file |
| Durability | Object-store native | PG WAL | fsync |
| Time travel | Infinite by default | Manual | None |
| Horizontal reads | Unlimited replicas | PG replicas | None |
| Credential separation | Built-in | Manual | N/A |
| GC/Excision | Audited, safe | Manual `DELETE` | N/A |
