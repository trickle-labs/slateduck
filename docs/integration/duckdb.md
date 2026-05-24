# DuckDB Integration

DuckDB is the primary client for SlateDuck. The integration uses DuckDB's `ducklake` extension, which connects to SlateDuck over the PostgreSQL wire protocol to manage lakehouse catalog metadata while executing analytical queries locally against Parquet data files in object storage. From DuckDB's perspective, SlateDuck is indistinguishable from a PostgreSQL-backed DuckLake catalog — the same SQL works, the same operations are supported, and the same transactional guarantees apply.

This page covers the complete DuckDB integration: installation, connection configuration, the full lifecycle of catalog operations, performance characteristics, multi-instance topologies, and operational best practices.

## Prerequisites

- **DuckDB v1.2.0 or later** (the `ducklake` extension was introduced in this version)
- **A running SlateDuck instance** (see [Deployment](../deployment/index.md))
- **Network connectivity** between DuckDB and SlateDuck (typically localhost or same-VPC)
- **Object storage credentials** configured for DuckDB (for reading/writing Parquet files)

## Installation

```sql
-- Install the ducklake extension (first time only)
INSTALL ducklake;

-- Load it into the current session
LOAD ducklake;
```

The extension is part of DuckDB's official extension repository and installs automatically from the DuckDB extension hub.

## Connecting to SlateDuck

### Basic Connection

```sql
-- Attach a SlateDuck catalog (local sidecar)
ATTACH 'ducklake:host=localhost;port=5432' AS my_lake;

-- Use the catalog as the default
USE my_lake;
```

### Connection with Authentication

```sql
-- With password authentication
ATTACH 'ducklake:host=slateduck.internal;port=5432;password=my-secret-token' AS my_lake;
```

### Connection with TLS

```sql
-- Require TLS (recommended for non-localhost connections)
ATTACH 'ducklake:host=slateduck.prod.internal;port=5432;sslmode=require' AS my_lake;

-- Full certificate verification
ATTACH 'ducklake:host=slateduck.prod.internal;port=5432;sslmode=verify-full;sslrootcert=/path/to/ca.pem' AS my_lake;
```

### Connection String Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `host` | `localhost` | SlateDuck server hostname or IP address |
| `port` | `5432` | SlateDuck server port |
| `password` | (none) | Authentication token (if configured on server) |
| `sslmode` | `prefer` | TLS mode: `disable`, `prefer`, `require`, `verify-ca`, `verify-full` |
| `connect_timeout` | `10` | Connection timeout in seconds |
| `application_name` | `duckdb` | Identifies this client in SlateDuck's logs |

### Multiple Catalogs

You can attach multiple SlateDuck catalogs simultaneously:

```sql
-- Attach production and staging catalogs
ATTACH 'ducklake:host=prod-slateduck;port=5432' AS prod;
ATTACH 'ducklake:host=staging-slateduck;port=5432' AS staging;

-- Query across catalogs
SELECT p.event_count, s.event_count 
FROM prod.analytics.daily_counts p
JOIN staging.analytics.daily_counts s ON p.date = s.date
WHERE p.event_count != s.event_count;
```

## How It Works Under the Hood

When you execute DDL or DML through DuckDB against a DuckLake-attached catalog, the following sequence occurs:

### Read Path (SELECT)

```mermaid
sequenceDiagram
    participant DuckDB
    participant DuckLake Extension
    participant SlateDuck
    participant Object Storage
    
    DuckDB->>DuckLake Extension: Plan query
    DuckLake Extension->>SlateDuck: List schemas (PG wire)
    SlateDuck-->>DuckLake Extension: Schema list
    DuckLake Extension->>SlateDuck: Get table metadata
    SlateDuck-->>DuckLake Extension: Table definition + columns
    DuckLake Extension->>SlateDuck: List data files
    SlateDuck-->>DuckLake Extension: File paths + statistics
    DuckLake Extension->>DuckDB: Execution plan with file list
    DuckDB->>Object Storage: Read Parquet files directly
    Object Storage-->>DuckDB: Data
    DuckDB-->>DuckDB: Execute query locally
```

### Write Path (INSERT / CREATE TABLE)

