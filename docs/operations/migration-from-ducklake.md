# Migration from DuckLake to RockLake

This guide explains how to migrate an existing DuckLake catalog into RockLake
using the `migrate-from-ducklake` command (v0.41.0+).

## Overview

RockLake reads directly from a DuckLake source and writes into a fresh RockLake
catalog. Data files (Parquet, etc.) are **not** copied — they remain at their
original object-store paths. Only the catalog metadata is migrated.

Supported sources:

- **SQLite** DuckLake catalog: `sqlite:/path/to/catalog.db`
- **NDJSON dump** (from `rocklake export-catalog`): `/path/to/dump.ndjson`

## Prerequisites

- RockLake v0.41.0 or later
- Access to the source DuckLake catalog
- A target RockLake catalog path (local or object-store URL)

## Basic Usage

### From a SQLite DuckLake catalog

```sh
rocklake migrate-from-ducklake \
  --source sqlite:/var/lib/ducklake/catalog.db \
  --catalog s3://my-bucket/rocklake/
```

### From an NDJSON dump

```sh
rocklake migrate-from-ducklake \
  --source /backup/ducklake-export.ndjson \
  --catalog s3://my-bucket/rocklake/
```

## DuckLake Version Support

| catalog_version | DuckLake Release | Accepted by default | Accept flag required |
|-----------------|-----------------|---------------------|----------------------|
| 7 (V1_0)        | 1.0             | Yes                 | None                 |
| 8 (V1_1_DEV_1)  | 1.1 (dev)       | No                  | `--accept-version V1_1_DEV_1` |

DuckLake v1.0 catalogs (`catalog_version = 7`) are accepted automatically.
Attempting to migrate from a v1.1 catalog without the flag returns `SQLSTATE 0A000`.

## Dry Run

Use `--dry-run` to validate the source without committing any writes:

```sh
rocklake migrate-from-ducklake \
  --source sqlite:/var/lib/ducklake/catalog.db \
  --catalog s3://my-bucket/rocklake/ \
  --dry-run
```

The command prints a migration report but makes no changes to the target catalog.

## Migration Report

After a successful migration, RockLake prints a JSON report:

```json
{
  "source_catalog_version": 7,
  "dry_run": false,
  "data_file_count": 1234,
  "tables": {
    "ducklake_snapshot": { "rows_migrated": 10, "rows_skipped": 0 },
    "ducklake_data_file": { "rows_migrated": 1234, "rows_skipped": 0 }
  }
}
```

Rows that could not be decoded are logged as warnings. A non-zero `rows_skipped`
count indicates a possible schema mismatch between the source and target versions.

## Atomicity

Each table's rows are written to SlateDB using a single `WriteBatch`. If the
process is interrupted, a partial migration can be retried safely against a
fresh target catalog.

## Post-Migration Verification

After migrating, validate the catalog with:

```sh
rocklake catalog-verify --catalog s3://my-bucket/rocklake/
```

Then start RockLake in read-only mode to validate queries before enabling writes:

```sh
rocklake serve --catalog s3://my-bucket/rocklake/ --mode reader
```

## See Also

- [DuckLake Version Upgrade](ducklake-version-upgrade.md)
- [Export / Import](export.md)
