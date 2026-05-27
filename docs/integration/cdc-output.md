# CDC Output (Change Data Capture)

Rocklake v0.10 adds CDC export: every committed snapshot is a natural change
stream.  The diff between snapshots `S_n` and `S_{n+1}` is the set of catalog
facts with `begin_snapshot = S_{n+1}` (newly added) or `end_snapshot = S_{n+1}`
(retired).

## Architecture

```
Rocklake catalog
     │ commit snapshot
     ▼
CatalogReader::snapshot_diff(from, to)
     │
     ├── S3 CDC files  →  {warehouse}/cdc/snapshot-{id}.jsonl
     │
     ├── CdcTailer     →  Kafka / NATS topic
     │
     └── WebhookPayload →  HTTP POST to configurable URL
```

## snapshot_diff API

```rust
use rocklake_catalog::{CatalogStore, SnapshotDiff};
use rocklake_core::mvcc::SnapshotId;

let reader = store.read_at(to_snapshot)?;
let diff: SnapshotDiff = reader
    .snapshot_diff(from_snapshot, to_snapshot)
    .await?;

println!("Added tables:    {}", diff.added_tables.len());
println!("Retired tables:  {}", diff.retired_tables.len());
println!("New data files:  {}", diff.added_data_files.len());
println!("Total changes:   {}", diff.change_count());
```

### Fields

| Field | Description |
|-------|-------------|
| `added_schemas` | Schema rows first written at `to_snapshot` |
| `retired_schemas` | Schema rows retired at `to_snapshot` |
| `added_tables` | Table rows first written at `to_snapshot` |
| `retired_tables` | Table rows retired at `to_snapshot` |
| `added_columns` | Column rows first written at `to_snapshot` |
| `retired_columns` | Column rows retired at `to_snapshot` |
| `added_data_files` | Data files registered at `to_snapshot` |

## S3 CDC Files

Per-snapshot JSON-lines diff files are written under:

```
{warehouse}/cdc/snapshot-{snapshot_id:020}.jsonl
```

### Format

Each file begins with a header line followed by one event per line:

```jsonl
{"_type":"cdc_snapshot_header","from_snapshot":4,"to_snapshot":5,"event_count":3}
{"snapshot_id":5,"table":"ducklake_table","kind":"add","row":{"table_id":7,...}}
{"snapshot_id":5,"table":"ducklake_data_file","kind":"add","row":{"data_file_id":12,...}}
{"snapshot_id":5,"table":"ducklake_column","kind":"add","row":{"column_id":15,...}}
```

### Writing CDC files

```rust
use rocklake_catalog::cdc::{CdcSnapshot, cdc_s3_path, write_cdc_jsonl};

let diff = reader.snapshot_diff(prev_snap, curr_snap).await?;
let cdc = CdcSnapshot::from_diff(&diff);

// Write to a Vec<u8> (replace with object_store put in production)
let path = cdc_s3_path("s3://bucket/warehouse", curr_snap.as_u64());
let mut buf = Vec::new();
write_cdc_jsonl(&cdc, &mut buf)?;
// object_store.put(&path.into(), buf.into()).await?;
```

## CDC Tailer (`rocklake-cdc` sidecar)

The `CdcTailer` polls the catalog at a configured interval and exports each new
snapshot diff.

```rust
use rocklake_catalog::cdc::CdcTailer;
use rocklake_core::mvcc::SnapshotId;

let mut tailer = CdcTailer::new(
    SnapshotId::new(0),     // start from the beginning
    "s3://bucket/warehouse",
);

loop {
    if let Some(cdc) = tailer.poll_once(&store).await? {
        // New snapshot diff available
        let path = rocklake_catalog::cdc::cdc_s3_path(
            &tailer.warehouse_prefix,
            cdc.to_snapshot,
        );
        let mut buf = Vec::new();
        rocklake_catalog::cdc::write_cdc_jsonl(&cdc, &mut buf)?;
        // Publish buf to Kafka/NATS or upload to S3
        println!("Exported {} events for snapshot {}", cdc.events.len(), cdc.to_snapshot);
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}
```

### Kafka CDC Producer

```rust
// In production: send cdc.to_jsonl() to a Kafka topic
let jsonl = cdc.to_jsonl();
kafka_producer
    .send(
        FutureRecord::to("rocklake.cdc")
            .key(&cdc.to_snapshot.to_string())
            .payload(jsonl.as_bytes()),
        Timeout::Never,
    )
    .await?;
```

### NATS CDC Publisher

```rust
let jsonl = cdc.to_jsonl();
nats_client
    .publish(
        format!("rocklake.cdc.{}", cdc.to_snapshot),
        jsonl.as_bytes(),
    )
    .await?;
```

## Webhook CDC

The `WebhookPayload` struct provides a lightweight JSON payload for HTTP
webhook delivery:

```rust
use rocklake_catalog::cdc::{CdcSnapshot, WebhookPayload};

let cdc = CdcSnapshot::from_diff(&diff);
let payload = WebhookPayload::from_cdc(
    &cdc,
    "https://s3.example.com/cdc/snapshot-5.jsonl",
);

// payload.snapshot_id == 5
// payload.affected_tables == [table_id, ...]
// payload.event_count == N
// payload.diff_url == "https://s3.example.com/cdc/snapshot-5.jsonl"

// HTTP POST (using reqwest or similar)
reqwest::Client::new()
    .post("https://my-service/cdc-hook")
    .json(&payload)
    .send()
    .await?;
```

## S3-Polling Pattern

Downstream consumers can poll for new CDC files using S3 event notifications or
simply listing the `cdc/` prefix:

```sql
-- In DuckDB: read all CDC files for a table
SELECT * FROM read_json_auto('s3://bucket/warehouse/cdc/snapshot-*.jsonl')
WHERE table = 'ducklake_data_file'
  AND kind = 'add'
  AND (row->>'table_id')::BIGINT = 42;
```

## Event Types

| `table` value | `kind` | Description |
|---------------|--------|-------------|
| `ducklake_schema` | `add` | New schema created |
| `ducklake_schema` | `retire` | Schema dropped |
| `ducklake_table` | `add` | New table created |
| `ducklake_table` | `retire` | Table dropped |
| `ducklake_column` | `add` | Column added |
| `ducklake_column` | `retire` | Column dropped |
| `ducklake_data_file` | `add` | New Parquet file registered |

## Examples

### Kafka Sink for Order Events

```
Kafka "orders" topic
  → pg-tide-relay
  → RocklakeSink (commit_batch with exactly-once offset)
  → DuckLake snapshot committed
  → CdcTailer polls
  → Publishes to Kafka "cdc.orders" topic
  → Downstream analytics service receives diff
```

### S3-Polling for Lambda Triggers

```
Rocklake commits snapshot
  → CdcTailer writes snapshot-N.jsonl to S3
  → S3 event notification triggers Lambda
  → Lambda reads diff and fans out to DynamoDB / SQS
```
