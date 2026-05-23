# Garbage Collection

GC reclaims storage from historical data exceeding the retention window.

## Commands

```bash
# Dry run
slateduck gc plan --catalog-path s3://bucket/catalogs/warehouse --retention-days 90

# Execute
slateduck gc apply --catalog-path s3://bucket/catalogs/warehouse --retention-days 90
```

## How It Works

1. Scan versioned rows
2. Identify rows where `end_snapshot <= oldest_retained_snapshot`
3. Physically delete eligible keys

## Safety

- Never deletes data needed for retained snapshots
- Idempotent (safe to run multiple times)
- Produces audit log entries
