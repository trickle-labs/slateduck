# Export

The export command extracts catalog data as NDJSON (Newline-Delimited JSON) for backup, migration, analysis, or compliance purposes. Each line in the output represents one catalog row with its table name and field values. Export is a read-only operation — it does not modify the catalog in any way.

This page covers the export command in detail: all available options, output format specification, filtering capabilities, performance characteristics, and practical integration patterns.

## Basic Usage

```bash
# Export current state to a file
rocklake export --catalog s3://bucket/catalog/ --output catalog.ndjson

# Export at a specific snapshot (point-in-time)
rocklake export --catalog s3://bucket/catalog/ --at-snapshot 1000 --output catalog-at-1000.ndjson

# Export at a specific timestamp
rocklake export --catalog s3://bucket/catalog/ --at-time "2024-12-15T00:00:00Z" --output catalog-yesterday.ndjson

# Export to stdout (for piping to other tools)
rocklake export --catalog s3://bucket/catalog/

# Export directly to S3
rocklake export --catalog s3://bucket/catalog/ --output s3://backup-bucket/exports/2024-12-16.ndjson
```

## Output Format

### NDJSON Structure

Each line is a self-contained JSON object with `table` and `data` fields:

```json
{"table":"ducklake_databases","data":{"database_id":1,"database_name":"my_lake","created_snapshot_id":1}}
{"table":"ducklake_schemas","data":{"schema_id":1,"schema_name":"analytics","database_id":1,"created_snapshot_id":2}}
{"table":"ducklake_tables","data":{"table_id":1,"table_name":"events","schema_id":1,"table_uuid":"550e8400-e29b-41d4-a716-446655440000","created_snapshot_id":5}}
{"table":"ducklake_columns","data":{"column_id":1,"table_id":1,"column_name":"event_id","data_type":"BIGINT","ordinal_position":0,"is_nullable":false,"created_snapshot_id":5}}
{"table":"ducklake_data_files","data":{"file_id":1,"table_id":1,"file_path":"s3://data-lake/events/part-001.parquet","file_format":"parquet","row_count":1000000,"file_size_bytes":45000000,"created_snapshot_id":10}}
```

### Ordering

Rows are emitted in dependency order:

1. Databases first
2. Schemas
3. Tables
4. Columns
5. Data files
6. Statistics
7. Other metadata (views, sequences, etc.)

This ordering ensures that import processes can resolve foreign key relationships without lookahead.

### Snapshot Semantics

Export includes ONLY rows visible at the target snapshot:

- Rows created before the snapshot AND not yet superseded → included
- Rows created after the snapshot → excluded
- Rows superseded (ended) before the snapshot → excluded

This gives you a clean, consistent point-in-time view of the catalog without any partially-committed state.

## Filtering Options

### Export Specific Schemas

```bash
# Export only the "analytics" schema
rocklake export --catalog s3://bucket/catalog/ --schema analytics --output analytics.ndjson

# Export multiple schemas
rocklake export --catalog s3://bucket/catalog/ --schema analytics --schema marketing --output subset.ndjson
```

### Export Specific Tables

```bash
# Export metadata for specific tables only
rocklake export --catalog s3://bucket/catalog/ --table analytics.events --table analytics.users --output tables.ndjson
```

### Export Specific DuckLake Tables (Internal)

For advanced use cases, export specific internal catalog tables:

```bash
# Export only column definitions
rocklake export --catalog s3://bucket/catalog/ --catalog-table ducklake_columns --output columns.ndjson

# Export only data file registrations
rocklake export --catalog s3://bucket/catalog/ --catalog-table ducklake_data_files --output files.ndjson
```

## Import (Restore from Export)

Create a new catalog from an NDJSON export:

```bash
# Import into a new location
rocklake import --catalog s3://bucket/new-catalog/ --input catalog.ndjson

# Import with overwrite (replaces existing catalog)
rocklake import --catalog s3://bucket/catalog/ --input catalog.ndjson --overwrite
```

### Import Behavior

- A fresh SlateDB instance is initialized at the target path
- Each NDJSON line is parsed and written as a key-value pair
- Counter values (snapshot IDs, entity IDs) are reassigned sequentially
- The import completes as a single atomic commit
- Original snapshot IDs and entity IDs are NOT preserved

### ID Mapping

During import, a mapping file can be generated showing old → new IDs:

```bash
rocklake import --catalog s3://bucket/new-catalog/ --input catalog.ndjson --id-map mapping.json
```

