# Catalog Immutability

Immutability is the single most important design principle in Rocklake, and it is the one from which nearly every other distinctive property flows. The commitment is simple to state: once a catalog entry is committed at a given snapshot ID, that entry is readable at that snapshot ID forever, and can only be physically removed through the explicit, audited `rocklake excise` command. Normal operations — including garbage collection — never delete committed bytes. This is not a default that can be changed through configuration; it is the architectural premise from which everything else is derived.

This page explains what immutability means in concrete operational terms, why it was chosen as the foundational design principle rather than just a useful feature, what it enables (time travel, read scale-out, crash safety, auditability), what it costs (monotonically growing storage, explicit operator action for physical deletion), and how Rocklake manages those costs without violating the principle.

## What Immutability Means in Practice

When you create a table in Rocklake, a `TableRow` is written to SlateDB with a key that includes the table's ID and a `begin_snapshot` value recording the snapshot at which it became visible. If you later rename that table, Rocklake does not overwrite the original row. Instead, it performs two operations atomically: it sets an `end_snapshot` on the original row (marking it as superseded from that point forward), and it writes a new row with the updated name and a new `begin_snapshot`. Both the old and new versions coexist in storage, occupying distinct keys.

This pattern applies uniformly to all versioned entities in the catalog: schemas, tables, columns, views, macros, data file registrations, column statistics, and inlined data. The catalog is, conceptually, an append-only log of facts. Each fact is annotated with the time range (measured in snapshot IDs) during which it was true. Facts are never erased; they are only superseded by newer facts that begin at a later snapshot.

Consider a concrete example. You create a table called `events` at snapshot 5. At snapshot 10, you rename it to `user_events`. At snapshot 15, you add a column. The storage now contains three distinct key-value entries for the table metadata:

1. `[table_tag][schema_id][table_id][begin_snapshot=5]` → `{name: "events", ...}` with `end_snapshot=10`
2. `[table_tag][schema_id][table_id][begin_snapshot=10]` → `{name: "user_events", ...}` with `end_snapshot=NULL`

And for the column metadata:

3. The original columns at `begin_snapshot=5`, plus the new column at `begin_snapshot=15`

A query at snapshot 7 sees a table called `events` with the original columns. A query at snapshot 12 sees a table called `user_events` with the original columns. A query at snapshot 20 sees `user_events` with the additional column. All three states coexist in storage simultaneously. No state was overwritten or deleted to produce the later states.

## Why Immutability Matters

The benefits of immutability compound across multiple dimensions of the system. Each benefit is not merely nice to have — it is a load-bearing engineering consequence that enables specific system behaviors.

### Time Travel is Free

If every committed fact is preserved at its original snapshot ID, then reading the state of the catalog at any historical point is simply an MVCC query at that snapshot ID. There is no special "time travel mode," no separate historical storage tier, no additional replication or backup overhead. The entire history is always present in the same key-value store as the current state, queryable with the same code path, at the same performance characteristics.

This means that time travel is not a premium feature that Rocklake charges extra for or gates behind a configuration flag. It is the natural read mode. Querying the current state is just "time travel to the most recent snapshot." Querying last week's state is "time travel to a specific snapshot ID." The code is identical; only the target snapshot parameter differs.

Compare this to systems where time travel requires maintaining separate point-in-time snapshots (additional storage), replaying WAL logs from a base snapshot (additional compute), or querying a specialized historical data store (additional infrastructure). In Rocklake, time travel costs nothing beyond the storage that the immutability guarantee already requires.

### Crash Safety is Automatic

