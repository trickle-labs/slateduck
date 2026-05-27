# DuckLake CDC Contract Reference

> **Note**: This document was originally titled `pg-trickle-ducklake-support.md`
> and described integration work with the pg-trickle project. pg-trickle has
> since dropped its DuckLake support. The underlying catalog capabilities
> described here — `table_changes()`, stable `rowid`, snapshot leases, `NOTIFY`,
> and `ducklake_latest_snapshot_id()` — are valid DuckLake v1.0 spec conformance
> items regardless of the downstream consumer. This file is retained as a generic
> **DuckLake CDC contract** reference for any system that needs to consume
> change-data-capture events from a RockLake catalog.

## Overview

RockLake exposes a DuckLake-compatible CDC interface via the PG-Wire facade.
Any client that speaks the PostgreSQL wire protocol can subscribe to catalog
change events by following the contract described in this document.

## Core CDC Primitives

### `table_changes(table_name, from_snapshot, to_snapshot)`

Returns a streaming diff of data-file additions and deletions between two
catalog snapshots.  The function is the primary entry point for CDC consumers.

```sql
-- Enumerate all file-level changes since snapshot 10.
SELECT * FROM table_changes('analytics.events', 10, (
    SELECT ducklake_latest_snapshot_id('analytics.events')
));
```

### `ducklake_latest_snapshot_id(table_regclass)`

Returns the current snapshot ID for a given table.  CDC consumers should poll
this function to detect new snapshots without issuing a full catalog scan.

```sql
SELECT ducklake_latest_snapshot_id('analytics.events');
```

### `LISTEN` / `NOTIFY`

RockLake emits a `NOTIFY` event on the `ducklake_snapshot` channel whenever a
new snapshot is committed.  Consumers can use this instead of polling.

```sql
LISTEN ducklake_snapshot;
-- The server will push a notification on every commit.
```

### Snapshot Leases

A consumer can acquire a snapshot lease to prevent GC from reclaiming snapshots
that it has not yet processed:

```sql
-- Acquire a lease on snapshot 42.
SELECT ducklake_hold_snapshot(42);

-- ... process changes ...

-- Release when done.
SELECT ducklake_release_snapshot(42);
```

## MVCC Visibility Contract

Data files visible to a CDC consumer at snapshot `S` must satisfy:

```
begin_snapshot <= S
AND (end_snapshot IS NULL OR end_snapshot > S)
```

This is the canonical DuckLake v1.0 visibility predicate. RockLake enforces it
in `list_data_files` and `list_delete_files` via the secondary index scan and
post-filter in `crates/rocklake-catalog/src/reader.rs`.

## File Order

`list_data_files` returns files sorted **ascending by `file_order`**. CDC
consumers must process files in this order to preserve the insertion-time
ordering guarantee required by DuckLake-compatible query engines.

## Drivers Known to Work

The following Postgres client libraries have been verified to connect and issue
the CDC queries above against a RockLake PG-Wire server:

| Driver           | Language | Startup Parameters Verified |
|------------------|----------|-----------------------------|
| `tokio-postgres`  | Rust     | Yes — R-01, R-02, R-03      |
| `pg` v8           | Node.js  | Yes — D-01                  |
| `psycopg` 3       | Python   | Yes — D-02                  |
| `pgx` v5          | Go       | Yes — D-03                  |
| `psql` CLI        | Shell    | Yes — C-01                  |
| `pgcli` CLI       | Python   | Yes — C-02                  |

Verification tests are in `crates/rocklake-pgwire/tests/driver_compat.rs`.

## History

- v0.18: Initial CDC implementation — `table_changes()`, stable `rowid`,
  snapshot leases, `NOTIFY`, extension schema.
- v0.27.11: `ducklake_latest_snapshot_id()` added for CDC startup bootstrap.
- v0.27.13: Multi-driver interoperability certification; this document
  reframed as a generic DuckLake CDC contract reference.
