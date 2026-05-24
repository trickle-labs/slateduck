# Time Travel

Time travel in SlateDuck is not a feature that was layered on top of the system after the fact. It is the natural consequence of the storage model — the thing that happens automatically when you commit to never deleting historical data. If every catalog fact is preserved at its original snapshot ID, then reading the state of the catalog at any historical point is simply an MVCC query parameterized by that snapshot ID. There is no special "time travel mode," no separate historical storage tier, no additional replication or backup infrastructure, and no performance penalty. Querying the catalog as it was six months ago uses the same code path and the same SST files as querying the catalog as it is right now.

This page explains what snapshots are, how they are created, how time travel queries work in practice, how retention policies control how far back you can travel, and how time travel interacts with garbage collection and excision. By the end, you will understand not just how to use time travel but why it costs nothing beyond the storage that the immutability guarantee already requires.

## What is a Snapshot?

A snapshot is a point-in-time view of the entire catalog. Every mutation to the catalog — creating a schema, adding a table, registering a data file, altering a column, dropping a view — creates a new snapshot with a monotonically increasing integer ID. The snapshot ID serves as the version number for the entire catalog state: "the catalog at snapshot 42" is a well-defined, immutable, queryable state that encompasses everything about the catalog at the moment snapshot 42 was created.

Each snapshot is recorded as a row in the `ducklake_snapshot` table with metadata about when it was created, what changed, and optionally who created it:

```sql
-- DuckDB creates a snapshot at the end of every catalog-modifying transaction
INSERT INTO ducklake_snapshot (snapshot_id, snapshot_time, schema_version)
VALUES (42, '2024-06-15 14:30:22 UTC', 8);
```

Snapshots are append-only: once created, a snapshot row is never modified or deleted (until excision physically removes it). The snapshot counter increases by exactly 1 with each new snapshot, guaranteed by the single-writer model — there is no concurrent allocation race.

## How Time Travel Works in Practice

DuckDB supports time travel through the `SNAPSHOT` parameter in the `ATTACH` statement:

```sql
-- Attach at the current latest snapshot (the default)
ATTACH '' AS current_catalog (TYPE ducklake, PG 'host=127.0.0.1 port=5432');

-- Attach at a specific historical snapshot
ATTACH '' AS historical_catalog (TYPE ducklake, PG 'host=127.0.0.1 port=5432', SNAPSHOT '42');
```

When you attach at snapshot 42, every subsequent catalog query through that attachment sees the catalog exactly as it was at snapshot 42:

- Tables created after snapshot 42 are invisible
- Columns added after snapshot 42 are invisible
- Data files registered after snapshot 42 are invisible
- Tables renamed after snapshot 42 still show their old names
- Tables dropped after snapshot 42 are still visible (they have not yet been superseded)

This is not an approximation or a "best effort" — it is the precise, deterministic state of the catalog at that exact snapshot. The MVCC visibility filter (`begin_snapshot <= 42 AND (end_snapshot IS NULL OR 42 < end_snapshot)`) mathematically guarantees that you see exactly the rows that were visible at snapshot 42.

## A Concrete Time Travel Scenario

Let's walk through a realistic scenario to make time travel concrete. Imagine you have a product analytics lakehouse:

```sql
-- Snapshot 10: Create a table
CREATE TABLE analytics.events (event_id BIGINT, event_type VARCHAR, created_at TIMESTAMP);

-- Snapshot 11: Load January data (registers 5 Parquet files)
INSERT INTO analytics.events SELECT * FROM read_parquet('s3://data/january/*.parquet');

-- Snapshot 12: Add a column for a new feature
ALTER TABLE analytics.events ADD COLUMN region VARCHAR;

-- Snapshot 13: Load February data (with the new column)
INSERT INTO analytics.events SELECT * FROM read_parquet('s3://data/february/*.parquet');

-- Snapshot 14: Rename the table
ALTER TABLE analytics.events RENAME TO analytics.user_events;
```

Now you can travel to any point in this history:

```sql
-- What did the table look like before the column was added?
ATTACH '' AS v11 (TYPE ducklake, PG 'host=... port=5432', SNAPSHOT '11');
SELECT * FROM v11.analytics.events;
-- Returns January data with 3 columns (event_id, event_type, created_at)

-- What about after the column was added but before the rename?
ATTACH '' AS v13 (TYPE ducklake, PG 'host=... port=5432', SNAPSHOT '13');
SELECT * FROM v13.analytics.events;
-- Returns January + February data with 4 columns (including region)
-- Note: the table is still called "events" at this snapshot

-- What about at the current state?
ATTACH '' AS current (TYPE ducklake, PG 'host=... port=5432');
SELECT * FROM current.analytics.user_events;
-- Returns all data with 4 columns, table is called "user_events"
```

This is enormously valuable for reproducibility. If a quarterly report was generated using the catalog state at snapshot 13, you can re-run exactly the same query at snapshot 13 and get exactly the same results — even months later, even after the table has been renamed and restructured.

## Why Time Travel is Free

In systems where time travel is a separate feature (like PostgreSQL's point-in-time recovery or Snowflake's time travel), there is additional infrastructure cost: WAL archives must be stored and retained, periodic base backups must be taken, and restoring a historical point requires replaying logs. The storage cost of time travel is additive — it is paid on top of the cost of storing the current state.