If existing data is never modified, then a crash during a write cannot corrupt existing data — because existing data was not being touched. A write either completes atomically (the new key-value entries are committed to SlateDB's WAL) or it does not complete at all (the WAL PUT never succeeded, so the new entries simply do not exist). There is no possibility of a "torn write" that leaves some rows updated and others in their original state, because updates never modify existing rows — they only append new rows and set `end_snapshot` values.

This means Rocklake does not need a repair mechanism for normal crash recovery. After a crash, the next writer simply opens the catalog and sees a consistent state: all previously committed transactions are fully present (their WAL segments were durably stored), and any in-flight transactions that had not yet committed are completely absent (their mutations were only in memory). There is no roll-forward or roll-back needed because the durable state is always consistent.

### Readers Never Block Writers, and Writers Never Block Readers

A reader operating at snapshot N sees a fixed, immutable set of rows: specifically, the rows whose `begin_snapshot <= N` and `end_snapshot IS NULL OR N < end_snapshot`. These rows are stored in immutable SST files in the object store. The reader is reading directly from these files, and nothing can change them — they are immutable once written.

Meanwhile, the writer may be actively creating snapshot N+1 by writing new rows and setting `end_snapshot` on superseded rows. But the writer's new rows have `begin_snapshot = N+1`, so they are invisible to the reader at snapshot N. And the `end_snapshot` values the writer sets are value changes in existing key-value entries, but the reader at snapshot N has already read those entries and is applying its own visibility filter — the reader's query is fully determined by the data it has already read, and subsequent writes cannot affect it.

This means there is no locking, no wait-for graph, no deadlock detection, no "reader blocks writer while holding a shared lock" scenario. Readers and writers are completely decoupled. This decoupling is what enables multiple independent reader processes to operate concurrently with the writer without any coordination protocol.

### Horizontal Read Scale-Out Requires No Coordination

This benefit deserves its own emphasis because it is perhaps the most operationally significant consequence of immutability. Because catalog rows, once committed at a given snapshot, occupy distinct keys that are never modified or deleted, any process that can read from the object store can serve catalog queries. Reader processes do not need to communicate with the writer to learn "what has been committed" — they open the SlateDB manifest (which points to the current set of SST files), read the immutable SST files, apply MVCC filtering, and return results.

Adding ten more reader processes requires no writer-side configuration change, no replication setup, no consensus protocol, no leader election. Each reader is independently opening the same immutable files from the same bucket. The object store handles concurrent reads natively (that is what object stores are designed for), and the readers have no shared mutable state that would require coordination.

This is fundamentally different from the read-replica model in PostgreSQL, where each replica must receive and apply a WAL stream from the primary. In that model, the primary's write throughput is bounded by how fast replicas can keep up, and adding replicas adds load to the primary. In Rocklake's model, readers impose zero load on the writer because they never communicate with it.

## The Costs of Immutability

Immutability is not free. It has real costs that operators must understand before deploying Rocklake in production.

### Storage Grows Monotonically

Every schema change, every data file registration, every column addition creates new rows that are never automatically reclaimed. For most workloads, this growth is negligible — catalog metadata is tiny compared to the actual data files (a catalog tracking 10,000 Parquet files might occupy a few megabytes of SlateDB storage, while the Parquet files themselves are terabytes). But catalogs with very high churn — thousands of schema changes per day, or applications that register and immediately unregister files at high rates — can accumulate significant historical data.

Over long timescales (years), even modest catalogs grow continuously. The underlying SST files in SlateDB accumulate, and while compaction merges them for read efficiency, it does not reduce the total data volume because no keys are deleted. Operators must understand that "infinite retention by default" means literally infinite — storage grows without bound unless they take explicit action.

### Physical Deletion Requires Explicit Operator Action

If you have regulatory requirements to physically delete data — GDPR right-to-erasure applied to metadata, for example — you cannot rely on automatic processes. Physical deletion requires two deliberate steps:

1. **Visibility GC** — Advance the `retain-from` horizon, which makes snapshots older than the horizon query-inaccessible. This is a logical operation that does not delete bytes; it only gates visibility.

2. **Excision** — Physically remove superseded rows whose `end_snapshot` is before the retention horizon. This is an irreversible operation that permanently destroys historical data and writes an audit entry recording what was deleted, when, by whom, and why.

Operators who are accustomed to PostgreSQL's `autovacuum` (which automatically reclaims space from dead tuples) or SQLite's automatic storage management will find Rocklake's model more explicit. Destructive operations are never automatic in Rocklake — this is by design, because automatic deletion of committed facts would violate the immutability contract, but it does require a different operational mindset.

### The Operator Must Actively Choose a Retention Policy

Rocklake ships with infinite retention as the default: `retain-from` starts at 0, meaning all historical snapshots are queryable. If you want bounded retention (for example, "keep 30 days of history and allow older snapshots to be garbage-collected"), you must configure this explicitly and schedule the GC process to run periodically. The system will not prompt you, will not warn you about storage growth, and will not automatically reclaim space. This is a conscious design choice — implicit deletion violates the immutability guarantee — but operators must be aware of it.

## How Rocklake Manages Growth

Rocklake provides three mechanisms for managing catalog growth without violating the immutability guarantee:

### Visibility GC (Logical Deletion)

The `rocklake gc advance` command moves the `retain-from` horizon forward. After advancement, queries at snapshot IDs older than the new horizon return a `snapshot-out-of-retention-window` error instead of results. However, the actual bytes are still in the object store — the GC only changes the visibility boundary, not the physical storage.

This is safe and reversible: if you advance `retain-from` from snapshot 100 to snapshot 500, and then realize you need to query at snapshot 200, you can reset the horizon back to 200 (as long as excision has not been run) and the data is still there.

### Excision (Physical Deletion)

The `rocklake excise` command physically removes superseded rows whose `end_snapshot` is older than the current `retain-from` horizon. This is irreversible — once bytes are removed from the object store, they cannot be recovered. Excision requires explicit confirmation (`--apply` flag), writes an audit trail entry recording what was deleted, and refuses to proceed if safety checks fail (for example, if a reader is still pinned to a snapshot within the excision range).

Excision is designed to be rare. Most deployments never need it. The primary use cases are compliance (GDPR erasure), cost management for very long-running catalogs, and recovery from data poisoning incidents. It is not part of normal operations.

### SlateDB Compaction (Storage-Level Optimization)

SlateDB's background compaction process merges SST files, eliminates duplicate keys across levels, and produces larger, more efficient SST files. Compaction does not delete catalog data (because no catalog keys are deleted), but it does reduce the number of files a reader must consult, improving read performance. Compaction is automatic and transparent — Rocklake does not need to trigger or monitor it in normal operation.

## Immutability and the Consistency Model

The combination of immutability and single-writer semantics gives Rocklake a very strong consistency model. Within the writer's session, every read observes a consistent snapshot — there is no possibility of seeing partial transactions or interleaved writes from other sessions. Across concurrent readers, every reader observes a consistent snapshot (potentially different snapshots if the writer has advanced the state between their reads, but each reader's snapshot is internally consistent).

