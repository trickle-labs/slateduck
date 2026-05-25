# Incremental Materialized Views — Operations Guide

## Prerequisites

- SlateDuck **v0.12** or later
- `slateduck-ivm` binary available on PATH (built from the `slateduck-ivm` crate)
- Access to the SlateDB store (local path or object-store URI)

## Creating an Incremental Materialized View

```sql
CREATE INCREMENTAL MATERIALIZED VIEW <schema>.<name>
AS SELECT <group_by_cols>, <aggregates>
FROM <base_table>
GROUP BY <group_by_cols>;
```

**Supported aggregates**: `COUNT(*)`, `SUM(col)`, `MIN(col)`, `MAX(col)`.

Example:

```sql
CREATE INCREMENTAL MATERIALIZED VIEW analytics.region_counts
AS SELECT region, COUNT(*) AS cnt, SUM(revenue) AS total_revenue
FROM sales
GROUP BY region;
```

This will:
1. Register the matview in the catalog.
2. Create an empty output table `analytics.region_counts`.
3. Record the dependency on `sales`.

The matview will not contain data until an IVM worker processes it.

## Starting the IVM Worker

```bash
slateduck-ivm serve --store-path /path/to/store --worker-id worker-0
```

Options:

| Flag | Default | Description |
|---|---|---|
| `--store-path` | (required) | Path or URI to the SlateDB store |
| `--worker-id` | (required) | Unique identifier for this worker process |
| `--lease-duration-ms` | `30000` | Lease duration (milliseconds) |
| `--poll-interval-ms` | `500` | Polling interval between ticks |
| `--max-rows-per-tick` | `10000` | Maximum input rows processed per tick |
| `--shard-limit` | `0` (unlimited) | Maximum shards this worker will hold simultaneously |
| `--max-drain-time-ms` | `60000` | Time to drain on SIGTERM before forced exit |
| `--cost-mode` | `standard` | `standard` or `spot` — affects retry/backoff behaviour |

## Monitoring Matviews

### List all matviews

```sql
SHOW MATERIALIZED VIEWS;
```

Returns: `schema`, `name`, `status`, `shard_count`, `base_table`, `created_at_snapshot`.

### Inspect shard state

```sql
SHOW MATVIEW SHARDS analytics.region_counts;
```

Returns per-shard: `shard_id`, `owner_worker`, `lease_expires_unix_ms`, `generation`.

### Explain the IVM plan

```sql
EXPLAIN MATVIEW analytics.region_counts;
```

Returns the parsed `IvmPlan` as JSON: `view_sql`, `group_by_cols`, `aggregates`.

## Checking Freshness

Query the checkpoint history programmatically:

```rust
let reader = store.read_latest();
let history = reader.read_checkpoint_history(matview_id, shard_id)?;
if let Some(last) = history.last() {
    let lag = reader.matview_lag_ms(matview_id, shard_id, now_unix_ms)?;
    println!("Last checkpoint seq={}, lag={}ms", last.seq, lag.unwrap_or(0));
}
```

## Dropping a Matview

```sql
DROP INCREMENTAL MATERIALIZED VIEW analytics.region_counts;
```

Performs a logical delete (sets `end_snapshot`). The output table data remains accessible at snapshots prior to the drop.

To also remove the output table:

```sql
DROP INCREMENTAL MATERIALIZED VIEW analytics.region_counts;
DROP TABLE analytics.region_counts;
```

## Alerting on Stale Matviews

The IVM worker sets matview status to `Stale` when:
- It cannot acquire any shard lease (all workers busy).
- The base table has advanced by more than `freshness_target_ms` without a checkpoint.

To detect staleness:

```sql
SHOW MATERIALIZED VIEWS;
-- Filter for status = 'Stale'
```

Trigger an alert and restart the IVM worker if staleness persists beyond your SLA.

## Operational Notes

- **Worker crashes**: The CAS lease protocol ensures another worker will re-acquire the shard after `lease_duration_ms` elapses. No manual intervention is required for typical crashes.
- **Output table is read-only**: Direct `INSERT`/`UPDATE`/`DELETE` on the output table is rejected. Only the IVM worker may write to it.
- **Snapshot isolation**: Readers query the output table at a consistent snapshot and never see partial aggregation results.
- **Scaling**: Run multiple `slateduck-ivm serve` instances pointing at the same store. Each will claim disjoint shards. The single-writer constraint ensures no two workers write the same shard simultaneously.

