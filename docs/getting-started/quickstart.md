# Quickstart — Local

This guide delivers a working Rocklake lakehouse in under five minutes on any machine with a Rust toolchain installed. No cloud credentials are required — you will use the local filesystem as the storage backend, which is functionally identical to object storage from Rocklake's perspective (the same code paths, the same durability guarantees within the scope of your local disk, the same catalog structure). By the end of this page, you will have created a catalog, defined a schema and table, inserted data, queried it through DuckDB, and demonstrated time travel by querying a historical snapshot.

Every command on this page is shown with its expected output. If your output differs, something has gone wrong — check the troubleshooting notes at the end of each step before continuing. The quickstart is designed to be followed exactly as written; deviations (different ports, different paths, different DuckDB versions) are fine but may produce slightly different output.

## Prerequisites

You need two tools installed before you begin:

**Rocklake.** Either download a pre-built binary from the [releases page](https://github.com/trickle-labs/rocklake/releases), or build from source:

```bash
git clone https://github.com/trickle-labs/rocklake.git
cd rocklake
cargo build --release
```

The binary will appear at `target/release/rocklake`. You can copy it to a directory on your PATH or reference it by its full path in the commands below.

**DuckDB 1.2 or later.** Download from [duckdb.org](https://duckdb.org). After installation, verify the version:

```bash
duckdb --version
```

You should see `v1.2.0` or later. Then install the `ducklake` extension (this only needs to happen once):

```bash
duckdb -c "INSTALL ducklake;"
```

## Step 1: Start the Rocklake Server

Open a terminal and start Rocklake, pointing it at a local directory where the catalog will be stored. The directory does not need to exist — Rocklake will create it and initialize a fresh catalog:

```bash
rocklake serve --catalog /tmp/my-lakehouse --bind 127.0.0.1:5432
```

You should see output like this:

```
INFO rocklake: Catalog opened successfully
INFO rocklake_pgwire::server: Rocklake serving on 127.0.0.1:5432
```

What just happened? Rocklake opened the specified path, determined that no existing catalog was present, and initialized a new one. The server is now listening for PostgreSQL wire protocol connections on port 5432. The catalog directory `/tmp/my-lakehouse` was created automatically and contains SlateDB's internal storage structure.

!!! tip "Port conflicts"
    If port 5432 is already in use (perhaps by an actual PostgreSQL installation), choose a different port: `--bind 127.0.0.1:5433`. Adjust the connection string in subsequent steps accordingly.

## Step 2: Connect DuckDB to the Catalog

Open a second terminal and launch DuckDB in interactive mode:

```bash
duckdb
```

Inside DuckDB, load the `ducklake` extension and attach the Rocklake catalog:

```sql
LOAD ducklake;
ATTACH 'host=127.0.0.1 port=5432' AS lakehouse (TYPE ducklake);
USE lakehouse;
```

You should see no errors. The connection string goes as the first argument to `ATTACH`. The `USE` command makes `lakehouse` the default catalog so you do not need to prefix table names.

Behind the scenes, DuckDB's `ducklake` extension just performed a handshake with Rocklake: it sent a startup message, authenticated (Rocklake accepts all connections by default in development mode), queried `pg_catalog.pg_type` and other system tables to learn the available types, and issued a `SELECT max(snapshot_id) FROM ducklake_snapshot` to discover the current catalog version. Rocklake responded to each query with the expected PostgreSQL wire messages, and DuckDB is now satisfied that it is talking to a valid DuckLake catalog backend.

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

What happened in the catalog? DuckDB's `ducklake` extension translated these DDL statements into a series of catalog mutations. For `CREATE SCHEMA`, it sent an `INSERT INTO ducklake_schema` with the schema name. For `CREATE TABLE`, it sent an `INSERT INTO ducklake_table` with the table name and schema reference, followed by an `INSERT INTO ducklake_column` for each of the five columns. Rocklake allocated unique IDs for each entity from its counter system, encoded the rows as Protobuf messages with SDKV headers, wrote them as key-value pairs to SlateDB, and committed each transaction atomically. The catalog now has snapshots for the initial empty state, the schema creation, and the table creation.

## Step 4: Insert Data

Insert a few rows of event data:

```sql
INSERT INTO analytics.events VALUES
    (1, 100, 'page_view', '2024-01-15 10:30:00', '{"page": "/home"}'),
    (2, 100, 'click', '2024-01-15 10:30:05', '{"button": "signup"}'),
    (3, 101, 'page_view', '2024-01-15 10:31:00', '{"page": "/pricing"}');
```

DuckDB will confirm the insert with a row count. The data is stored inline within the Rocklake catalog (no separate Parquet files are written for the local quickstart). Each insert creates a new catalog snapshot, so the data becomes visible to future queries at that snapshot ID. For production deployments with S3 or GCS as the object store, DuckDB would write the data as Parquet files to the specified `DATA_PATH` and register them with Rocklake.

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
┌────────────┬───────┐
│ event_type │  cnt  │
│  varchar   │ int64 │
├────────────┼───────┤
│ page_view  │     2 │
│ click      │     1 │
└────────────┴───────┘
```

When DuckDB executed this query, it asked Rocklake for the data at the current snapshot. Rocklake read the inline data from its catalog store and returned it. DuckDB then applied the `GROUP BY` and `ORDER BY` locally and returned the results.

## Step 6: Time Travel

Every catalog mutation creates a new snapshot. You can query any table at any historical snapshot version. First, see what snapshots exist:

```sql
SELECT snapshot_id, changes FROM ducklake_snapshots('lakehouse');
```

This returns a list of all snapshots with their IDs and what changed at each one. You will see entries like `{schemas_created=[analytics]}` and `{tables_created=[analytics.events]}`. The snapshot immediately after the table was created (but before any data was inserted) will show the empty table.

To query the table as it existed at an earlier snapshot — say, snapshot 2 (right after the table was created, before any inserts) — use DuckDB's `AT` clause:

```sql
SELECT * FROM analytics.events AT (VERSION => 2);
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

The table exists (it was created at snapshot 2) but contains no data (the data was inserted at later snapshots). This is time travel: you are seeing the catalog exactly as it was at snapshot 2, not the current state. Rocklake did not need to restore a backup, replay a log, or do anything special — it simply filtered the catalog rows by their `begin_snapshot` and `end_snapshot` bounds to show only what was visible at that version.

This works because Rocklake never overwrites or deletes catalog entries during normal operation. Every row has a `begin_snapshot` marking when it became visible. When a row is superseded (for example, by a schema change), it gets an `end_snapshot` marking when it stopped being visible. Querying at a specific snapshot is just an MVCC filter — two integer comparisons per row. There is no performance penalty, no additional storage cost, and no configuration required.

!!! note "Snapshot IDs"
    Use `SELECT snapshot_id, changes FROM ducklake_snapshots('lakehouse')` to see your catalog's actual snapshot IDs — they may differ from the examples above if you ran other commands between steps.

## Step 7: Inspect the Catalog Internals

Stop the Rocklake server (Ctrl+C in the first terminal), then inspect the internal state of the catalog using the Rocklake CLI:

```bash
rocklake inspect snapshot --latest --catalog /tmp/my-lakehouse
```

Expected output (approximately):

```
Catalog State:
  Latest snapshot ID: 5
  Schema version: 1
  Snapshot time: 2024-01-15T10:31:00+00:00
  Next snapshot ID: 6
  Next catalog ID: 8
  Next file ID: 2
  Schemas: 1
  Tables: 1
  Columns: 5
  Data files: 0
  Delete files: 0
  Retain-from: 0
  Writer epoch: 1748095000000
  Format version: 1
```

This gives you a quick view of the catalog's health: how many snapshots have been created, what entities exist, and whether GC has advanced the retention horizon.

!!! tip "Run inspect offline"
    Run `rocklake inspect` only when the server is **not** running. Opening the catalog while the server is active will fence the server out of its own storage, causing errors.

## What Lives on Disk

The local directory at `/tmp/my-lakehouse` now contains SlateDB's LSM-tree structure:

```
/tmp/my-lakehouse/
├── manifest/      # SlateDB manifest — points to the current set of SST files
├── wal/           # Write-ahead log segments (recent writes, not yet compacted)
└── compactions/   # Compaction metadata (tracks background compaction state)
```

The catalog metadata lives entirely within this SlateDB structure. In a cloud deployment, all of these would be objects in your S3/GCS/Azure bucket — same structure, same semantics, just served by object-storage APIs instead of filesystem calls. Data files (Parquet) would additionally be written to a separate `DATA_PATH` location in object storage.

Every catalog mutation was first written to the WAL (a single `PUT` to storage — fast, durable, sequential), then later compacted in the background into sorted SST files (which are optimized for range scans and binary search). The catalog state is fully reconstructable from the manifest and SST files alone; WAL segments are only needed for recent writes that have not yet been compacted.

## Cleanup

When you are done experimenting, stop the Rocklake server (Ctrl+C in the first terminal) and optionally remove the catalog directory:

```bash
rm -rf /tmp/my-lakehouse
```

## Troubleshooting

**"Connection refused" when attaching from DuckDB.** Rocklake is not running, or it is bound to a different port than the one in your connection string. Check that the `rocklake` process is still running in the first terminal and verify the port number.

**"relation ducklake_snapshot does not exist" after ATTACH.** You may be using an older version of the `ducklake` extension. Run `UPDATE EXTENSIONS;` in DuckDB and try again.

**"Address already in use" when starting Rocklake.** Port 5432 is occupied by another process (likely PostgreSQL). Use `--bind 127.0.0.1:5433` and update the connection string in DuckDB accordingly.

**DuckDB shows 0 rows after INSERT.** Make sure you are connected to `lakehouse` as your active catalog. Run `USE lakehouse;` and try the query again.

**"Unsupported option pg for DuckLake".** You are using the old ATTACH syntax. Use `ATTACH 'host=127.0.0.1 port=5432' AS lakehouse (TYPE ducklake)` — the connection string goes as the first argument, not as a `PG` option.

**"No function matches ducklake_snapshots()".** The `ducklake_snapshots` function requires a catalog name argument in DuckDB 1.2+. Use `ducklake_snapshots('lakehouse')`.

## Next Steps

You now have a working Rocklake catalog running locally. From here, you can:

- **[Quickstart — Cloud](quickstart-cloud.md)** — Run the same workflow against S3, GCS, or Azure for a production-realistic deployment
- **[Your First Lakehouse](first-lakehouse.md)** — A deeper tutorial covering schema evolution, multiple tables, and garbage collection
- **[Concepts](../concepts/index.md)** — Understand the principles behind what you just saw: immutability, MVCC, time travel, and reader scale-out
