# Inspect

Human-readable catalog view without DuckDB.

## Usage

```bash
slateduck inspect --catalog-path s3://bucket/catalogs/warehouse
slateduck inspect --catalog-path s3://bucket/catalogs/warehouse --table analytics.events
slateduck inspect --catalog-path s3://bucket/catalogs/warehouse --snapshot 42
```

## Example Output

```
Catalog: warehouse
  Current Snapshot: 47
  Format Version: 1

Schemas: main (id=1), analytics (id=2)

Tables:
  analytics.events (5 columns, 127 files, 2.3 GB)
  analytics.users (4 columns, 12 files, 45 MB)
```
