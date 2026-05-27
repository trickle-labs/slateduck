# Inspect

The inspect command provides a comprehensive summary of a catalog's internal state without requiring a DuckDB connection. It reads system keys, entity counts, snapshot metadata, and storage configuration directly from SlateDB, presenting them in a human-readable format. Inspect is the first tool you should reach for when diagnosing operational issues, verifying deployments, or simply understanding what a catalog contains.

Think of inspect as Rocklake's equivalent of `SHOW STATUS` in MySQL or `pg_stat_activity` in PostgreSQL — a quick health check that reveals the catalog's vital signs.

## Basic Usage

```bash
# Inspect a catalog on S3
rocklake inspect --catalog s3://bucket/catalog/

# Inspect a local catalog
rocklake inspect --catalog ./local-catalog/

# Inspect with JSON output (for scripts)
rocklake inspect --catalog s3://bucket/catalog/ --format json

# Inspect at a specific snapshot (historical state)
rocklake inspect --catalog s3://bucket/catalog/ --at-snapshot 500
```

## Output

### Human-Readable Format

```
Rocklake Catalog Inspection
════════════════════════════════════════════════════════════════
Storage:           s3://my-bucket/lakehouse/catalog/
Format Version:    1
Writer Epoch:      3
Writer Fenced:     No (this instance holds the lease)
Latest Snapshot:   1,247

Schema Version:    1
Catalog Version:   DuckLake 0.1.0 compatible

Entity Counts (at latest snapshot):
  Databases:       1
  Schemas:         4
  Tables:          23
  Columns:         187
  Data Files:      1,892
  Delete Files:    12
  Views:           3
  Sequences:       0
  Inlined Inserts: 0

Historical Versions:
  Total versioned rows: 3,412
  Superseded rows:      1,267 (37%)
  Rows beyond GC horizon: 890

Counters:
  Next Snapshot ID:  1,248
  Next Catalog ID:   215
  Next File ID:      1,905

Retention:
  Retain From:     1,100 (snapshots 1–1,099 are GC'd)
  Pinned Snapshots: [none]
  Oldest accessible: snapshot 1,100 (2024-02-15T08:30:00Z)

Storage Details:
  SST Files:       47
  Manifest Size:   12.3 KB
  Estimated Total: 4.7 MB

Last Snapshot:
  ID:              1,247
  Time:            2024-03-15T14:30:22Z
  Author:          etl-pipeline
  Message:         "Registered 15 new data files for orders table"
  Tables Modified: [orders]

════════════════════════════════════════════════════════════════
Inspection completed in 320ms
```

### JSON Format

For programmatic consumption:

```bash
rocklake inspect --catalog s3://bucket/catalog/ --format json
```

```json
{
  "storage": "s3://my-bucket/lakehouse/catalog/",
  "format_version": 1,
  "writer_epoch": 3,
  "writer_fenced": false,
  "latest_snapshot": 1247,
  "schema_version": 1,
  "counts": {
    "databases": 1,
    "schemas": 4,
    "tables": 23,
    "columns": 187,
    "data_files": 1892,
    "delete_files": 12,
    "views": 3,
    "sequences": 0,
    "inlined_inserts": 0
  },
  "counters": {
    "next_snapshot_id": 1248,
    "next_catalog_id": 215,
    "next_file_id": 1905
  },
  "retention": {
    "retain_from": 1100,
    "pinned_snapshots": [],
    "oldest_accessible_time": "2024-02-15T08:30:00Z"
  },
  "last_snapshot": {
    "id": 1247,
    "time": "2024-03-15T14:30:22Z",
    "author": "etl-pipeline",
    "message": "Registered 15 new data files for orders table"
  }
}
```

## Field Reference

### System Fields

| Field | Description | Typical Values |
|-------|-------------|----------------|
| Format Version | Catalog schema version. Determines binary compatibility. | `1` (currently only version) |
| Writer Epoch | Increments each time a new writer claims the catalog. | 1–10 for stable deployments, higher means frequent restarts |
| Writer Fenced | Whether the current instance has been fenced by a newer writer. | `No` for the active writer |
| Latest Snapshot | Highest committed snapshot ID. This is the "current" state. | Monotonically increasing |
| Schema Version | Internal schema version for protobuf encoding. | `1` |

### Entity Counts

Entity counts reflect the state at the latest snapshot (or the specified `--at-snapshot`). They include only live (visible) rows — superseded historical versions are not counted.