This is useful if external systems reference specific IDs from the original catalog.

## Use Cases

### Backup and Disaster Recovery

Export regularly and store in a separate location:

```bash
# Daily backup script
DATE=$(date +%Y-%m-%d)
rocklake export --catalog s3://bucket/catalog/ --output s3://backup-bucket/daily/$DATE.ndjson

# Retain 30 days of backups
aws s3 ls s3://backup-bucket/daily/ | sort | head -n -30 | awk '{print $4}' | xargs -I{} aws s3 rm s3://backup-bucket/daily/{}
```

### Migration Between Storage Backends

Move from local development to production cloud storage:

```bash
# Export from local
rocklake export --catalog ./dev-catalog/ --output migration.ndjson

# Import to S3
rocklake import --catalog s3://production-bucket/catalog/ --input migration.ndjson
```

### Analysis and Auditing

Load into DuckDB for ad-hoc queries:

```sql
-- Load the export
CREATE TABLE catalog_export AS SELECT * FROM read_ndjson('catalog.ndjson');

-- How many tables per schema?
SELECT data->>'schema_name' as schema, count(*) 
FROM catalog_export 
WHERE "table" = 'ducklake_tables' 
GROUP BY 1;

-- Total data size per table
SELECT data->>'table_name' as table_name, 
       sum((data->>'file_size_bytes')::BIGINT) as total_bytes
FROM catalog_export 
WHERE "table" = 'ducklake_data_files' 
GROUP BY 1 ORDER BY 2 DESC;
```

### Compliance Reporting

Generate a snapshot of catalog state for regulatory review:

```bash
# Export at the audit date
rocklake export --catalog s3://bucket/catalog/ \
    --at-time "2024-09-30T23:59:59Z" \
    --output compliance-report-Q3-2024.ndjson

# Convert to CSV for non-technical reviewers
cat compliance-report-Q3-2024.ndjson | jq -r 'select(.table == "ducklake_tables") | [.data.table_name, .data.schema_id] | @csv' > tables-Q3.csv
```

### Diff Between Snapshots

Compare catalog state at two points in time:

```bash
# Export at two snapshots
rocklake export --catalog s3://bucket/catalog/ --at-snapshot 500 --output snap500.ndjson
rocklake export --catalog s3://bucket/catalog/ --at-snapshot 600 --output snap600.ndjson

# Diff
diff <(sort snap500.ndjson) <(sort snap600.ndjson) > changes.diff
```

## Performance Characteristics

Export performance depends primarily on the number of catalog rows and the I/O characteristics of the underlying object store. Understanding these factors helps you plan export schedules and set appropriate timeouts.

### Throughput Expectations

A typical export processes catalog rows at approximately the following rates:

| Storage Backend | Rows per Second | Notes |
|----------------|----------------|-------|
| Local filesystem | 500,000+ | Limited by CPU and disk I/O |
| S3 / GCS / Azure | 50,000–200,000 | Limited by read latency and SST block fetches |
| MinIO (local) | 200,000–400,000 | Network-local object store |

For a catalog with 100,000 rows (representing perhaps 500 tables with columns, files, and statistics), a full export typically completes in under 2 seconds on cloud storage. Extremely large catalogs with millions of registered data files may take 10–30 seconds.

### Memory Usage

Export streams rows through memory without buffering the entire dataset. Peak memory usage is approximately:

- Base overhead: ~50 MB (SlateDB read state, SST block cache)
- Per-row: negligible (each row is serialized and written immediately)
- Output buffering: 64 KB write buffer for the output file

This means export works well even on memory-constrained environments like AWS Lambda or small containers.

### Concurrent Access

Export is a read-only operation that uses a consistent snapshot. It does not interfere with concurrent writes to the catalog. Multiple exports can run simultaneously, each reading from its own snapshot without coordination. The only shared resource is the object store read bandwidth — running many exports concurrently may increase latency for all of them.

## Automation and Scheduling

### Cron-Based Backup

A common pattern is scheduling daily exports via cron or a cloud scheduler:

```bash
#!/bin/bash
# /usr/local/bin/rocklake-backup.sh
set -euo pipefail

STORAGE="s3://production/catalog/"
BACKUP_BUCKET="s3://backups/rocklake/"
DATE=$(date +%Y-%m-%d-%H%M%S)
OUTPUT="${BACKUP_BUCKET}${DATE}.ndjson"

# Perform the export
rocklake export --catalog "$STORAGE" --output "$OUTPUT"

# Verify the export is non-empty
SIZE=$(aws s3 ls "$OUTPUT" | awk '{print $3}')
if [ "$SIZE" -lt 100 ]; then
    echo "ERROR: Export appears empty (${SIZE} bytes)" >&2
    exit 1
fi

echo "Export completed: ${OUTPUT} (${SIZE} bytes)"
```

