# Streaming Ingest

Rocklake v0.10 adds a zero-infrastructure streaming ingest path from Kafka or
NATS directly into a DuckLake catalog backed by S3 — with exactly-once delivery
semantics.

## Architecture

```
Kafka / NATS
     │
     ▼
RocklakeSink  ──── Parquet files ──►  S3 bucket
     │
     ▼ (one atomic catalog transaction)
CatalogWriter::set_metadata  ← consumer offset
CatalogWriter::register_inlined_insert / register_data_file
     │
     ▼
DuckLake snapshot committed
```

The `RocklakeSink` connects to the catalog write path (which may sit behind the
PG-wire sidecar via `pg-tide-relay`).  No external database is required.

## Application Metadata Key Namespace

Consumer offsets and other application state are stored in `ducklake_metadata`
under the **dotted-prefix convention**:

```
{application}.{instance}.{key}
```

Examples:

| Key | Value |
|-----|-------|
| `pg_tide.orders-to-lake.offset` | `"4782"` |
| `kafka-consumer.payments.last-seq` | `"19283746"` |
| `nats.events.sequence` | `"9001"` |

The key must contain **at least two dots** (three non-empty parts).  Plain
DuckDB system keys without dots (e.g. `data_path`) are always accepted.

Multiple applications coexist safely by using distinct prefixes.  Application
metadata rows participate in snapshot transactions, enabling exactly-once
semantics.

## Exactly-Once Delivery Guarantee

The two-phase commit pattern is:

1. **Write Parquet files to S3** (outside the catalog transaction).
2. **One catalog transaction**: register data files AND update the consumer
   offset key under `ducklake_metadata`.

```rust
let sink = RocklakeSink::new("pg_tide.orders-to-lake.offset")?;

// Phase 1: write Parquet files to S3 (handled by the ingest client)

// Phase 2: atomic catalog commit
sink.commit_batch(
    &mut store,
    &records,
    table_id,
    Some("4781"),   // expected current offset (fencing check)
    "4782",         // next offset
    Some("ingest-worker-1"),
).await?;
```

### What happens on crash

If the process dies **between phases 1 and 2**:
- The orphaned Parquet files are cleaned up by the orphan-file sweep after
  the grace period.
- The consumer re-reads from its last committed offset (not yet advanced).
- The consumer re-registers the same data files.  Because data-file
  registration is idempotent for a given Parquet path, the retry is safe.

### Idempotent retry

If `next_offset` is already stored in the catalog (i.e., the batch was
committed on a previous attempt), `commit_batch` returns immediately with
`records_committed = 0`.

### Fencing

If `expected_current_offset` is provided and does not match the stored value,
`commit_batch` returns `CatalogError::InvalidInput`.  This prevents a lagging
consumer from accidentally overwriting a more advanced offset.

## Kafka → Rocklake Example

```rust
use rocklake_catalog::{RocklakeSink, IngestRecord};

let sink = RocklakeSink::new("kafka.orders-topic.offset")?;

// In your Kafka consumer loop:
for batch in kafka_consumer.poll() {
    // Step 1: write Parquet to S3 (use Arrow/Parquet crate)
    let parquet_path = write_parquet_to_s3(&batch).await?;

    // Step 2: register in catalog (atomic with offset update)
    let file_id = writer.register_data_file(
        table_id, &parquet_path, "parquet",
        batch.len() as u64, parquet_size_bytes,
    ).await?;

    sink.commit_batch(
        &mut store,
        &[], // inlined records are optional when using data files
        table_id,
        Some(&current_offset.to_string()),
        &(current_offset + batch.len() as u64).to_string(),
        None,
    ).await?;

    current_offset += batch.len() as u64;
}
```

## NATS → Rocklake Example

```rust
let sink = RocklakeSink::new("nats.events-subject.seq")?;

let mut sub = nats_client.subscribe("events.>").await?;
while let Some(msg) = sub.next().await {
    let records = vec![IngestRecord {
        key: "id".to_string(),
        value: serde_json::from_slice(&msg.payload)?,
    }];

    sink.commit_batch(
        &mut store,
        &records,
        table_id,
        expected_seq.as_deref(),
        &msg.sequence.to_string(),
        None,
    ).await?;

    expected_seq = Some(msg.sequence.to_string());
}
```

## pg-tide-relay Integration

[pg-tide](https://github.com/trickle-labs/pg-tide) v0.34.0 registers
`RocklakeSink` as a valid reverse-pipeline sink.  This allows the following
patterns with no external database other than the SlateDB-backed catalog:

- **Kafka → Rocklake**
- **NATS → Rocklake**
- **Redis Streams → Rocklake**
- **SQS → Rocklake**

The pg-tide SQL corpus is bounded by the patterns validated in v0.6 and v1.0.
Key additional patterns:

```sql
-- pg-tide offset tracking
SELECT max(snapshot_id) FROM ducklake_snapshot WHERE snapshot_id > $1;

-- Store consumer offset
INSERT INTO ducklake_metadata (metadata_key, value, scope)
  VALUES ('pg_tide.orders-to-lake.offset', '4782', 'global')
  ON CONFLICT (metadata_key, scope) DO UPDATE SET value = EXCLUDED.value;

-- Retrieve consumer offset
SELECT value FROM ducklake_metadata
  WHERE metadata_key = $1 AND scope = 'global';
```

## Offset Recovery Procedure

If a consumer becomes confused about its offset:

1. Query the catalog for the stored offset:
   ```sql
   SELECT value FROM ducklake_metadata
     WHERE metadata_key = 'pg_tide.orders-to-lake.offset';
   ```
2. Re-position the Kafka consumer to the stored offset.
3. Resume ingest — the exactly-once check will skip any already-committed
   batches automatically.

## Failure Mode Handling

| Scenario | Outcome |
|----------|---------|
| Crash after Parquet write, before catalog commit | Orphaned file cleaned by GC; consumer retries from last committed offset |
| Crash during catalog transaction | Transaction rolls back; catalog unchanged |
| Wrong `expected_current_offset` | `CatalogError::InvalidInput` — fencing prevents double-commit |
| Duplicate `next_offset` | Idempotent no-op — `records_committed = 0` |
| Two concurrent writers | `CatalogError::WriterEpochMismatch` — single-writer guarantee |

## Performance

In release mode, `RocklakeSink::commit_batch` achieves:

- **Throughput**: ≥ 10,000 records/sec to S3 (catalog commit path only)
- **p95 commit latency**: ≤ 50ms

These targets are validated by the `ingest_throughput_meets_performance_target`
integration test, which runs in both debug and release modes with
environment-appropriate thresholds.