| Entity | Description |
|--------|-------------|
| Databases | DuckLake database records |
| Schemas | SQL schemas (analogous to PostgreSQL schemas) |
| Tables | User-created tables tracked by the catalog |
| Columns | Total columns across all tables |
| Data Files | Parquet/CSV files registered in the catalog |
| Delete Files | Files tracking row-level deletes |
| Views | Registered views |
| Sequences | Auto-increment sequences |
| Inlined Inserts | Rows stored directly in the catalog (small inserts) |

### Counters

Counters represent the next value that will be allocated. They should always be strictly greater than any existing ID of the same type. If a counter is <= an existing ID, this indicates corruption (the verify command will catch this).

### Retention

| Field | Description |
|-------|-------------|
| Retain From | Snapshot ID below which time travel is unavailable. Set by GC. |
| Pinned Snapshots | Snapshots protected from GC advancement. |
| Oldest Accessible | Timestamp of the oldest snapshot still available for time travel. |

## Inspecting Specific Keys

For low-level debugging, inspect individual keys:

```bash
# Look up a specific table's current version
rocklake inspect --catalog s3://bucket/catalog/ --key "t/5/latest"

# Look up a specific historical version
rocklake inspect --catalog s3://bucket/catalog/ --key "t/5/v/300"

# Look up system keys
rocklake inspect --catalog s3://bucket/catalog/ --key "sys/epoch"
rocklake inspect --catalog s3://bucket/catalog/ --key "sys/format_version"
rocklake inspect --catalog s3://bucket/catalog/ --key "sys/retain_from"
```

Output for key inspection:

```
Key: t/5/latest
  Table:     ducklake_tables
  Decoded:   table_id=5, table_name="events", schema_id=1, 
             table_uuid="550e8400-...", created_snapshot_id=10
  Raw Size:  142 bytes
  Encoding:  protobuf v1
```

### Prefix Scans

List all keys under a prefix:

```bash
# List all versions of table 5
rocklake inspect --catalog s3://bucket/catalog/ --prefix "t/5/"

# List all column entries for table 5
rocklake inspect --catalog s3://bucket/catalog/ --prefix "c/5/"

# List all system keys
rocklake inspect --catalog s3://bucket/catalog/ --prefix "sys/"
```

## Operational Patterns

### Health Check Script

```bash
#!/bin/bash
# health-check.sh - Returns exit code 0 if catalog is healthy

OUTPUT=$(rocklake inspect --catalog "$CATALOG_URL" --format json 2>&1)
if [ $? -ne 0 ]; then
    echo "CRITICAL: Cannot reach catalog"
    exit 2
fi

EPOCH=$(echo "$OUTPUT" | jq '.writer_epoch')
FENCED=$(echo "$OUTPUT" | jq '.writer_fenced')
LATEST=$(echo "$OUTPUT" | jq '.latest_snapshot')

if [ "$FENCED" = "true" ]; then
    echo "WARNING: This instance is fenced (epoch $EPOCH)"
    exit 1
fi

echo "OK: Catalog healthy, snapshot $LATEST, epoch $EPOCH"
exit 0
```

### Monitoring Integration

Feed inspect output into Prometheus/Datadog:

```bash
# Emit metrics in StatsD format
OUTPUT=$(rocklake inspect --catalog s3://bucket/catalog/ --format json)
echo "rocklake.latest_snapshot:$(echo $OUTPUT | jq '.latest_snapshot')|g" | nc -u -w1 localhost 8125
echo "rocklake.writer_epoch:$(echo $OUTPUT | jq '.writer_epoch')|g" | nc -u -w1 localhost 8125
echo "rocklake.tables:$(echo $OUTPUT | jq '.counts.tables')|g" | nc -u -w1 localhost 8125
echo "rocklake.data_files:$(echo $OUTPUT | jq '.counts.data_files')|g" | nc -u -w1 localhost 8125
```

### Deployment Verification

After deploying a new Rocklake instance:

```bash
# Verify the new instance can read the catalog
rocklake inspect --catalog s3://bucket/catalog/

# Compare with expected state
LATEST=$(rocklake inspect --catalog s3://bucket/catalog/ --format json | jq '.latest_snapshot')
if [ "$LATEST" -lt 1000 ]; then
    echo "ERROR: Snapshot too low — possible wrong catalog path"
    exit 1
fi
```

### Debugging Stale Reads

When DuckDB reports stale data:

```bash
# Check what snapshot the catalog is at
rocklake inspect --catalog s3://bucket/catalog/ --format json | jq '.last_snapshot'

# If last_snapshot.time is old, the writer may have stopped
# If last_snapshot.id is current, the reader may be pinned to an old snapshot
```

## Comparing Multiple Catalogs

For multi-region or multi-environment setups:

```bash
# Compare production vs staging
echo "=== Production ==="
rocklake inspect --catalog s3://prod-bucket/catalog/
echo "=== Staging ==="
rocklake inspect --catalog s3://staging-bucket/catalog/
```

## Performance

Inspect reads a small fixed set of system keys plus performs one prefix scan for counts. The operation is lightweight:

| Catalog Size | Inspect Time |
|-------------|-------------|
| Small (< 100 entities) | 100–200ms |
| Medium (100–1,000 entities) | 200–500ms |
| Large (1,000–10,000 entities) | 500ms–2s |
| Very large (10,000+ entities) | 2–5s |

The time is dominated by object storage round-trips, not computation. The prefix scan for entity counts is the most expensive part — it must iterate through all live keys to count them. For very large catalogs, consider using `--skip-counts` to get system state without entity enumeration, which reduces the operation to a handful of point reads completing in under 200ms regardless of catalog size.

## Interpreting Results

### Diagnosing Growth Patterns

The relationship between entity counts and historical versions tells you about catalog churn:

- **High superseded percentage (>50%)**: The catalog has significant history. Schema evolution or frequent ALTER TABLE operations create many superseded rows. If garbage collection is running, this is normal. If not, consider enabling GC to reclaim space.
- **Many data files per table**: Indicates frequent small writes rather than batch operations. Consider whether your ingestion pipeline could batch more aggressively.
- **Inlined inserts > 0**: Small rows are stored directly in the catalog rather than in separate Parquet files. This is efficient for small datasets but should be monitored — many inlined inserts may indicate the inlining threshold is too high.

### Writer Epoch Analysis

The writer epoch indicates how many times ownership has transferred:

- **Epoch 1**: The catalog has only ever had one writer. This is the simplest case.
- **Epoch 2–5**: Normal operational pattern — a few restarts or failovers.
- **Epoch > 10**: May indicate instability. Check whether the deployment is crash-looping or if multiple instances are fighting for the writer lease.
- **Writer Fenced = Yes**: This instance lost its lease. It can still serve reads, but all writes will fail until it re-acquires the lease. The most common cause is a deployment rolling update where the new instance claimed the lease before the old one shut down.

### Retention Window Assessment

The gap between `retain_from` and `latest_snapshot` defines your time-travel window:

```
Time Travel Window = latest_snapshot - retain_from
```

If this window is too narrow, queries with `AT SNAPSHOT` may fail for recent history. If too wide, the catalog accumulates unbounded history, consuming storage and slowing prefix scans.

A healthy retention window is typically 1,000–10,000 snapshots, corresponding to hours or days of operational history depending on write frequency.

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Inspection completed successfully |
| 1 | Catalog exists but has warnings (e.g., stale counters) |
| 2 | Cannot reach storage or catalog does not exist |
| 3 | Format version mismatch (binary too old) |

## Automation Recipes

### Slack Alert on Stale Catalog

```bash
#!/bin/bash
# Alert if no new snapshots in 1 hour
LATEST_TIME=$(rocklake inspect --catalog s3://bucket/catalog/ --format json | jq -r '.last_snapshot.time')
LATEST_EPOCH=$(date -d "$LATEST_TIME" +%s 2>/dev/null || date -j -f "%Y-%m-%dT%H:%M:%SZ" "$LATEST_TIME" +%s)
NOW=$(date +%s)
AGE=$((NOW - LATEST_EPOCH))

if [ "$AGE" -gt 3600 ]; then
    curl -X POST "$SLACK_WEBHOOK" \
        -H 'Content-Type: application/json' \
        -d "{\"text\":\"⚠️ Rocklake catalog stale: last snapshot was ${AGE}s ago\"}"
fi
```

### Capacity Planning Report

```bash
#!/bin/bash
# Weekly capacity report
OUTPUT=$(rocklake inspect --catalog s3://bucket/catalog/ --format json)
TABLES=$(echo "$OUTPUT" | jq '.counts.tables')
FILES=$(echo "$OUTPUT" | jq '.counts.data_files')
VERSIONS=$(echo "$OUTPUT" | jq '.history.total_versioned_rows')

echo "Weekly Capacity Report"
echo "======================"
echo "Tables: $TABLES"
echo "Data Files: $FILES"
echo "Total Versioned Rows: $VERSIONS"
echo "Avg Files per Table: $((FILES / TABLES))"
echo ""
echo "Growth vs Last Week:"
# Compare with last week's report...
```

## Further Reading

- **[Health Checks](health-checks.md)** — Automated health monitoring
- **[Monitoring](monitoring.md)** — Continuous metrics collection
- **[Verify & Repair](verify-repair.md)** — Deep integrity verification
- **[Troubleshooting](troubleshooting.md)** — Diagnosing common issues