### Retention Policy

Combine exports with lifecycle policies to manage storage costs:

```json
{
  "Rules": [
    {
      "ID": "RocklakeBackupRetention",
      "Status": "Enabled",
      "Filter": { "Prefix": "rocklake/" },
      "Transitions": [
        { "Days": 30, "StorageClass": "STANDARD_IA" },
        { "Days": 90, "StorageClass": "GLACIER" }
      ],
      "Expiration": { "Days": 365 }
    }
  ]
}
```

This keeps recent exports readily accessible while archiving older ones for compliance.

### CI/CD Integration

Export before deployments to create restore points:

```yaml
# GitHub Actions example
- name: Pre-deploy catalog backup
  run: |
    rocklake export \
      --catalog s3://production/catalog/ \
      --output s3://backups/pre-deploy/${{ github.sha }}.ndjson
    
- name: Deploy new version
  run: ./deploy.sh

- name: Verify catalog health
  run: rocklake verify --catalog s3://production/catalog/
```

## Troubleshooting Export Issues

### Export Hangs or Times Out

If an export appears to hang, the most common causes are:

- **Network connectivity**: The process cannot reach the object store. Verify with `aws s3 ls s3://bucket/catalog/` or equivalent.
- **Large SST files**: If SlateDB has not run compaction recently, it may need to read many small files. Run garbage collection first.
- **Snapshot resolution**: Specifying `--at-time` requires scanning the snapshot history to find the corresponding snapshot ID. For catalogs with millions of snapshots, this can take time.

### Empty Export

An export that produces zero rows usually means:

- The storage path is wrong (points to an empty or non-existent catalog)
- The specified snapshot or time predates any catalog activity
- All data was excised before the target snapshot

Verify with `rocklake inspect --catalog s3://bucket/catalog/` to confirm the catalog exists and has data at the expected snapshot.

### Partial Export (Incomplete Output)

If the output file exists but appears truncated:

- Check disk space on the output target
- Verify the process was not killed (OOM, timeout)
- Re-run with `--output -` (stdout) to isolate output I/O issues from catalog reading issues

## Security Considerations

Export files contain the full catalog metadata, which may include:

- Table and column names (business-sensitive schema information)
- File paths to data lake objects (location of sensitive data)
- Snapshot history (temporal access patterns)

Protect export files with appropriate access controls:

```bash
# Encrypt export at rest
rocklake export --catalog s3://bucket/catalog/ | gpg --encrypt --recipient ops@company.com > catalog.ndjson.gpg

# Use server-side encryption for S3 output
rocklake export --catalog s3://bucket/catalog/ --output s3://backup-bucket/catalog.ndjson --sse aws:kms --sse-kms-key-id alias/backup-key
```

Export files should be treated with the same security classification as the catalog itself — anyone who can read the export can reconstruct the complete catalog structure.

## Performance

| Catalog Size | Export Time | Output Size |
|-------------|-------------|-------------|
| 10 tables, 50 columns | <1 second | 10–50 KB |
| 100 tables, 500 columns | 1–3 seconds | 200 KB – 1 MB |
| 1,000 tables, 5,000 columns | 5–15 seconds | 2–10 MB |
| 10,000 tables, 50,000 columns | 30–120 seconds | 20–100 MB |

Export performance is dominated by the prefix scan of all catalog keys. Larger catalogs require more SST block reads from object storage.

### Compression

For large exports, pipe through compression:

```bash
rocklake export --catalog s3://bucket/catalog/ | gzip > catalog.ndjson.gz
```

NDJSON compresses well (70–80% reduction) because of repetitive field names.

## Error Handling

If export encounters an error mid-stream:

- **Storage error:** Export retries (3 attempts) then fails. Partial output file should be discarded.
- **Corrupt row:** Export logs a warning and skips the row. The output contains a comment line: `# SKIPPED: corrupt row at key t/5/v/300`
- **Timeout:** For very large catalogs, ensure adequate timeout (600+ seconds).

## Further Reading

- **[Backup & Restore](backup-restore.md)** — Complete backup strategy
- **[Inspect](inspect.md)** — Examining individual catalog entries
- **[Garbage Collection](garbage-collection.md)** — How retention affects exportable data
