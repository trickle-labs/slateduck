# Export

Dump catalog to NDJSON for analysis, migration, or backup.

## Usage

```bash
slateduck export --catalog-path s3://bucket/catalogs/warehouse --output dump.ndjson
slateduck export --catalog-path s3://bucket/catalogs/warehouse --snapshot 42 --output snap42.ndjson
```

## Format

```json
{"table":"ducklake_table","key":{"schema_id":1,"table_id":1},"value":{"table_name":"events"}}
```

## Use Cases

- Migration to PostgreSQL-backed DuckLake
- Debugging with grep/jq
- Auditing at any point in time
