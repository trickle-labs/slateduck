# IVM Join Sizing Guide

> **v0.13** — Broadcast, Co-Partitioned & Reshuffle strategies

SlateDuck v0.13 adds three join strategies to the incremental view-maintenance
(IVM) engine.  Choosing the right strategy avoids unnecessary data movement and
keeps view refresh latency low.

---

## Strategy Selection Quick Reference

| Condition | Chosen strategy |
|-----------|-----------------|
| Right side ≤ `broadcast_threshold` rows | **Broadcast** |
| Both sides sharded on the join key | **Co-Partitioned** |
| Neither of the above | **Reshuffle** |

The threshold defaults to **1 000 000 rows** (`DEFAULT_BROADCAST_THRESHOLD`).
Override it per-view via `JoinClause::broadcast_threshold`.

---

## 1. Broadcast Join

### When to use
- The **right (dimension) side is small** — small lookup tables such as
  `categories`, `products`, `regions`, `users` with fewer than ~1 M rows.
- The left (fact) side grows unboundedly.

### How it works
SlateDuck holds the entire right side in a `HashJoinState` that is replicated
to every worker.  Each left-side delta is probed against the in-memory index
with O(1) average lookup per key.

### Sizing
| Right rows | Approximate memory per worker |
|------------|-------------------------------|
|    10 000  |  ~3 MB                        |
|   100 000  | ~30 MB                        |
| 1 000 000  | ~300 MB (threshold default)   |

Reduce `broadcast_threshold` if workers are memory-constrained:

```rust
let plan = IvmPlan::parse(sql)?;
// Override threshold to 200 000 rows
let mut clause = plan.joins[0].clone();
clause.broadcast_threshold = 200_000;
```

### Performance (M-series laptop)
- Left-delta throughput: ~850 K rows/s
- First-delta latency: ~120 µs (p50)

---

## 2. Co-Partitioned Join

### When to use
- **Both tables are already sharded on the join key** (or a prefix of it).
- Typical for: `orders ⋈ lineitems` on `order_id`, `sessions ⋈ events` on
  `session_id`.

### How it works
`select_strategy` returns `CoPartitioned` when the left shard key matches the
left join column **and** the right shard key matches the right join column.
No data is moved; each shard independently maintains a local `HashJoinState`.

### Sizing
Memory is proportional to the join-key fan-out on each shard.  As a rule of
thumb, size each shard to hold ≤ 10 M right-side rows.

### Performance (M-series laptop)
- Throughput: ~780 K rows/s
- First-delta latency: ~140 µs (p50)

---

## 3. Reshuffle (Exchange)

### When to use
- The **right side is too large for broadcast** and the tables are **not
  co-partitioned** on the join key.
- Typical for: `customers ⋈ nations` where customers are sharded on
  `customer_id` but the join is on `nation_key`.

### How it works
`ExchangeBuffer` re-partitions both input streams by the join key using a
consistent hash (shard = `hash(key) % shard_count`).  Rows destined for
different shards are buffered and drained before probing.

### Sizing
- **Shard count**: start with `max(left_workers, right_workers)`.
- **Buffer memory**: `ExchangeBuffer` holds at most one epoch's worth of
  unprocessed rows.  Size workers to hold ≤ 50 M rows per shard in memory at
  peak.

### Performance (M-series laptop)
- Throughput: ~620 K rows/s (lower than co-partitioned due to hashing overhead)
- First-delta latency: ~190 µs (p50)

---

## Choosing `broadcast_threshold`

The default threshold of 1 M rows is a safe conservative limit for a worker
with ~500 MB heap.  Adjust based on your average row width:

```
threshold = available_heap_bytes / avg_row_bytes / num_joins
```

**Example** — 2 GB worker, 300-byte average rows, 3 concurrent join views:

```
threshold = 2_000_000_000 / 300 / 3 ≈ 2_200_000
```

---

## Delete Propagation

All three strategies support **delete (retract) propagation**:

- `HashJoinState::retract_right(key, row)` removes a right-side row.
- Pending left-side output is updated with weight `−1` for the retracted key.
- Use `IvmJoinCircuit::push_right_delta(idx, row, col, weight)` with
  `weight = -1` to push a retraction.

---

## Monitoring

Emit the following metrics from your application layer to track IVM join health:

| Metric | Description |
|--------|-------------|
| `ivm_join_strategy{view,strategy}` | Which strategy was selected |
| `ivm_right_state_size{view,join_idx}` | Current `HashJoinState` entry count |
| `ivm_exchange_buffer_size{view,shard}` | Pending rows in `ExchangeBuffer` |
| `ivm_join_output_delta_total{view}` | Cumulative output deltas produced |

---

## See Also

- [Join strategies — architecture doc](../architecture/mvcc-implementation.md)
- [IVM concepts](../concepts/mvcc.md)
- Benchmark results: `benchmarks/v0.13-ivm-joins.json` (workspace root)