## Sharding (v0.12+)

### Creating a Sharded Matview

Set `shard_count > 1` (or default to `1`) when creating the view:

```sql
CREATE INCREMENTAL MATERIALIZED VIEW analytics.region_counts
AS SELECT region, COUNT(*) AS cnt, SUM(revenue) AS total_revenue
FROM sales
GROUP BY region
WITH (shard_count = 8);
```

The IVM engine auto-detects `region` as the shard key (first GROUP BY column). Rows are assigned to shards by FNV-1a hash of the shard-key value, divided evenly over the u64 hash space. Each shard is independent; workers that hold different shards never contend.

### Choosing Shard Count

| Cardinality of shard key | Recommended `shard_count` |
|---|---|
| < 100 distinct values | 1–4 |
| 100 – 10 000 | 4–16 |
| > 10 000 | 16–64 |

Start conservatively. Re-sharding is supported but triggers a full rebuild.

### Limiting Shards Per Worker

Use `--shard-limit` to cap how many shards a single worker process will claim. Useful when running heterogeneous worker pools:

```bash
# Heavy workers: no limit
slateduck-ivm serve --store-path /store --worker-id heavy-0 --shard-limit 0

# Light workers (spot instances): at most 4 shards
slateduck-ivm serve --store-path /store --worker-id light-0 --shard-limit 4 --cost-mode spot
```

### Re-Sharding an Existing Matview

```sql
ALTER INCREMENTAL MATERIALIZED VIEW analytics.region_counts
SET (shard_count = 16);
```

This triggers a parallel rebuild: the view transitions to `Rebuilding` status, new shards process from snapshot 0, and the view returns to `Active` once all shards have caught up.  Old output rows are replaced atomically.

**Warning**: Re-sharding during high ingest may temporarily double the catalog write volume. Schedule during a low-traffic window if possible.

### Consistent vs Per-Shard Output Mode

- **`consistent`** (default): The global output snapshot only advances once _all_ shards have checkpointed. Readers always see a complete, consistent aggregate across all shards.
- **`per_shard`**: Each shard publishes independently. Higher freshness but readers may see partial results if they query between shard checkpoints.

```sql
CREATE INCREMENTAL MATERIALIZED VIEW analytics.region_counts
AS SELECT region, COUNT(*) AS cnt FROM sales GROUP BY region
WITH (shard_count = 8, output_mode = 'per_shard');
```

### Observability

```sql
-- Per-shard ownership, lease expiry, and lag
SHOW MATVIEW SHARDS analytics.region_counts;
```

| Column | Description |
|---|---|
| `shard_id` | Shard index (0 … shard_count−1) |
| `owner_worker` | Worker ID currently holding the lease |
| `lease_expires_unix_ms` | Lease expiry timestamp |
| `generation` | Heartbeat generation counter (monotonically increasing) |
| `last_input_snapshot` | Last input snapshot processed by this shard |
| `lag_ms` | Estimated lag behind current input snapshot |

Max lag across all shards:

```sql
SELECT MATVIEW_LAG('analytics.region_counts');
-- Returns NULL if no shard has checkpointed yet
```

### Graceful Shutdown

Send SIGTERM to the IVM worker process. It will:

1. Finish the current batch (bounded by `--max-drain-time-ms`, default 60 s).
2. Checkpoint all held shards.
3. Release all leases so peer workers can immediately claim them.
4. Exit 0.

For Kubernetes, set `terminationGracePeriodSeconds` to at least `max-drain-time-ms / 1000 + 10`.

### Rolling Updates (Zero-Downtime)

Use `maxSurge: 1, maxUnavailable: 0` in the Kubernetes Deployment spec. The new pod will acquire leases gradually as the old pod releases them during drain. No shard gap occurs as long as the lease TTL (default 30 s) is shorter than the total drain period.

## See Also

- [Concepts: Incremental Views](../concepts/incremental-views.md)
- [IVM Architecture](../architecture/ivm-plane.md)
- [SQL Reference: IVM DDL](../reference/sql-ivm.md)