```mermaid
sequenceDiagram
    participant DuckDB
    participant DuckLake Extension
    participant SlateDuck
    participant Object Storage
    
    DuckDB->>Object Storage: Write Parquet file
    Object Storage-->>DuckDB: File written (path, size, stats)
    DuckDB->>DuckLake Extension: Register file in catalog
    DuckLake Extension->>SlateDuck: BEGIN + INSERT file metadata
    SlateDuck-->>DuckLake Extension: OK
    DuckLake Extension->>SlateDuck: COMMIT
    SlateDuck-->>DuckLake Extension: Snapshot ID
```

**Key insight:** SlateDuck never sees your actual data. It only manages metadata — which tables exist, what columns they have, where the data files are stored, and what statistics describe them. DuckDB reads and writes data files directly in object storage.

## Supported Operations

### Schema Management

```sql
-- Create schemas
CREATE SCHEMA analytics;
CREATE SCHEMA staging;
CREATE SCHEMA IF NOT EXISTS archive;

-- Drop schemas
DROP SCHEMA staging CASCADE;  -- Drops all contained tables

-- Rename schemas
ALTER SCHEMA analytics RENAME TO reporting;
```

### Table Management

```sql
-- Create tables with all DuckDB types
CREATE TABLE analytics.events (
    event_id BIGINT,
    event_type VARCHAR,
    user_id INTEGER,
    timestamp TIMESTAMP WITH TIME ZONE,
    properties JSON,
    amount DECIMAL(18, 4),
    tags VARCHAR[],
    is_active BOOLEAN
);

-- Alter tables
ALTER TABLE analytics.events ADD COLUMN source VARCHAR DEFAULT 'unknown';
ALTER TABLE analytics.events DROP COLUMN is_active;
ALTER TABLE analytics.events RENAME COLUMN properties TO metadata;

-- Drop tables
DROP TABLE analytics.events;

-- Move tables between schemas
ALTER TABLE analytics.events SET SCHEMA archive;
```

### Data Operations

```sql
-- Insert data (DuckDB writes Parquet, then registers the file)
INSERT INTO analytics.events 
SELECT * FROM read_parquet('s3://incoming/events-2024-03-15.parquet');

-- Copy from local files
COPY analytics.events FROM 'local-data/*.parquet';

-- Query with full DuckDB SQL
SELECT event_type, count(*), avg(amount)
FROM analytics.events
WHERE timestamp > '2024-03-01'
GROUP BY event_type
ORDER BY count(*) DESC;
```

### Time Travel

```sql
-- Read catalog at a specific snapshot
SELECT * FROM analytics.events AT SNAPSHOT 1000;

-- Read catalog at a specific timestamp
SELECT * FROM analytics.events AT TIMESTAMP '2024-03-01 00:00:00';

-- Compare current vs historical state
SELECT 'current' as version, count(*) FROM analytics.events
UNION ALL
SELECT 'march-1' as version, count(*) FROM analytics.events AT SNAPSHOT 500;
```

### Transaction Control

```sql
-- Explicit transactions for atomic multi-table operations
BEGIN TRANSACTION;
CREATE TABLE analytics.new_events LIKE analytics.events;
INSERT INTO analytics.new_events SELECT * FROM analytics.events WHERE timestamp > '2024-03-01';
DROP TABLE analytics.events;
ALTER TABLE analytics.new_events RENAME TO events;
COMMIT;
```

## Performance Characteristics

### Catalog Overhead Per Query

Each DuckDB query that touches a SlateDuck-backed table involves multiple catalog round-trips:

| Operation | Round-trips | Typical Latency |
|-----------|-------------|-----------------|
| List schemas | 1 | 1–5ms |
| Get table metadata | 1 | 1–5ms |
| List columns | 1 | 1–5ms |
| List data files | 1 | 1–10ms (depends on file count) |
| Get column statistics | 1 | 1–5ms |
| **Total catalog overhead** | **~5** | **5–30ms** |

For analytical queries scanning gigabytes of Parquet data (seconds to minutes of execution), 5–30ms of catalog overhead is negligible. For interactive OLAP dashboards with sub-second query expectations, this overhead is more noticeable.

### Reducing Catalog Overhead

| Approach | Effect | Trade-off |
|----------|--------|-----------|
| Co-locate in same AZ | Reduces round-trip to <1ms | Must deploy together |
| S3 Express One Zone | Reduces SlateDB reads to <5ms | Higher storage cost |
| Native extension (Strategy C) | Eliminates network entirely | Process coupling |
| Fewer data files per table | Fewer entries to list | Larger individual files |

### Data Path Performance

