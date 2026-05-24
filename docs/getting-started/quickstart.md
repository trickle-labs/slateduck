# Quickstart — Local

This guide delivers a working SlateDuck lakehouse in under five minutes on any machine with a Rust toolchain installed. No cloud credentials are required — you will use the local filesystem as the storage backend, which is functionally identical to object storage from SlateDuck's perspective (the same code paths, the same durability guarantees within the scope of your local disk, the same catalog structure). By the end of this page, you will have created a catalog, defined a schema and table, inserted data, queried it through DuckDB, and demonstrated time travel by querying a historical snapshot.

Every command on this page is shown with its expected output. If your output differs, something has gone wrong — check the troubleshooting notes at the end of each step before continuing. The quickstart is designed to be followed exactly as written; deviations (different ports, different paths, different DuckDB versions) are fine but may produce slightly different output.

## Prerequisites

You need two tools installed before you begin:

**SlateDuck.** Either download a pre-built binary from the [releases page](https://github.com/geir-gronmo/slateduck/releases), or build from source:

```bash
git clone https://github.com/geir-gronmo/slateduck.git
cd slateduck
cargo build --release
```

The binary will appear at `target/release/slateduck`. You can copy it to a directory on your PATH or reference it by its full path in the commands below.

**DuckDB 1.2 or later.** Download from [duckdb.org](https://duckdb.org). After installation, verify the version:

```bash
duckdb --version
```

You should see `v1.2.0` or later. Then install the `ducklake` extension (this only needs to happen once):

```bash
duckdb -c "INSTALL ducklake;"
```

## Step 1: Start the SlateDuck Server

Open a terminal and start SlateDuck, pointing it at a local directory where the catalog will be stored. The directory does not need to exist — SlateDuck will create it and initialize a fresh catalog:

```bash
slateduck serve --catalog file:///tmp/my-lakehouse --bind 127.0.0.1:5432
```

You should see output like this:

```
SlateDuck v0.8.0
Catalog: file:///tmp/my-lakehouse
Listening: 127.0.0.1:5432
Writer epoch: 1
```

What just happened? SlateDuck opened the specified path, determined that no existing catalog was present, and initialized a new one. "Writer epoch: 1" means this is the first writer to open this catalog — the epoch advances each time a new writer takes over (for example, after a restart or failover). The server is now listening for PostgreSQL wire protocol connections on port 5432.

!!! tip "Port conflicts"
    If port 5432 is already in use (perhaps by an actual PostgreSQL installation), choose a different port: `--bind 127.0.0.1:5433`. Adjust the connection string in subsequent steps accordingly.

## Step 2: Connect DuckDB to the Catalog

Open a second terminal and launch DuckDB in interactive mode:

```bash
duckdb
```

Inside DuckDB, load the `ducklake` extension and attach the SlateDuck catalog:

```sql
LOAD ducklake;
ATTACH '' AS lakehouse (TYPE ducklake, PG 'host=127.0.0.1 port=5432');
USE lakehouse;
```

You should see no errors. The `ATTACH` command establishes a PostgreSQL wire connection to SlateDuck. The `USE` command makes `lakehouse` the default catalog so you do not need to prefix table names.

Behind the scenes, DuckDB's `ducklake` extension just performed a handshake with SlateDuck: it sent a startup message, authenticated (SlateDuck accepts all connections by default in development mode), queried `pg_catalog.pg_type` and other system tables to learn the available types, and issued a `SELECT max(snapshot_id) FROM ducklake_snapshot` to discover the current catalog version. SlateDuck responded to each query with the expected PostgreSQL wire messages, and DuckDB is now satisfied that it is talking to a valid DuckLake catalog backend.

## Step 3: Create a Schema and Table

Now create a schema and a table within it:

```sql
CREATE SCHEMA analytics;
CREATE TABLE analytics.events (
    event_id BIGINT,
    user_id BIGINT,
    event_type VARCHAR,
    created_at TIMESTAMP,
    payload VARCHAR
);
```

Expected output:

```
OK
OK
```

What happened in the catalog? DuckDB's `ducklake` extension translated these DDL statements into a series of catalog mutations. For `CREATE SCHEMA`, it sent an `INSERT INTO ducklake_schema` with the schema name. For `CREATE TABLE`, it sent an `INSERT INTO ducklake_table` with the table name and schema reference, followed by an `INSERT INTO ducklake_column` for each of the five columns. SlateDuck allocated unique IDs for each entity from its counter system (snapshot counter advanced from 1 to 3, schema_id got 1, table_id got 1, column IDs got 1 through 5), encoded the rows as Protobuf messages with SDKV headers, wrote them as key-value pairs to SlateDB, and committed each transaction atomically. The catalog now has three snapshots: the initial empty state (snapshot 0), the schema creation (snapshot 1), and the table creation (snapshot 2).

## Step 4: Insert Data

Insert a few rows of event data:

```sql
INSERT INTO analytics.events VALUES
    (1, 100, 'page_view', '2024-01-15 10:30:00', '{"page": "/home"}'),
    (2, 100, 'click', '2024-01-15 10:30:05', '{"button": "signup"}'),
    (3, 101, 'page_view', '2024-01-15 10:31:00', '{"page": "/pricing"}');
```

Expected output:

```
3 rows affected
```

This step illustrates the two-plane separation beautifully. DuckDB wrote the actual data to a Parquet file in the storage location (under `/tmp/my-lakehouse/data/` for local storage). Then it told SlateDuck about the file by inserting a row into `ducklake_data_file` with the file's path, row count (3), byte size, and column-level min/max statistics. SlateDuck committed this file registration atomically with a new snapshot, so the data became visible to future queries at snapshot 3. At no point did SlateDuck read or write the Parquet file itself — it only knows the file exists and what statistics describe its contents.

## Step 5: Query the Data

Run an analytical query against the table:

```sql
SELECT event_type, COUNT(*) as cnt
FROM analytics.events
GROUP BY event_type
ORDER BY cnt DESC;
```

Expected output:

```
┌────────────┬─────┐
│ event_type │ cnt │
│  varchar   │ int │
├────────────┼─────┤
│ page_view  │   2 │
│ click      │   1 │
└────────────┴─────┘
```

When DuckDB executed this query, the first thing it did was ask SlateDuck for the list of data files belonging to `analytics.events` at the current snapshot. SlateDuck performed a prefix scan over the `ducklake_data_file` keys filtered to the table's ID and the current snapshot visibility bounds, and returned one row: the Parquet file registered in step 4. DuckDB then read that Parquet file directly from `/tmp/my-lakehouse/data/`, applied the `GROUP BY` and `ORDER BY` locally, and returned the results. The catalog lookup took a few milliseconds (local filesystem); the actual query execution was entirely within DuckDB's vectorized engine.

## Step 6: Time Travel

Every catalog mutation created a new snapshot. You can query the catalog at any historical state by specifying a snapshot ID. First, let's see what snapshots exist:

```sql
SELECT * FROM ducklake_snapshots();
```

This returns a list of all snapshots with their IDs and timestamps. Now let's attach a second connection to the catalog at an earlier snapshot — specifically, before the data was inserted:

```sql
ATTACH '' AS lakehouse_v2 (TYPE ducklake, PG 'host=127.0.0.1 port=5432', SNAPSHOT '2');
SELECT * FROM lakehouse_v2.analytics.events;
```

Expected output:

```
┌──────────┬─────────┬────────────┬────────────┬─────────┐
│ event_id │ user_id │ event_type │ created_at │ payload │
│  int64   │  int64  │  varchar   │ timestamp  │ varchar │
├──────────┴─────────┴────────────┴────────────┴─────────┤
│                        0 rows                           │
└─────────────────────────────────────────────────────────┘
```

The table exists (it was created at snapshot 2) but contains no data (the data file was registered at snapshot 3). This is time travel: you are seeing the catalog exactly as it was at snapshot 2, not the current state. SlateDuck did not need to restore a backup, replay a log, or do anything special — it simply filtered the catalog rows by their `begin_snapshot` and `end_snapshot` bounds to show only what was visible at snapshot 2.

This works because SlateDuck never overwrites or deletes catalog entries during normal operation. Every row has a `begin_snapshot` marking when it became visible. When a row is superseded (for example, by a schema change), it gets an `end_snapshot` marking when it stopped being visible. Querying at a specific snapshot is just an MVCC filter — two integer comparisons per row. There is no performance penalty, no additional storage cost, and no configuration required.

## Step 7: Inspect the Catalog Internals

You can inspect the internal state of the catalog using the SlateDuck CLI:

```bash
slateduck inspect --catalog file:///tmp/my-lakehouse
```

Expected output (approximately):

```
Catalog Format Version: 1
Current Snapshot: 3
Schema Version: 2
Writer Epoch: 1
Retain-From: 0 (infinite retention)
Objects: 1 schema, 1 table, 5 columns, 1 data file
Storage: 3 WAL segments, 1 SST file
```

This gives you a quick view of the catalog's health: how many snapshots have been created, what entities exist, whether GC has advanced the retention horizon, and what the underlying SlateDB storage looks like.

## What Lives on Disk

The local directory at `/tmp/my-lakehouse` now contains SlateDB's LSM-tree structure:

```
/tmp/my-lakehouse/
├── manifest/          # SlateDB manifest — points to the current set of SST files
├── wal/               # Write-ahead log segments (recent writes, not yet compacted)
├── compacted/         # Sorted String Tables (immutable, the catalog's durable state)
└── data/              # Parquet files written by DuckDB (the actual analytical data)
```

The catalog metadata lives in `manifest/`, `wal/`, and `compacted/`. The Parquet data files live in `data/`. In a cloud deployment, all of these would be objects in your S3/GCS/Azure bucket — same structure, same semantics, just served by object-storage APIs instead of filesystem calls.

Every catalog mutation was first written to the WAL (a single `PUT` to storage — fast, durable, sequential), then later compacted in the background into sorted SST files (which are optimized for range scans and binary search). The catalog state is fully reconstructable from the manifest and SST files alone; WAL segments are only needed for recent writes that have not yet been compacted.

## Cleanup

When you are done experimenting, stop the SlateDuck server (Ctrl+C in the first terminal) and optionally remove the catalog directory:

```bash
rm -rf /tmp/my-lakehouse
```

## Troubleshooting

**"Connection refused" when attaching from DuckDB.** SlateDuck is not running, or it is bound to a different port than the one in your connection string. Check that the `slateduck` process is still running in the first terminal and verify the port number.

**"relation ducklake_snapshot does not exist" after ATTACH.** You may be using an older version of the `ducklake` extension. Run `UPDATE EXTENSIONS;` in DuckDB and try again.

**"Address already in use" when starting SlateDuck.** Port 5432 is occupied by another process (likely PostgreSQL). Use `--bind 127.0.0.1:5433` and update the connection string in DuckDB accordingly.

**DuckDB shows 0 rows after INSERT.** Make sure you are querying the same catalog attachment (not `lakehouse_v2` which is pinned to an earlier snapshot). Run `USE lakehouse;` and try the query again.

## Next Steps

You now have a working SlateDuck catalog running locally. From here, you can:

- **[Quickstart — Cloud](quickstart-cloud.md)** — Run the same workflow against S3, GCS, or Azure for a production-realistic deployment
- **[Your First Lakehouse](first-lakehouse.md)** — A deeper tutorial covering schema evolution, multiple tables, and garbage collection
- **[Concepts](../concepts/index.md)** — Understand the principles behind what you just saw: immutability, MVCC, time travel, and reader scale-out
