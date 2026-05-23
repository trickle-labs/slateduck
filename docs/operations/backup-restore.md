# Backup & Restore

## Checkpoint

```bash
slateduck checkpoint --catalog-path s3://bucket/catalogs/warehouse \
  --output s3://bucket/backups/warehouse-2024-01-01.checkpoint
```

A checkpoint contains all SST data, manifest, counter values, and format version.

## Restore

```bash
slateduck restore --checkpoint s3://bucket/backups/warehouse-2024-01-01.checkpoint \
  --target s3://bucket/catalogs/warehouse-restored
```

Restore creates a new catalog at the target path without overwriting the original.

## Recommended Frequency

Daily for production catalogs. Checkpoints are incremental if the previous is available.