Data operations (reading/writing Parquet files) bypass SlateDuck entirely. Performance is determined by:

- DuckDB's query engine (parallelism, vectorized execution)
- Object storage throughput (S3 bandwidth: ~100 MB/s per connection, parallelizable)
- Parquet file organization (row groups, column pruning, partition structure)

## Multiple DuckDB Instances

### Read Scaling

Multiple DuckDB instances can connect to the same SlateDuck catalog simultaneously for read operations. Each instance:

- Gets a consistent snapshot view of the catalog
- Reads data files directly from object storage (no bottleneck through SlateDuck)
- Scales horizontally without catalog coordination

```
DuckDB Instance 1 ──┐
DuckDB Instance 2 ──┼──→ SlateDuck ──→ SlateDB (S3)
DuckDB Instance 3 ──┘
     │
     └──→ Read Parquet directly from S3 (parallel)
```

### Write Coordination

Only one DuckDB instance at a time should perform write operations. This is enforced by DuckLake's transaction semantics — if two writers attempt concurrent catalog mutations, one will fail with a conflict error and should retry.

In practice, most architectures designate a single "writer" DuckDB instance (typically the ETL pipeline) while all other instances are read-only consumers.

### Connection Pooling

For deployments with many DuckDB instances, SlateDuck handles connections efficiently:

- Each connection is lightweight (a few KB of state)
- Connections are stateless between queries
- No connection limit in SlateDuck (limited only by OS file descriptors)

## Object Storage Configuration

DuckDB needs its own object storage credentials to read/write data files:

```sql
-- Configure S3 credentials for DuckDB
SET s3_region = 'us-east-1';
SET s3_access_key_id = 'AKIAIOSFODNN7EXAMPLE';
SET s3_secret_access_key = 'wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY';

-- Or use instance profile (recommended for AWS deployments)
SET s3_use_ssl = true;

-- Then attach and query
ATTACH 'ducklake:host=localhost;port=5432' AS lake;
SELECT * FROM lake.analytics.events;
```

## Error Handling

### Common Errors

| Error | SQLSTATE | Cause | Solution |
|-------|----------|-------|----------|
| Connection refused | 08001 | SlateDuck not running | Start the server |
| WriterFenced | 57P04 | Another instance claimed the catalog | Reconnect (new instance handles requests) |
| SnapshotNotFound | 22023 | Requested snapshot has been GC'd | Use a more recent snapshot |
| TransactionConflict | 40001 | Concurrent write conflict | Retry the transaction |
| UnsupportedSQL | 42601 | SQL not recognized by bounded dispatcher | Check DuckDB/ducklake version compatibility |

### Retry Pattern

```sql
-- DuckDB does not have built-in retry, but applications can wrap:
-- Pseudo-code for application layer
-- while retries < 3:
--     try: execute_query()
--     except TransactionConflict: retries++; sleep(backoff)
```

## Real-World Workflow Examples

### ETL Pipeline: Ingest Daily Parquet Export

A common pattern is a nightly job that deposits Parquet files into an S3 prefix and registers them with SlateDuck so analysts can immediately query them:

```sql
LOAD ducklake;
ATTACH 'ducklake:host=slateduck.internal;port=5432' AS lake;
USE lake;

-- Create the target table if it doesn't exist
CREATE TABLE IF NOT EXISTS analytics.daily_events (
    event_date DATE,
    event_type VARCHAR,
    user_id BIGINT,
    amount DECIMAL(18, 4),
    region VARCHAR
);

-- Register yesterday's export (DuckDB writes Parquet to S3, then registers metadata with SlateDuck)
INSERT INTO analytics.daily_events
SELECT * FROM read_parquet('s3://data-lake/exports/events_2024_03_15.parquet');

-- Verify the load
SELECT count(*) FROM analytics.daily_events WHERE event_date = '2024-03-15';
```

After the `INSERT`, the new Parquet file is immediately visible to all other DuckDB instances connected to the same SlateDuck catalog. The analyst's session that was already running sees the new data without reconnecting — snapshot isolation ensures they see a consistent point in time, and advancing to the latest snapshot is as simple as re-running the query.

### Schema Evolution: Adding a Column

DuckLake supports adding columns without rewriting existing data files. Old files return `NULL` for the new column; new files contain actual values:

```sql
-- Add a new column to an existing table
ALTER TABLE analytics.daily_events ADD COLUMN campaign_id VARCHAR;

-- Old Parquet files: campaign_id will be NULL
-- New inserts can include campaign_id values
INSERT INTO analytics.daily_events
SELECT *, 'summer-2024' AS campaign_id
FROM read_parquet('s3://data-lake/exports/events_2024_06_01.parquet');

-- Query works across old and new files
SELECT campaign_id, count(*)
FROM analytics.daily_events
GROUP BY campaign_id
ORDER BY count(*) DESC;
-- campaign_id = NULL for rows from old files
-- campaign_id = 'summer-2024' for rows from the new file
```

### Time Travel Audit Query

A compliance team needs to verify what the catalog state looked like at the start of Q2 2024:

```sql
LOAD ducklake;
ATTACH 'ducklake:host=slateduck.internal;port=5432' AS lake;

-- Find the snapshot closest to April 1, 2024
-- (In practice you'd look up the snapshot ID from your audit log or a checkpoint)

-- Query the table as it was at snapshot 500
SELECT *
FROM lake.analytics.daily_events AT SNAPSHOT 500
LIMIT 10;

-- Compare row counts across snapshots
SELECT 
    'April 1' as period, count(*) as rows
FROM lake.analytics.daily_events AT SNAPSHOT 500
UNION ALL
SELECT 
    'July 1' as period, count(*) as rows
FROM lake.analytics.daily_events AT SNAPSHOT 1200;
```

This does not require any backup restoration, any extra infrastructure, or any special tools — it is just SQL with a snapshot qualifier.

### Snapshot Pinning for Long-Running Reports

If you are running a multi-hour report and want a consistent catalog view even as new data arrives, pin your session to a specific snapshot:

```sql
-- Start your session pinned to the current latest snapshot
-- (prevents catalog changes from affecting this query)
SET ducklake.snapshot = 1247;

-- Now run your long query — catalog state is frozen at snapshot 1247
SELECT region, sum(amount) as total_revenue
FROM analytics.daily_events
GROUP BY region;

-- Even if new files are registered during this query,
-- this session won't see them
```

## Operational Best Practices

### Keep DuckDB and SlateDuck Versions Aligned

The DuckLake wire protocol evolves with DuckDB releases. The `ducklake` extension in DuckDB v1.3 may emit SQL patterns that SlateDuck's bounded dispatcher does not recognize if built against the v1.2 protocol. Always test version upgrades in staging before production.

### Minimize ATTACH/DETACH Overhead

Each `ATTACH` call opens a new connection to SlateDuck and reads the catalog manifest. For interactive sessions or dashboards with per-query connections, reuse a long-lived DuckDB connection with a cached `ATTACH` rather than attaching and detaching on every query.

### Partition Large Tables

Very large tables with millions of Parquet files create long `file listing` catalog queries. Partition data files by date or region so that queries with date/region filters only need to list files in the relevant partitions. SlateDuck's file listing is scoped to the queried partition.

### Monitor Catalog Size

As data accumulates over time, the catalog itself grows. Run GC regularly to advance the `retain-from` horizon and run `excise` to physically remove superseded catalog entries:

```bash
# Monthly GC and excision
slateduck gc apply --catalog s3://my-bucket/catalog/ --retain-days 90
slateduck excise plan --catalog s3://my-bucket/catalog/
slateduck excise apply --catalog s3://my-bucket/catalog/ --yes
```

A healthy catalog for most workloads stays under 100 MB of SST files with regular GC.

## Troubleshooting

### "Cannot attach: connection refused"

```bash
# Is SlateDuck running?
ps aux | grep slateduck

# Is it listening on the expected port?
nc -zv localhost 5432
```

### "Slow queries after ATTACH"

This typically means high network latency between DuckDB and SlateDuck. Verify they are in the same region/AZ:

```bash
# Measure latency
time psql -h <slateduck-host> -p 5432 -c "SELECT 1"
```

### "Missing tables/schemas"

Check that you are reading at the correct snapshot and that GC has not removed the target:

```sql
-- What snapshot are we at?
SELECT current_setting('ducklake.snapshot');
```

## Further Reading

- **[DuckDB Compatibility](duckdb-compatibility.md)** — Full SQL compatibility matrix
- **[Native Extension](native-extension.md)** — In-process alternative (Strategy C)
- **[Configuration](../deployment/configuration.md)** — Server-side configuration
- **[Troubleshooting](../operations/troubleshooting.md)** — Comprehensive problem resolution
