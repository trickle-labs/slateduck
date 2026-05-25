# Incremental Materialized Views

SlateDuck v0.11 introduces **Incremental Materialized Views (IMV)**: persistent, automatically-maintained query results that are updated incrementally as new data arrives — without recomputing from scratch.

## What is an Incremental Materialized View?

A traditional materialized view is a snapshot of a query result stored on disk. Refreshing it requires re-executing the full query over all input data, which is expensive for large tables.

An **incremental** materialized view uses a *delta-processing* model: instead of recomputing from all data, only the rows that changed since the last refresh are processed. For GROUP BY aggregation queries, this means:

- `COUNT(*)` by group is tracked as a running counter.
- `SUM(col)` by group is tracked as a running sum.
- `MIN(col)` / `MAX(col)` by group is tracked via a sorted multiset.

When new rows arrive, only the affected groups are updated — not the entire result.

## Why Incremental?

SlateDuck is designed for OLAP workloads with high append throughput and many concurrent readers. Incremental maintenance provides:

| Property | Batch Refresh | Incremental |
|---|---|---|
| Cost per refresh | O(full table) | O(new rows only) |
| Freshness | Bounded by batch interval | Near-real-time |
| Reader impact | Refresh blocks readers | Readers unaffected |
| Scalability | Degrades with data growth | Stable cost |

## Supported Query Shapes (v0.11)

v0.11 supports a focused subset of SQL:

```sql
SELECT <group_by_cols>, <aggregates>
FROM <base_table>
GROUP BY <group_by_cols>
```

Supported aggregates: `COUNT(*)`, `SUM(col)`, `MIN(col)`, `MAX(col)`.

Planned for v0.12: `AVG`, multi-table joins, `HAVING`, window functions.

## DDL Surface

```sql
-- Create an incremental materialized view.
CREATE INCREMENTAL MATERIALIZED VIEW main.region_counts
AS SELECT region, COUNT(*) AS cnt FROM events GROUP BY region;

-- Drop it.
DROP INCREMENTAL MATERIALIZED VIEW main.region_counts;

-- Show all IMVs.
SHOW MATERIALIZED VIEWS;

-- Inspect shards.
SHOW MATVIEW SHARDS main.region_counts;

-- Explain the plan.
EXPLAIN MATVIEW main.region_counts;
```

## How It Works

1. **Create**: The DDL registers the matview in the catalog (`slateduck_matview`), creates an empty *output table*, and records the dependency on the base table.
2. **Worker claims shard**: An `slateduck-ivm` worker process discovers the matview, acquires a lease on one or more shards via CAS, and starts processing.
3. **Incremental loop**: On each tick, the worker reads new inlined-insert rows from the base table (those with `begin_snapshot > last_checkpoint`), pushes them through the DBSP-inspired Z-difference circuit, and stages the updated aggregation result as new inlined inserts in the output table.
4. **Checkpoint**: After each successful tick, a `MatviewCheckpointRow` is written with the current `seq`, `last_input_snapshot`, and `durable_at_unix_ms`.
5. **Read**: Readers query the output table at any snapshot. Time-travel before the first matview output returns an empty result (not an error).

## See Also

- [IVM Architecture](../architecture/ivm-plane.md)
- [IVM Operations Guide](../operations/incremental-materialized-views.md)
- [SQL Reference: IVM DDL](../reference/sql-ivm.md)
- [Design Decision: IVM on Immutable Substrate](../design-decisions/ivm-on-immutable-substrate.md)
