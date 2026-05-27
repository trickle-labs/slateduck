# Migrating from DuckLake to Rocklake

This guide covers how to migrate an existing DuckLake deployment to Rocklake,
including cutover steps, rollback procedures, and known incompatibilities.

## Overview

DuckLake is a catalog format that stores metadata in a PostgreSQL or SQLite
database. Rocklake implements the same DuckLake v1.0 catalog protocol but
stores metadata in an object-store-native key-value format (SlateDB).

The `rocklake migrate-from-ducklake` command provides a migration path from
any DuckLake deployment that can produce an NDJSON dump to a Rocklake catalog.

## Prerequisites

- Rocklake v0.27 or later
- An NDJSON export of the source DuckLake catalog (see [Exporting from DuckLake](#exporting-from-ducklake))
- Write access to the destination object store (S3, GCS, Azure Blob, or local filesystem)

## Exporting from DuckLake

Use `rocklake export-catalog` to produce an NDJSON dump of the current catalog
snapshot:

```sh
rocklake export-catalog --catalog ./source-catalog --out source-dump.ndjson
```

For an existing DuckLake deployment backed by PostgreSQL or SQLite, export the
metadata tables using the DuckLake `COPY TO` facility:

```sql
-- From DuckDB with ducklake extension attached:
ATTACH 'ducklake:postgres://user:pass@host/db' AS lake;
COPY (SELECT * FROM lake.ducklake_snapshot) TO 'snapshot.csv';
-- ... repeat for all 28 catalog tables ...
```

Then convert the CSV files to the Rocklake NDJSON format using the
`rocklake pg-migrate` tool:

```sh
rocklake pg-migrate --input snapshot.csv --output snapshot.ndjson
```

## Running the Migration

```sh
rocklake migrate-from-ducklake \
  --source source-dump.ndjson \
  --catalog s3://my-bucket/my-catalog
```

On success, the command prints a migration report:

```
migrate-from-ducklake: source=source-dump.ndjson, catalog=s3://my-bucket/my-catalog
Migration complete:
  Rows imported:   1428
  Tables imported: 28
  Catalog written to: s3://my-bucket/my-catalog
```

## Verifying the Migration

After migration, use `rocklake inspect` to confirm the catalog state:

```sh
rocklake inspect snapshot s3://my-bucket/my-catalog
```

Then start Rocklake in serve mode and run a quick connectivity check from DuckDB:

```sql
ATTACH 'ducklake:postgres://127.0.0.1:5555/' AS lake;
SELECT COUNT(*) FROM lake.ducklake_snapshot;
SELECT COUNT(*) FROM lake.ducklake_schema;
SELECT COUNT(*) FROM lake.ducklake_table;
```

## Cutover Procedure

1. **Freeze writes** on the source DuckLake deployment.
2. **Export** the final snapshot: `rocklake export-catalog --catalog ./source --out final.ndjson`
3. **Migrate**: `rocklake migrate-from-ducklake --source final.ndjson --catalog <dest>`
4. **Verify** row counts and schema presence as described above.
5. **Update connection strings** in all DuckDB clients to point to the Rocklake PG-Wire sidecar.
6. **Detach** the old DuckLake attachment and **attach** Rocklake.

## Rollback

To roll back to the original DuckLake deployment:

1. Stop the Rocklake sidecar.
2. Revert DuckDB connection strings to the original PostgreSQL or SQLite endpoint.
3. Resume writes on the original DuckLake deployment.

There is no data loss risk during the migration because Rocklake writes to a
separate catalog. The original DuckLake catalog is read-only during cutover.

## Known Incompatibilities

| Feature | DuckLake | Rocklake | Notes |
|---------|----------|-----------|-------|
| `ducklake_encrypted_secret` | Yes | Partial | Encryption keys must be re-registered |
| Partition pruning (complex predicates) | Yes | Partial | Zone-map pruning is supported; bloom filters are planned |
| `ducklake_inlined_data_table` | Yes | Yes | Supported |
| Write conflict resolution | Optimistic | Optimistic | Compatible |

## See Also

- [Export and Import](export.md)
- [CLI Reference](cli-reference.md)
- [Upgrades](upgrades.md)
