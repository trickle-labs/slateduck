# DuckDB Compatibility Matrix

This document tracks which DuckDB versions and non-DuckDB clients are tested and supported.

## Supported DuckDB Versions

| DuckDB Version | Status | Wire Corpus | Notes |
|----------------|--------|-------------|-------|
| 1.2.2 | Baseline | `tests/fixtures/wire-corpus/duckdb-1.2.2.jsonl` | Phase 0 capture |

## Non-DuckDB Clients

| Client | Version | Status | Wire Corpus | Notes |
|--------|---------|--------|-------------|-------|
| pg-tide-relay | 0.34 | Validated | `tests/fixtures/wire-corpus/pgtide-0.34.jsonl` | v0.6 onboarding |

### pg-tide-relay Extensions

The following SQL patterns were added to support pg-tide-relay (all within or trivially near the bounded set):

- `ORDER BY ... ASC LIMIT 1` on `ducklake_snapshot`
- `SELECT max(snapshot_id) FROM ducklake_snapshot WHERE snapshot_id > $1`
- Parameterized `LIMIT $N` on data-file SELECT
- `gen_random_uuid()` function call
- `INSERT INTO ducklake_metadata` / `SELECT value FROM ducklake_metadata WHERE metadata_key = $1`

### Application Metadata Key Namespace

Non-DuckDB clients use a dotted-prefix convention for application state:
```
{application}.{instance}.{key}  →  stored in ducklake_metadata, scope = global
e.g. pg_tide.orders-to-lake.offset  →  "4782"
```

Multiple applications coexist by using distinct prefixes. Application keys participate in snapshot transactions, enabling exactly-once semantics for streaming pipelines.

## Object Store Backends

| Backend | Status | CI Validation | Notes |
|---------|--------|---------------|-------|
| LocalFileSystem | Production | All tests | Development and CI |
| InMemory | Production | Unit tests | Unit test backend |
| AWS S3 Standard | Validated | Acceptance tests | v0.4+ |
| AWS S3 Express One Zone | Benchmarked | Performance tests | v0.5+ |
| Google Cloud Storage | Validated | Builder + config tests | v0.6: `object_store::gcp` integration |
| Azure Blob Storage | Validated | Builder + config tests | v0.6: `object_store::azure` integration |

### GCS Configuration

```bash
slateduck serve \
  --catalog gs://bucket/catalogs/warehouse-a \
  --bind 0.0.0.0:5432
```

Requires `GOOGLE_SERVICE_ACCOUNT` or application default credentials.

### Azure Configuration

```bash
slateduck serve \
  --catalog az://container/catalogs/warehouse-a \
  --bind 0.0.0.0:5432
```

Requires `AZURE_STORAGE_ACCOUNT_NAME` and `AZURE_STORAGE_ACCESS_KEY` (or Managed Identity).

## Version Policy

- **Patch releases** (e.g., 1.2.x → 1.2.y): Automated CI replay; pass = no action needed.
- **Minor version bumps** (e.g., 1.2.x → 1.3.0): New corpus capture + regression test required.
- **Major version bumps** (e.g., 1.x → 2.x): Full new client treatment — complete recapture and validation.
- **Non-DuckDB clients**: Wire corpus onboarding process per `ROADMAP.md` v0.6.

## CI Workflow

The `compatibility.yml` workflow runs:
- On every push to `main` and on every PR
- On a weekly schedule (Monday 06:00 UTC) to catch new DuckDB releases
- Wire-corpus replay tests for all supported clients
- Object store builder validation tests for GCS and Azure

## Known Differences

None at this time. Both DuckDB 1.2.2 and pg-tide-relay 0.34 operate correctly against SlateDuck.