There is never a case where a reader sees a "torn" state — half of a table creation visible but not the columns, or a file registered but its containing table not yet created. Because all mutations within a transaction are committed atomically (a single SlateDB `DbTransaction`), a reader either sees all of them or none of them.

This consistency model is analogous to PostgreSQL's MVCC (which also uses begin/end transaction IDs to control row visibility) but simpler in several ways: there is only one writer (eliminating the need for vacuum to handle XID wraparound), there is no shared buffer pool (eliminating the need for buffer-level locking), and there is no undo log (because rows are never modified in place, there is nothing to undo).

## The Immutability Contract

To summarize, Rocklake makes the following binding commitment to its operators:

1. **Committed catalog facts are never physically deleted by normal operation.** Normal operation includes all `rocklake serve` activity, all DuckDB client operations, and all `rocklake gc advance` operations.

2. **Physical deletion only occurs through explicit `rocklake excise` commands.** Excision requires operator authentication, a stated reason, safety-check validation, and produces a permanent audit entry.

3. **The audit trail of excision events is itself immutable.** You can always see what was deleted, when, by whom, and for what reason — even after the actual data is gone.

This contract is the foundation of trust for operators who rely on Rocklake for compliance, auditability, or long-term historical access. It is not a feature that can be disabled or a default that can be overridden. It is the architecture.

## Further Reading

- **[MVCC and Snapshot Isolation](mvcc.md)** — How immutable rows are filtered by snapshot visibility
- **[Time Travel](snapshots.md)** — The practical consequence of immutability: querying any historical state
- **[Operations: GC & Retention](../operations/garbage-collection.md)** — How to manage catalog growth in production
- **[Operations: Excision](../operations/excision.md)** — The physical deletion process
- **[Design Decisions: Immutability Trade-offs](../design-decisions/immutability-tradeoffs.md)** — The full cost-benefit analysis
