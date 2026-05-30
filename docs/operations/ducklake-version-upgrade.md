# DuckLake Version Upgrade Guide

This guide explains how to work with multiple DuckLake catalog versions in
RockLake and how to prepare for future version upgrades.

## Supported DuckLake Versions

| DuckLake Version | catalog_version | RockLake Support |
|------------------|-----------------|------------------|
| 1.0              | 7               | Full support (default) |
| 1.1 (dev)        | 8 (`V1_1_DEV_1`)| Experimental — requires `--accept-version V1_1_DEV_1` |

RockLake always reports `ducklake_version = "1.0"` and `catalog_version = 7`
to clients. It will not self-report as v1.1 until those features are fully
validated and promoted to stable.

## Checking Your Source Catalog Version

For a SQLite-backed DuckLake catalog:

```sh
sqlite3 /var/lib/ducklake/catalog.db \
  "SELECT MAX(schema_version) FROM ducklake_snapshot"
```

- Result `7` → DuckLake v1.0. Migration is accepted by default.
- Result `8` → DuckLake v1.1-dev. Pass `--accept-version V1_1_DEV_1` to proceed.

## Migrating from DuckLake v1.1 (dev)

```sh
rocklake migrate-from-ducklake \
  --source sqlite:/var/lib/ducklake/catalog.db \
  --catalog s3://my-bucket/rocklake/ \
  --accept-version V1_1_DEV_1
```

!!! warning
    DuckLake v1.1 schema changes are not yet finalized. Migrating from a v1.1
    catalog may result in missing metadata for features introduced after v1.0.

## Version Gate and SQLSTATE

When a source catalog version is not in the accepted list, RockLake returns:

```
ERROR 0A000: unsupported DuckLake catalog version <N>: ...
```

`SQLSTATE 0A000` is the PostgreSQL `feature_not_supported` code, which surfaces
correctly in clients that understand the PG wire protocol.

## Future Version Support

RockLake tracks the DuckLake specification. When DuckLake v1.1 is finalized and
promoted to stable, RockLake will add full support in a subsequent minor release.
At that point, `--accept-version V1_1_DEV_1` will no longer be required.

## See Also

- [Migration from DuckLake](migration-from-ducklake.md)
