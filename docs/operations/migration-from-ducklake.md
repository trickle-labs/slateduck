# Migrating from DuckLake to RockLake

This guide covers how to migrate an existing DuckLake deployment to RockLake,
including cutover steps, rollback procedures, and known incompatibilities.

## Overview

DuckLake is a catalog format that stores metadata in a PostgreSQL or SQLite
database. RockLake implements the same DuckLake v1.0 catalog protocol but
stores metadata in an object-store-native key-value format (SlateDB).

The migration path uses RockLake's NDJSON export/import commands:

1. Export the source catalog to NDJSON using `rocklake export`.
2. Import the NDJSON into a new RockLake catalog using `rocklake import`.

## Prerequisites

- RockLake v0.30 or later
- An NDJSON export of the source DuckLake catalog (see [Exporting from DuckLake](#exporting-from-ducklake))
- Write access to the destination object store (S3, GCS, Azure Blob, or local filesystem)

## Exporting from DuckLake

If you have an existing RockLake catalog, export it with:

```sh
rocklake export --catalog ./source-catalog --output source-dump.ndjson
```

If you have a DuckLake deployment backed by PostgreSQL or SQLite, you first need
to stand up a RockLake PG-Wire sidecar pointed at the source catalog, run
`rocklake export` against it, and then import the result into the destination.

!!! note "CSV migration path"
    A direct CSV-to-NDJSON migration tool (`rocklake pg-migrate`) converts
    NDJSON catalog exports to PostgreSQL `INSERT` statements. Full CSV import
    from DuckLake's raw `COPY TO` output is **not yet implemented** and is
    planned for a future release.

## Running the Migration

```sh
rocklake import \
  --catalog s3://my-bucket/my-catalog \
  --input source-dump.ndjson
```

On success, the command prints a migration report:

```
Import complete:
  Rows imported:   1428
  Tables imported: 28
```

## Verifying the Migration

After migration, use `rocklake inspect` to confirm the catalog state:

```sh
rocklake inspect snapshot --latest --catalog s3://my-bucket/my-catalog
```

Then start RockLake in serve mode and run a quick connectivity check from DuckDB:

```sql
ATTACH 'ducklake:postgres://127.0.0.1:5555/' AS lake;
SELECT COUNT(*) FROM lake.ducklake_snapshot;
SELECT COUNT(*) FROM lake.ducklake_schema;
SELECT COUNT(*) FROM lake.ducklake_table;
```

## Cutover Procedure

1. **Freeze writes** on the source DuckLake deployment.
2. **Export** the final snapshot: `rocklake export --catalog ./source --output final.ndjson`
3. **Import**: `rocklake import --catalog <dest> --input final.ndjson`
4. **Verify** row counts and schema presence as described above.
5. **Update connection strings** in all DuckDB clients to point to the RockLake PG-Wire sidecar.
6. **Detach** the old DuckLake attachment and **attach** RockLake.

## Rollback

To roll back to the original DuckLake deployment:

1. Stop the RockLake sidecar.
2. Revert DuckDB connection strings to the original PostgreSQL or SQLite endpoint.
3. Resume writes on the original DuckLake deployment.

There is no data loss risk during the migration because RockLake writes to a
separate catalog. The original DuckLake catalog is read-only during cutover.

## Known Incompatibilities

| Feature | DuckLake | RockLake | Notes |
|---------|----------|-----------|-------|
| `ducklake_encrypted_secret` | Yes | Partial | Encryption keys must be re-registered |
| Partition pruning (complex predicates) | Yes | Partial | Zone-map pruning is supported; bloom filters are planned |
| `ducklake_inlined_data_table` | Yes | Yes | Supported |
| Write conflict resolution | Optimistic | Optimistic | Compatible |

## See Also

- [Export and Import](export.md)
- [CLI Reference](cli-reference.md)
- [Upgrades](upgrades.md)
