# Streaming Pipeline Architecture

This document describes the end-to-end streaming pipeline when SlateDuck v0.10
(streaming ingest + CDC output) is deployed with v0.11+ IVM.

## Overview

```
                      ┌─────────────────────────────────────────┐
                      │           SlateDuck Node                │
                      │                                         │
Kafka / NATS ─────────►  SlateDuckSink  ─── commit_batch() ──►│
                      │        │                                │
                      │        ▼                                │
                      │  DuckLake Snapshot ◄── atomic tx ──────│
                      │        │                                │
                      │        ├──► IVM Worker (v0.11+)        │
                      │        │       │                        │
                      │        │       ▼                        │
                      │        │  Materialized View Snapshot    │
                      │        │                                │
                      │        ▼                                │
                      │  CdcTailer polls snapshot_diff()        │
                      │        │                                │
                      └────────┼────────────────────────────────┘
                               │
                    ┌──────────┼───────────────────────────────┐
                    │          │    CDC Outputs                │
                    │          ├──► S3: cdc/snapshot-N.jsonl  │
                    │          ├──► Kafka topic               │
                    │          ├──► NATS subject              │
                    │          └──► Webhook HTTP POST         │
                    └──────────────────────────────────────────┘
```

## Component Responsibilities

### SlateDuckSink (v0.10)

Accepts record batches from Kafka/NATS and commits them atomically:
1. Write Parquet files to S3 (outside the catalog)
2. In one SlateDB transaction: register data files + advance consumer offset

**Exactly-once**: if the process dies between steps 1 and 2, the consumer
re-reads from the last committed offset and re-registers the same files.

### CatalogReader::snapshot_diff (v0.10)

Returns the structured diff between two snapshots:
- `added_schemas`, `retired_schemas`
- `added_tables`, `retired_tables`
- `added_columns`, `retired_columns`
- `added_data_files`

### CdcTailer (v0.10)

Polls `snapshot_diff` at a configurable interval and publishes each diff to
one or more CDC targets (S3 files, Kafka, NATS, webhook).

### IVM Worker (v0.11+)

Processes base table updates and updates materialized views within a configured
freshness window.  IVM output snapshots are ordinary DuckLake snapshots, so the
CdcTailer sees them as regular diff events.

## Latency Budget

For the complete pipeline from ingest to downstream CDC delivery:

| Stage | Budget |
|-------|--------|
| Parquet write to S3 | 10–50ms (S3 Standard) |
| Catalog commit (`create_snapshot`) | ≤ 50ms p95 (v0.10 target) |
| IVM processing (v0.11+ dependent) | ≤ freshness target |
| CdcTailer poll interval | 100ms (configurable) |
| CDC S3 write | 10–50ms |
| **Total (ingest → CDC delivery)** | **≤ ingest batch interval + 200ms** |

For a 500ms ingest batch interval, the end-to-end latency is ≤ 700ms to
downstream CDC delivery.  For real-time requirements, reduce the batch interval
and poll interval.

## Data Flow Example

```
t=0ms:   Kafka consumer receives 500 records
t=10ms:  Parquet file written to S3 (10ms S3 put)
t=25ms:  Catalog commit: register data file + advance offset (15ms)
t=125ms: CdcTailer polls: detects new snapshot (100ms interval)
t=135ms: CDC JSONL written to S3 (10ms)
t=135ms: Kafka CDC producer publishes diff to "slateduck.cdc" topic
         Downstream analytics service receives row count change
```

## IVM Integration (v0.11+)

When IVM is deployed alongside streaming ingest, the pipeline extends:

```
Kafka → SlateDuckSink → base table snapshot → IVM Worker → view snapshot → CdcTailer → CDC
```

The IVM worker operates on the same SlateDB instance. Base table updates
trigger incremental view computation. The view snapshot is committed
atomically — its CDC events are indistinguishable from base table events
to downstream consumers that don't filter by table name.

### Consumer Filter Pattern

```sql
-- Subscribe only to materialized view updates
SELECT * FROM cdc_events
WHERE table = 'ducklake_data_file'
  AND (row->>'table_id')::BIGINT IN (
    SELECT table_id FROM ducklake_table
    WHERE table_name LIKE '%_mv' OR table_name LIKE '%_view'
  );
```

## Configuration

### CdcTailer

```toml
[cdc]
# Warehouse prefix for S3 CDC files
warehouse_prefix = "s3://my-bucket/warehouse"

# Poll interval (milliseconds)
poll_interval_ms = 100

# Enable Kafka CDC output
[cdc.kafka]
bootstrap_servers = "kafka:9092"
topic = "slateduck.cdc"

# Enable webhook CDC output
[cdc.webhook]
url = "https://my-service/cdc-hook"
timeout_ms = 5000
```

### SlateDuckSink

```toml
[streaming]
# Application metadata key namespace
offset_key_prefix = "pg_tide.orders-to-lake"

# Ingest batch size
batch_size = 500

# S3 path prefix for Parquet files
parquet_prefix = "s3://my-bucket/warehouse/events"
```
