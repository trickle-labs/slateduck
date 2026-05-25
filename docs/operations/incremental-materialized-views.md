# Incremental Materialized Views — Operations Guide

## Prerequisites

- SlateDuck v0.11 or later
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

## See Also

- [Concepts: Incremental Views](../concepts/incremental-views.md)
- [IVM Architecture](../architecture/ivm-plane.md)
- [SQL Reference: IVM DDL](../reference/sql-ivm.md)