In SlateDuck, time travel has zero marginal cost beyond what the immutability guarantee already requires. Here is why:

1. **No separate historical storage.** Old versions of catalog rows live in the same SST files as current versions. They are not copied to a separate archive or maintained in a separate log.

2. **No replay cost.** Querying at snapshot 42 does not require replaying 42 transactions from an initial state. The data for snapshot 42 is directly readable — it is just key-value entries in SST files that satisfy the MVCC filter.

3. **No base snapshot cost.** There is no periodic "full backup" needed for time travel efficiency. Every snapshot is directly queryable from the current SST files.

4. **Same code path.** The code that handles a query at the current snapshot is identical to the code that handles a query at snapshot 42. The only difference is the integer parameter passed to the MVCC filter.

The storage cost of time travel is the cost of keeping old versions of superseded rows — but this cost is already paid by the immutability guarantee (which keeps old versions regardless of whether anyone queries them). Time travel does not add any storage beyond what immutability already requires.

## Retention Policies and the Query Horizon

While infinite time travel is the default, operators can choose to limit how far back queries are allowed to travel. The `retain-from` system value defines the oldest snapshot that readers are permitted to query:

```bash
# Set the query horizon to 30 days ago
slateduck gc advance --retain-days 30 --catalog s3://bucket/catalog/
```

After this command, queries at snapshots older than 30 days return an error:

```
ERROR: Snapshot 15 is before the retention horizon (retain-from: 200).
Queries at snapshots older than the retention horizon are not permitted.
SQLSTATE: 22023 (invalid parameter value)
```

Important: advancing `retain-from` does not delete any data. It only changes the query-visibility boundary. The historical rows are still present in the SST files. If you later reset `retain-from` to an earlier value (and no excision has occurred), the old snapshots become queryable again.

This design gives operators full control over the trade-off between query range and storage cost:

- **Infinite retention (default):** `retain-from = 0`. All historical snapshots are queryable. Storage grows without bound.
- **Bounded retention:** `retain-from` advances periodically (e.g., every day, keeping 30 days of history). Snapshots older than the horizon are query-inaccessible but still physically present.
- **Bounded retention with excision:** After advancing `retain-from`, run `slateduck excise` to physically remove superseded rows whose `end_snapshot` is older than the horizon. This frees storage but is irreversible.

## Pinned Snapshots

Sometimes you need to prevent the retention horizon from advancing past a specific snapshot. For example, a long-running ETL job might need to read at a fixed snapshot throughout its multi-hour execution, or a compliance requirement might mandate preserving a specific catalog state indefinitely.

SlateDuck supports pinned snapshots for this use case:

```bash
slateduck pin-snapshot --catalog s3://bucket/catalog/ --snapshot-id 42 --reason "Quarterly audit"
```

A pinned snapshot prevents `gc advance` from moving `retain-from` past snapshot 42. The pin must be explicitly removed when no longer needed:

```bash
slateduck unpin-snapshot --catalog s3://bucket/catalog/ --snapshot-id 42
```

Pinned snapshots also block excision: `slateduck excise` will refuse to remove data that a pinned snapshot might need. This ensures that pinned snapshots remain fully queryable regardless of GC activity.

## Finding the Right Snapshot

To travel to a specific point in time, you need to know the snapshot ID. SlateDuck provides several ways to discover snapshot IDs:

```sql
-- List all snapshots with their timestamps
SELECT * FROM ducklake_snapshots();

-- Find snapshots from a specific date range
SELECT snapshot_id, snapshot_time
FROM ducklake_snapshots()
WHERE snapshot_time BETWEEN '2024-06-01' AND '2024-06-30'
ORDER BY snapshot_id;

-- Find the snapshot where a specific table was created
-- (Look for the first snapshot where the table appears)
```

The `slateduck inspect` CLI command also shows the current snapshot ID and recent snapshot history, which is useful for quick operational queries.

## Snapshot Growth Over Time

Snapshots accumulate at the rate of catalog mutations. A typical analytics workload might create:

- 1–5 snapshots per schema change (relatively rare)
- 1 snapshot per data load batch (every file registration)
- 100–1,000 snapshots per day for an active pipeline loading data every few minutes

Each snapshot record itself is small (approximately 100–200 bytes), so even 100,000 snapshots per year occupy less than 20 MB of catalog storage. The storage growth concern is not the snapshot records themselves but the versioned catalog rows (old column definitions, superseded table metadata, old file registrations) that accumulate across those snapshots.

For most workloads, this growth is negligible. Catalog metadata is orders of magnitude smaller than the actual Parquet data files it describes. A catalog tracking 100,000 Parquet files (terabytes of actual data) might occupy 50–100 MB of SlateDB storage including all historical versions. Object storage costs for that volume are measured in cents per month.

## Further Reading

- **[MVCC and Snapshot Isolation](mvcc.md)** — The mechanism that makes time travel queries work
- **[Catalog Immutability](immutability.md)** — The principle that makes time travel free
- **[Operations: GC & Retention](../operations/garbage-collection.md)** — Managing the retention horizon in production
- **[Operations: Excision](../operations/excision.md)** — Physical deletion after retention horizon advancement
- **[Getting Started: Your First Lakehouse](../getting-started/first-lakehouse.md)** — A hands-on tutorial including time travel queries
